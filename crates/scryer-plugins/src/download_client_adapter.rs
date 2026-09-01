use std::{
    collections::HashMap,
    fs::File,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadClientFeedbackScope,
    DownloadClientMarkImportedRequest, DownloadClientStatus, DownloadGrabResult,
    DownloadSourceKind, ResolvedDownloadArtifact, StagedNzbRef,
};
use scryer_domain::{
    CompletedDownload, DownloadQueueItem, DownloadQueueState, DownloadSeedingSnapshot,
};
use scryer_plugin_sdk::command::{
    PluginCommand, PluginCommandRequest, PluginCommandResult, PluginDownloadClientCommand,
    PluginDownloadClientCommandResult, PluginDownloadGetCompletedRequest,
};
use scryer_plugin_sdk::torrent::normalize_info_hash_pair;
use scryer_plugin_sdk::{
    PluginDownloadFeedbackScope as PluginFeedbackScope, PluginDownloadScopedListRequest,
    PluginDownloadScopedListResponse, PluginDownloadScopedRecentCompletedRequest, PluginError,
    PluginErrorCode, PluginResult,
};
use tracing::{debug, info, warn};

use crate::runtime_backing::PluginInstanceSpec;
use crate::seeding_trust::apply_seeding_trust_floor;
use crate::types::{
    DownloadControlAction, DownloadInputKind, DownloadIsolationMode, DownloadItemState,
    PluginCompletedDownload, PluginDescriptor, PluginDownloadClientAddRequest,
    PluginDownloadClientAddResponse, PluginDownloadClientControlRequest,
    PluginDownloadClientMarkImportedRequest, PluginDownloadIsolation, PluginDownloadItem,
    PluginDownloadListRecentCompletedRequest, PluginDownloadRelease, PluginDownloadRouting,
    PluginDownloadSource, PluginDownloadTitle, PluginTorrentOptions, PluginTorrentQueuePlacement,
};
use crate::wasmtime_host::command_host::CommandHost;
use crate::wasmtime_host::{DownloadClientComponentInvocation, process_download_client_component};

// Keep plugin work below the outer download-feedback gate while leaving enough
// room for large client responses on slower hosts.
pub(crate) const DOWNLOAD_CLIENT_PLUGIN_TIMEOUT: std::time::Duration =
    scryer_outbound_http::DOWNLOAD_CLIENT_PLUGIN_TIMEOUT;
/// Which runtime serves this client's operations.
///
/// The legacy reactor is instantiated once and re-entered under a mutex. The
/// other two arms carry *identical* state — the artifact bytes and the
/// client-scoped [`CommandHost`] — because the wasip1 command ABI and the
/// `scryer:download-client/download-client@1.0.0` component world differ only
/// in transport: the same `PluginCommandRequest` envelope, the same host
/// services, the same absence of filesystem authority. Everything a client
/// observes across invocations, the [`CommandHost`] state map included, is
/// therefore the same object on both.
pub struct WasmDownloadClient {
    runtime: Arc<ComponentDownloadClient>,
    descriptor: PluginDescriptor,
    client_name: String,
    client_id: String,
    /// Last torrent seeding observation seen for each client item, from the
    /// most recent successful queue listing.
    ///
    /// A torrent client reports every torrent it holds from `list_queue`,
    /// including the finished ones, and the finished ones are exactly the
    /// rows the seeding gate has to reason about. `retain_queue_item` drops
    /// them from the *queue* (they belong to history, otherwise the live queue
    /// would carry every torrent the client has ever kept), and the history
    /// listing is framed as `PluginCompletedDownload`, which has no seeding
    /// fields at all. Recording the observation on the way past is what keeps
    /// it reachable without a second plugin call per poll.
    ///
    /// The map is *replaced* on every successful listing rather than merged,
    /// so an entry can never outlive the poll that produced it: a torrent that
    /// left the client loses its observation, and the gate then holds instead
    /// of acting on a stale one.
    seeding_observations: SeedingObservationCache,
}

/// See `WasmDownloadClient::seeding_observations`.
#[derive(Default)]
struct SeedingObservationCache(Mutex<HashMap<String, DownloadSeedingSnapshot>>);

impl SeedingObservationCache {
    /// Replace the cache from a full plugin queue listing, before the
    /// completed/seeding rows are filtered out of the queue itself.
    fn record(&self, items: &[PluginDownloadItem]) {
        let mut observations = HashMap::new();
        for item in items {
            let Some(snapshot) = seeding_snapshot_for_item(item) else {
                continue;
            };
            for key in seeding_observation_keys(item) {
                observations.insert(key, snapshot.clone());
            }
        }
        if let Ok(mut cache) = self.0.lock() {
            *cache = observations;
        }
    }

    /// Stamp the last observation onto rows that arrived without one — history
    /// rows framed as completed downloads, which carry no seeding fields.
    fn apply(&self, items: &mut [DownloadQueueItem]) {
        let Ok(cache) = self.0.lock() else {
            return;
        };
        if cache.is_empty() {
            return;
        }
        for item in items.iter_mut() {
            if item.seeding.is_some() {
                continue;
            }
            let candidates = [
                normalized_plugin_info_hash(Some(item.download_client_item_id.as_str())),
                Some(item.download_client_item_id.clone()),
                item.download_id.clone(),
            ];
            item.seeding = candidates
                .into_iter()
                .flatten()
                .find_map(|key| cache.get(&key).cloned());
        }
    }
}

/// State for one `scryer:download-client/download-client@1.0.0` client.
struct ComponentDownloadClient {
    wasm: Arc<Vec<u8>>,
    command_host: CommandHost,
    invocation_lock: tokio::sync::Mutex<()>,
}

impl ComponentDownloadClient {
    fn new(wasm: Vec<u8>, command_host: CommandHost) -> Arc<Self> {
        Arc::new(Self {
            wasm: Arc::new(wasm),
            command_host,
            invocation_lock: tokio::sync::Mutex::new(()),
        })
    }
}

impl WasmDownloadClient {
    /// A `scryer:download-client/download-client@1.0.0` component client.
    pub fn new_component(
        wasm: Vec<u8>,
        descriptor: PluginDescriptor,
        client_id: String,
        client_name: String,
        command_host: CommandHost,
    ) -> Self {
        Self {
            runtime: ComponentDownloadClient::new(wasm, command_host),
            descriptor,
            client_name,
            client_id,
            seeding_observations: SeedingObservationCache::default(),
        }
    }

    fn supports_category_scoped_feedback(&self) -> bool {
        self.descriptor
            .download_client()
            .is_some_and(|provider| provider.capabilities.category_scoped_feedback)
    }

    fn plugin_feedback_scope(scope: &DownloadClientFeedbackScope) -> PluginFeedbackScope {
        PluginFeedbackScope {
            categories: scope.categories.clone(),
        }
    }

    async fn invoke_command(
        &self,
        command: PluginDownloadClientCommand,
        operation: &'static str,
    ) -> AppResult<PluginDownloadClientCommandResult> {
        let client = &self.runtime;
        // One invocation at a time: the component is instance-per-request, but
        // the client's `CommandHost` state map — where it keeps the session
        // cookie it re-uses between calls — is not.
        let _guard = client.invocation_lock.lock().await;
        let spec = PluginInstanceSpec {
            wasm: Arc::clone(&client.wasm),
            // This family has never been granted filesystem authority on any
            // runtime; see the world docs.
            preopens: Vec::new(),
            timeout: DOWNLOAD_CLIENT_PLUGIN_TIMEOUT,
            memory_max_bytes: None,
            command_host: client.command_host.clone(),
        };
        let request = PluginCommandRequest::new(PluginCommand::DownloadClient(command));
        let response = process_download_client_component(
            &spec,
            &request,
            DownloadClientComponentInvocation {
                plugin_id: &self.descriptor.id,
                plugin_version: &self.descriptor.version,
                operation,
            },
        )
        .await?;
        match response.response {
            PluginCommandResult::DownloadClient(result) => Ok(result),
            _ => Err(AppError::Repository(format!(
                "command plugin {} returned a response for another plugin family",
                self.descriptor.id
            ))),
        }
    }
}

fn decode_command_result<T>(result: PluginResult<T>, context: &str) -> AppResult<T> {
    match result {
        PluginResult::Ok(value) => Ok(value),
        PluginResult::Err(error) => Err(plugin_error_as_repository(error, context)),
    }
}

fn decode_scoped_command_result<T>(
    result: PluginResult<PluginDownloadScopedListResponse<T>>,
    context: &str,
    client_id: &str,
    provider_type: &str,
) -> AppResult<Vec<T>> {
    let response = decode_command_result(result, context)?;
    if response.failures.is_empty() {
        return Ok(response.items);
    }
    let failed_categories = response
        .failures
        .iter()
        .map(|failure| failure.category.clone())
        .collect::<Vec<_>>();
    for failure in response.failures {
        warn!(
            client_id,
            provider_type,
            category = %failure.category,
            error_code = ?failure.error.code,
            error = %failure.error.public_message,
            "download-client category feedback read returned a partial snapshot"
        );
    }
    Err(AppError::Repository(format!(
        "{context}: category feedback read was partial for {}",
        failed_categories.join(", ")
    )))
}

fn feedback_scope_is_empty(scope: &DownloadClientFeedbackScope) -> bool {
    scope.categories.is_empty()
}

fn decode_download_add_result<T>(result: PluginResult<T>, context: &str) -> AppResult<T> {
    match result {
        PluginResult::Ok(value) => Ok(value),
        PluginResult::Err(error) => Err(map_download_add_plugin_error(error, context)),
    }
}

fn plugin_error_message(error: &PluginError, context: &str) -> String {
    format!(
        "{context}: plugin error {:?}: {}",
        error.code, error.public_message
    )
}

fn plugin_error_as_repository(error: PluginError, context: &str) -> AppError {
    AppError::Repository(plugin_error_message(&error, context))
}

fn map_download_add_plugin_error(error: PluginError, context: &str) -> AppError {
    let message = plugin_error_message(&error, context);
    match error.code {
        PluginErrorCode::RateLimited
        | PluginErrorCode::UpstreamUnavailable
        | PluginErrorCode::Temporary => AppError::DownloadSubmitUnavailable(message),
        PluginErrorCode::InvalidConfig
        | PluginErrorCode::AuthFailed
        | PluginErrorCode::Unsupported
        | PluginErrorCode::Permanent => AppError::DownloadSubmitRejected(message),
    }
}

fn parse_timestamp(raw: Option<String>) -> Option<DateTime<Utc>> {
    let value = raw?.trim().to_string();
    chrono::DateTime::parse_from_rfc3339(&value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
        .or_else(|| {
            value
                .parse::<i64>()
                .ok()
                .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0))
        })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolvedTorrentSource {
    download_url: Option<String>,
    magnet_uri: Option<String>,
    torrent_bytes_base64: Option<String>,
    torrent_url: Option<String>,
    torrent_file_name: Option<String>,
    torrent_content_type: Option<String>,
    nzb_bytes_base64: Option<String>,
    nzb_file_name: Option<String>,
    nzb_content_type: Option<String>,
}

fn map_source_kind(kind: DownloadSourceKind) -> DownloadInputKind {
    match kind {
        DownloadSourceKind::NzbFile => DownloadInputKind::Nzb,
        DownloadSourceKind::NzbUrl => DownloadInputKind::NzbUrl,
        DownloadSourceKind::TorrentFile => DownloadInputKind::TorrentFile,
        DownloadSourceKind::MagnetUri => DownloadInputKind::MagnetUri,
    }
}

fn map_state(state: DownloadItemState) -> DownloadQueueState {
    match state {
        DownloadItemState::Queued => DownloadQueueState::Queued,
        DownloadItemState::Downloading => DownloadQueueState::Downloading,
        DownloadItemState::Verifying => DownloadQueueState::Verifying,
        DownloadItemState::Repairing => DownloadQueueState::Repairing,
        DownloadItemState::Extracting => DownloadQueueState::Extracting,
        DownloadItemState::Paused => DownloadQueueState::Paused,
        DownloadItemState::Completed | DownloadItemState::Seeding => DownloadQueueState::Completed,
        DownloadItemState::ImportPending => DownloadQueueState::ImportPending,
        // A plugin reports `Warning` for a condition the operator can still
        // fix, and says so explicitly to keep failed-download handling off the
        // download. Flattening it into `Failed` here would blocklist and remove
        // a grab that is only stuck.
        DownloadItemState::Warning => DownloadQueueState::Warning,
        DownloadItemState::Failed | DownloadItemState::Error => DownloadQueueState::Failed,
    }
}

fn attention_required(item: &PluginDownloadItem) -> bool {
    matches!(
        item.state,
        DownloadItemState::Failed | DownloadItemState::Error | DownloadItemState::Warning
    )
}

fn normalized_plugin_info_hash(raw: Option<&str>) -> Option<String> {
    scryer_application::normalize_torrent_info_hash(raw)
}

/// Carry the plugin's seeding observation through to the domain queue item.
///
/// Every field is copied verbatim, tri-state and all: `None` means the client
/// cannot answer and must never be flattened into `false` (which asserts a
/// limit nobody can see) or `true` (which invites a hit and run on a private
/// tracker). `None` is returned only when the plugin said nothing at all, so
/// "no observation" and "an observation full of unknowns" stay distinguishable
/// downstream.
fn seeding_snapshot_for_item(item: &PluginDownloadItem) -> Option<DownloadSeedingSnapshot> {
    let torrent = item.torrent.as_ref();
    let snapshot = DownloadSeedingSnapshot {
        can_remove: item.can_remove,
        can_move_files: item.can_move_files,
        seed_ratio: torrent.and_then(|torrent| torrent.seed_ratio),
        seed_time_seconds: torrent.and_then(|torrent| torrent.seed_time_seconds),
        is_private: torrent.and_then(|torrent| torrent.is_private),
        uploaded_bytes: torrent.and_then(|torrent| torrent.uploaded_bytes),
        completed_at: item
            .completed_at
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        // Goals are policy, not client state: the queue projection joins them
        // from the resolution that was frozen on the submission row at grab.
        seed_goal_ratio: None,
        seed_goal_seconds: None,
        never_remove: false,
    };
    (!snapshot.is_empty()).then_some(snapshot)
}

/// The removal request to send for one item, with `remove_data` clamped to what
/// the plugin says it can do.
///
/// This is the first reader of `DownloadClientCapabilities::remove_with_data`.
/// Three published clients declare it `false` and mean it: rTorrent and
/// DownloadStation answer a data-removal request with `Unsupported` (Scryer
/// deletes their files through host filesystem access, not through the client
/// ABI) and aria2 ignores it. Forwarding the flag verbatim would turn every
/// terminal cleanup on those two into an error, and the queue entry would never
/// be removed at all — strictly worse than leaving the payload behind. Sonarr
/// has the same division of labour and still removes the entry
/// (`RTorrent.cs:217-225`, `TorrentDownloadStation.cs:144-153`), so dropping the
/// data half is the right half to lose.
///
/// The caller's `remove_data` is a *policy* decision (see
/// `import::workflow::results::reconcile_terminal_download_cleanup`); this is
/// purely "can this client carry it out".
fn remove_control_request(
    descriptor: &PluginDescriptor,
    id: &str,
    is_history: bool,
    remove_data: bool,
) -> PluginDownloadClientControlRequest {
    let can_remove_with_data = descriptor
        .download_client()
        .is_some_and(|provider| provider.capabilities.remove_with_data);
    PluginDownloadClientControlRequest {
        action: DownloadControlAction::Remove,
        client_item_id: id.to_string(),
        remove_data: remove_data && can_remove_with_data,
        is_history,
    }
}

/// Every identifier a later listing might use to find this item again.
fn seeding_observation_keys(item: &PluginDownloadItem) -> Vec<String> {
    let mut keys = Vec::new();
    let mut push = |value: Option<String>| {
        if let Some(value) = value.map(|value| value.trim().to_string())
            && !value.is_empty()
            && !keys.contains(&value)
        {
            keys.push(value);
        }
    };
    push(normalized_plugin_info_hash(item.info_hash.as_deref()));
    push(
        item.torrent
            .as_ref()
            .and_then(|torrent| normalized_plugin_info_hash(torrent.info_hash_v1.as_deref())),
    );
    push(normalized_plugin_info_hash(Some(
        item.client_item_id.as_str(),
    )));
    push(Some(item.client_item_id.clone()));
    push(item.download_id.clone());
    keys
}

fn map_add_response_to_grab_result(
    response: PluginDownloadClientAddResponse,
    request: &DownloadClientAddRequest,
    client_type: &str,
) -> DownloadGrabResult {
    let client_item_id = response.client_item_id;
    let info_hash = normalized_plugin_info_hash(response.info_hash.as_deref()).or_else(|| {
        if matches!(
            request.source_kind,
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri)
        ) {
            normalized_plugin_info_hash(Some(client_item_id.as_str()))
        } else {
            None
        }
    });

    DownloadGrabResult {
        download_id: None,
        job_id: client_item_id,
        client_id: None,
        client_type: client_type.to_string(),
        info_hash,
        seed_goals: None,
    }
}

fn map_queue_item(
    item: PluginDownloadItem,
    client_id: &str,
    client_name: &str,
    client_type: &str,
) -> DownloadQueueItem {
    let attention = attention_required(&item);
    let attention_reason = item.message.clone();
    let info_hash = normalized_plugin_info_hash(item.info_hash.as_deref())
        .or_else(|| {
            item.torrent
                .as_ref()
                .and_then(|torrent| normalized_plugin_info_hash(torrent.info_hash_v1.as_deref()))
        })
        .or_else(|| normalized_plugin_info_hash(Some(item.client_item_id.as_str())));
    let observed_identity = scryer_application::observed_download_identity(
        scryer_application::ObservedDownloadIdentityInput {
            download_id: item.download_id.as_deref(),
            parameters: &[],
            info_hash_hint: info_hash.as_deref(),
        },
    );
    let seeding = seeding_snapshot_for_item(&item);
    DownloadQueueItem {
        id: format!(
            "{client_type}:{}",
            info_hash
                .clone()
                .unwrap_or_else(|| item.client_item_id.clone())
        ),
        title_id: None,
        episode_id: None,
        title_name: item.title,
        facet: None,
        category: item
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        client_id: client_id.to_string(),
        client_name: client_name.to_string(),
        client_type: client_type.to_string(),
        state: map_state(item.state),
        progress_percent: item.progress_percent.unwrap_or(0),
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: item.total_size_bytes,
        remaining_seconds: item.eta_seconds,
        queued_at: None,
        last_updated_at: None,
        attention_required: attention,
        attention_reason,
        download_client_item_id: item.client_item_id,
        download_id: observed_identity.download_id,
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
        seeding,
    }
}

fn retain_queue_item(item: &PluginDownloadItem) -> bool {
    !matches!(
        item.state,
        DownloadItemState::Completed | DownloadItemState::Seeding
    )
}

fn map_completed_download(
    item: PluginCompletedDownload,
    client_id: &str,
    client_type: &str,
) -> CompletedDownload {
    let dest_dir = {
        let mut content_paths = item
            .content_paths
            .iter()
            .map(|path| path.trim())
            .filter(|path| !path.is_empty());
        match (content_paths.next(), content_paths.next()) {
            (Some(content_path), None) => content_path.to_string(),
            _ => item.dest_dir.clone(),
        }
    };
    let info_hash = normalized_plugin_info_hash(item.info_hash.as_deref())
        .or_else(|| normalized_plugin_info_hash(Some(item.client_item_id.as_str())));
    let observed_identity = scryer_application::observed_download_identity(
        scryer_application::ObservedDownloadIdentityInput {
            download_id: item.download_id.as_deref(),
            parameters: &item.parameters,
            info_hash_hint: info_hash.as_deref(),
        },
    );
    CompletedDownload {
        client_type: client_type.to_string(),
        client_id: client_id.to_string(),
        download_client_item_id: item.client_item_id,
        download_id: observed_identity.download_id,
        name: item.name,
        release_name: item.release_name,
        dest_dir,
        category: item.category,
        size_bytes: item.size_bytes,
        completed_at: parse_timestamp(item.completed_at),
        parameters: item.parameters,
    }
}

fn map_history_item_from_completed(
    item: PluginCompletedDownload,
    client_id: &str,
    client_name: &str,
    client_type: &str,
) -> DownloadQueueItem {
    let info_hash = normalized_plugin_info_hash(item.info_hash.as_deref())
        .or_else(|| normalized_plugin_info_hash(Some(item.client_item_id.as_str())));
    let observed_identity = scryer_application::observed_download_identity(
        scryer_application::ObservedDownloadIdentityInput {
            download_id: item.download_id.as_deref(),
            parameters: &item.parameters,
            info_hash_hint: info_hash.as_deref(),
        },
    );
    let download_client_item_id = info_hash.unwrap_or_else(|| item.client_item_id.clone());
    DownloadQueueItem {
        id: format!("{client_type}:{download_client_item_id}"),
        title_id: None,
        episode_id: None,
        title_name: item.name,
        facet: None,
        category: item
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        client_id: client_id.to_string(),
        client_name: client_name.to_string(),
        client_type: client_type.to_string(),
        state: DownloadQueueState::Completed,
        progress_percent: 100,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: item.size_bytes,
        remaining_seconds: Some(0),
        queued_at: None,
        last_updated_at: item.completed_at,
        attention_required: false,
        attention_reason: None,
        download_client_item_id,
        download_id: observed_identity.download_id,
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
        // A completed-download envelope carries no seeding fields; the
        // adapter stamps the last queue observation on afterwards.
        seeding: None,
    }
}

fn build_isolation_entries(value: Option<&str>) -> Vec<PluginDownloadIsolation> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };

    [
        DownloadIsolationMode::Category,
        DownloadIsolationMode::Tag,
        DownloadIsolationMode::Label,
        DownloadIsolationMode::View,
    ]
    .into_iter()
    .map(|mode| PluginDownloadIsolation {
        mode,
        value: value.to_string(),
    })
    .collect()
}

fn queue_placement(queue_priority: Option<&str>) -> Option<PluginTorrentQueuePlacement> {
    match queue_priority
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("first") | Some("top") | Some("high") => Some(PluginTorrentQueuePlacement::First),
        Some("last") | Some("bottom") | Some("low") => Some(PluginTorrentQueuePlacement::Last),
        _ => None,
    }
}

fn derive_torrent_file_name(request: &DownloadClientAddRequest) -> Option<String> {
    request
        .source_title
        .clone()
        .or_else(|| request.release_title.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn derive_nzb_file_name(request: &DownloadClientAddRequest) -> Option<String> {
    derive_torrent_file_name(request).map(|value| {
        if value.to_ascii_lowercase().ends_with(".nzb") {
            value
        } else {
            format!("{value}.nzb")
        }
    })
}

fn load_staged_nzb_payload(staged_nzb: &StagedNzbRef) -> AppResult<String> {
    let file = File::open(&staged_nzb.compressed_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to open staged nzb artifact {}: {error}",
            staged_nzb.compressed_path.display()
        ))
    })?;
    let bytes = zstd::stream::decode_all(file).map_err(|error| {
        AppError::Repository(format!(
            "failed to decompress staged nzb artifact {}: {error}",
            staged_nzb.compressed_path.display()
        ))
    })?;
    if staged_nzb.raw_size_bytes > 0 && bytes.len() as u64 != staged_nzb.raw_size_bytes {
        return Err(AppError::Repository(format!(
            "staged nzb artifact {} decompressed to {} bytes, expected {}",
            staged_nzb.id,
            bytes.len(),
            staged_nzb.raw_size_bytes
        )));
    }
    Ok(BASE64.encode(bytes))
}

fn select_plugin_input_kind(
    source_kind: DownloadSourceKind,
    resolved: &ResolvedTorrentSource,
) -> DownloadInputKind {
    if resolved.nzb_bytes_base64.is_some() {
        DownloadInputKind::Nzb
    } else if resolved.magnet_uri.is_some() {
        DownloadInputKind::MagnetUri
    } else if resolved.torrent_bytes_base64.is_some() {
        DownloadInputKind::TorrentBytes
    } else if resolved.torrent_url.is_some() {
        DownloadInputKind::TorrentUrl
    } else {
        map_source_kind(source_kind)
    }
}

fn resolved_artifact_source(
    artifact: &ResolvedDownloadArtifact,
) -> (DownloadSourceKind, ResolvedTorrentSource) {
    match artifact {
        ResolvedDownloadArtifact::Nzb {
            bytes,
            file_name,
            content_type,
        } => (
            DownloadSourceKind::NzbFile,
            ResolvedTorrentSource {
                download_url: None,
                nzb_bytes_base64: Some(BASE64.encode(bytes)),
                nzb_file_name: file_name.clone(),
                nzb_content_type: content_type
                    .clone()
                    .or_else(|| Some("application/x-nzb".to_string())),
                ..ResolvedTorrentSource::default()
            },
        ),
        ResolvedDownloadArtifact::Magnet {
            uri,
            info_hash_hint: _,
        } => (
            DownloadSourceKind::MagnetUri,
            ResolvedTorrentSource {
                download_url: None,
                magnet_uri: Some(uri.clone()),
                ..ResolvedTorrentSource::default()
            },
        ),
        ResolvedDownloadArtifact::TorrentFile {
            bytes,
            file_name,
            content_type,
            info_hash_hint: _,
        } => (
            DownloadSourceKind::TorrentFile,
            ResolvedTorrentSource {
                download_url: None,
                torrent_url: None,
                torrent_bytes_base64: Some(BASE64.encode(bytes)),
                torrent_file_name: file_name.clone(),
                torrent_content_type: content_type
                    .clone()
                    .or_else(|| Some("application/x-bittorrent".to_string())),
                ..ResolvedTorrentSource::default()
            },
        ),
    }
}

fn build_plugin_add_request(
    request: &DownloadClientAddRequest,
    source_kind: DownloadSourceKind,
    resolved: ResolvedTorrentSource,
) -> PluginDownloadClientAddRequest {
    let (info_hash_v1, info_hash_v2) = normalize_info_hash_pair(&PluginDownloadRelease {
        info_hash_hint: request.info_hash_hint.clone(),
        info_hash_v1: request.info_hash_hint.clone(),
        info_hash_v2: request.info_hash_hint.clone(),
        ..PluginDownloadRelease::default()
    });
    let source_preference = [
        (resolved.magnet_uri.is_some(), DownloadInputKind::MagnetUri),
        (
            resolved.torrent_bytes_base64.is_some(),
            DownloadInputKind::TorrentBytes,
        ),
        (
            resolved.torrent_url.is_some(),
            DownloadInputKind::TorrentUrl,
        ),
        (
            matches!(source_kind, DownloadSourceKind::TorrentFile)
                && (resolved.torrent_bytes_base64.is_some()
                    || resolved.torrent_url.is_some()
                    || resolved.download_url.is_some()),
            DownloadInputKind::TorrentFile,
        ),
    ]
    .into_iter()
    .filter_map(|(enabled, kind)| enabled.then_some(kind))
    .collect::<Vec<_>>();
    let isolation = build_isolation_entries(request.category.as_deref());

    PluginDownloadClientAddRequest {
        source: PluginDownloadSource {
            kind: select_plugin_input_kind(source_kind, &resolved),
            download_url: resolved.download_url,
            magnet_uri: resolved.magnet_uri,
            torrent_bytes_base64: resolved.torrent_bytes_base64,
            torrent_url: resolved.torrent_url,
            torrent_file_name: resolved
                .torrent_file_name
                .or_else(|| derive_torrent_file_name(request)),
            torrent_content_type: resolved.torrent_content_type,
            nzb_bytes_base64: resolved.nzb_bytes_base64,
            nzb_file_name: resolved.nzb_file_name,
            nzb_content_type: resolved.nzb_content_type,
            source_title: request.source_title.clone(),
            source_password: request.source_password.clone(),
        },
        release: PluginDownloadRelease {
            download_id: request.download_id.map(|id| id.to_wire()),
            release_title: request
                .release_title
                .clone()
                .or_else(|| request.source_title.clone()),
            import_purpose: Some(request.purpose.as_str().to_string()),
            is_recent: request.is_recent,
            season_pack: request.season_pack,
            indexer_name: request.indexer_name.clone(),
            info_hash_hint: request.info_hash_hint.clone(),
            info_hash_v1,
            info_hash_v2,
            seed_goal_ratio: request.seed_goal_ratio,
            seed_goal_seconds: request.seed_goal_seconds,
        },
        title: PluginDownloadTitle {
            title_id: Some(request.title.id.clone()),
            title_name: request.title.name.clone(),
            media_facet: request.title.facet.as_str().to_string(),
            title_slug: request.title.slug.clone(),
            year: request.title.year,
            language: request.title.language.clone(),
            network: request.title.network.clone(),
            tags: request.title.tags.clone(),
        },
        routing: PluginDownloadRouting {
            isolation_value: request.category.clone(),
            isolation: isolation.clone(),
            post_import_isolation: isolation,
            queue_priority: request.queue_priority.clone(),
            download_directory: request.download_directory.clone(),
        },
        torrent: matches!(
            source_kind,
            DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri
        )
        .then_some(PluginTorrentOptions {
            source_preference,
            seed_goal_ratio: request.seed_goal_ratio,
            seed_goal_seconds: request.seed_goal_seconds,
            initial_state: None,
            queue_placement: queue_placement(request.queue_priority.as_deref()),
            priority_hint: request.queue_priority.clone(),
            sequential_download: None,
            first_last_piece_priority: None,
            content_layout: None,
            skip_checking: None,
            auto_management: None,
            force_start: None,
            safe_seeding: None,
            anonymity_hops: None,
            selected_file_indices: Vec::new(),
        }),
    }
}

#[async_trait]
impl DownloadClient for WasmDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let source_hint = request.source_hint.clone();
        let source_kind = request
            .source_kind
            .or_else(|| DownloadSourceKind::infer_from_hint(source_hint.as_deref()))
            .unwrap_or(DownloadSourceKind::TorrentFile);
        let resolved_artifact = request
            .resolved_download_artifact
            .as_ref()
            .map(resolved_artifact_source);
        let source_kind = resolved_artifact
            .as_ref()
            .map(|(source_kind, _)| *source_kind)
            .unwrap_or(source_kind);

        // When the source is a .torrent HTTP URL and we have no info_hash_hint,
        // pre-fetch the torrent file so the plugin can compute the hash directly.
        // Some trackers redirect .torrent URLs to magnet URIs — detect that and
        // switch to the magnet path.
        let torrent_bytes_base64 = resolved_artifact
            .as_ref()
            .and_then(|(_, resolved)| resolved.torrent_bytes_base64.clone());
        let resolved_magnet_uri: Option<String> = resolved_artifact
            .as_ref()
            .and_then(|(_, resolved)| resolved.magnet_uri.clone());
        let resolved_download_url = resolved_artifact
            .is_none()
            .then(|| source_hint.clone())
            .flatten();
        let torrent_url = if resolved_artifact.is_none() {
            source_hint.clone().filter(|url| {
                matches!(source_kind, DownloadSourceKind::TorrentFile)
                    && (url.starts_with("http://") || url.starts_with("https://"))
            })
        } else {
            None
        };
        let torrent_content_type = resolved_artifact
            .as_ref()
            .and_then(|(_, resolved)| resolved.torrent_content_type.clone());
        let torrent_file_name = resolved_artifact
            .as_ref()
            .and_then(|(_, resolved)| resolved.torrent_file_name.clone());
        let mut nzb_bytes_base64 = resolved_artifact
            .as_ref()
            .and_then(|(_, resolved)| resolved.nzb_bytes_base64.clone());
        let mut nzb_file_name = resolved_artifact
            .as_ref()
            .and_then(|(_, resolved)| resolved.nzb_file_name.clone());
        let mut nzb_content_type = resolved_artifact
            .as_ref()
            .and_then(|(_, resolved)| resolved.nzb_content_type.clone());
        if matches!(
            source_kind,
            DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl
        ) && let Some(staged_nzb) = request.staged_nzb.as_ref()
        {
            nzb_bytes_base64 = Some(load_staged_nzb_payload(staged_nzb)?);
            nzb_file_name = derive_nzb_file_name(request);
            nzb_content_type = Some("application/x-nzb".to_string());
        }

        let magnet_uri = resolved_magnet_uri.or_else(|| {
            source_hint
                .as_ref()
                .filter(|v| v.starts_with("magnet:"))
                .cloned()
        });

        let plugin_request = build_plugin_add_request(
            request,
            source_kind,
            ResolvedTorrentSource {
                download_url: resolved_download_url,
                magnet_uri,
                torrent_bytes_base64,
                torrent_url,
                torrent_file_name,
                torrent_content_type,
                nzb_bytes_base64,
                nzb_file_name,
                nzb_content_type,
            },
        );

        let result = self
            .invoke_command(PluginDownloadClientCommand::Add(plugin_request), "add")
            .await?;
        let PluginDownloadClientCommandResult::Add(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for add".to_string(),
            ));
        };
        let response = decode_download_add_result(result, "download add")?;
        Ok(map_add_response_to_grab_result(
            response,
            request,
            self.descriptor.provider_type(),
        ))
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let result = self
            .invoke_command(PluginDownloadClientCommand::ListQueue, "list_queue")
            .await?;
        let PluginDownloadClientCommandResult::ListQueue(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for list_queue".to_string(),
            ));
        };
        let mut items: Vec<PluginDownloadItem> =
            decode_command_result(result, "download list_queue")?;
        apply_seeding_trust_floor(&self.descriptor, &mut items);
        self.seeding_observations.record(&items);

        Ok(items
            .into_iter()
            .filter(retain_queue_item)
            .map(|item| {
                map_queue_item(
                    item,
                    &self.client_id,
                    &self.client_name,
                    self.descriptor.provider_type(),
                )
            })
            .collect())
    }

    async fn list_queue_with_feedback_scope(
        &self,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if feedback_scope_is_empty(scope) || !self.supports_category_scoped_feedback() {
            return self.list_queue().await;
        }
        let result = self
            .invoke_command(
                PluginDownloadClientCommand::ListQueueScoped(PluginDownloadScopedListRequest {
                    scope: Self::plugin_feedback_scope(scope),
                }),
                "list_queue_scoped",
            )
            .await?;
        let PluginDownloadClientCommandResult::ListQueueScoped(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for list_queue_scoped"
                    .to_string(),
            ));
        };
        let mut items = decode_scoped_command_result(
            result,
            "download list_queue_scoped",
            &self.client_id,
            self.descriptor.provider_type(),
        )?;
        apply_seeding_trust_floor(&self.descriptor, &mut items);
        self.seeding_observations.record(&items);
        Ok(items
            .into_iter()
            .filter(retain_queue_item)
            .map(|item| {
                map_queue_item(
                    item,
                    &self.client_id,
                    &self.client_name,
                    self.descriptor.provider_type(),
                )
            })
            .collect())
    }

    async fn list_queue_for_title_with_feedback_scope(
        &self,
        _title_id: &str,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_queue_with_feedback_scope(scope).await
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let result = self
            .invoke_command(PluginDownloadClientCommand::ListHistory, "list_history")
            .await?;
        let PluginDownloadClientCommandResult::ListHistory(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for list_history".to_string(),
            ));
        };
        let mut items = decode_command_result(result, "download list_history")?
            .into_iter()
            .map(|item| {
                map_history_item_from_completed(
                    item,
                    &self.client_id,
                    &self.client_name,
                    self.descriptor.provider_type(),
                )
            })
            .collect::<Vec<_>>();
        self.seeding_observations.apply(&mut items);
        Ok(items)
    }

    async fn list_history_with_feedback_scope(
        &self,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if feedback_scope_is_empty(scope) || !self.supports_category_scoped_feedback() {
            return self.list_history().await;
        }
        let result = self
            .invoke_command(
                PluginDownloadClientCommand::ListHistoryScoped(PluginDownloadScopedListRequest {
                    scope: Self::plugin_feedback_scope(scope),
                }),
                "list_history_scoped",
            )
            .await?;
        let PluginDownloadClientCommandResult::ListHistoryScoped(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for list_history_scoped"
                    .to_string(),
            ));
        };
        let mut items = decode_scoped_command_result(
            result,
            "download list_history_scoped",
            &self.client_id,
            self.descriptor.provider_type(),
        )?
        .into_iter()
        .map(|item| {
            map_history_item_from_completed(
                item,
                &self.client_id,
                &self.client_name,
                self.descriptor.provider_type(),
            )
        })
        .collect::<Vec<_>>();
        self.seeding_observations.apply(&mut items);
        Ok(items)
    }

    async fn list_history_page_with_feedback_scope(
        &self,
        offset: usize,
        limit: usize,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        Ok(self
            .list_history_with_feedback_scope(scope)
            .await?
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect())
    }

    async fn list_recent_activity_with_feedback_scope(
        &self,
        limit: usize,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_history_page_with_feedback_scope(0, limit, scope)
            .await
    }

    async fn list_recent_activity_for_title_with_feedback_scope(
        &self,
        _title_id: &str,
        limit: usize,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_recent_activity_with_feedback_scope(limit, scope)
            .await
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        let result = self
            .invoke_command(PluginDownloadClientCommand::ListCompleted, "list_completed")
            .await?;
        let PluginDownloadClientCommandResult::ListCompleted(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for list_completed".to_string(),
            ));
        };
        Ok(decode_command_result(result, "download list_completed")?
            .into_iter()
            .map(|item| {
                map_completed_download(item, &self.client_id, self.descriptor.provider_type())
            })
            .collect())
    }

    async fn list_completed_downloads_with_feedback_scope(
        &self,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<CompletedDownload>> {
        if feedback_scope_is_empty(scope) || !self.supports_category_scoped_feedback() {
            return self.list_completed_downloads().await;
        }
        let result = self
            .invoke_command(
                PluginDownloadClientCommand::ListCompletedScoped(PluginDownloadScopedListRequest {
                    scope: Self::plugin_feedback_scope(scope),
                }),
                "list_completed_scoped",
            )
            .await?;
        let PluginDownloadClientCommandResult::ListCompletedScoped(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for list_completed_scoped"
                    .to_string(),
            ));
        };
        Ok(decode_scoped_command_result(
            result,
            "download list_completed_scoped",
            &self.client_id,
            self.descriptor.provider_type(),
        )?
        .into_iter()
        .map(|item| map_completed_download(item, &self.client_id, self.descriptor.provider_type()))
        .collect())
    }

    async fn get_completed_download_for_source(
        &self,
        client_id: &str,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<CompletedDownload>> {
        let download_client_item_id = download_client_item_id.trim();
        if client_id.trim() != self.client_id
            || !client_type.eq_ignore_ascii_case(self.descriptor.provider_type())
        {
            return Ok(None);
        }
        let result = self
            .invoke_command(
                PluginDownloadClientCommand::GetCompleted(PluginDownloadGetCompletedRequest {
                    client_item_id: download_client_item_id.to_string(),
                }),
                "get_completed",
            )
            .await?;
        let PluginDownloadClientCommandResult::GetCompleted(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for get_completed".to_string(),
            ));
        };
        let completed = decode_command_result(result, "download get_completed")?;
        let Some(completed) = completed else {
            return Ok(None);
        };
        if completed.client_item_id != download_client_item_id {
            return Err(AppError::Repository(format!(
                "download-client command returned completed item '{}' for requested item '{}'",
                completed.client_item_id, download_client_item_id
            )));
        }
        Ok(Some(map_completed_download(
            completed,
            &self.client_id,
            self.descriptor.provider_type(),
        )))
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<CompletedDownload>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let result = self
            .invoke_command(
                PluginDownloadClientCommand::ListRecentCompleted(
                    PluginDownloadListRecentCompletedRequest { limit },
                ),
                "list_recent_completed",
            )
            .await?;
        let PluginDownloadClientCommandResult::ListRecentCompleted(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for list_recent_completed"
                    .to_string(),
            ));
        };
        Ok(
            decode_command_result(result, "download list_recent_completed")?
                .into_iter()
                .map(|item| {
                    map_completed_download(item, &self.client_id, self.descriptor.provider_type())
                })
                .collect(),
        )
    }

    async fn list_recent_completed_downloads_with_feedback_scope(
        &self,
        limit: usize,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<CompletedDownload>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        if feedback_scope_is_empty(scope) || !self.supports_category_scoped_feedback() {
            return self.list_recent_completed_downloads(limit).await;
        }
        let result = self
            .invoke_command(
                PluginDownloadClientCommand::ListRecentCompletedScoped(
                    PluginDownloadScopedRecentCompletedRequest {
                        limit,
                        scope: Self::plugin_feedback_scope(scope),
                    },
                ),
                "list_recent_completed_scoped",
            )
            .await?;
        let PluginDownloadClientCommandResult::ListRecentCompletedScoped(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for list_recent_completed_scoped"
                    .to_string(),
            ));
        };
        Ok(decode_scoped_command_result(
            result,
            "download list_recent_completed_scoped",
            &self.client_id,
            self.descriptor.provider_type(),
        )?
        .into_iter()
        .map(|item| map_completed_download(item, &self.client_id, self.descriptor.provider_type()))
        .collect())
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        let request = PluginDownloadClientControlRequest {
            action: DownloadControlAction::Pause,
            client_item_id: id.to_string(),
            remove_data: false,
            is_history: false,
        };
        let result = self
            .invoke_command(PluginDownloadClientCommand::Control(request), "control")
            .await?;
        let PluginDownloadClientCommandResult::Control(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for control".to_string(),
            ));
        };
        decode_command_result(result, "download control")
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        let request = PluginDownloadClientControlRequest {
            action: DownloadControlAction::Resume,
            client_item_id: id.to_string(),
            remove_data: false,
            is_history: false,
        };
        let result = self
            .invoke_command(PluginDownloadClientCommand::Control(request), "control")
            .await?;
        let PluginDownloadClientCommandResult::Control(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for control".to_string(),
            ));
        };
        decode_command_result(result, "download control")
    }

    /// Remove the item, asking for its data only if the client can actually
    /// delete it.
    ///
    /// See `remove_control_request`: a plugin that declares
    /// `remove_with_data: false` is sent an entry-only removal rather than a
    /// request it would refuse.
    async fn delete_queue_item(
        &self,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        let request = remove_control_request(&self.descriptor, id, is_history, remove_data);
        if remove_data && !request.remove_data {
            debug!(
                provider = %self.descriptor.id,
                client_id = %self.client_id,
                client_item_id = id,
                "download client cannot delete downloaded data; removing the entry only and leaving the payload in place"
            );
        }
        let result = self
            .invoke_command(PluginDownloadClientCommand::Control(request), "control")
            .await?;
        let PluginDownloadClientCommandResult::Control(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for control".to_string(),
            ));
        };
        decode_command_result(result, "download control")
    }

    async fn mark_imported(&self, request: &DownloadClientMarkImportedRequest) -> AppResult<()> {
        let command_request = PluginDownloadClientMarkImportedRequest {
            client_item_id: request.client_item_id.clone(),
            info_hash: request.info_hash.clone(),
            title_id: request.title_id.clone(),
            title_name: request.title_name.clone(),
            category: request.category.clone(),
            post_import_isolation: build_isolation_entries(request.category.as_deref()),
            imported_path: request.imported_path.clone(),
            download_path: request.download_path.clone(),
        };
        let result = self
            .invoke_command(
                PluginDownloadClientCommand::MarkImported(command_request),
                "mark_imported",
            )
            .await?;
        let PluginDownloadClientCommandResult::MarkImported(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for mark_imported".to_string(),
            ));
        };
        decode_command_result(result, "download mark_imported")
    }

    async fn mark_imported_non_destructive(
        &self,
        request: &DownloadClientMarkImportedRequest,
    ) -> AppResult<()> {
        let supported = self
            .descriptor
            .download_client()
            .is_some_and(|provider| provider.capabilities.mark_imported_non_destructive);
        if !supported {
            info!(
                plugin_id = %self.descriptor.id,
                client_id = %self.client_id,
                client_name = %self.client_name,
                client_item_id = %request.client_item_id,
                "skipping non-destructive import mark because the plugin does not advertise the capability"
            );
            return Ok(());
        }

        let command_request = PluginDownloadClientMarkImportedRequest {
            client_item_id: request.client_item_id.clone(),
            info_hash: request.info_hash.clone(),
            title_id: request.title_id.clone(),
            title_name: request.title_name.clone(),
            category: request.category.clone(),
            post_import_isolation: build_isolation_entries(request.category.as_deref()),
            imported_path: request.imported_path.clone(),
            download_path: request.download_path.clone(),
        };
        let result = self
            .invoke_command(
                PluginDownloadClientCommand::MarkImportedNonDestructive(command_request),
                "mark_imported_non_destructive",
            )
            .await?;
        let PluginDownloadClientCommandResult::MarkImportedNonDestructive(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for non-destructive import mark"
                    .to_string(),
            ));
        };
        decode_command_result(result, "download mark_imported_non_destructive")?;
        info!(
            plugin_id = %self.descriptor.id,
            client_id = %self.client_id,
            client_name = %self.client_name,
            client_item_id = %request.client_item_id,
            "marked imported download in client non-destructively"
        );
        Ok(())
    }

    async fn get_client_status(&self) -> AppResult<DownloadClientStatus> {
        let result = self
            .invoke_command(PluginDownloadClientCommand::Status, "status")
            .await?;
        let PluginDownloadClientCommandResult::Status(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for status".to_string(),
            ));
        };
        let status = decode_command_result(result, "download status")?;
        Ok(DownloadClientStatus {
            version: status.version,
            is_localhost: status.is_localhost,
            remote_output_roots: status.remote_output_roots,
            removes_completed_downloads: status.removes_completed_downloads,
            sorting_mode: status.sorting_mode,
            warnings: status.warnings,
        })
    }

    async fn test_connection(&self) -> AppResult<String> {
        let result = self
            .invoke_command(
                PluginDownloadClientCommand::TestConnection,
                "test_connection",
            )
            .await?;
        let PluginDownloadClientCommandResult::TestConnection(result) = result else {
            return Err(AppError::Repository(
                "download-client command returned the wrong result for test_connection".to_string(),
            ));
        };
        decode_command_result(result, "download test_connection")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_plugin_sdk::PluginTorrentItem;

    #[test]
    fn parse_timestamp_accepts_rfc3339_and_unix_seconds() {
        let expected = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        assert_eq!(
            parse_timestamp(Some("2023-11-14T22:13:20Z".to_string())),
            Some(expected)
        );
        assert_eq!(
            parse_timestamp(Some(" 1700000000 ".to_string())),
            Some(expected)
        );
        assert_eq!(parse_timestamp(Some("not-a-time".to_string())), None);
    }

    fn queue_filter_item(state: DownloadItemState) -> PluginDownloadItem {
        PluginDownloadItem {
            client_item_id: format!("{state:?}"),
            download_id: None,
            info_hash: None,
            title: format!("{state:?}"),
            state,
            message: None,
            category: None,
            remote_output_path: None,
            torrent: None,
            total_size_bytes: None,
            remaining_size_bytes: None,
            eta_seconds: None,
            progress_percent: None,
            can_move_files: None,
            can_remove: None,
            removed: None,
            raw_state: None,
            completed_at: None,
        }
    }

    #[test]
    fn partial_scoped_response_is_not_reported_as_complete() {
        let item = queue_filter_item(DownloadItemState::Downloading);
        let items = decode_scoped_command_result(
            PluginResult::Ok(PluginDownloadScopedListResponse {
                items: vec![item.clone()],
                failures: vec![scryer_plugin_sdk::PluginDownloadScopeFailure {
                    category: "TV / Anime".to_string(),
                    error: PluginError {
                        details: None,
                        code: PluginErrorCode::UpstreamUnavailable,
                        public_message: "category request timed out".to_string(),
                        debug_message: None,
                        retry_after_seconds: None,
                    },
                }],
            }),
            "scoped test",
            "qbit",
            "qbittorrent",
        )
        .expect_err("a failed category must not be reported as a complete snapshot");

        assert!(items.to_string().contains("TV / Anime"));
    }

    #[test]
    fn empty_feedback_scope_uses_the_unfiltered_poll_path() {
        assert!(feedback_scope_is_empty(&DownloadClientFeedbackScope {
            categories: Vec::new(),
        }));
        assert!(!feedback_scope_is_empty(&DownloadClientFeedbackScope {
            categories: vec!["series".to_string()],
        }));
    }

    #[test]
    fn queue_filter_retains_failed_and_error_items_only_as_terminal_observations() {
        assert!(retain_queue_item(&queue_filter_item(
            DownloadItemState::Failed
        )));
        assert!(retain_queue_item(&queue_filter_item(
            DownloadItemState::Error
        )));
        assert!(retain_queue_item(&queue_filter_item(
            DownloadItemState::Downloading
        )));
        assert!(!retain_queue_item(&queue_filter_item(
            DownloadItemState::Completed
        )));
        assert!(!retain_queue_item(&queue_filter_item(
            DownloadItemState::Seeding
        )));
    }

    #[test]
    fn a_warning_keeps_its_own_state_and_its_message() {
        // The plugins report `Warning` for recoverable client conditions
        // (qBittorrent `error` / `missingFiles`) precisely so the host does not
        // run failed-download handling on them.
        let mut item = queue_filter_item(DownloadItemState::Warning);
        item.message = Some("files are missing from the save path".to_string());

        assert!(retain_queue_item(&item));
        assert!(attention_required(&item));

        let queue_item = map_queue_item(item, "client-1", "qBittorrent", "qbittorrent");

        assert_eq!(queue_item.state, DownloadQueueState::Warning);
        assert!(queue_item.attention_required);
        assert_eq!(
            queue_item.attention_reason.as_deref(),
            Some("files are missing from the save path")
        );
    }

    #[test]
    fn a_reported_failure_still_maps_to_failed() {
        for state in [DownloadItemState::Failed, DownloadItemState::Error] {
            assert_eq!(map_state(state), DownloadQueueState::Failed, "{state:?}");
        }
    }

    #[test]
    fn a_client_that_reports_nothing_produces_no_observation() {
        // "No observation" and "an observation full of unknowns" are different
        // answers; a usenet item that says nothing must not manufacture one.
        assert_eq!(
            seeding_snapshot_for_item(&queue_filter_item(DownloadItemState::Downloading)),
            None
        );
    }

    #[test]
    fn an_unknown_client_verdict_is_never_flattened_into_a_bool() {
        let mut item = queue_filter_item(DownloadItemState::Completed);
        item.torrent = Some(PluginTorrentItem {
            seed_ratio: Some(1.5),
            ..PluginTorrentItem::default()
        });
        item.can_remove = None;
        item.can_move_files = None;

        let snapshot =
            seeding_snapshot_for_item(&item).expect("a reported ratio is an observation");
        assert_eq!(snapshot.can_remove, None);
        assert_eq!(snapshot.can_move_files, None);
        assert_eq!(snapshot.seed_ratio, Some(1.5));

        // And each explicit state survives untouched.
        for verdict in [Some(true), Some(false), None] {
            let mut item = item.clone();
            item.can_remove = verdict;
            assert_eq!(
                seeding_snapshot_for_item(&item)
                    .expect("observation")
                    .can_remove,
                verdict
            );
        }
    }

    #[test]
    fn an_absent_private_flag_is_never_reported_as_public() {
        let mut item = queue_filter_item(DownloadItemState::Completed);
        item.can_remove = Some(true);
        item.torrent = Some(PluginTorrentItem::default());

        let snapshot = seeding_snapshot_for_item(&item).expect("observation");
        assert_eq!(snapshot.is_private, None);
    }

    #[test]
    fn a_history_row_inherits_the_last_queue_observation_for_its_torrent() {
        // The finished torrents the seeding gate cares about are filtered out of
        // the queue and come back framed as completed downloads, which carry no
        // seeding fields at all. The observation recorded on the way past is the
        // only thing that keeps them answerable.
        let client = SeedingObservationCache::default();

        let info_hash = "abcdef0123456789abcdef0123456789abcdef01";
        let mut seeding_torrent = queue_filter_item(DownloadItemState::Seeding);
        seeding_torrent.client_item_id = info_hash.to_ascii_uppercase();
        seeding_torrent.can_remove = Some(false);
        seeding_torrent.can_move_files = Some(true);
        seeding_torrent.torrent = Some(PluginTorrentItem {
            seed_ratio: Some(0.9),
            seed_time_seconds: Some(4_200),
            is_private: Some(true),
            ..PluginTorrentItem::default()
        });
        client.record(&[seeding_torrent]);

        let mut history = vec![map_history_item_from_completed(
            PluginCompletedDownload {
                client_item_id: info_hash.to_string(),
                download_id: None,
                info_hash: Some(info_hash.to_string()),
                name: "Finished Torrent".to_string(),
                release_name: None,
                dest_dir: "/downloads".to_string(),
                category: None,
                output_kind: None,
                content_paths: Vec::new(),
                size_bytes: Some(1),
                completed_at: None,
                parameters: Vec::new(),
            },
            "client-1",
            "qBittorrent",
            "qbittorrent",
        )];
        assert_eq!(history[0].seeding, None);

        client.apply(&mut history);
        let seeding = history[0]
            .seeding
            .clone()
            .expect("the history row should inherit the queue observation");
        assert_eq!(seeding.can_remove, Some(false));
        assert_eq!(seeding.seed_ratio, Some(0.9));
        assert_eq!(seeding.seed_time_seconds, Some(4_200));
        assert_eq!(seeding.is_private, Some(true));
    }

    #[test]
    fn an_observation_never_outlives_the_listing_that_produced_it() {
        let client = SeedingObservationCache::default();

        let mut torrent = queue_filter_item(DownloadItemState::Seeding);
        torrent.client_item_id = "abcdef0123456789abcdef0123456789abcdef01".to_string();
        torrent.can_remove = Some(true);
        torrent.torrent = Some(PluginTorrentItem {
            seed_ratio: Some(3.0),
            ..PluginTorrentItem::default()
        });
        client.record(std::slice::from_ref(&torrent));

        // The torrent is gone from the next listing: its observation goes with
        // it, so the gate holds rather than acting on last cycle's answer.
        let other = queue_filter_item(DownloadItemState::Downloading);
        client.record(&[other]);

        let mut history = vec![map_history_item_from_completed(
            PluginCompletedDownload {
                client_item_id: torrent.client_item_id.clone(),
                download_id: None,
                info_hash: Some(torrent.client_item_id.clone()),
                name: "Removed Torrent".to_string(),
                release_name: None,
                dest_dir: "/downloads".to_string(),
                category: None,
                output_kind: None,
                content_paths: Vec::new(),
                size_bytes: Some(1),
                completed_at: None,
                parameters: Vec::new(),
            },
            "client-1",
            "qBittorrent",
            "qbittorrent",
        )];
        client.apply(&mut history);
        assert_eq!(history[0].seeding, None);
    }

    fn sample_request() -> DownloadClientAddRequest {
        DownloadClientAddRequest {
            search_facet: None,
            title: scryer_domain::Title {
                id: "title-1".to_string(),
                name: "Example".to_string(),
                facet: scryer_domain::MediaFacet::Series,
                library_id: scryer_domain::default_library_id_for_facet(
                    &scryer_domain::MediaFacet::Series,
                ),
                monitored: true,
                tags: Vec::new(),
                canonical_tags: vec![],
                external_ids: Vec::new(),
                root_folder_id: scryer_domain::root_folder_id_for_path("/data/series"),
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
                aliases: Vec::new(),
                tagged_aliases: Vec::new(),
                metadata_language: None,
                metadata_fetched_at: None,
                min_availability: None,
                digital_release_date: None,
                folder_path: None,
            },
            purpose: scryer_application::DownloadSubmissionPurpose::Standard,
            download_id: None,
            source_hint: Some("https://tracker.example/release.torrent".to_string()),
            staged_nzb: None,
            resolved_download_artifact: None,
            source_kind: Some(DownloadSourceKind::TorrentFile),
            source_title: Some("Example.Release.torrent".to_string()),
            source_password: None,
            category: Some("scryer-series".to_string()),
            queue_priority: Some("first".to_string()),
            download_directory: Some("/downloads/series".to_string()),
            release_title: Some("Example.Release".to_string()),
            indexer_name: Some("Torrent Indexer".to_string()),
            indexer_id: None,
            info_hash_hint: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
            seed_goal_ratio: Some(1.5),
            seed_goal_seconds: Some(3661),
            tracker_min_seed_ratio: None,
            tracker_min_seed_time_minutes: None,
            season_pack_seed_ratio: None,
            season_pack_seed_time_minutes: None,
            is_recent: Some(true),
            season_pack: Some(false),
        }
    }

    #[test]
    fn add_response_forwards_plugin_info_hash() {
        let request = sample_request();
        let grab = map_add_response_to_grab_result(
            PluginDownloadClientAddResponse {
                client_item_id: "native-item-id".to_string(),
                info_hash: Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string()),
            },
            &request,
            "qbittorrent",
        );

        assert_eq!(grab.job_id, "native-item-id");
        assert_eq!(
            grab.info_hash.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
    }

    #[test]
    fn add_response_falls_back_to_hash_shaped_torrent_item_id() {
        let request = sample_request();
        let grab = map_add_response_to_grab_result(
            PluginDownloadClientAddResponse {
                client_item_id: "ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string(),
                info_hash: None,
            },
            &request,
            "qbittorrent",
        );

        assert_eq!(
            grab.info_hash.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
    }

    #[test]
    fn build_plugin_add_request_populates_v11_torrent_fields() {
        let request = sample_request();
        let plugin_request = build_plugin_add_request(
            &request,
            DownloadSourceKind::TorrentFile,
            ResolvedTorrentSource {
                download_url: request.source_hint.clone(),
                magnet_uri: None,
                torrent_bytes_base64: Some("dG9ycmVudA==".to_string()),
                torrent_url: request.source_hint.clone(),
                torrent_file_name: Some("Example.Release.torrent".to_string()),
                torrent_content_type: Some("application/x-bittorrent".to_string()),
                nzb_bytes_base64: None,
                nzb_file_name: None,
                nzb_content_type: None,
            },
        );

        assert_eq!(plugin_request.source.kind, DownloadInputKind::TorrentBytes);
        assert_eq!(
            plugin_request.source.torrent_content_type.as_deref(),
            Some("application/x-bittorrent")
        );
        assert_eq!(
            plugin_request.release.info_hash_v1.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
        assert_eq!(
            plugin_request.release.info_hash_hint.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
        assert_eq!(plugin_request.routing.isolation.len(), 4);
        assert_eq!(
            plugin_request
                .torrent
                .as_ref()
                .and_then(|torrent| torrent.queue_placement),
            Some(PluginTorrentQueuePlacement::First)
        );
        assert_eq!(
            plugin_request
                .torrent
                .as_ref()
                .map(|torrent| torrent.source_preference.clone()),
            Some(vec![
                DownloadInputKind::TorrentBytes,
                DownloadInputKind::TorrentUrl,
                DownloadInputKind::TorrentFile,
            ])
        );
    }

    #[test]
    fn command_add_permanent_plugin_error_is_rejected_without_losing_its_message() {
        let error = decode_download_add_result::<()>(
            PluginResult::Err(PluginError {
                details: None,
                code: PluginErrorCode::Permanent,
                public_message: "rTorrent add requires an info hash from the release".to_string(),
                debug_message: None,
                retry_after_seconds: None,
            }),
            "download add",
        )
        .expect_err("permanent plugin error should reject the submission");

        assert!(!error.is_retryable_download_submit_failure());
        assert!(matches!(
            error,
            AppError::DownloadSubmitRejected(message)
                if message.contains("rTorrent add requires an info hash from the release")
        ));
    }

    #[test]
    fn command_add_transient_plugin_errors_remain_retryable() {
        for code in [
            PluginErrorCode::RateLimited,
            PluginErrorCode::UpstreamUnavailable,
            PluginErrorCode::Temporary,
        ] {
            let error = decode_download_add_result::<()>(
                PluginResult::Err(PluginError {
                    details: None,
                    code,
                    public_message: "client is temporarily unavailable".to_string(),
                    debug_message: None,
                    retry_after_seconds: None,
                }),
                "download add",
            )
            .expect_err("transient plugin error should fail the submission");

            assert!(error.is_retryable_download_submit_failure());
        }
    }

    #[test]
    fn add_permanent_plugin_error_is_rejected() {
        let error = decode_download_add_result::<PluginDownloadClientAddResponse>(
            PluginResult::Err(PluginError {
                details: None,
                code: PluginErrorCode::Permanent,
                public_message: "download source is invalid".to_string(),
                debug_message: None,
                retry_after_seconds: None,
            }),
            "download add",
        )
        .expect_err("a permanent plugin error should reject the submission");

        assert!(!error.is_retryable_download_submit_failure());
        assert!(matches!(error, AppError::DownloadSubmitRejected(_)));
    }

    #[test]
    fn build_plugin_add_request_prefers_magnet_after_redirect() {
        let request = sample_request();
        let plugin_request = build_plugin_add_request(
            &request,
            DownloadSourceKind::TorrentFile,
            ResolvedTorrentSource {
                download_url: None,
                magnet_uri: Some(
                    "magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01".to_string(),
                ),
                torrent_bytes_base64: None,
                torrent_url: None,
                torrent_file_name: None,
                torrent_content_type: None,
                nzb_bytes_base64: None,
                nzb_file_name: None,
                nzb_content_type: None,
            },
        );

        assert_eq!(plugin_request.source.kind, DownloadInputKind::MagnetUri);
        assert_eq!(
            plugin_request
                .torrent
                .as_ref()
                .map(|torrent| torrent.source_preference.clone()),
            Some(vec![DownloadInputKind::MagnetUri])
        );
    }

    #[test]
    fn build_plugin_add_request_preserves_nzb_url_without_torrent_projection() {
        let mut request = sample_request();
        request.source_hint = Some("https://indexer.example/download/release.nzb".to_string());
        request.source_kind = Some(DownloadSourceKind::NzbUrl);
        request.source_title = Some("Example.Release.nzb".to_string());
        request.info_hash_hint = None;
        let plugin_request = build_plugin_add_request(
            &request,
            DownloadSourceKind::NzbUrl,
            ResolvedTorrentSource {
                download_url: request.source_hint.clone(),
                magnet_uri: None,
                torrent_bytes_base64: None,
                torrent_url: None,
                torrent_file_name: None,
                torrent_content_type: None,
                nzb_bytes_base64: None,
                nzb_file_name: None,
                nzb_content_type: None,
            },
        );

        assert_eq!(plugin_request.source.kind, DownloadInputKind::NzbUrl);
        assert_eq!(
            plugin_request.source.download_url.as_deref(),
            Some("https://indexer.example/download/release.nzb")
        );
        assert_eq!(plugin_request.source.torrent_url, None);
        assert!(plugin_request.torrent.is_none());
    }

    #[test]
    fn build_plugin_add_request_populates_staged_nzb_fields() {
        let mut request = sample_request();
        request.source_hint = Some("https://indexer.example/download/release.nzb".to_string());
        request.source_kind = Some(DownloadSourceKind::NzbUrl);
        request.source_title = Some("Example.Release".to_string());
        request.info_hash_hint = None;
        let plugin_request = build_plugin_add_request(
            &request,
            DownloadSourceKind::NzbUrl,
            ResolvedTorrentSource {
                download_url: request.source_hint.clone(),
                magnet_uri: None,
                torrent_bytes_base64: None,
                torrent_url: None,
                torrent_file_name: None,
                torrent_content_type: None,
                nzb_bytes_base64: Some("bmti".to_string()),
                nzb_file_name: Some("Example.Release.nzb".to_string()),
                nzb_content_type: Some("application/x-nzb".to_string()),
            },
        );

        assert_eq!(plugin_request.source.kind, DownloadInputKind::Nzb);
        assert_eq!(
            plugin_request.source.nzb_bytes_base64.as_deref(),
            Some("bmti")
        );
        assert_eq!(
            plugin_request.source.nzb_file_name.as_deref(),
            Some("Example.Release.nzb")
        );
        assert_eq!(
            plugin_request.source.nzb_content_type.as_deref(),
            Some("application/x-nzb")
        );
        assert_eq!(plugin_request.source.torrent_url, None);
        assert!(plugin_request.torrent.is_none());
    }

    #[test]
    fn mark_imported_post_import_isolation_matches_legacy_value() {
        let entries = build_isolation_entries(Some("series-cat"));
        assert_eq!(entries.len(), 4);
        assert!(
            entries
                .iter()
                .any(|entry| entry.mode == DownloadIsolationMode::Category)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.mode == DownloadIsolationMode::Label)
        );
    }

    #[test]
    fn completed_history_fallback_maps_to_completed_queue_item() {
        let queue_item = map_history_item_from_completed(
            PluginCompletedDownload {
                client_item_id: "native-1".to_string(),
                download_id: None,
                info_hash: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
                name: "Example Release".to_string(),
                release_name: None,
                dest_dir: "/downloads/series".to_string(),
                category: Some("series".to_string()),
                output_kind: None,
                content_paths: vec!["/downloads/series/Example.Release.mkv".to_string()],
                size_bytes: Some(1234),
                completed_at: Some("2026-05-02T00:00:00Z".to_string()),
                parameters: vec![],
            },
            "client-1",
            "qBittorrent",
            "qbittorrent",
        );

        assert_eq!(
            queue_item.id,
            "qbittorrent:abcdef0123456789abcdef0123456789abcdef01"
        );
        assert_eq!(queue_item.title_name, "Example Release");
        assert_eq!(queue_item.client_name, "qBittorrent");
        assert_eq!(queue_item.state, DownloadQueueState::Completed);
        assert_eq!(queue_item.category.as_deref(), Some("series"));
        assert_eq!(queue_item.progress_percent, 100);
        assert_eq!(queue_item.remaining_seconds, Some(0));
    }

    #[test]
    fn plugin_queue_and_history_keep_their_current_identity_projections() {
        const INFO_HASH: &str = "abcdef0123456789abcdef0123456789abcdef01";
        const DOWNLOAD_ID: &str = "scryer-download:plugin-token";

        let queue_item = map_queue_item(
            PluginDownloadItem {
                client_item_id: "native-plugin-item".to_string(),
                download_id: Some(DOWNLOAD_ID.to_string()),
                info_hash: Some(INFO_HASH.to_string()),
                title: "Plugin Queue Item".to_string(),
                state: DownloadItemState::Downloading,
                message: None,
                category: Some("series".to_string()),
                remote_output_path: None,
                torrent: None,
                total_size_bytes: Some(2048),
                remaining_size_bytes: Some(1024),
                eta_seconds: Some(60),
                progress_percent: Some(50),
                can_move_files: None,
                can_remove: None,
                removed: None,
                raw_state: None,
                completed_at: None,
            },
            "client-1",
            "Plugin Client",
            "plugin-client",
        );
        assert_eq!(queue_item.id, format!("plugin-client:{INFO_HASH}"));
        assert_eq!(queue_item.download_id.as_deref(), Some(DOWNLOAD_ID));
        assert_eq!(queue_item.download_client_item_id, "native-plugin-item");
        assert!(!queue_item.is_scryer_origin);

        let history_item = map_history_item_from_completed(
            PluginCompletedDownload {
                client_item_id: "native-plugin-item".to_string(),
                download_id: Some(DOWNLOAD_ID.to_string()),
                info_hash: Some(INFO_HASH.to_string()),
                name: "Plugin Queue Item".to_string(),
                release_name: None,
                dest_dir: "/downloads/series".to_string(),
                category: Some("series".to_string()),
                output_kind: None,
                content_paths: vec![],
                size_bytes: Some(2048),
                completed_at: Some("2026-05-02T00:00:00Z".to_string()),
                parameters: vec![],
            },
            "client-1",
            "Plugin Client",
            "plugin-client",
        );
        assert_eq!(history_item.id, format!("plugin-client:{INFO_HASH}"));
        assert_eq!(history_item.download_id.as_deref(), Some(DOWNLOAD_ID));
        assert_eq!(history_item.download_client_item_id, INFO_HASH);
        assert!(!history_item.is_scryer_origin);
    }

    #[test]
    fn legacy_plugin_completion_without_release_name_remains_readable() {
        let legacy: PluginCompletedDownload = serde_json::from_value(serde_json::json!({
            "client_item_id": "legacy-qbit-item",
            "info_hash": "abcdef0123456789abcdef0123456789abcdef01",
            "name": "Mutable qBit Display Label",
            "dest_dir": "/downloads/legacy-qbit-item"
        }))
        .expect("deserialize pre-release_name plugin payload");

        let completed = map_completed_download(legacy, "client-1", "qbittorrent");
        assert_eq!(completed.name, "Mutable qBit Display Label");
        assert_eq!(completed.release_name, None);
    }

    #[test]
    fn queue_item_falls_back_to_nested_torrent_info_hash_for_qbit_facades() {
        let queue_item = map_queue_item(
            PluginDownloadItem {
                client_item_id: "native-id-1".to_string(),
                download_id: None,
                info_hash: None,
                title: "Decypharr Queue Item".to_string(),
                state: DownloadItemState::Seeding,
                message: None,
                category: Some("series".to_string()),
                remote_output_path: Some("/downloads/series".to_string()),
                torrent: Some(PluginTorrentItem {
                    info_hash_v1: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
                    seed_ratio: Some(0.4),
                    seed_time_seconds: Some(900),
                    is_private: Some(true),
                    uploaded_bytes: Some(819),
                    ..PluginTorrentItem::default()
                }),
                total_size_bytes: Some(2048),
                remaining_size_bytes: Some(0),
                eta_seconds: Some(0),
                progress_percent: Some(100),
                // Data is complete, the seeding obligation is not discharged.
                // These two answer different questions and must not be equal.
                can_move_files: Some(true),
                can_remove: Some(false),
                removed: Some(false),
                raw_state: Some("uploading".to_string()),
                completed_at: Some("2026-05-02T00:00:00Z".to_string()),
            },
            "client-1",
            "Decypharr qBit",
            "qbittorrent",
        );

        assert_eq!(
            queue_item.id,
            "qbittorrent:abcdef0123456789abcdef0123456789abcdef01"
        );
        assert_eq!(
            queue_item.download_client_item_id,
            "native-id-1".to_string()
        );
        assert_eq!(queue_item.state, DownloadQueueState::Completed);
        assert_eq!(queue_item.category.as_deref(), Some("series"));

        let seeding = queue_item
            .seeding
            .expect("a torrent item should carry its seeding observation");
        assert_eq!(seeding.can_remove, Some(false));
        assert_eq!(seeding.can_move_files, Some(true));
        assert_eq!(seeding.seed_ratio, Some(0.4));
        assert_eq!(seeding.seed_time_seconds, Some(900));
        assert_eq!(seeding.is_private, Some(true));
        assert_eq!(seeding.uploaded_bytes, Some(819));
        assert_eq!(
            seeding.completed_at.as_deref(),
            Some("2026-05-02T00:00:00Z")
        );
        // Goals are joined later from the persisted resolution, never invented
        // here.
        assert_eq!(seeding.seed_goal_ratio, None);
        assert_eq!(seeding.seed_goal_seconds, None);
        assert!(!seeding.never_remove);
    }

    fn client_descriptor(id: &str, remove_with_data: bool) -> PluginDescriptor {
        let mut descriptor = crate::seeding_trust::descriptor(id, "1.1.0");
        if let scryer_plugin_sdk::ProviderDescriptor::DownloadClient(provider) =
            &mut descriptor.provider
        {
            provider.capabilities.remove_with_data = remove_with_data;
        }
        descriptor
    }

    /// rTorrent and DownloadStation answer a data-removal request with
    /// `Unsupported` — they expect Scryer to delete their files itself — so a
    /// verbatim `remove_data: true` would fail the whole control call and leave
    /// the queue entry in the client forever. Downgrading to an entry-only
    /// removal is what Sonarr ends up doing for the same clients, and it is the
    /// request shape their `Unsupported` guard lets through.
    #[test]
    fn a_client_that_cannot_delete_data_is_asked_for_an_entry_only_removal() {
        let request = remove_control_request(
            &client_descriptor("rtorrent", false),
            "torrent-1",
            true,
            true,
        );

        assert!(
            !request.remove_data,
            "a plugin that declares it cannot delete data must never be asked to"
        );
        assert!(matches!(request.action, DownloadControlAction::Remove));
        assert_eq!(request.client_item_id, "torrent-1");
        assert!(request.is_history);
    }

    #[test]
    fn a_client_that_can_delete_data_still_obeys_the_callers_policy() {
        let descriptor = client_descriptor("qbittorrent", true);

        let asked = remove_control_request(&descriptor, "torrent-2", true, true);
        assert!(asked.remove_data, "the capability alone does not force it");

        // The caller decides *whether* to delete data; the capability only
        // decides whether the client can be asked.
        let not_asked = remove_control_request(&descriptor, "torrent-2", true, false);
        assert!(!not_asked.remove_data);
    }

    /// The whole point of the floor: what the gate reads for a stale plugin.
    ///
    /// A pre-audit qBittorrent reports `can_remove: Some(true)` on every item;
    /// the observation that reaches the domain queue row — and from there the
    /// seeding gate — must carry `None` instead, which is the gate's
    /// `no_resolved_goals_and_client_verdict_unknown` hold. Everything the
    /// client actually measured (ratio, seed time, private flag) still comes
    /// through: the floor distrusts the *verdicts*, not the observations.
    #[test]
    fn a_below_floor_plugins_verdicts_never_reach_the_domain_observation() {
        let descriptor = crate::seeding_trust::descriptor("qbittorrent", "1.0.5");
        let mut items = vec![PluginDownloadItem {
            torrent: Some(PluginTorrentItem {
                seed_ratio: Some(0.1),
                seed_time_seconds: Some(30),
                is_private: Some(false),
                ..PluginTorrentItem::default()
            }),
            can_move_files: Some(true),
            can_remove: Some(true),
            ..queue_filter_item(DownloadItemState::Seeding)
        }];

        apply_seeding_trust_floor(&descriptor, &mut items);
        let queue_item = map_queue_item(
            items.remove(0),
            "client-1",
            "Stale qBit",
            descriptor.provider_type(),
        );

        let seeding = queue_item
            .seeding
            .expect("the measured observation still stands");
        assert_eq!(seeding.can_remove, None);
        assert_eq!(seeding.can_move_files, None);
        assert_eq!(seeding.seed_ratio, Some(0.1));
        assert_eq!(seeding.seed_time_seconds, Some(30));
        assert_eq!(seeding.is_private, Some(false));
    }

    /// The same item from the audited build keeps its verdicts, so the floor
    /// cannot be read as "never trust a plugin".
    #[test]
    fn an_at_floor_plugins_verdicts_reach_the_domain_observation_intact() {
        let descriptor = crate::seeding_trust::descriptor("qbittorrent", "1.1.0");
        let mut items = vec![PluginDownloadItem {
            can_move_files: Some(true),
            can_remove: Some(true),
            ..queue_filter_item(DownloadItemState::Seeding)
        }];

        apply_seeding_trust_floor(&descriptor, &mut items);
        let queue_item = map_queue_item(
            items.remove(0),
            "client-1",
            "Current qBit",
            descriptor.provider_type(),
        );

        let seeding = queue_item
            .seeding
            .expect("client verdicts are an observation");
        assert_eq!(seeding.can_remove, Some(true));
        assert_eq!(seeding.can_move_files, Some(true));
    }

    #[test]
    fn queue_item_prefers_explicit_plugin_download_identity() {
        let queue_item = map_queue_item(
            PluginDownloadItem {
                client_item_id: "native-id-3".to_string(),
                download_id: Some("scryer-download:plugin-1".to_string()),
                info_hash: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
                title: "Plugin Queue Item".to_string(),
                state: DownloadItemState::Downloading,
                message: None,
                category: Some("series".to_string()),
                remote_output_path: Some("/downloads/series".to_string()),
                torrent: None,
                total_size_bytes: Some(2048),
                remaining_size_bytes: Some(1024),
                eta_seconds: Some(60),
                progress_percent: Some(50),
                // Still downloading: nothing is complete and the client has no
                // verdict to give.
                can_move_files: Some(false),
                can_remove: None,
                removed: Some(false),
                raw_state: None,
                completed_at: None,
            },
            "client-1",
            "Plugin Client",
            "plugin-client",
        );

        assert_eq!(
            queue_item.download_id.as_deref(),
            Some("scryer-download:plugin-1")
        );
        let seeding = queue_item
            .seeding
            .expect("an item with a client verdict should carry an observation");
        assert_eq!(seeding.can_remove, None);
        assert_eq!(seeding.can_move_files, Some(false));
        assert_eq!(seeding.seed_ratio, None);
        assert_eq!(seeding.is_private, None);
    }

    #[test]
    fn completed_download_uses_info_hash_identity_without_replacing_live_handle() {
        let info_hash = "fedcba9876543210fedcba9876543210fedcba98";
        let completed = map_completed_download(
            PluginCompletedDownload {
                client_item_id: "native-id-2".to_string(),
                download_id: None,
                info_hash: Some(info_hash.to_string()),
                name: "Decypharr Completed".to_string(),
                release_name: None,
                dest_dir: "/downloads/movies".to_string(),
                category: Some("movies".to_string()),
                output_kind: None,
                content_paths: vec!["/downloads/movies/Decypharr.Completed.mkv".to_string()],
                size_bytes: Some(4096),
                completed_at: Some("2026-05-03T00:00:00Z".to_string()),
                parameters: vec![("source".to_string(), "decypharr".to_string())],
            },
            "client-1",
            "qbittorrent",
        );

        assert_eq!(completed.download_client_item_id, "native-id-2");
        assert_eq!(completed.download_id.as_deref(), Some(info_hash));
        assert_eq!(
            completed.dest_dir,
            "/downloads/movies/Decypharr.Completed.mkv"
        );
        assert_eq!(
            completed.parameters,
            vec![("source".to_string(), "decypharr".to_string())]
        );
    }

    #[test]
    fn completed_download_uses_single_directory_content_path_for_series_pack() {
        for content_paths in [
            vec!["/downloads/series/Show Season 1".to_string()],
            vec![
                "   ".to_string(),
                "/downloads/series/Show Season 1".to_string(),
            ],
        ] {
            let completed = map_completed_download(
                PluginCompletedDownload {
                    client_item_id: "series-pack-id".to_string(),
                    download_id: None,
                    info_hash: None,
                    name: "Show Season 1".to_string(),
                    release_name: None,
                    dest_dir: "/downloads/series".to_string(),
                    category: Some("series".to_string()),
                    output_kind: None,
                    content_paths,
                    size_bytes: None,
                    completed_at: None,
                    parameters: Vec::new(),
                },
                "client-1",
                "plugin-client",
            );

            assert_eq!(completed.dest_dir, "/downloads/series/Show Season 1");
        }
    }

    #[test]
    fn completed_download_keeps_reported_root_for_zero_or_multiple_content_paths() {
        for content_paths in [
            Vec::new(),
            vec![
                "/downloads/series/Show.S01E01.mkv".to_string(),
                "/downloads/series/Show.S01E02.mkv".to_string(),
            ],
        ] {
            let completed = map_completed_download(
                PluginCompletedDownload {
                    client_item_id: "native-id-3".to_string(),
                    download_id: None,
                    info_hash: None,
                    name: "Show Season 1".to_string(),
                    release_name: None,
                    dest_dir: "/downloads/series/Show Season 1".to_string(),
                    category: Some("series".to_string()),
                    output_kind: None,
                    content_paths,
                    size_bytes: None,
                    completed_at: None,
                    parameters: Vec::new(),
                },
                "client-1",
                "plugin-client",
            );

            assert_eq!(completed.dest_dir, "/downloads/series/Show Season 1");
        }
    }
}

#[cfg(test)]
mod component_routing_tests {
    use super::*;
    use crate::wasmtime_host::download_client_component_host::tests::{
        FIXTURE_STATE_VALUE, fixture_component,
    };

    fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: "fixture-download-client".to_string(),
            name: "Fixture Download Client".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: crate::types::SDK_VERSION.to_string(),
            sdk_constraint: crate::types::current_sdk_constraint(),
            socket_permissions: Vec::new(),
            provider: crate::types::ProviderDescriptor::DownloadClient(
                crate::types::DownloadClientDescriptor {
                    provider_type: "fixture-download-client".to_string(),
                    provider_aliases: Vec::new(),
                    config_fields: Vec::new(),
                    default_base_url: None,
                    allowed_hosts: Vec::new(),
                    accepted_inputs: Vec::new(),
                    isolation_modes: Vec::new(),
                    capabilities: crate::types::DownloadClientCapabilities::default(),
                },
            ),
        }
    }

    fn command_host() -> CommandHost {
        CommandHost::with_archive_provider(
            "fixture-download-client".to_string(),
            std::collections::BTreeMap::new(),
            Vec::new(),
            DOWNLOAD_CLIENT_PLUGIN_TIMEOUT,
            None,
            None,
        )
    }

    fn component_client(wasm: Vec<u8>) -> WasmDownloadClient {
        WasmDownloadClient::new_component(
            wasm,
            descriptor(),
            "config-1".to_string(),
            "Fixture".to_string(),
            command_host(),
        )
    }

    /// A component client answers a real `DownloadClient` trait call through
    /// the component host, and the client-scoped `CommandHost` reaches the
    /// guest: the value only exists in this client's host state.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_component_client_answers_through_the_component_host() {
        let client = component_client(fixture_component());

        let message = client
            .test_connection()
            .await
            .expect("the component client must complete a test-connection exchange");

        assert_eq!(message, format!("host-call:{FIXTURE_STATE_VALUE}"));
    }

    /// The client-scoped host survives between trait calls, so the session
    /// value a client stores on one operation is readable on the next — the
    /// cookie-persistence contract seen from the adapter rather than from the
    /// host.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_component_clients_host_state_survives_between_operations() {
        let client = component_client(fixture_component());

        client
            .test_connection()
            .await
            .expect("first exchange must succeed");
        let message = client
            .test_connection()
            .await
            .expect("second exchange must succeed");

        assert_eq!(message, format!("host-call:{FIXTURE_STATE_VALUE}"));
    }
}
