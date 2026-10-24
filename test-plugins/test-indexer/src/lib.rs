//! Test indexer fixture: a `scryer:indexer/indexer-plugin@1.1.0` component
//! built for `wasm32-wasip2`, the only shape the plugin loader accepts.
//!
//! It answers every search with one deterministic result. Its only use of the
//! host imports is a stand-in sign-in session: `checkCaptcha` opens one in
//! component state and shows an image challenge, and a search made with an
//! answer signs that session in and names it in the result title, so tests can
//! see which session a client started from. The descriptor lives in
//! `test-indexer-descriptor` so the xtask fixture builder can embed the same
//! bytes as the artifact's custom section.

use scryer::indexer::host;
use scryer_plugin_sdk::command::{PluginActionRequest, PluginActionResponse};
use scryer_plugin_sdk::{
    PluginResult, PluginSearchRequest, PluginSearchResponse, PluginSearchResult,
};
use std::collections::HashMap;

wit_bindgen::generate!({
    world: "indexer-plugin",
    path: "../../crates/scryer-plugins/wit/indexer-v1.1.0",
});

const SESSION_KEY: &str = "session";
/// The first four bytes of a PNG file, base64.
const CHALLENGE_IMAGE: &str = "iVBORw==";

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

        let mut title = format!("{} 2024 2160p WEB-DL H.265", req.query);
        if let Some(session) = host::state_get(SESSION_KEY) {
            title.push_str(&format!(" session-{}", String::from_utf8_lossy(&session)));
            let answered = host::config_get("challenge_answer")
                .is_some_and(|answer| !answer.trim().is_empty());
            if answered {
                host::state_cas(SESSION_KEY, Some(&session), Some(b"signed-in"));
            }
        }

        let results = vec![PluginSearchResult {
            title,
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

    async fn action(request: Vec<u8>) -> Result<Vec<u8>, InvocationError> {
        let request: PluginActionRequest =
            serde_json::from_slice(&request).map_err(|_| InvocationError::InvalidResponse)?;
        if request.action != "checkCaptcha" {
            return Err(InvocationError::Failed);
        }
        if host::state_get(SESSION_KEY).is_none() {
            host::state_cas(SESSION_KEY, None, Some(b"challenge-session"));
        }
        serde_json::to_vec(&PluginResult::Ok(PluginActionResponse {
            payload: serde_json::json!({
                "captchaRequest": {
                    "type": "image",
                    "contentType": "image/png; charset=binary",
                    "imageData": CHALLENGE_IMAGE,
                }
            }),
        }))
        .map_err(|_| InvocationError::Failed)
    }
}

export!(TestIndexer);
