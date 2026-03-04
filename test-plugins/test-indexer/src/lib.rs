use extism_pdk::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize)]
struct PluginDescriptor {
    name: String,
    version: String,
    sdk_version: String,
    plugin_type: String,
    provider_type: String,
    capabilities: Capabilities,
    scoring_policies: Vec<ScoringPolicy>,
}

#[derive(Serialize)]
struct Capabilities {
    search: bool,
    imdb_search: bool,
    tvdb_search: bool,
}

#[derive(Serialize)]
struct ScoringPolicy {
    name: String,
    rego_source: String,
    applied_facets: Vec<String>,
}

#[derive(Deserialize)]
struct SearchRequest {
    query: String,
    #[serde(default)]
    imdb_id: Option<String>,
    #[serde(default)]
    tvdb_id: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    limit: usize,
}

#[derive(Serialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

#[derive(Serialize)]
struct SearchResult {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    grabs: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    languages: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    extra: HashMap<String, serde_json::Value>,
}

#[plugin_fn]
pub fn describe(_input: String) -> FnResult<String> {
    let descriptor = PluginDescriptor {
        name: "Test Indexer".to_string(),
        version: "0.1.0".to_string(),
        sdk_version: "0.1".to_string(),
        plugin_type: "indexer".to_string(),
        provider_type: "test".to_string(),
        capabilities: Capabilities {
            search: true,
            imdb_search: true,
            tvdb_search: false,
        },
        scoring_policies: vec![],
    };
    Ok(serde_json::to_string(&descriptor)?)
}

#[plugin_fn]
pub fn search(input: String) -> FnResult<String> {
    let req: SearchRequest = serde_json::from_str(&input)?;

    let limit = if req.limit == 0 { 10 } else { req.limit };

    let results = vec![SearchResult {
        title: format!("{} 2024 2160p WEB-DL H.265", req.query),
        link: Some("https://example.com/details/12345".to_string()),
        download_url: Some("https://example.com/download/12345.nzb".to_string()),
        size_bytes: Some(8_000_000_000),
        published_at: Some("2024-06-15T00:00:00Z".to_string()),
        grabs: Some(42),
        languages: vec!["English".to_string()],
        extra: HashMap::new(),
    }];

    let response = SearchResponse {
        results: results.into_iter().take(limit).collect(),
    };
    Ok(serde_json::to_string(&response)?)
}
