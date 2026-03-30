//! WebSocket subscription bridge for the Weaver download client.
//!
//! Connects to Weaver's GraphQL WebSocket endpoint using the `graphql-ws`
//! protocol and receives real-time job snapshots. These are mapped to
//! scryer's `DownloadQueueItem` and broadcast through the same channel
//! that the HTTP-based download queue poller uses for NZBGet/SABnzbd.
//!
//! If the WebSocket connection fails repeatedly, the bridge automatically
//! falls back to GraphQL HTTP polling so the UI stays up-to-date. When the
//! WebSocket reconnects the poller is stopped and real-time push resumes.

use std::collections::HashSet;

use futures_util::{SinkExt, StreamExt};
use scryer_application::AppUseCase;
use scryer_domain::DownloadQueueState;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{ClientRequestBuilder, Message};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::weaver::{WeaverQueueItem, weaver_item_to_queue_item};

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

/// Start a WebSocket subscription bridge to Weaver.
///
/// This replaces the HTTP polling loop (`start_download_queue_poller`) when
/// Weaver is the active download client. It connects to Weaver's `jobUpdates`
/// subscription and:
///
/// 1. Maps incoming job snapshots to `Vec<DownloadQueueItem>`
/// 2. Broadcasts them through `download_queue_broadcast`
/// 3. Triggers auto-import for newly completed downloads
///
/// Reconnects automatically on disconnect with exponential backoff.
/// After [`POLL_FALLBACK_THRESHOLD`] consecutive failures the bridge starts
/// a GraphQL HTTP polling loop so that download-queue data keeps flowing to
/// the UI. When the WebSocket reconnects the poller is stopped automatically.
pub async fn start_weaver_subscription_bridge(
    app: AppUseCase,
    token: CancellationToken,
    ws_url: String,
    api_key: Option<String>,
) {
    let actor = match app.find_or_create_default_user().await {
        Ok(actor) => actor,
        Err(error) => {
            warn!(error = %error, "weaver subscription bridge failed to resolve actor");
            return;
        }
    };

    let mut backoff_secs: u64 = 5;
    let max_backoff: u64 = 60;
    let mut consecutive_failures: u32 = 0;
    let mut last_cursor: Option<String> = None;
    // Token used to stop fallback polling when WS reconnects.
    let mut poll_cancel: Option<CancellationToken> = None;

    loop {
        if token.is_cancelled() {
            info!("weaver subscription bridge shutting down");
            return;
        }

        info!(url = ws_url.as_str(), "connecting to weaver WebSocket");

        match run_subscription(
            &app,
            &actor,
            &ws_url,
            api_key.as_deref(),
            &token,
            &mut last_cursor,
        )
        .await
        {
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
                    tokio::spawn(run_fallback_poller(app.clone(), actor.clone(), poll_token));
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
/// Polls `list_download_queue` every [`POLL_FALLBACK_INTERVAL_SECS`] seconds,
/// broadcasting results through the same channel the subscription uses.
async fn run_fallback_poller(
    app: AppUseCase,
    actor: scryer_domain::User,
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
                match app.list_download_queue(&actor, true, false).await {
                    Ok(items) => {
                        scryer_application::try_import_completed_downloads(
                            &app, &actor, &items,
                        )
                        .await;

                        emit_queue_metrics(&items);

                        let _ = app.services.download_queue_broadcast.send(items);
                    }
                    Err(error) => {
                        warn!(error = %error, "weaver fallback poll failed");
                    }
                }
            }
        }
    }
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

async fn run_subscription(
    app: &AppUseCase,
    actor: &scryer_domain::User,
    ws_url: &str,
    api_key: Option<&str>,
    token: &CancellationToken,
    last_cursor: &mut Option<String>,
) -> SubscriptionOutcome {
    let uri: tokio_tungstenite::tungstenite::http::Uri = match ws_url.parse() {
        Ok(uri) => uri,
        Err(e) => return SubscriptionOutcome::ConnectError(format!("invalid WebSocket URL: {e}")),
    };
    let mut request = ClientRequestBuilder::new(uri).with_sub_protocol("graphql-transport-ws");
    if let Some(api_key) = api_key {
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
            match api_key {
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

    let mut imported_job_ids: HashSet<String> = HashSet::new();

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
                if let Err(e) = handle_ws_message(
                    text.as_ref(),
                    app,
                    actor,
                    &mut write,
                    &mut imported_job_ids,
                    last_cursor,
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
    app: &AppUseCase,
    actor: &scryer_domain::User,
    write: &mut futures_util::stream::SplitSink<S, Message>,
    imported_job_ids: &mut HashSet<String>,
    last_cursor: &mut Option<String>,
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
                        process_job_snapshot(
                            &parsed.queue_snapshots.items,
                            app,
                            actor,
                            imported_job_ids,
                        )
                        .await;
                    }
                    "events" => {
                        let parsed: QueueEventsPayload = serde_json::from_value(payload)
                            .map_err(|e| format!("invalid queueEvents payload: {e}"))?;
                        *last_cursor = Some(parsed.queue_events.cursor.clone());
                        if parsed.queue_events.kind == "ITEM_COMPLETED"
                            && let Some(item) = parsed.queue_events.item.as_ref()
                        {
                            let items = vec![weaver_item_to_queue_item(item)];
                            maybe_import_completed_items(&items, app, actor, imported_job_ids)
                                .await;
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

fn emit_queue_metrics(items: &[scryer_domain::DownloadQueueItem]) {
    let mut counts = [0u64; 9];
    for item in items {
        match item.state {
            DownloadQueueState::Queued => counts[0] += 1,
            DownloadQueueState::Downloading => counts[1] += 1,
            DownloadQueueState::Paused => counts[2] += 1,
            DownloadQueueState::Completed => counts[3] += 1,
            DownloadQueueState::ImportPending => counts[4] += 1,
            DownloadQueueState::Failed => counts[5] += 1,
            DownloadQueueState::Verifying => counts[6] += 1,
            DownloadQueueState::Repairing => counts[7] += 1,
            DownloadQueueState::Extracting => counts[8] += 1,
        }
    }
    let labels = [
        "queued",
        "downloading",
        "paused",
        "completed",
        "import_pending",
        "failed",
        "verifying",
        "repairing",
        "extracting",
    ];
    for (label, &count) in labels.iter().zip(&counts) {
        metrics::gauge!("scryer_download_queue_items", "state" => *label).set(count as f64);
    }
}

async fn process_job_snapshot(
    jobs: &[WeaverQueueItem],
    app: &AppUseCase,
    actor: &scryer_domain::User,
    imported_job_ids: &mut HashSet<String>,
) {
    let mut items: Vec<scryer_domain::DownloadQueueItem> =
        jobs.iter().map(weaver_item_to_queue_item).collect();

    // Enrich items from the download_submissions table so that is_scryer_origin,
    // title_id, and facet are populated even when the Weaver job metadata is
    // missing (mirrors the enrichment in list_download_queue).
    for item in &mut items {
        if item.is_scryer_origin {
            continue;
        }
        if let Ok(Some(submission)) = app
            .services
            .download_submissions
            .find_by_client_item_id(&item.client_type, &item.download_client_item_id)
            .await
        {
            item.is_scryer_origin = true;
            item.title_id = Some(submission.title_id);
            item.facet = Some(submission.facet);
        }
    }

    emit_queue_metrics(&items);

    // Broadcast to scryer's download queue channel (feeds the UI subscription).
    let _ = app.services.download_queue_broadcast.send(items.clone());

    maybe_import_completed_items(&items, app, actor, imported_job_ids).await;
}

async fn maybe_import_completed_items(
    items: &[scryer_domain::DownloadQueueItem],
    app: &AppUseCase,
    actor: &scryer_domain::User,
    imported_job_ids: &mut HashSet<String>,
) {
    // Trigger import for newly completed downloads.
    let newly_completed: Vec<&scryer_domain::DownloadQueueItem> = items
        .iter()
        .filter(|item| item.state == DownloadQueueState::Completed)
        .filter(|item| !imported_job_ids.contains(&item.download_client_item_id))
        .collect();

    if !newly_completed.is_empty() {
        tracing::info!(
            count = newly_completed.len(),
            items = %newly_completed.iter().map(|i| format!(
                "{}(id={}, origin={})", i.title_name, i.download_client_item_id, i.is_scryer_origin
            )).collect::<Vec<_>>().join(", "),
            "weaver: newly completed downloads detected via WS subscription"
        );

        let processed =
            scryer_application::try_import_completed_downloads(app, actor, items).await;

        tracing::debug!(
            processed_count = processed.len(),
            deferred_count = newly_completed.len()
                - newly_completed
                    .iter()
                    .filter(|i| processed.contains(&i.download_client_item_id))
                    .count(),
            "weaver: import attempt complete — deferred items will be retried on next snapshot"
        );

        // Only suppress future retries for IDs that were actually processed
        // (imported, already-imported, or permanently non-importable).
        // Items skipped due to transient conditions (no matching
        // CompletedDownload yet, empty dest_dir) will be retried on the
        // next snapshot.
        for id in processed {
            imported_job_ids.insert(id);
        }
    }
}
