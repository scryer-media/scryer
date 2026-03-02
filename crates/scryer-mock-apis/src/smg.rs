use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;
use std::sync::Arc;

use crate::fixtures::load_fixture;
use crate::ScenarioState;

/// Build the SMG (Scryer Metadata Gateway) GraphQL mock router.
///
/// Handles:
/// - GET `/graphql` — APQ (Automatic Persisted Query) cache hit path
/// - POST `/graphql` — full query fallback path
pub fn router() -> Router<Arc<ScenarioState>> {
    Router::new().route(
        "/graphql",
        get(graphql_get_handler).post(graphql_post_handler),
    )
}

/// APQ GET handler — parses the extensions to determine the query type.
async fn graphql_get_handler(
    State(state): State<Arc<ScenarioState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let scenario = state.current_scenario();
    let variables_str = params.get("variables").cloned().unwrap_or_default();
    tracing::debug!(scenario = %scenario, "smg graphql GET (APQ)");

    resolve_response(&variables_str)
}

/// POST handler — parses the query body to determine response.
async fn graphql_post_handler(
    State(state): State<Arc<ScenarioState>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let scenario = state.current_scenario();
    let query = body.get("query").and_then(Value::as_str).unwrap_or("");
    let variables = body.get("variables").cloned().unwrap_or(Value::Null);
    let variables_str = serde_json::to_string(&variables).unwrap_or_default();

    tracing::debug!(scenario = %scenario, query_len = query.len(), "smg graphql POST");

    // Detect query type from the query string
    if query.contains("series") || query.contains("getSeries") {
        let fixture = load_fixture("smg/get_series.json");
        let parsed: Value = serde_json::from_str(&fixture).expect("valid fixture");
        return Json(parsed);
    }

    if query.contains("movie") || query.contains("getMovie") {
        let fixture = load_fixture("smg/get_movie.json");
        let parsed: Value = serde_json::from_str(&fixture).expect("valid fixture");
        return Json(parsed);
    }

    resolve_response(&variables_str)
}

/// Resolve the response based on variables (handles both GET and POST).
fn resolve_response(variables_str: &str) -> Json<Value> {
    // Try to parse variables to detect query intent
    let variables: Value = serde_json::from_str(variables_str).unwrap_or(Value::Null);

    // If variables contain tvdbId (get movie/series), return appropriate fixture
    if variables.get("tvdbId").is_some() || variables.get("tvdb_id").is_some() {
        // Check if it looks like a series or movie request
        let fixture = load_fixture("smg/get_movie.json");
        let parsed: Value = serde_json::from_str(&fixture).expect("valid fixture");
        return Json(parsed);
    }

    // Default: search response
    let fixture = load_fixture("smg/search_tvdb_rich.json");
    let parsed: Value = serde_json::from_str(&fixture).expect("valid fixture");
    Json(parsed)
}
