//! WebSocket subscription bridge for the Weaver download client.
//!
//! Connects to Weaver's GraphQL WebSocket endpoint using the `graphql-ws`
//! protocol and receives real-time job snapshots. These are mapped to
//! scryer's `DownloadQueueItem` and fed into scryer's tracked-download
//! runtime.
//!
//! If the WebSocket connection fails repeatedly, the bridge automatically
//! falls back to GraphQL HTTP polling so the UI stays up-to-date. When the
//! WebSocket reconnects the poller is stopped and real-time push resumes.

use std::collections::HashSet;

use futures_util::{SinkExt, StreamExt};
use scryer_application::{
    AppResult, DownloadClient, DownloadClientRemotePathMapping,
    apply_remote_path_mappings_to_completed_download, parse_download_client_remote_path_mappings,
    tracked_downloads::{
        TrackedDownloadSnapshotIngestHandle, TrackedDownloadSnapshotScope,
        TrackedDownloadSnapshotUpdate,
    },
};
use scryer_domain::{CompletedDownload, DownloadClientConfig, DownloadQueueItem};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{ClientRequestBuilder, Message};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::weaver::{
    WeaverDownloadClient, WeaverQueueItem, WeaverQueueState, weaver_item_to_queue_item,
};

const QUEUE_SNAPSHOTS_QUERY: &str = r#"
    subscription {
        queueSnapshots {
            items {
                id
                name
                state
                error
                progressPercent
                totalBytes
                downloadedBytes
                failedBytes
                health
                category
                outputDir
                createdAt
                clientRequestId
                attributes { key value }
                attention { code message }
            }
            latestCursor
        }
    }
"#;

const QUEUE_EVENTS_QUERY: &str = r#"
    subscription($after: String) {
        queueEvents(after: $after) {
            cursor
            kind
            itemId
            item {
                id
                name
                state
                error
                progressPercent
                totalBytes
                downloadedBytes
                failedBytes
                health
                category
                outputDir
                createdAt
                clientRequestId
                attributes { key value }
                attention { code message }
            }
        }
    }
"#;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueSnapshotsPayload {
    queue_snapshots: QueueSnapshotPayload,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueSnapshotPayload {
    items: Vec<WeaverQueueItem>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueEventsPayload {
    queue_events: QueueEventPayload,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueEventPayload {
    cursor: String,
    kind: String,
    item: Option<WeaverQueueItem>,
}

/// Number of consecutive WebSocket failures before falling back to HTTP polling.
const POLL_FALLBACK_THRESHOLD: u32 = 3;

/// Interval between HTTP polls when in fallback mode (seconds).
const POLL_FALLBACK_INTERVAL_SECS: u64 = 2;
const POLL_FALLBACK_RECENT_ACTIVITY_LIMIT: usize = 100;

/// Interval for the always-on reconciliation poll that runs alongside the
/// WebSocket subscription (seconds).
///
/// The subscription is the bridge's ONLY realtime source, and when it is
/// active the generic download queue poller excludes Weaver entirely (the
/// bridge supervisor marks it bridge-covered). A dropped
/// `ITEM_COMPLETED` — Weaver's replay evict/consume race, or any gap while the
/// socket looked healthy — therefore used to be unrecoverable: the tracked
/// download sat at Downloading forever, the import never ran, the wanted item
/// stayed unfilled, and the acquisition sweep re-grabbed the same release only
/// for Weaver to reject the re-submission as DUPLICATE_BLOCKED. The 2s HTTP
/// fallback never engaged because it only starts after consecutive CONNECT
/// failures — a lossy-but-connected socket kept it off.
///
/// This reconcile loop runs for the bridge's whole lifetime and republishes
/// queue + recent history as a DELTA snapshot (upsert-only, never prunes), so
/// it cannot fight the live event stream — it only backfills whatever the
/// stream lost. Override via `SCRYER_WEAVER_BRIDGE_RECONCILE_INTERVAL_SECS`;
/// `0` disables it.
const BRIDGE_RECONCILE_INTERVAL_SECS: u64 = 30;
const BRIDGE_RECONCILE_INTERVAL_ENV: &str = "SCRYER_WEAVER_BRIDGE_RECONCILE_INTERVAL_SECS";

fn bridge_reconcile_interval() -> Option<std::time::Duration> {
    let secs = std::env::var(BRIDGE_RECONCILE_INTERVAL_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(BRIDGE_RECONCILE_INTERVAL_SECS);
    if secs == 0 {
        None
    } else {
        Some(std::time::Duration::from_secs(secs))
    }
}

#[derive(Clone)]
pub struct WeaverSubscriptionBridgeClient {
    client_id: String,
    client_name: String,
    client_type: String,
    ws_url: String,
    api_key: Option<String>,
    download_client: WeaverDownloadClient,
    remote_path_mappings: Option<Vec<DownloadClientRemotePathMapping>>,
}

impl WeaverSubscriptionBridgeClient {
    pub fn from_config(config: &DownloadClientConfig) -> AppResult<Self> {
        Self::from_config_with_proxy(config, None)
    }

    /// Build a bridge client whose HTTP polling fallback egresses through the
    /// operator's assigned proxy.
    ///
    /// The subscription itself is a `graphql-ws` WebSocket dialled outside
    /// reqwest, so no proxy applies to it. The fallback is ordinary HTTP and
    /// does honour the assignment; a proxy that cannot produce a client (a
    /// tunnel, today) fails here, which makes the client un-bridgeable and
    /// leaves the router's own proxied polling in charge.
    pub fn from_config_with_proxy(
        config: &DownloadClientConfig,
        proxy_config: Option<&scryer_domain::ProxyConfig>,
    ) -> AppResult<Self> {
        let http_client = super::native_download_client_http_client(&config.name, proxy_config)?;
        let download_client =
            WeaverDownloadClient::from_config(config)?.with_http_client(http_client);
        let remote_path_mappings =
            match parse_download_client_remote_path_mappings(&config.config_json) {
                Ok(mappings) => Some(mappings),
                Err(error) => {
                    warn!(
                        client_id = %config.id,
                        client = %config.name,
                        error = %error,
                        "failed to parse remote path mappings for weaver subscription bridge"
                    );
                    None
                }
            };
        Ok(Self {
            client_id: config.id.clone(),
            client_name: config.name.clone(),
            client_type: config.client_type.clone(),
            ws_url: download_client.ws_url(),
            api_key: download_client.api_key().map(str::to_string),
            download_client,
            remote_path_mappings,
        })
    }

    fn stamp_queue_item(&self, item: &mut DownloadQueueItem) {
        item.client_id.clone_from(&self.client_id);
        item.client_name.clone_from(&self.client_name);
        item.client_type.clone_from(&self.client_type);
    }

    fn map_queue_item(&self, job: &WeaverQueueItem) -> DownloadQueueItem {
        let mut item = weaver_item_to_queue_item(job);
        self.stamp_queue_item(&mut item);
        item
    }

    fn stamp_completed_download(&self, item: &mut CompletedDownload) {
        item.client_id.clone_from(&self.client_id);
        item.client_type.clone_from(&self.client_type);
        if let Some(mappings) = self.remote_path_mappings.as_deref() {
            apply_remote_path_mappings_to_completed_download(item, mappings);
        }
    }
}

/// Start a WebSocket subscription bridge to Weaver.
///
/// This replaces generic HTTP queue polling when Weaver is the active download
/// client. It connects to Weaver's queue subscriptions and:
///
/// 1. Maps incoming job snapshots to `Vec<DownloadQueueItem>`
/// 2. Looks up exact completed-download rows when Weaver reports completion
/// 3. Feeds observations into Scryer's tracked-download runtime
///
/// Reconnects automatically on disconnect with exponential backoff.
/// After [`POLL_FALLBACK_THRESHOLD`] consecutive failures the bridge starts
/// a GraphQL HTTP polling loop so that download-queue data keeps flowing to
/// the UI. When the WebSocket reconnects the poller is stopped automatically.
pub async fn start_weaver_subscription_bridge(
    token: CancellationToken,
    bridge_client: WeaverSubscriptionBridgeClient,
    ingest: TrackedDownloadSnapshotIngestHandle,
) {
    let mut backoff_secs: u64 = 5;
    let max_backoff: u64 = 60;
    let mut consecutive_failures: u32 = 0;
    let mut last_cursor: Option<String> = None;
    // Token used to stop fallback polling when WS reconnects.
    let mut poll_cancel: Option<CancellationToken> = None;

    // Loss-tolerance reconcile: runs for the bridge's whole lifetime,
    // independent of WebSocket health, and dies with the bridge token.
    if let Some(interval_duration) = bridge_reconcile_interval() {
        info!(
            interval_secs = interval_duration.as_secs(),
            "starting weaver reconcile loop (subscription loss tolerance)"
        );
        tokio::spawn(run_reconcile_loop(
            bridge_client.clone(),
            ingest.clone(),
            token.child_token(),
            interval_duration,
        ));
    }

    loop {
        if token.is_cancelled() {
            info!("weaver subscription bridge shutting down");
            return;
        }

        info!(
            url = bridge_client.ws_url.as_str(),
            client_id = bridge_client.client_id.as_str(),
            "connecting to weaver WebSocket"
        );

        match run_subscription(&bridge_client, &ingest, &token, &mut last_cursor).await {
            SubscriptionOutcome::Shutdown => {
                stop_fallback_poller(&mut poll_cancel);
                info!("weaver subscription bridge stopped cleanly");
                return;
            }
            SubscriptionOutcome::ConnectError(error) => {
                consecutive_failures += 1;
                warn!(
                    error = %error,
                    backoff_secs,
                    consecutive_failures,
                    "weaver WebSocket connect failed; retrying"
                );

                // Start fallback polling after repeated connect failures.
                if consecutive_failures >= POLL_FALLBACK_THRESHOLD && poll_cancel.is_none() {
                    info!("weaver WebSocket unreliable — starting GraphQL HTTP polling fallback");
                    let poll_token = token.child_token();
                    poll_cancel = Some(poll_token.clone());
                    tokio::spawn(run_fallback_poller(
                        bridge_client.clone(),
                        ingest.clone(),
                        poll_token,
                    ));
                }
            }
            SubscriptionOutcome::Disconnected(error) => {
                // The subscription *was* working. Reset failure state and stop
                // the poller (if any) on the next successful reconnect — but
                // since we know the server was reachable, reset backoff now
                // and try again quickly.
                warn!(error = %error, "weaver subscription disconnected; reconnecting");
                backoff_secs = 5;
                consecutive_failures = 0;
                stop_fallback_poller(&mut poll_cancel);
            }
        }

        // Exponential backoff before reconnect.
        tokio::select! {
            _ = token.cancelled() => {
                stop_fallback_poller(&mut poll_cancel);
                info!("weaver subscription bridge shutting down during backoff");
                return;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)) => {}
        }
        backoff_secs = (backoff_secs * 2).min(max_backoff);
    }
}

/// Cancel the fallback poller if one is running.
fn stop_fallback_poller(poll_cancel: &mut Option<CancellationToken>) {
    if let Some(cancel) = poll_cancel.take() {
        info!("stopping GraphQL HTTP polling fallback");
        cancel.cancel();
    }
}

/// HTTP polling loop used as fallback when the WebSocket is down.
///
/// Polls Weaver directly every [`POLL_FALLBACK_INTERVAL_SECS`] seconds and
/// broadcasts results through the same channel the subscription uses.
async fn run_fallback_poller(
    bridge_client: WeaverSubscriptionBridgeClient,
    ingest: TrackedDownloadSnapshotIngestHandle,
    token: CancellationToken,
) {
    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(POLL_FALLBACK_INTERVAL_SECS));

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("weaver fallback poller stopped");
                return;
            }
            _ = interval.tick() => {
                match collect_weaver_fallback_items(&bridge_client).await {
                    Ok(items) => {
                        publish_authoritative_weaver_snapshot(&bridge_client, &ingest, items).await;
                    }
                    Err(error) => {
                        warn!(error = %error, "weaver fallback poll failed");
                    }
                }
            }
        }
    }
}

/// Always-on loss-tolerance loop: periodically republish Weaver's queue and
/// recent history as a Delta snapshot so completions whose subscription events
/// were dropped still reach the tracked runtime. See
/// [`BRIDGE_RECONCILE_INTERVAL_SECS`] for why this exists.
async fn run_reconcile_loop(
    bridge_client: WeaverSubscriptionBridgeClient,
    ingest: TrackedDownloadSnapshotIngestHandle,
    token: CancellationToken,
    interval_duration: std::time::Duration,
) {
    let mut interval = tokio::time::interval(interval_duration);
    // The immediate first tick is welcome here: it closes the boot window in
    // which jobs submitted before the WebSocket finished its handshake could
    // complete unobserved.
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("weaver reconcile loop stopped");
                return;
            }
            _ = interval.tick() => {
                match collect_weaver_fallback_items(&bridge_client).await {
                    Ok(items) => {
                        publish_weaver_reconcile_delta(&bridge_client, &ingest, items).await;
                    }
                    Err(error) => {
                        warn!(error = %error, "weaver reconcile poll failed");
                    }
                }
            }
        }
    }
}

/// Publish reconcile results as a DELTA: upsert-only, so a poll racing the live
/// WebSocket stream can only add missed observations, never prune items the
/// stream just delivered. The authoritative scope stays reserved for the
/// connect-failure fallback poller, where the stream is known dead.
async fn publish_weaver_reconcile_delta(
    bridge_client: &WeaverSubscriptionBridgeClient,
    ingest: &TrackedDownloadSnapshotIngestHandle,
    items: Vec<DownloadQueueItem>,
) {
    if items.is_empty() {
        return;
    }
    let completed_downloads = load_completed_downloads_for_import(bridge_client, &items).await;
    let update = TrackedDownloadSnapshotUpdate {
        scope: TrackedDownloadSnapshotScope::Delta,
        items,
        completed_downloads,
        actor_id: None,
    };
    if let Err(error) = ingest.publish(update).await {
        warn!(error = %error, "weaver: failed to publish reconcile delta snapshot");
    }
}

async fn collect_weaver_fallback_items(
    bridge_client: &WeaverSubscriptionBridgeClient,
) -> AppResult<Vec<DownloadQueueItem>> {
    let mut items = bridge_client.download_client.list_queue().await?;
    let mut recent_items = bridge_client
        .download_client
        .list_recent_activity(POLL_FALLBACK_RECENT_ACTIVITY_LIMIT)
        .await?;
    items.append(&mut recent_items);
    for item in &mut items {
        bridge_client.stamp_queue_item(item);
    }
    Ok(items)
}

/// Outcome of a single `run_subscription` attempt. Tells the caller whether
/// the WebSocket ever became fully operational (subscribed and received at
/// least one handshake) so backoff/fallback state can be reset appropriately.
enum SubscriptionOutcome {
    /// Clean shutdown via cancellation token — no reconnect needed.
    Shutdown,
    /// Failed before the subscription was active (connect, handshake, or
    /// subscribe failed). Counts toward `consecutive_failures`.
    ConnectError(String),
    /// Was active but later disconnected. Backoff should be reset since the
    /// connection *did* work, but we still need to reconnect.
    Disconnected(String),
}

struct WsMessageState<'a> {
    last_cursor: &'a mut Option<String>,
}

async fn run_subscription(
    bridge_client: &WeaverSubscriptionBridgeClient,
    ingest: &TrackedDownloadSnapshotIngestHandle,
    token: &CancellationToken,
    last_cursor: &mut Option<String>,
) -> SubscriptionOutcome {
    let uri: tokio_tungstenite::tungstenite::http::Uri = match bridge_client.ws_url.parse() {
        Ok(uri) => uri,
        Err(e) => return SubscriptionOutcome::ConnectError(format!("invalid WebSocket URL: {e}")),
    };
    let mut request = ClientRequestBuilder::new(uri).with_sub_protocol("graphql-transport-ws");
    if let Some(api_key) = bridge_client.api_key.as_deref() {
        request = request.with_header("Authorization", format!("Bearer {api_key}"));
    }

    let (ws_stream, _response) = match tokio_tungstenite::connect_async(request).await {
        Ok(pair) => pair,
        Err(e) => {
            return SubscriptionOutcome::ConnectError(format!("WebSocket connect failed: {e}"));
        }
    };

    let (mut write, mut read) = ws_stream.split();

    // --- graphql-ws handshake: connection_init ---
    if let Err(e) = write
        .send(Message::Text(
            match bridge_client.api_key.as_deref() {
                Some(api_key) => json!({
                    "type": "connection_init",
                    "payload": {
                        "authorization": format!("Bearer {api_key}"),
                    }
                }),
                None => json!({
                    "type": "connection_init",
                    "payload": {},
                }),
            }
            .to_string()
            .into(),
        ))
        .await
    {
        return SubscriptionOutcome::ConnectError(format!("failed to send connection_init: {e}"));
    }

    // Wait for connection_ack.
    let ack = match tokio::time::timeout(std::time::Duration::from_secs(10), read.next()).await {
        Ok(Some(Ok(msg))) => msg,
        Ok(Some(Err(e))) => {
            return SubscriptionOutcome::ConnectError(format!(
                "WebSocket error waiting for ack: {e}"
            ));
        }
        Ok(None) => {
            return SubscriptionOutcome::ConnectError(
                "WebSocket closed before connection_ack".into(),
            );
        }
        Err(_) => {
            return SubscriptionOutcome::ConnectError("timeout waiting for connection_ack".into());
        }
    };

    let ack_text = match &ack {
        Message::Text(t) => t.as_ref(),
        _ => {
            return SubscriptionOutcome::ConnectError(
                "expected text message for connection_ack".into(),
            );
        }
    };
    let ack_json: Value = match serde_json::from_str(ack_text) {
        Ok(v) => v,
        Err(e) => return SubscriptionOutcome::ConnectError(format!("invalid ack json: {e}")),
    };
    let msg_type = ack_json.get("type").and_then(Value::as_str).unwrap_or("");
    if msg_type != "connection_ack" {
        return SubscriptionOutcome::ConnectError(format!(
            "expected connection_ack, got {msg_type}"
        ));
    }

    debug!("weaver WebSocket connection_ack received");

    // --- Subscribe to queueSnapshots ---
    let snapshot_subscribe_msg = json!({
        "id": "snapshot",
        "type": "subscribe",
        "payload": {
            "query": QUEUE_SNAPSHOTS_QUERY,
        }
    });
    if let Err(e) = write
        .send(Message::Text(snapshot_subscribe_msg.to_string().into()))
        .await
    {
        return SubscriptionOutcome::ConnectError(format!(
            "failed to subscribe to queueSnapshots: {e}"
        ));
    }

    let events_subscribe_msg = json!({
        "id": "events",
        "type": "subscribe",
        "payload": {
            "query": QUEUE_EVENTS_QUERY,
            "variables": {
                "after": last_cursor,
            }
        }
    });
    if let Err(e) = write
        .send(Message::Text(events_subscribe_msg.to_string().into()))
        .await
    {
        return SubscriptionOutcome::ConnectError(format!(
            "failed to subscribe to queueEvents: {e}"
        ));
    }

    info!("weaver subscription active");

    // ── From here on the subscription is live; any failure is a Disconnected. ──

    loop {
        let msg = tokio::select! {
            _ = token.cancelled() => return SubscriptionOutcome::Shutdown,
            msg = read.next() => {
                match msg {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => return SubscriptionOutcome::Disconnected(format!("WebSocket read error: {e}")),
                    None => return SubscriptionOutcome::Disconnected("WebSocket stream ended".into()),
                }
            }
        };

        match msg {
            Message::Text(text) => {
                let mut message_state = WsMessageState {
                    last_cursor: &mut *last_cursor,
                };
                if let Err(e) = handle_ws_message(
                    text.as_ref(),
                    &mut write,
                    bridge_client,
                    ingest,
                    &mut message_state,
                )
                .await
                {
                    return SubscriptionOutcome::Disconnected(e);
                }
            }
            Message::Ping(data) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Message::Close(_) => {
                return SubscriptionOutcome::Disconnected("WebSocket closed by server".into());
            }
            _ => {}
        }
    }
}

async fn handle_ws_message<S>(
    text: &str,
    write: &mut futures_util::stream::SplitSink<S, Message>,
    bridge_client: &WeaverSubscriptionBridgeClient,
    ingest: &TrackedDownloadSnapshotIngestHandle,
    state: &mut WsMessageState<'_>,
) -> Result<(), String>
where
    S: futures_util::Sink<Message> + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    let json: Value =
        serde_json::from_str(text).map_err(|e| format!("invalid ws message json: {e}"))?;
    let msg_type = json.get("type").and_then(Value::as_str).unwrap_or("");

    match msg_type {
        "next" => {
            let subscription_id = json.get("id").and_then(Value::as_str).unwrap_or("");
            let payload = json.get("payload").and_then(|p| p.get("data")).cloned();

            if let Some(payload) = payload {
                match subscription_id {
                    "snapshot" => {
                        let parsed: QueueSnapshotsPayload = serde_json::from_value(payload)
                            .map_err(|e| format!("invalid queueSnapshots payload: {e}"))?;
                        let items = map_weaver_items(bridge_client, &parsed.queue_snapshots.items);
                        publish_authoritative_weaver_snapshot(bridge_client, ingest, items).await;
                    }
                    "events" => {
                        let parsed: QueueEventsPayload = serde_json::from_value(payload)
                            .map_err(|e| format!("invalid queueEvents payload: {e}"))?;
                        *state.last_cursor = Some(parsed.queue_events.cursor.clone());
                        if let Some(item) = parsed.queue_events.item.as_ref()
                            && should_publish_weaver_terminal_delta(
                                parsed.queue_events.kind.as_str(),
                                item,
                            )
                        {
                            publish_weaver_terminal_delta(bridge_client, ingest, item).await;
                        }
                    }
                    _ => {
                        debug!(subscription_id, "ignoring unknown subscription id");
                    }
                }
            }
        }
        "ping" => {
            let _ = write
                .send(Message::Text(json!({"type": "pong"}).to_string().into()))
                .await;
        }
        "error" => {
            let payload = json.get("payload");
            warn!(?payload, "weaver subscription error");
            return Err("subscription error from server".into());
        }
        "complete" => {
            return Err("subscription completed by server".into());
        }
        _ => {
            debug!(msg_type, "ignoring unknown graphql-ws message type");
        }
    }

    Ok(())
}

fn should_publish_weaver_terminal_delta(kind: &str, item: &WeaverQueueItem) -> bool {
    kind == "ITEM_COMPLETED"
        || (kind == "ITEM_STATE_CHANGED" && item.state == WeaverQueueState::Failed)
}

fn map_weaver_items(
    bridge_client: &WeaverSubscriptionBridgeClient,
    jobs: &[WeaverQueueItem],
) -> Vec<DownloadQueueItem> {
    jobs.iter()
        .map(|job| bridge_client.map_queue_item(job))
        .collect()
}

async fn publish_authoritative_weaver_snapshot(
    bridge_client: &WeaverSubscriptionBridgeClient,
    ingest: &TrackedDownloadSnapshotIngestHandle,
    items: Vec<DownloadQueueItem>,
) {
    let completed_downloads = load_completed_downloads_for_import(bridge_client, &items).await;
    let update = TrackedDownloadSnapshotUpdate {
        scope: TrackedDownloadSnapshotScope::AuthoritativeForClient {
            client_id: Some(bridge_client.client_id.clone()),
            client_type: bridge_client.client_type.clone(),
        },
        items,
        completed_downloads,
        actor_id: None,
    };
    if let Err(error) = ingest.publish(update).await {
        warn!(error = %error, "weaver: failed to publish authoritative snapshot");
    }
}

async fn publish_weaver_terminal_delta(
    bridge_client: &WeaverSubscriptionBridgeClient,
    ingest: &TrackedDownloadSnapshotIngestHandle,
    item: &WeaverQueueItem,
) {
    let item = bridge_client.map_queue_item(item);
    let completed_downloads =
        load_completed_downloads_for_import(bridge_client, std::slice::from_ref(&item)).await;
    let update = TrackedDownloadSnapshotUpdate {
        scope: TrackedDownloadSnapshotScope::Delta,
        items: vec![item],
        completed_downloads,
        actor_id: None,
    };
    if let Err(error) = ingest.publish(update).await {
        warn!(error = %error, "weaver: failed to publish terminal delta");
    }
}

async fn load_completed_downloads_for_import(
    bridge_client: &WeaverSubscriptionBridgeClient,
    completed_items: &[DownloadQueueItem],
) -> Vec<CompletedDownload> {
    let mut seen = HashSet::new();
    let mut downloads = Vec::new();

    for item in completed_items {
        if item.state != scryer_domain::DownloadQueueState::Completed {
            continue;
        }
        let source_ref = item.download_client_item_id.trim();
        if source_ref.is_empty() || !seen.insert(source_ref.to_string()) {
            continue;
        }

        match bridge_client
            .download_client
            .get_completed_download(source_ref)
            .await
        {
            Ok(Some(mut completed)) => {
                bridge_client.stamp_completed_download(&mut completed);
                downloads.push(completed);
            }
            Ok(None) => {
                debug!(
                    source_ref,
                    "weaver: completed history item not available yet; import will retry"
                );
            }
            Err(error) => {
                warn!(
                    source_ref,
                    error = %error,
                    "weaver: failed direct completed history lookup; import will retry"
                );
            }
        }
    }

    downloads
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use scryer_domain::{DownloadClientConfig, DownloadClientStatus, DownloadQueueState};
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::weaver::{WeaverQueueItem, WeaverQueueState};
    use super::*;

    fn test_config() -> DownloadClientConfig {
        test_config_with_host("weaver.local", "9090")
    }

    fn test_config_with_host(host: &str, port: &str) -> DownloadClientConfig {
        DownloadClientConfig {
            id: "weaver-client".to_string(),
            name: "Weaver".to_string(),
            client_type: "weaver".to_string(),
            config_json: format!(r#"{{"api_key":"wvr_test","host":"{host}","port":"{port}"}}"#),
            client_priority: 1,
            is_enabled: true,
            status: DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            proxy_config_id: None,
        }
    }

    fn test_config_for_server(server: &MockServer) -> DownloadClientConfig {
        let uri = server.uri();
        let endpoint = uri
            .strip_prefix("http://")
            .expect("wiremock server should use http");
        let (host, port) = endpoint
            .rsplit_once(':')
            .expect("wiremock server uri should include a port");
        test_config_with_host(host, port)
    }

    fn queue_item(id: u64, state: WeaverQueueState) -> WeaverQueueItem {
        WeaverQueueItem {
            id,
            name: format!("Weaver Job {id}"),
            original_title: None,
            state,
            error: None,
            progress_percent: 0.0,
            total_bytes: 100,
            category: Some("movie".to_string()),
            attributes: Vec::new(),
            client_request_id: None,
            output_dir: None,
            created_at: Utc::now(),
            completed_at: None,
            attention: None,
        }
    }

    #[tokio::test]
    async fn authoritative_snapshot_publishes_ingest_update() {
        let bridge = WeaverSubscriptionBridgeClient::from_config(&test_config())
            .expect("bridge client should parse");
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let ingest = TrackedDownloadSnapshotIngestHandle::new(tx);

        publish_authoritative_weaver_snapshot(
            &bridge,
            &ingest,
            vec![bridge.map_queue_item(&queue_item(42, WeaverQueueState::Queued))],
        )
        .await;

        let update = rx.recv().await.expect("snapshot update should be sent");
        assert!(matches!(
            update.scope,
            TrackedDownloadSnapshotScope::AuthoritativeForClient {
                ref client_id,
                ref client_type,
            } if client_id.as_deref() == Some("weaver-client") && client_type == "weaver"
        ));
        assert_eq!(update.items.len(), 1);
        assert_eq!(update.items[0].client_id, "weaver-client");
        assert_eq!(update.items[0].client_type, "weaver");
        assert_eq!(update.items[0].state, DownloadQueueState::Queued);
        assert!(update.completed_downloads.is_empty());
    }

    #[test]
    fn failed_item_state_change_is_a_terminal_delta() {
        assert!(should_publish_weaver_terminal_delta(
            "ITEM_STATE_CHANGED",
            &queue_item(42, WeaverQueueState::Failed),
        ));
        assert!(!should_publish_weaver_terminal_delta(
            "ITEM_STATE_CHANGED",
            &queue_item(42, WeaverQueueState::Downloading),
        ));
        assert!(should_publish_weaver_terminal_delta(
            "ITEM_COMPLETED",
            &queue_item(42, WeaverQueueState::Completed),
        ));
    }

    #[tokio::test]
    async fn failed_delta_publishes_without_completed_history_lookup() {
        let bridge = WeaverSubscriptionBridgeClient::from_config(&test_config())
            .expect("bridge client should parse");
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let ingest = TrackedDownloadSnapshotIngestHandle::new(tx);
        let mut failed = queue_item(42, WeaverQueueState::Failed);
        failed.error = Some("verification failed".to_string());

        publish_weaver_terminal_delta(&bridge, &ingest, &failed).await;

        let update = rx.recv().await.expect("failed delta should be sent");
        assert!(matches!(update.scope, TrackedDownloadSnapshotScope::Delta));
        assert_eq!(update.items.len(), 1);
        assert_eq!(update.items[0].state, DownloadQueueState::Failed);
        assert_eq!(
            update.items[0].attention_reason.as_deref(),
            Some("verification failed")
        );
        assert!(update.completed_downloads.is_empty());
    }

    #[tokio::test]
    async fn completed_delta_publishes_empty_completed_rows_when_history_is_not_ready() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("historyItem(id"))
            .and(body_string_contains("\"id\":42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "historyItem": null
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let bridge = WeaverSubscriptionBridgeClient::from_config(&test_config_for_server(&server))
            .expect("bridge client should parse");
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let ingest = TrackedDownloadSnapshotIngestHandle::new(tx);

        publish_weaver_terminal_delta(
            &bridge,
            &ingest,
            &queue_item(42, WeaverQueueState::Completed),
        )
        .await;

        let update = rx.recv().await.expect("delta update should be sent");
        assert!(matches!(update.scope, TrackedDownloadSnapshotScope::Delta));
        assert_eq!(update.items.len(), 1);
        assert_eq!(update.items[0].state, DownloadQueueState::Completed);
        assert!(update.completed_downloads.is_empty());
    }

    #[tokio::test]
    async fn completed_delta_publishes_stamped_completed_row_when_history_is_ready() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("historyItem(id"))
            .and(body_string_contains("\"id\":42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "historyItem": {
                        "id": 42,
                        "name": "Weaver Job 42",
                        "state": "COMPLETE",
                        "error": null,
                        "progressPercent": 100.0,
                        "totalBytes": 123_u64,
                        "category": "movie",
                        "attributes": [],
                        "clientRequestId": null,
                        "outputDir": "/downloads/Weaver Job 42",
                        "createdAt": "2024-01-01T00:00:00Z",
                        "completedAt": "2024-01-01T00:10:00Z",
                        "attention": null
                    }
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let bridge = WeaverSubscriptionBridgeClient::from_config(&test_config_for_server(&server))
            .expect("bridge client should parse");
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let ingest = TrackedDownloadSnapshotIngestHandle::new(tx);

        publish_weaver_terminal_delta(
            &bridge,
            &ingest,
            &queue_item(42, WeaverQueueState::Completed),
        )
        .await;

        let update = rx.recv().await.expect("delta update should be sent");
        assert!(matches!(update.scope, TrackedDownloadSnapshotScope::Delta));
        assert_eq!(update.completed_downloads.len(), 1);
        assert_eq!(update.completed_downloads[0].download_client_item_id, "42");
        assert_eq!(update.completed_downloads[0].client_id, "weaver-client");
        assert_eq!(update.completed_downloads[0].client_type, "weaver");
    }
}
