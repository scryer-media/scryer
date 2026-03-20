use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, DownloadSourceKind, IndexerClient, IndexerRoutingPlan,
    IndexerSearchResponse, IndexerSearchResult, SearchMode,
};
use scryer_domain::TaggedAlias;
use tracing::warn;

use crate::types::{PluginDescriptor, PluginSearchRequest, PluginSearchResponse};

pub struct WasmIndexerClient {
    plugin: Arc<Mutex<extism::Plugin>>,
    descriptor: PluginDescriptor,
    indexer_name: String,
}

impl WasmIndexerClient {
    pub fn new(plugin: extism::Plugin, descriptor: PluginDescriptor, indexer_name: String) -> Self {
        Self {
            plugin: Arc::new(Mutex::new(plugin)),
            descriptor,
            indexer_name,
        }
    }
}

#[async_trait]
impl IndexerClient for WasmIndexerClient {
    async fn search(
        &self,
        query: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        anidb_id: Option<String>,
        category: Option<String>,
        newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        _absolute_episode: Option<u32>,
        tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        let request = PluginSearchRequest {
            query,
            imdb_id,
            tvdb_id,
            anidb_id,
            category,
            categories: newznab_categories.unwrap_or_default(),
            limit: 1000,
            season,
            episode,
            tagged_aliases,
        };

        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize plugin request: {e}"))
        })?;

        tracing::debug!(plugin = %self.descriptor.name, %input, "plugin search request");

        let plugin_name = self.descriptor.name.clone();
        let indexer_name = self.indexer_name.clone();

        // Plugin::call takes &mut self, so we use spawn_blocking + Arc<Mutex>
        // to avoid blocking the async runtime. The Arc clone is cheap and
        // satisfies spawn_blocking's 'static + Send requirements.
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
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
