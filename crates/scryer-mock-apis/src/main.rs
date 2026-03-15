mod fixtures;
mod nzbgeek;
mod nzbget;
mod smg;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;

/// Shared state for scenario switching.
/// E2E tests can POST to `/scenario/:name` to change the active scenario,
/// which mock handlers use to vary their responses.
pub struct ScenarioState {
    scenario: std::sync::RwLock<String>,
}

impl ScenarioState {
    fn new() -> Self {
        Self {
            scenario: std::sync::RwLock::new("default".to_string()),
        }
    }

    pub fn current_scenario(&self) -> String {
        self.scenario.read().unwrap().clone()
    }

    fn set_scenario(&self, name: String) {
        *self.scenario.write().unwrap() = name;
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "scryer_mock_apis=debug,tower_http=debug".into()),
        )
        .init();

    let nzbget_port: u16 = std::env::var("MOCK_NZBGET_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6789);
    let nzbgeek_port: u16 = std::env::var("MOCK_NZBGEEK_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6790);
    let smg_port: u16 = std::env::var("MOCK_SMG_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6791);
    let control_port: u16 = std::env::var("MOCK_CONTROL_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6799);

    let state = Arc::new(ScenarioState::new());

    // Control API for scenario switching
    let control_router = Router::new()
        .route("/scenario/{name}", post(set_scenario))
        .route("/scenario", get(get_scenario))
        .route("/health", get(|| async { Json(json!({"ok": true})) }))
        .with_state(state.clone());

    // Spawn all servers concurrently
    let nzbget_state = state.clone();
    let nzbget_handle = tokio::spawn(async move {
        let app = nzbget::router().with_state(nzbget_state);
        let listener = TcpListener::bind(format!("0.0.0.0:{nzbget_port}"))
            .await
            .expect("failed to bind nzbget mock");
        tracing::info!("NZBGet mock listening on :{nzbget_port}");
        axum::serve(listener, app).await.unwrap();
    });

    let nzbgeek_state = state.clone();
    let nzbgeek_handle = tokio::spawn(async move {
        let app = nzbgeek::router().with_state(nzbgeek_state);
        let listener = TcpListener::bind(format!("0.0.0.0:{nzbgeek_port}"))
            .await
            .expect("failed to bind nzbgeek mock");
        tracing::info!("NZBGeek mock listening on :{nzbgeek_port}");
        axum::serve(listener, app).await.unwrap();
    });

    let smg_state = state.clone();
    let smg_handle = tokio::spawn(async move {
        let app = smg::router().with_state(smg_state);
        let listener = TcpListener::bind(format!("0.0.0.0:{smg_port}"))
            .await
            .expect("failed to bind smg mock");
        tracing::info!("SMG mock listening on :{smg_port}");
        axum::serve(listener, app).await.unwrap();
    });

    let control_handle = tokio::spawn(async move {
        let listener = TcpListener::bind(format!("0.0.0.0:{control_port}"))
            .await
            .expect("failed to bind control api");
        tracing::info!("Control API listening on :{control_port}");
        axum::serve(listener, control_router).await.unwrap();
    });

    tracing::info!(
        "scryer-mock-apis running (nzbget={nzbget_port}, nzbgeek={nzbgeek_port}, \
         smg={smg_port}, control={control_port})"
    );

    tokio::select! {
        _ = nzbget_handle => {}
        _ = nzbgeek_handle => {}
        _ = smg_handle => {}
        _ = control_handle => {}
    }
}

async fn set_scenario(
    State(state): State<Arc<ScenarioState>>,
    Path(name): Path<String>,
) -> Json<Value> {
    tracing::info!(scenario = %name, "scenario switched");
    state.set_scenario(name.clone());
    Json(json!({"scenario": name}))
}

async fn get_scenario(State(state): State<Arc<ScenarioState>>) -> Json<Value> {
    Json(json!({"scenario": state.current_scenario()}))
}
