use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, DownloadSourceKind, IndexerClient, IndexerRoutingPlan,
    IndexerSearchResponse, IndexerSearchResult, SearchMode,
};
use scryer_domain::IndexerConfig;
use scryer_domain::TaggedAlias;
use tracing::warn;

use crate::loader::{apply_allowed_hosts, build_plugin, parse_config_json_entries};
use crate::types::{PluginDescriptor, PluginSearchRequest, PluginSearchResponse};

pub struct WasmIndexerClient {
    wasm_bytes: Vec<u8>,
    descriptor: PluginDescriptor,
    indexer_name: String,
    config: IndexerConfig,
}

impl WasmIndexerClient {
    pub fn new(
        wasm_bytes: Vec<u8>,
        descriptor: PluginDescriptor,
        indexer_name: String,
        config: IndexerConfig,
    ) -> Self {
        Self {
            wasm_bytes,
            descriptor,
            indexer_name,
            config,
        }
    }
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
        let indexer_name_for_spawn = indexer_name.clone();
        let descriptor = self.descriptor.clone();
        let wasm_bytes = self.wasm_bytes.clone();
        let config = self.config.clone();
        let output = tokio::task::spawn_blocking(move || {
            let mut manifest = extism::Manifest::new([extism::Wasm::data(wasm_bytes)]);
            manifest = apply_allowed_hosts(
                manifest,
                &descriptor,
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
                            indexer = indexer_name_for_spawn.as_str(),
                            error = %error,
                            "failed to parse config_json; extra config keys will not be injected"
                        );
                    }
                }
            }

            let mut plugin = build_plugin(manifest).map_err(|e| {
                AppError::Repository(format!("failed to instantiate WASM plugin: {e}"))
            })?;

            plugin
                .call::<&str, String>("search", &input)
                .map_err(|e| AppError::Repository(format!("plugin search() failed: {e}")))
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let response: PluginSearchResponse = serde_json::from_str(&output).map_err(|e| {
            warn!(
                plugin = plugin_name.as_str(),
                indexer = indexer_name.as_str(),
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
