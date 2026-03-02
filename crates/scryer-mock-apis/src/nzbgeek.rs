use axum::extract::{Query, State};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use std::sync::Arc;

use crate::fixtures::load_fixture;
use crate::ScenarioState;

/// Build the NZBGeek/Newznab mock router.
///
/// Handles GET to `/api` with Newznab query parameters (`t`, `q`, `apikey`, etc.).
pub fn router() -> Router<Arc<ScenarioState>> {
    Router::new()
        .route("/api", get(newznab_handler))
        .route("/getnzb", get(nzb_download_handler))
}

#[derive(Debug, Deserialize)]
struct NewznabQuery {
    t: Option<String>,
    #[allow(dead_code)]
    q: Option<String>,
    apikey: Option<String>,
    #[allow(dead_code)]
    o: Option<String>,
}

async fn newznab_handler(
    State(state): State<Arc<ScenarioState>>,
    Query(params): Query<NewznabQuery>,
) -> axum::response::Response {
    let scenario = state.current_scenario();
    let search_type = params.t.as_deref().unwrap_or("search");

    tracing::debug!(search_type, scenario = %scenario, "nzbgeek search request");

    // Validate API key
    if params.apikey.as_deref().unwrap_or("").is_empty() {
        return axum::response::Response::builder()
            .status(401)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"error":{"code":"100","description":"Incorrect user credentials"}}"#,
            ))
            .unwrap();
    }

    let fixture = match search_type {
        "movie" => load_fixture("nzbgeek/search_movie.json"),
        "tvsearch" => load_fixture("nzbgeek/search_tv.json"),
        "search" => load_fixture("nzbgeek/search_movie.json"),
        _ => load_fixture("nzbgeek/search_empty.json"),
    };

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(fixture))
        .unwrap()
}

async fn nzb_download_handler() -> axum::response::Response {
    let nzb = load_fixture("nzbgeek/nzb_content.xml");

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/x-nzb")
        .body(axum::body::Body::from(nzb))
        .unwrap()
}
