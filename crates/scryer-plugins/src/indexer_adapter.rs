use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, DownloadSourceKind, IndexerClient, IndexerRoutingPlan,
    IndexerSearchResponse, IndexerSearchResult, SearchMode,
};
use scryer_domain::IndexerConfig;
use scryer_domain::TaggedAlias;
use std::sync::Arc;
use tracing::{info, warn};

use crate::loader::{apply_allowed_hosts, build_plugin, parse_config_json_entries};
use crate::types::{PluginDescriptor, PluginSearchRequest, PluginSearchResponse};

/// Wrapper to allow `extism::Plugin` inside `Send + Sync` structs.
///
/// `extism::Plugin` is `!Send` because wasmtime internals use `!Send` types.
/// This is safe because we only access the plugin through a `Mutex` inside
/// `spawn_blocking`, ensuring single-threaded exclusive access.
struct SendPlugin(extism::Plugin);

// SAFETY: Access is serialized via Mutex and confined to spawn_blocking tasks.
unsafe impl Send for SendPlugin {}

pub struct WasmIndexerClient {
    descriptor: PluginDescriptor,
    indexer_name: String,
    plugin: Arc<std::sync::Mutex<SendPlugin>>,
}

impl WasmIndexerClient {
    pub fn new(
        wasm_bytes: Vec<u8>,
        descriptor: PluginDescriptor,
        indexer_name: String,
        config: IndexerConfig,
    ) -> Result<Self, AppError> {
        let manifest = build_manifest(&wasm_bytes, &descriptor, &indexer_name, &config);
        let plugin = build_plugin(manifest).map_err(|e| {
            AppError::Repository(format!(
                "failed to compile WASM plugin for {}: {e}",
                indexer_name
            ))
        })?;

        info!(
            indexer = indexer_name.as_str(),
            plugin = descriptor.name.as_str(),
            "WASM plugin compiled and cached"
        );

        Ok(Self {
            descriptor,
            indexer_name,
            plugin: Arc::new(std::sync::Mutex::new(SendPlugin(plugin))),
        })
    }
}

fn build_manifest(
    wasm_bytes: &[u8],
    descriptor: &PluginDescriptor,
    indexer_name: &str,
    config: &IndexerConfig,
) -> extism::Manifest {
    let mut manifest = extism::Manifest::new([extism::Wasm::data(wasm_bytes.to_vec())]);
    manifest = apply_allowed_hosts(
        manifest,
        descriptor,
        Some(&config.base_url),
        config.config_json.as_deref(),
    );
    manifest = manifest.with_timeout(std::time::Duration::from_secs(30));
    manifest = manifest.with_config_key("base_url", &config.base_url);
    if let Some(ref api_key) = config.api_key_encrypted {
        manifest = manifest.with_config_key("api_key", api_key);
    }

    if let Some(ref json_str) = config.config_json {
        match parse_config_json_entries(json_str) {
            Ok(map) => {
                for (key, value) in &map {
                    manifest = manifest.with_config_key(key, value);
                }
            }
            Err(error) => {
                warn!(
                    indexer = indexer_name,
                    error = %error,
                    "failed to parse config_json; extra config keys will not be injected"
                );
            }
        }
    }

    manifest
}

#[async_trait]
impl IndexerClient for WasmIndexerClient {
    async fn search(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        let request = PluginSearchRequest {
            query,
            ids,
            facet,
            category,
            categories: newznab_categories.unwrap_or_default(),
            limit: 1000,
            season,
            episode,
            absolute_episode,
            tagged_aliases,
        };

        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize plugin request: {e}"))
        })?;

        tracing::debug!(plugin = %self.descriptor.name, %input, "plugin search request");

        let plugin_name = self.descriptor.name.clone();
        let indexer_name = self.indexer_name.clone();
        let plugin = Arc::clone(&self.plugin);

        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;

            let start = std::time::Instant::now();
            let result = guard
                .0
                .call::<&str, String>("search", &input)
                .map_err(|e| AppError::Repository(format!("plugin search() failed: {e}")));
            let elapsed = start.elapsed();

            tracing::debug!(
                plugin = plugin_name.as_str(),
                indexer = indexer_name.as_str(),
                elapsed_ms = elapsed.as_millis() as u64,
                "WASM plugin search call completed"
            );

            result
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let response: PluginSearchResponse = serde_json::from_str(&output).map_err(|e| {
            warn!(
                plugin = self.descriptor.name.as_str(),
                indexer = self.indexer_name.as_str(),
                error = %e,
                "plugin returned invalid search response JSON"
            );
            AppError::Repository(format!("plugin returned invalid JSON: {e}"))
        })?;

        let source = format!("{} ({})", self.indexer_name, self.descriptor.provider_type);
        let results = response
            .results
            .into_iter()
            .map(|r| {
                let thumbs_up = r
                    .extra
                    .get("thumbs_up")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                let thumbs_down = r
                    .extra
                    .get("thumbs_down")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                let subtitles: Option<Vec<String>> = r
                    .extra
                    .get("subtitles")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let password_protected = r
                    .extra
                    .get("password")
                    .or_else(|| r.extra.get("password_protected"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let source_kind = DownloadSourceKind::infer_from_indexer_result(
                    Some(&self.descriptor.plugin_type),
                    r.download_url.as_deref(),
                    r.link.as_deref(),
                    &r.extra,
                );

                IndexerSearchResult {
                    source: source.clone(),
                    title: r.title,
                    link: r.link,
                    download_url: r.download_url,
                    source_kind,
                    size_bytes: r.size_bytes,
                    published_at: r.published_at,
                    thumbs_up,
                    thumbs_down,
                    nzbgeek_languages: if r.languages.is_empty() {
                        None
                    } else {
                        Some(r.languages)
                    },
                    nzbgeek_subtitles: subtitles,
                    nzbgeek_grabs: r.grabs,
                    nzbgeek_password_protected: password_protected,
                    parsed_release_metadata: None,
                    quality_profile_decision: None,
                    extra: r.extra,
                    guid: r.guid,
                    info_url: r.info_url,
                }
            })
            .collect();

        Ok(IndexerSearchResponse {
            results,
            api_current: response.api_current,
            api_max: response.api_max,
            grab_current: response.grab_current,
            grab_max: response.grab_max,
        })
    }
}
