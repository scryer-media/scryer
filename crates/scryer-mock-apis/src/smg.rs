use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;
use std::sync::Arc;

use crate::ScenarioState;
use crate::fixtures::load_fixture;

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
    let operation_name = params
        .get("operationName")
        .map(String::as_str)
        .unwrap_or_default();
    tracing::debug!(scenario = %scenario, "smg graphql GET (APQ)");

    fixture_response(operation_name, "")
}

/// POST handler — parses the query body to determine response.
async fn graphql_post_handler(
    State(state): State<Arc<ScenarioState>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let scenario = state.current_scenario();
    let query = body.get("query").and_then(Value::as_str).unwrap_or("");
    let operation_name = body
        .get("operationName")
        .and_then(Value::as_str)
        .unwrap_or_default();

    tracing::debug!(scenario = %scenario, query_len = query.len(), "smg graphql POST");

    fixture_response(operation_name, query)
}

fn fixture_response(operation_name: &str, query: &str) -> Json<Value> {
    let operation = if query.is_empty() {
        operation_name
    } else {
        query
    };
    let fixture =
        if operation.contains("searchTitlesBatch") || operation.contains("SearchTitlesBatch") {
            "smg/search_titles_batch.json"
        } else if operation.contains("searchTitles(") || operation.contains("SearchTitles") {
            "smg/search_titles.json"
        } else if operation.contains("resolveTitles(") || operation.contains("ResolveTitles") {
            "smg/resolve_titles.json"
        } else if operation.contains("titles(") || operation.contains("Titles") {
            "smg/titles_movie.json"
        } else if operation.contains("metadataBulk(") || operation.contains("MetadataBulk") {
            "smg/metadata_bulk_movie.json"
        } else if operation.contains("searchTvdbBatch") || operation.contains("SearchTvdbBatch") {
            "smg/search_tvdb_batch.json"
        } else if operation.contains("searchTvdbMulti") || operation.contains("SearchTvdbMulti") {
            "smg/search_tvdb_multi.json"
        } else if operation.contains("series(") || operation.contains("GetSeries") {
            "smg/get_series.json"
        } else if operation.contains("movie(") || operation.contains("GetMovie") {
            "smg/get_movie.json"
        } else {
            "smg/search_tvdb_rich.json"
        };
    let fixture = load_fixture(fixture);
    let parsed: Value = serde_json::from_str(&fixture).expect("valid fixture");
    Json(parsed)
}

#[cfg(test)]
mod tests {
    use super::fixture_response;

    #[test]
    fn routes_title_id_operations_by_operation_name() {
        let response = fixture_response("Titles", "");
        assert!(response["data"]["titles"].is_object());

        let response = fixture_response(
            "",
            "query { searchTitles(query: \"x\") { results { title_id } } }",
        );
        assert!(response["data"]["searchTitles"].is_object());
    }
}
