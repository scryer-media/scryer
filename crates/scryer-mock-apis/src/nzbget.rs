use axum::extract::State;
use axum::routing::post;
use axum::Json;
use axum::Router;
use serde_json::Value;
use std::sync::Arc;

use crate::fixtures::load_fixture;
use crate::ScenarioState;

/// Build the NZBGet JSON-RPC mock router.
///
/// Handles POST to `/jsonrpc` and dispatches based on the `method` field in
/// the JSON-RPC request body.
pub fn router() -> Router<Arc<ScenarioState>> {
    Router::new().route("/jsonrpc", post(jsonrpc_handler))
}

async fn jsonrpc_handler(
    State(state): State<Arc<ScenarioState>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    let scenario = state.current_scenario();

    tracing::debug!(method, scenario = %scenario, "nzbget rpc call");

    let fixture = match method {
        "version" => load_fixture("nzbget/version.json"),
        "append" => load_fixture("nzbget/append.json"),
        "listgroups" => load_fixture("nzbget/listgroups.json"),
        "history" => {
            let raw = load_fixture("nzbget/history.json");
            // Patch timestamps to be recent so they pass the 7-day cutoff
            let now = chrono_now_unix();
            raw.replace("1706832000", &now.to_string())
                .replace("1706745600", &(now - 3600).to_string())
        }
        "postqueue" => load_fixture("nzbget/postqueue.json"),
        _ => {
            tracing::warn!(method, "unknown nzbget rpc method");
            return Json(serde_json::json!({
                "version": "2.0",
                "id": "scryer-rpc",
                "error": { "code": -32601, "message": format!("method not found: {method}") }
            }));
        }
    };

    let response: Value = serde_json::from_str(&fixture).expect("fixture should be valid JSON");
    Json(response)
}

fn chrono_now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
