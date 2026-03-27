use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadClientConfigRepository,
    DownloadClientPluginProvider, DownloadGrabResult, DownloadSourceKind, SettingsRepository,
    accepted_inputs_for_client,
};
use scryer_domain::{DownloadClientConfig, DownloadQueueItem, MediaFacet};
use tracing::warn;

use super::nzbget::NzbgetDownloadClient;
use super::sabnzbd::SabnzbdDownloadClient;
use super::weaver::WeaverDownloadClient;
use super::{
    parse_download_client_config_json, read_config_string, resolve_download_client_base_url,
};

const DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY: &str = "download_client.routing";
const LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY: &str = "nzbget.client_routing";

#[derive(Clone)]
pub struct PrioritizedDownloadClientRouter {
    download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    settings: Arc<dyn SettingsRepository>,
    fallback_client: Arc<dyn DownloadClient>,
    plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
}

struct FacetClientSelection {
    clients: Vec<DownloadClientConfig>,
    all_disabled_for_facet: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DownloadClientRoutingEntry {
    enabled: bool,
    category: Option<String>,
    recent_queue_priority: Option<String>,
    older_queue_priority: Option<String>,
    remove_completed: bool,
    remove_failed: bool,
}

impl PrioritizedDownloadClientRouter {
    pub fn new(
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        settings: Arc<dyn SettingsRepository>,
        fallback_client: Arc<dyn DownloadClient>,
        plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
    ) -> Self {
        Self {
            download_client_configs,
            settings,
            fallback_client,
            plugin_provider,
        }
    }

    async fn list_enabled_clients_by_priority(&self) -> AppResult<Vec<DownloadClientConfig>> {
        let mut clients = self
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .collect::<Vec<_>>();
        clients.sort_by_key(|config| config.client_priority);
        Ok(clients)
    }

    fn request_source_kind(request: &DownloadClientAddRequest) -> Option<DownloadSourceKind> {
        request
            .source_kind
            .or_else(|| DownloadSourceKind::infer_from_hint(request.source_hint.as_deref()))
            .or_else(|| {
                request
                    .info_hash_hint
                    .as_ref()
                    .map(|_| DownloadSourceKind::TorrentFile)
            })
    }

    fn source_kind_label(kind: DownloadSourceKind) -> &'static str {
        match kind {
            DownloadSourceKind::NzbFile => "NZB file",
            DownloadSourceKind::NzbUrl => "NZB URL",
            DownloadSourceKind::TorrentFile => "torrent file",
            DownloadSourceKind::MagnetUri => "magnet",
        }
    }

    fn config_accepts_source_kind(
        config: &DownloadClientConfig,
        source_kind: DownloadSourceKind,
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
    ) -> bool {
        let accepted_inputs = accepted_inputs_for_client(&config.client_type, plugin_provider);
        if accepted_inputs.is_empty() {
            return false;
        }
        accepted_inputs.iter().any(|&accepted_kind| {
            // NzbFile and NzbUrl are interchangeable — scryer fetches the URL
            // and sends the file content, so any NZB-capable client handles both.
            match (accepted_kind, source_kind) {
                (DownloadSourceKind::NzbFile, DownloadSourceKind::NzbUrl)
                | (DownloadSourceKind::NzbUrl, DownloadSourceKind::NzbFile) => true,
                _ => accepted_kind == source_kind,
            }
        })
    }

    fn read_trimmed_string(raw_value: Option<&serde_json::Value>) -> Option<String> {
        raw_value
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    fn read_bool(raw_value: Option<&serde_json::Value>, default: bool) -> bool {
        match raw_value {
            Some(serde_json::Value::Bool(value)) => *value,
            Some(serde_json::Value::String(value)) => !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "false" | "0" | "no"
            ),
            Some(serde_json::Value::Number(value)) => value.as_i64() != Some(0),
            _ => default,
        }
    }

    fn parse_routing_object(raw_json: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
        serde_json::from_str::<serde_json::Value>(raw_json)
            .ok()?
            .as_object()
            .cloned()
    }

    fn parse_routing_entry(config: &serde_json::Value) -> DownloadClientRoutingEntry {
        DownloadClientRoutingEntry {
            enabled: Self::read_bool(config.get("enabled"), true),
            category: Self::read_trimmed_string(config.get("category")),
            recent_queue_priority: Self::read_trimmed_string(
                config
                    .get("recentQueuePriority")
                    .or_else(|| config.get("recentPriority"))
                    .or_else(|| config.get("recent_priority")),
            ),
            older_queue_priority: Self::read_trimmed_string(
                config
                    .get("olderQueuePriority")
                    .or_else(|| config.get("olderPriority"))
                    .or_else(|| config.get("older_priority")),
            ),
            remove_completed: Self::read_bool(
                config
                    .get("removeCompleted")
                    .or_else(|| config.get("remove_completed"))
                    .or_else(|| config.get("removeComplete")),
                false,
            ),
            remove_failed: Self::read_bool(
                config
                    .get("removeFailed")
                    .or_else(|| config.get("remove_failed"))
                    .or_else(|| config.get("removeFailure")),
                false,
            ),
        }
    }

    fn facet_scope_id(facet: &MediaFacet) -> &'static str {
        facet.as_str()
    }

    async fn get_download_client_routing_json(&self, scope_id: &str) -> AppResult<Option<String>> {
        if let Some(routing_json) = self
            .settings
            .get_setting_json(
                "system",
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await?
        {
            return Ok(Some(routing_json));
        }

        self.settings
            .get_setting_json(
                "system",
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await
    }

    /// Return enabled clients ordered by per-facet routing priority.
    /// Falls back to global `client_priority` if the facet has no routing config.
    async fn list_clients_for_facet(&self, facet: &MediaFacet) -> AppResult<FacetClientSelection> {
        let scope_id = Self::facet_scope_id(facet);

        let routing_json = self.get_download_client_routing_json(scope_id).await?;

        let mut clients = self
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .collect::<Vec<_>>();
        let any_globally_enabled = !clients.is_empty();
        let mut all_disabled_for_facet = false;

        match routing_json {
            Some(json_str) => {
                let routing_object = Self::parse_routing_object(&json_str);

                if let Some(routing_object) = routing_object {
                    let ordered_ids: Vec<String> = routing_object.keys().cloned().collect();
                    clients.retain(|client| {
                        routing_object
                            .get(&client.id)
                            .map(|entry| Self::read_bool(entry.get("enabled"), true))
                            .unwrap_or(true)
                    });
                    all_disabled_for_facet = any_globally_enabled && clients.is_empty();

                    if ordered_ids.is_empty() {
                        clients.sort_by_key(|c| c.client_priority);
                    } else {
                        clients.sort_by_key(|c| {
                            ordered_ids
                                .iter()
                                .position(|id| id == &c.id)
                                .unwrap_or(usize::MAX)
                        });
                    }
                } else {
                    clients.sort_by_key(|c| c.client_priority);
                }
            }
            None => {
                clients.sort_by_key(|c| c.client_priority);
            }
        }

        Ok(FacetClientSelection {
            clients,
            all_disabled_for_facet,
        })
    }

    async fn routing_entry_for_client(
        &self,
        facet: &MediaFacet,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientRoutingEntry>> {
        let scope_id = Self::facet_scope_id(facet);

        let Some(raw_json) = self.get_download_client_routing_json(scope_id).await? else {
            return Ok(None);
        };

        let Some(routing_object) = Self::parse_routing_object(&raw_json) else {
            return Ok(None);
        };

        Ok(routing_object.get(client_id).map(Self::parse_routing_entry))
    }

    fn normalized_request_category(request: &DownloadClientAddRequest) -> Option<String> {
        request
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    async fn apply_selected_client_routing(
        &self,
        request: &DownloadClientAddRequest,
        client_id: &str,
    ) -> AppResult<DownloadClientAddRequest> {
        let mut effective_request = request.clone();
        let routing_entry = self
            .routing_entry_for_client(&request.title.facet, client_id)
            .await?;

        effective_request.category = routing_entry
            .as_ref()
            .and_then(|entry| entry.category.clone())
            .or_else(|| Self::normalized_request_category(request));

        let is_recent = request.is_recent.unwrap_or(false);
        effective_request.queue_priority = routing_entry.and_then(|entry| {
            if is_recent {
                entry.recent_queue_priority
            } else {
                entry.older_queue_priority
            }
        });

        Ok(effective_request)
    }

    fn client_from_config(
        config: &DownloadClientConfig,
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
    ) -> AppResult<Arc<dyn DownloadClient>> {
        if let Some(provider) = plugin_provider
            && let Some(client) = provider.client_for_config(config)
        {
            return Ok(client);
        }

        match config.client_type.as_str() {
            "nzbget" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url =
                    resolve_download_client_base_url(&parsed_config).ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let username = read_config_string(&parsed_config, &["username"]);
                let password = read_config_string(&parsed_config, &["password"]);
                let dupe_mode = read_config_string(&parsed_config, &["dupe_mode", "dupeMode"])
                    .unwrap_or_else(|| "SCORE".to_string());
                let client = NzbgetDownloadClient::new(base_url, username, password, dupe_mode);
                Ok(Arc::new(client))
            }
            "sabnzbd" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url =
                    resolve_download_client_base_url(&parsed_config).ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let api_key = read_config_string(&parsed_config, &["api_key", "apiKey", "apikey"])
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} (sabnzbd) requires an API key",
                            config.id
                        ))
                    })?;
                let client = SabnzbdDownloadClient::new(base_url, api_key);
                Ok(Arc::new(client))
            }
            "weaver" => {
                let client = WeaverDownloadClient::from_config(config)?;
                Ok(Arc::new(client))
            }
            _ => Err(AppError::Validation(format!(
                "unsupported download client type '{}' for config {}",
                config.client_type, config.id
            ))),
        }
    }

    async fn resolve_client_for_queue_action(
        &self,
        id: &str,
        is_history: bool,
    ) -> AppResult<Option<Arc<dyn DownloadClient>>> {
        let configs = self.list_enabled_clients_by_priority().await?;
        if configs.is_empty() {
            return Ok(None);
        }

        let mut clients = Vec::new();
        for config in configs {
            match Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                Ok(client) => clients.push((config, client)),
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped while routing queue action"
                    );
                }
            }
        }

        if clients.is_empty() {
            return Ok(None);
        }

        for (config, client) in &clients {
            let items = if is_history {
                client.list_history().await
            } else {
                client.list_queue().await
            };

            match items {
                Ok(items) => {
                    if items.iter().any(|item| item.download_client_item_id == id) {
                        return Ok(Some(Arc::clone(client)));
                    }
                }
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        queue_item_id = id,
                        history = is_history,
                        error = %error,
                        "failed to inspect download client while routing queue action"
                    );
                }
            }
        }

        if clients.len() == 1 {
            return Ok(Some(Arc::clone(&clients[0].1)));
        }

        Err(AppError::Validation(format!(
            "download client item not found: {id}"
        )))
    }
}

#[async_trait]
impl DownloadClient for PrioritizedDownloadClientRouter {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let selection = match self.list_clients_for_facet(&request.title.facet).await {
            Ok(configs) => configs,
            Err(error) => {
                warn!(
                    error = %error,
                    title = request.title.name.as_str(),
                    facet = ?request.title.facet,
                    "failed to load prioritized download clients; falling back to default client"
                );
                return self.fallback_client.submit_download(request).await;
            }
        };

        if selection.all_disabled_for_facet {
            return Err(AppError::Validation(
                "no download client enabled for this facet".to_string(),
            ));
        }

        let mut clients = selection.clients;

        if clients.is_empty() {
            return self.fallback_client.submit_download(request).await;
        }

        if let Some(source_kind) = Self::request_source_kind(request) {
            clients.retain(|config| {
                let compatible = Self::config_accepts_source_kind(
                    config,
                    source_kind,
                    self.plugin_provider.as_ref(),
                );
                if !compatible {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        source_kind = source_kind.as_str(),
                        "download client skipped because it cannot handle this release type"
                    );
                }
                compatible
            });

            if clients.is_empty() {
                return Err(AppError::Validation(format!(
                    "no enabled download client can handle {} releases",
                    Self::source_kind_label(source_kind)
                )));
            }
        }

        let mut last_error: Option<AppError> = None;
        for config in clients {
            let client = match Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                Ok(client) => client,
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped due to invalid configuration"
                    );
                    last_error = Some(error);
                    continue;
                }
            };

            let effective_request = match self
                .apply_selected_client_routing(request, &config.id)
                .await
            {
                Ok(effective_request) => effective_request,
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped because routing configuration could not be resolved"
                    );
                    last_error = Some(error);
                    continue;
                }
            };

            match client.submit_download(&effective_request).await {
                Ok(result) => {
                    return Ok(DownloadGrabResult {
                        job_id: result.job_id,
                        client_type: config.client_type.clone(),
                    });
                }
                Err(error) => {
                    let should_failover = matches!(error, AppError::Repository(_));
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        failover = should_failover,
                        "download client enqueue failed"
                    );
                    if should_failover {
                        last_error = Some(error);
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::Repository(
                "all prioritized download clients failed to enqueue this release".to_string(),
            )
        }))
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_queue().await;
        }
        let mut all_items = Vec::new();
        for config in clients {
            let client = match Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for queue listing");
                    continue;
                }
            };
            match client.list_queue().await {
                Ok(mut items) => {
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list queue");
                }
            }
        }
        Ok(all_items)
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_history().await;
        }
        let mut all_items = Vec::new();
        for config in clients {
            let client = match Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for history listing");
                    continue;
                }
            };
            match client.list_history().await {
                Ok(mut items) => {
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list history");
                }
            }
        }
        Ok(all_items)
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_history_page(offset, limit).await;
        }

        let fetch_limit = offset.saturating_add(limit);
        let mut all_items = Vec::new();
        for config in clients {
            let client = match Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for paged history listing");
                    continue;
                }
            };
            match client.list_history_page(0, fetch_limit).await {
                Ok(mut items) => {
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list paged history");
                }
            }
        }

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items.sort_by(compare_history_items_desc);

        Ok(all_items.into_iter().skip(offset).take(limit).collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_completed_downloads().await;
        }
        let mut all_items = Vec::new();
        for config in clients {
            let client = match Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for completed downloads");
                    continue;
                }
            };
            match client.list_completed_downloads().await {
                Ok(mut items) => {
                    tracing::debug!(
                        client = %config.name,
                        client_type = %config.client_type,
                        count = items.len(),
                        "completed downloads from client"
                    );
                    for item in &mut items {
                        item.client_id = config.id.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    tracing::warn!(client_id = %config.id, client = %config.name, error = %error, "failed to list completed downloads");
                }
            }
        }
        Ok(all_items)
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_queue_action(id, false).await? {
            return client.pause_queue_item(id).await;
        }
        self.fallback_client.pause_queue_item(id).await
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_queue_action(id, false).await? {
            return client.resume_queue_item(id).await;
        }
        self.fallback_client.resume_queue_item(id).await
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_queue_action(id, is_history).await? {
            return client.delete_queue_item(id, is_history).await;
        }
        self.fallback_client.delete_queue_item(id, is_history).await
    }
}

fn parse_history_timestamp(value: Option<&str>) -> i64 {
    value
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
}

fn compare_history_items_desc(
    left: &DownloadQueueItem,
    right: &DownloadQueueItem,
) -> std::cmp::Ordering {
    parse_history_timestamp(right.last_updated_at.as_deref())
        .cmp(&parse_history_timestamp(left.last_updated_at.as_deref()))
        .then_with(|| right.id.cmp(&left.id))
}

fn download_queue_history_key(item: &DownloadQueueItem) -> String {
    if item.client_type.is_empty() && item.download_client_item_id.is_empty() {
        return item.id.clone();
    }

    format!("{}:{}", item.client_type, item.download_client_item_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct MockDownloadClientConfigRepository {
        configs: Vec<DownloadClientConfig>,
    }

    #[async_trait]
    impl DownloadClientConfigRepository for MockDownloadClientConfigRepository {
        async fn list(
            &self,
            _provider_type: Option<String>,
        ) -> AppResult<Vec<DownloadClientConfig>> {
            Ok(self.configs.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
            Ok(self.configs.iter().find(|config| config.id == id).cloned())
        }

        async fn create(&self, _config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
            unreachable!("not used in router tests")
        }

        async fn update(
            &self,
            _id: &str,
            _name: Option<String>,
            _client_type: Option<String>,
            _base_url: Option<String>,
            _config_json: Option<String>,
            _is_enabled: Option<bool>,
        ) -> AppResult<DownloadClientConfig> {
            unreachable!("not used in router tests")
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            unreachable!("not used in router tests")
        }

        async fn reorder(&self, _ordered_ids: Vec<String>) -> AppResult<()> {
            unreachable!("not used in router tests")
        }
    }

    #[derive(Default)]
    struct MockSettingsRepository {
        routing_by_scope: HashMap<String, String>,
    }

    #[async_trait]
    impl SettingsRepository for MockSettingsRepository {
        async fn get_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(scope_id.and_then(|id| self.routing_by_scope.get(&id).cloned()))
        }

        async fn upsert_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
            _value_json: String,
            _source: &str,
            _updated_by_user_id: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockDownloadClient {
        submissions: Mutex<Vec<DownloadClientAddRequest>>,
        queue_items: Mutex<Vec<DownloadQueueItem>>,
        history_items: Mutex<Vec<DownloadQueueItem>>,
        paused: Mutex<Vec<String>>,
        resumed: Mutex<Vec<String>>,
        deleted: Mutex<Vec<(String, bool)>>,
    }

    #[async_trait]
    impl DownloadClient for MockDownloadClient {
        async fn submit_download(
            &self,
            request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            self.submissions.lock().unwrap().push(request.clone());
            Ok(DownloadGrabResult {
                job_id: "job-1".to_string(),
                client_type: "mock".to_string(),
            })
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.queue_items.lock().unwrap().clone())
        }

        async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.history_items.lock().unwrap().clone())
        }

        async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
            self.paused.lock().unwrap().push(id.to_string());
            Ok(())
        }

        async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
            self.resumed.lock().unwrap().push(id.to_string());
            Ok(())
        }

        async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
            self.deleted
                .lock()
                .unwrap()
                .push((id.to_string(), is_history));
            Ok(())
        }
    }

    struct MockDownloadClientPluginProvider {
        accepted_inputs: Vec<String>,
        clients: Vec<(String, Arc<dyn DownloadClient>)>,
    }

    impl DownloadClientPluginProvider for MockDownloadClientPluginProvider {
        fn client_for_config(
            &self,
            config: &DownloadClientConfig,
        ) -> Option<Arc<dyn DownloadClient>> {
            self.clients
                .iter()
                .find(|(id, _)| id == &config.id)
                .map(|(_, client)| Arc::clone(client))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["qbittorrent".to_string()]
        }

        fn accepted_inputs_for_provider(&self, _provider_type: &str) -> Vec<String> {
            self.accepted_inputs.clone()
        }
    }

    fn test_title_for_facet(facet: MediaFacet) -> scryer_domain::Title {
        scryer_domain::Title {
            id: "title-1".to_string(),
            name: "Test Title".to_string(),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn test_title() -> scryer_domain::Title {
        test_title_for_facet(MediaFacet::Movie)
    }

    fn test_config(id: &str, name: &str, client_type: &str, priority: i64) -> DownloadClientConfig {
        DownloadClientConfig {
            id: id.to_string(),
            name: name.to_string(),
            client_type: client_type.to_string(),
            config_json: "{}".to_string(),
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            client_priority: priority,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn disabled_test_config(
        id: &str,
        name: &str,
        client_type: &str,
        priority: i64,
    ) -> DownloadClientConfig {
        DownloadClientConfig {
            is_enabled: false,
            ..test_config(id, name, client_type, priority)
        }
    }

    fn test_queue_item(id: &str) -> DownloadQueueItem {
        DownloadQueueItem {
            id: format!("queue-{id}"),
            title_id: None,
            title_name: "Test Download".to_string(),
            facet: None,
            client_id: String::new(),
            client_name: String::new(),
            client_type: "mock".to_string(),
            state: scryer_domain::DownloadQueueState::Queued,
            progress_percent: 0,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: id.to_string(),
            import_status: None,
            import_error_message: None,
            imported_at: None,
            is_scryer_origin: false,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        }
    }

    #[tokio::test]
    async fn submit_download_skips_incompatible_clients_by_source_kind() {
        let torrent_client = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string(), "magnet_uri".to_string()],
                clients: vec![("torrent".to_string(), torrent_client.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("nzb", "NZBGet", "nzbget", 0),
                    test_config("torrent", "qBittorrent", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            Some(plugin_provider),
        );

        let result = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                source_hint: Some("https://tracker.example/file.torrent".to_string()),
                source_kind: Some(DownloadSourceKind::TorrentFile),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("torrent request should route to torrent client");

        assert_eq!(result.client_type, "qbittorrent");
        assert_eq!(torrent_client.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_errors_when_no_enabled_client_can_handle_source_kind() {
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("nzb", "NZBGet", "nzbget", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            None,
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                source_hint: Some("magnet:?xt=urn:btih:abcdef".to_string()),
                source_kind: Some(DownloadSourceKind::MagnetUri),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("magnet request should fail when only nzb clients are enabled");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("magnet"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_download_skips_clients_disabled_for_facet() {
        let primary = Arc::new(MockDownloadClient::default());
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": false },
                        "secondary": { "enabled": true }
                    }"#
                    .to_string(),
                )]),
            }),
            Arc::new(MockDownloadClient::default()),
            Some(plugin_provider),
        );

        let result = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("secondary client should be used when primary is disabled for facet");

        assert_eq!(result.client_type, "qbittorrent");
        assert!(primary.submissions.lock().unwrap().is_empty());
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_respects_facet_specific_enablement_per_scope() {
        let primary = Arc::new(MockDownloadClient::default());
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([
                    (
                        "movie".to_string(),
                        r#"{
                            "primary": { "enabled": false },
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                    (
                        "anime".to_string(),
                        r#"{
                            "primary": { "enabled": true },
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                ]),
            }),
            Arc::new(MockDownloadClient::default()),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title_for_facet(MediaFacet::Movie),
                source_hint: Some("https://example.invalid/movie.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Movie Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("movie request should use secondary");

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title_for_facet(MediaFacet::Anime),
                source_hint: Some("https://example.invalid/anime.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Anime Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("anime request should use primary");

        assert_eq!(primary.submissions.lock().unwrap().len(), 1);
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_ignores_facet_enabled_flag_for_globally_disabled_clients() {
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("secondary".to_string(), secondary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    disabled_test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": true },
                        "secondary": { "enabled": true }
                    }"#
                    .to_string(),
                )]),
            }),
            Arc::new(MockDownloadClient::default()),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("secondary client should be used because primary is globally disabled");

        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_applies_selected_client_category_and_recent_queue_priority() {
        let primary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": {
                            "enabled": true,
                            "category": "Movies",
                            "recentQueuePriority": "high",
                            "olderQueuePriority": "low"
                        }
                    }"#
                    .to_string(),
                )]),
            }),
            Arc::new(MockDownloadClient::default()),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: Some("Fallback".to_string()),
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: Some(true),
                season_pack: None,
            })
            .await
            .expect("request should be routed");

        let submissions = primary.submissions.lock().unwrap();
        let request = submissions.first().expect("submission should be recorded");
        assert_eq!(request.category.as_deref(), Some("Movies"));
        assert_eq!(request.queue_priority.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn submit_download_uses_older_queue_priority_when_request_is_not_recent() {
        let primary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": {
                            "enabled": true,
                            "olderPriority": "very low"
                        }
                    }"#
                    .to_string(),
                )]),
            }),
            Arc::new(MockDownloadClient::default()),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: Some(false),
                season_pack: None,
            })
            .await
            .expect("request should be routed");

        let submissions = primary.submissions.lock().unwrap();
        let request = submissions.first().expect("submission should be recorded");
        assert_eq!(request.queue_priority.as_deref(), Some("very low"));
    }

    #[tokio::test]
    async fn submit_download_fails_when_all_clients_disabled_for_facet() {
        let fallback = Arc::new(MockDownloadClient::default());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": false }
                    }"#
                    .to_string(),
                )]),
            }),
            fallback.clone(),
            Some(Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![(
                    "primary".to_string(),
                    Arc::new(MockDownloadClient::default()),
                )],
            })),
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("facet-disabled clients should fail fast");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("no download client enabled"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }

        assert!(fallback.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn pause_queue_item_routes_to_matching_client_item_id() {
        let nzb_client = Arc::new(MockDownloadClient::default());
        nzb_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("123"));

        let sab_client = Arc::new(MockDownloadClient::default());
        sab_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("SABnzbd_nzo_95u9pco9"));

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("nzb".to_string(), nzb_client.clone()),
                    ("sab".to_string(), sab_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("nzb", "NZBGet", "nzbget", 0),
                    test_config("sab", "SABnzbd", "sabnzbd", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            Some(plugin_provider),
        );

        router
            .pause_queue_item("SABnzbd_nzo_95u9pco9")
            .await
            .expect("pause should route to sabnzbd client");

        assert!(nzb_client.paused.lock().unwrap().is_empty());
        assert_eq!(
            sab_client.paused.lock().unwrap().as_slice(),
            ["SABnzbd_nzo_95u9pco9"]
        );
    }

    #[tokio::test]
    async fn delete_history_item_routes_to_matching_client_item_id() {
        let nzb_client = Arc::new(MockDownloadClient::default());
        nzb_client
            .history_items
            .lock()
            .unwrap()
            .push(test_queue_item("42"));

        let sab_client = Arc::new(MockDownloadClient::default());
        sab_client
            .history_items
            .lock()
            .unwrap()
            .push(test_queue_item("SABnzbd_nzo_hist01"));

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("nzb".to_string(), nzb_client.clone()),
                    ("sab".to_string(), sab_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("nzb", "NZBGet", "nzbget", 0),
                    test_config("sab", "SABnzbd", "sabnzbd", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            Some(plugin_provider),
        );

        router
            .delete_queue_item("SABnzbd_nzo_hist01", true)
            .await
            .expect("history delete should route to sabnzbd client");

        assert!(nzb_client.deleted.lock().unwrap().is_empty());
        assert_eq!(
            sab_client.deleted.lock().unwrap().as_slice(),
            [("SABnzbd_nzo_hist01".to_string(), true)]
        );
    }

    #[tokio::test]
    async fn list_history_page_merges_clients_before_slicing() {
        let client_a = Arc::new(MockDownloadClient::default());
        let client_b = Arc::new(MockDownloadClient::default());

        let mut a1 = test_queue_item("a-1");
        a1.last_updated_at = Some("300".to_string());
        let mut a2 = test_queue_item("a-2");
        a2.last_updated_at = Some("100".to_string());
        client_a.history_items.lock().unwrap().extend([a1, a2]);

        let mut b1 = test_queue_item("b-1");
        b1.last_updated_at = Some("200".to_string());
        let mut b2 = test_queue_item("b-2");
        b2.last_updated_at = Some("50".to_string());
        client_b.history_items.lock().unwrap().extend([b1, b2]);

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("client-a".to_string(), client_a.clone()),
                    ("client-b".to_string(), client_b.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("client-a", "Client A", "qbittorrent", 0),
                    test_config("client-b", "Client B", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            Some(plugin_provider),
        );

        let page = router
            .list_history_page(1, 2)
            .await
            .expect("paged history should succeed");

        let ids = page
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["b-1".to_string(), "a-2".to_string()]);
    }
}
