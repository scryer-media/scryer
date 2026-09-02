//! Test indexer fixture: a `scryer:indexer/indexer-plugin@1.1.0` component
//! built for `wasm32-wasip2`, the only shape the plugin loader accepts.
//!
//! It answers every search with one deterministic result and never touches the
//! host imports. The descriptor lives in `test-indexer-descriptor` so the xtask
//! fixture builder can embed the same bytes as the artifact's custom section.

use scryer_plugin_sdk::{
    PluginResult, PluginSearchRequest, PluginSearchResponse, PluginSearchResult,
};
use std::collections::HashMap;

wit_bindgen::generate!({
    world: "indexer-plugin",
    path: "../../crates/scryer-plugins/wit/indexer-v1.1.0",
});

struct TestIndexer;

impl Guest for TestIndexer {
    fn describe() -> Vec<u8> {
        serde_json::to_vec(&test_indexer_descriptor::descriptor())
            .expect("test indexer descriptor serializes")
    }

    async fn search(request: Vec<u8>) -> Result<Vec<u8>, InvocationError> {
        let req: PluginSearchRequest =
            serde_json::from_slice(&request).map_err(|_| InvocationError::InvalidResponse)?;
        let limit = if req.limit == 0 { 10 } else { req.limit };

        let results = vec![PluginSearchResult {
            title: format!("{} 2024 2160p WEB-DL H.265", req.query),
            link: Some("https://example.com/details/12345".to_string()),
            download_url: Some("https://example.com/download/12345.nzb".to_string()),
            size_bytes: Some(8_000_000_000),
            published_at: Some("2024-06-15T00:00:00Z".to_string()),
            grabs: Some(42),
            languages: vec!["English".to_string()],
            provider_extra: HashMap::new(),
            ..PluginSearchResult::default()
        }];

        serde_json::to_vec(&PluginResult::Ok(PluginSearchResponse {
            results: results.into_iter().take(limit).collect(),
            ..PluginSearchResponse::default()
        }))
        .map_err(|_| InvocationError::Failed)
    }

    async fn search_plan(_request: Vec<u8>) -> Result<Vec<u8>, InvocationError> {
        // The descriptor declares no strategy plan, so the host never asks.
        Err(InvocationError::Failed)
    }

    async fn action(_request: Vec<u8>) -> Result<Vec<u8>, InvocationError> {
        Err(InvocationError::Failed)
    }
}

export!(TestIndexer);
