//! WebSocket subscription bridge for the Weaver download client.
//!
//! Connects to Weaver's GraphQL WebSocket endpoint using the `graphql-ws`
//! protocol and receives real-time job snapshots. These are mapped to
//! scryer's `DownloadQueueItem` and broadcast through the same channel
//! that the HTTP-based download queue poller uses for NZBGet/SABnzbd.

use std::collections::HashSet;

use futures_util::{SinkExt, StreamExt};
use scryer_application::AppUseCase;
use scryer_domain::DownloadQueueState;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::weaver::weaver_job_to_queue_item;

const JOB_UPDATES_QUERY: &str = r#"
    subscription {
        jobUpdates {
            jobs {
                id name status error progress totalBytes downloadedBytes
                failedBytes health hasPassword category outputDir createdAt
                metadata { key value }
            }
            isPaused
        }
    }
"#;

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
pub async fn start_weaver_subscription_bridge(
    app: AppUseCase,
    token: CancellationToken,
    ws_url: String,
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

    loop {
        if token.is_cancelled() {
            info!("weaver subscription bridge shutting down");
            return;
        }

        info!(url = ws_url.as_str(), "connecting to weaver WebSocket");

        match run_subscription(&app, &actor, &ws_url, &token).await {
            Ok(()) => {
                // Clean shutdown (e.g. cancellation token fired).
                info!("weaver subscription bridge stopped cleanly");
                return;
            }
            Err(error) => {
                warn!(
                    error = %error,
                    backoff_secs,
                    "weaver subscription disconnected; reconnecting"
                );
            }
        }

        // Exponential backoff before reconnect.
        tokio::select! {
            _ = token.cancelled() => {
                info!("weaver subscription bridge shutting down during backoff");
                return;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)) => {}
        }
        backoff_secs = (backoff_secs * 2).min(max_backoff);
    }
}

async fn run_subscription(
    app: &AppUseCase,
    actor: &scryer_domain::User,
    ws_url: &str,
    token: &CancellationToken,
) -> Result<(), String> {
    let (ws_stream, _response) = tokio_tungstenite::connect_async(ws_url)
        .await
        .map_err(|e| format!("WebSocket connect failed: {e}"))?;

    let (mut write, mut read) = ws_stream.split();

    // --- graphql-ws handshake: connection_init ---
    write
        .send(Message::Text(
            json!({"type": "connection_init"}).to_string().into(),
        ))
        .await
        .map_err(|e| format!("failed to send connection_init: {e}"))?;

    // Wait for connection_ack.
    let ack = tokio::time::timeout(std::time::Duration::from_secs(10), read.next())
        .await
        .map_err(|_| "timeout waiting for connection_ack".to_string())?
        .ok_or("WebSocket closed before connection_ack")?
        .map_err(|e| format!("WebSocket error waiting for ack: {e}"))?;

    let ack_text = match &ack {
        Message::Text(t) => t.as_ref(),
        _ => return Err("expected text message for connection_ack".into()),
    };
    let ack_json: Value =
        serde_json::from_str(ack_text).map_err(|e| format!("invalid ack json: {e}"))?;
    let msg_type = ack_json
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("");
    if msg_type != "connection_ack" {
        return Err(format!("expected connection_ack, got {msg_type}"));
    }

    debug!("weaver WebSocket connection_ack received");

    // --- Subscribe to jobUpdates ---
    let subscribe_msg = json!({
        "id": "1",
        "type": "subscribe",
        "payload": {
            "query": JOB_UPDATES_QUERY,
        }
    });
    write
        .send(Message::Text(subscribe_msg.to_string().into()))
        .await
        .map_err(|e| format!("failed to send subscribe: {e}"))?;

    info!("weaver subscription active");

    // Track which completed jobs we've already triggered imports for.
    let mut imported_job_ids: HashSet<String> = HashSet::new();

    // Message loop.
    loop {
        let msg = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            msg = read.next() => {
                match msg {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => return Err(format!("WebSocket read error: {e}")),
                    None => return Err("WebSocket stream ended".into()),
                }
            }
        };

        match msg {
            Message::Text(text) => {
                handle_ws_message(
                    text.as_ref(),
                    app,
                    actor,
                    &mut write,
                    &mut imported_job_ids,
                )
                .await?;
            }
            Message::Ping(data) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Message::Close(_) => {
                return Err("WebSocket closed by server".into());
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
            let snapshot = json
                .get("payload")
                .and_then(|p| p.get("data"))
                .and_then(|d| d.get("jobUpdates"));

            if let Some(snapshot) = snapshot {
                process_job_snapshot(snapshot, app, actor, imported_job_ids).await;
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

async fn process_job_snapshot(
    snapshot: &Value,
    app: &AppUseCase,
    actor: &scryer_domain::User,
    imported_job_ids: &mut HashSet<String>,
) {
    let jobs = match snapshot.get("jobs").and_then(Value::as_array) {
        Some(jobs) => jobs,
        None => return,
    };

    let items: Vec<scryer_domain::DownloadQueueItem> =
        jobs.iter().filter_map(weaver_job_to_queue_item).collect();

    // Emit download queue gauge by state.
    let mut counts = [0u64; 9];
    for item in &items {
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

    // Broadcast to scryer's download queue channel (feeds the UI subscription).
    let _ = app.services.download_queue_broadcast.send(items.clone());

    // Trigger import for newly completed downloads.
    let newly_completed: Vec<&scryer_domain::DownloadQueueItem> = items
        .iter()
        .filter(|item| item.state == DownloadQueueState::Completed)
        .filter(|item| !imported_job_ids.contains(&item.download_client_item_id))
        .collect();

    if !newly_completed.is_empty() {
        scryer_application::try_import_completed_downloads(app, actor, &items).await;

        for item in &newly_completed {
            imported_job_ids.insert(item.download_client_item_id.clone());
        }
    }
}
