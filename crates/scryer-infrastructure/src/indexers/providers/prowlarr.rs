use std::collections::{BTreeSet, HashMap};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use reqwest::StatusCode;
use scryer_application::{
    AppError, AppResult, ExternalPluginWasm, IndexerClient, IndexerManagementClient,
    IndexerPluginProvider, IndexerRoutingPlan, IndexerSearchResponse, IndexerSyncPlan,
    IndexerValidationResult, ManagedIndexerChildPlan, ManagedIndexerRoutingScope,
    RateLimitCooldownAction, RuntimePluginLoad, SearchMode,
    external_import::{EXTERNAL_IMPORT_HOST_RPS_LANE, EXTERNAL_IMPORT_HOST_RPS_PROFILE},
};
use scryer_domain::{
    ConfigFieldDef, ConfigFieldRole, ConfigFieldType, ConfigFieldValueSource,
    IndexerCapsSearchNode as DomainCapsSearchNode, IndexerCapsSnapshot as DomainCapsSnapshot,
    IndexerConfig, TaggedAlias,
};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy, generic_reqwest_client,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, warn};

pub const PROWLARR_PROVIDER_TYPE: &str = "prowlarr";

const USER_AGENT: &str = "scryer-prowlarr/0.1";
const PROWLARR_CHILD_CAPS_FETCH_CONCURRENCY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProwlarrApiBucket {
    V2,
}

impl ProwlarrApiBucket {
    const fn system_status_path(self) -> &'static str {
        match self {
            Self::V2 => "/api/v1/system/status",
        }
    }

    const fn indexer_path(self) -> &'static str {
        match self {
            Self::V2 => "/api/v1/indexer",
        }
    }

    const fn app_profile_path(self) -> &'static str {
        match self {
            Self::V2 => "/api/v1/appprofile",
        }
    }

    const fn supported_major(self) -> u64 {
        match self {
            Self::V2 => 2,
        }
    }

    const fn request_namespace(self) -> &'static str {
        match self {
            Self::V2 => "prowlarr_v2",
        }
    }

    fn validate_status(self, status: &ProwlarrSystemStatus) -> Result<(), ProwlarrRequestError> {
        let app_name = status.app_name.trim();
        if !app_name.eq_ignore_ascii_case("Prowlarr") {
            let message = if app_name.is_empty() {
                "base_url responded but did not identify itself as Prowlarr".to_string()
            } else {
                format!("base_url responded as '{}', not Prowlarr", app_name)
            };
            return Err(ProwlarrRequestError::InvalidConfig(message));
        }

        let version = status.version.trim();
        let Some(version_major) = prowlarr_version_major(version) else {
            return Err(ProwlarrRequestError::Unsupported(format!(
                "could not determine Prowlarr version from '{}'",
                version
            )));
        };

        if version_major != self.supported_major() {
            return Err(ProwlarrRequestError::Unsupported(format!(
                "unsupported Prowlarr version '{}'; expected major {}",
                version,
                self.supported_major()
            )));
        }

        Ok(())
    }
}

const SYSTEM_STATUS_PATH: &str = ProwlarrApiBucket::V2.system_status_path();
const INDEXER_PATH: &str = ProwlarrApiBucket::V2.indexer_path();
const APP_PROFILE_PATH: &str = ProwlarrApiBucket::V2.app_profile_path();

fn prowlarr_version_major(version: &str) -> Option<u64> {
    version.trim().split('.').next()?.parse().ok()
}

#[derive(Debug, Clone)]
struct ProwlarrConfig {
    base_url: String,
    api_key: String,
}

impl ProwlarrConfig {
    fn from_indexer_config(config: &IndexerConfig) -> Result<Self, String> {
        let value = config
            .config_json
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(|error| format!("Prowlarr config_json is invalid: {error}"))?
            .unwrap_or(Value::Null);

        let base_url = value
            .get("base_url")
            .and_then(Value::as_str)
            .or_else(|| (!config.base_url.trim().is_empty()).then_some(config.base_url.as_str()))
            .unwrap_or_default()
            .trim()
            .trim_end_matches('/')
            .to_string();
        let api_key = value
            .get("api_key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();

        if base_url.is_empty() {
            return Err("Prowlarr requires a base_url".to_string());
        }
        if api_key.is_empty() {
            return Err("Prowlarr requires an api_key".to_string());
        }
        if api_key == "********" {
            return Err("Prowlarr API key is masked; enter the real key".to_string());
        }

        Ok(Self { base_url, api_key })
    }
}

#[derive(Debug)]
enum ProwlarrRequestError {
    InvalidConfig(String),
    AuthFailed(String),
    RateLimited(String, Option<i64>, RateLimitCooldownAction),
    Unreachable(String),
    Unsupported(String),
}

impl ProwlarrRequestError {
    fn to_validation_result(&self) -> IndexerValidationResult {
        match self {
            Self::InvalidConfig(message) => {
                validation_result("invalid_config", Some(message), None)
            }
            Self::AuthFailed(message) => validation_result("auth_failed", Some(message), None),
            Self::RateLimited(message, retry_after_seconds, _) => {
                validation_result("rate_limited", Some(message), *retry_after_seconds)
            }
            Self::Unreachable(message) => validation_result("unreachable", Some(message), None),
            Self::Unsupported(message) => validation_result("unsupported", Some(message), None),
        }
    }

    fn into_app_error(self) -> AppError {
        match self {
            Self::InvalidConfig(message) | Self::AuthFailed(message) => {
                AppError::Validation(message)
            }
            Self::Unreachable(message) | Self::Unsupported(message) => {
                AppError::Repository(message)
            }
            Self::RateLimited(message, Some(retry_after_seconds), cooldown_action) => {
                AppError::rate_limited_temporary_unavailable(
                    message,
                    Some(std::time::Duration::from_secs(
                        retry_after_seconds.max(1) as u64
                    )),
                    cooldown_action,
                )
            }
            Self::RateLimited(message, None, cooldown_action) => {
                AppError::rate_limited_temporary_unavailable(message, None, cooldown_action)
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ProwlarrSystemStatus {
    #[serde(default, rename = "appName")]
    app_name: String,
    version: String,
}

#[derive(Debug, Clone)]
struct ResolvedProwlarrApi {
    bucket: ProwlarrApiBucket,
    status: ProwlarrSystemStatus,
}

#[derive(Debug, Clone, Deserialize)]
struct ProwlarrIndexerResource {
    id: i64,
    name: String,
    #[serde(default, rename = "enable")]
    enable: bool,
    #[serde(default, rename = "appProfileId")]
    app_profile_id: i64,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    capabilities: ProwlarrIndexerCapabilities,
    #[serde(default)]
    priority: i64,
    #[serde(default, rename = "downloadClientId")]
    download_client_id: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ProwlarrIndexerCapabilities {
    #[serde(default)]
    categories: Vec<ProwlarrCategory>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ProwlarrCategory {
    id: i64,
    #[serde(default)]
    name: String,
    #[serde(default, rename = "subCategories")]
    sub_categories: Vec<ProwlarrCategory>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProwlarrAppProfile {
    id: i64,
    #[serde(default = "default_true", rename = "enableRss")]
    enable_rss: bool,
    #[serde(default = "default_true", rename = "enableAutomaticSearch")]
    enable_automatic_search: bool,
    #[serde(default = "default_true", rename = "enableInteractiveSearch")]
    enable_interactive_search: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProwlarrCapsSearchNode {
    pub available: bool,
    #[serde(default)]
    pub supported_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_engine: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProwlarrCapsSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits_default: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits_max: Option<i64>,
    #[serde(default)]
    pub search: ProwlarrCapsSearchNode,
    #[serde(default)]
    pub tv_search: ProwlarrCapsSearchNode,
    #[serde(default)]
    pub movie_search: ProwlarrCapsSearchNode,
    #[serde(default)]
    pub music_search: ProwlarrCapsSearchNode,
    #[serde(default)]
    pub audio_search: ProwlarrCapsSearchNode,
    #[serde(default)]
    pub book_search: ProwlarrCapsSearchNode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ManagedChildMetadata {
    indexer_id: i64,
    protocol: String,
    app_profile_id: i64,
    priority: i64,
    download_client_id: i64,
    enable_rss: bool,
    enable_automatic_search: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    caps_snapshot: Option<ProwlarrCapsSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RoutingScope {
    Movie,
    Series,
    Anime,
}

impl RoutingScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Anime => "anime",
        }
    }
}

pub struct NativeProwlarrIndexerProvider {
    delegate: Arc<dyn IndexerPluginProvider>,
}

impl NativeProwlarrIndexerProvider {
    pub fn new(delegate: Arc<dyn IndexerPluginProvider>) -> Self {
        Self { delegate }
    }
}

pub struct ProwlarrManagementClient {
    config: Result<ProwlarrConfig, String>,
    outbound_http: OutboundHttpClient,
    api_state: Arc<RwLock<Option<ResolvedProwlarrApi>>>,
}

impl ProwlarrManagementClient {
    fn new(config: &IndexerConfig) -> Self {
        let http_client = generic_reqwest_client();
        Self {
            config: ProwlarrConfig::from_indexer_config(config),
            outbound_http: OutboundHttpClient::new(http_client, RateLimitRegistry::new()),
            api_state: Arc::new(RwLock::new(None)),
        }
    }

    fn config(&self) -> AppResult<&ProwlarrConfig> {
        self.config
            .as_ref()
            .map_err(|message| AppError::Validation(message.clone()))
    }

    async fn fetch_system_status(&self) -> Result<ProwlarrSystemStatus, ProwlarrRequestError> {
        Ok(self.ensure_supported_api_bucket().await?.status)
    }

    async fn get_json<T>(&self, path: &str) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.get_json_raw(path)
            .await
            .map_err(ProwlarrRequestError::into_app_error)
    }

    async fn ensure_supported_api_bucket(
        &self,
    ) -> Result<ResolvedProwlarrApi, ProwlarrRequestError> {
        if let Some(api) = self.api_state.read().await.clone() {
            return Ok(api);
        }

        let mut guard = self.api_state.write().await;
        if let Some(api) = guard.clone() {
            return Ok(api);
        }

        let status: ProwlarrSystemStatus = self.get_json_raw(SYSTEM_STATUS_PATH).await?;
        let api = ResolvedProwlarrApi {
            bucket: ProwlarrApiBucket::V2,
            status,
        };
        api.bucket.validate_status(&api.status)?;
        *guard = Some(api.clone());
        Ok(api)
    }

    async fn get_json_raw<T>(&self, path: &str) -> Result<T, ProwlarrRequestError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let config = self
            .config
            .as_ref()
            .map_err(|message| ProwlarrRequestError::InvalidConfig(message.clone()))?;
        let base_url = config.base_url.clone();
        let api_key = config.api_key.clone();
        let url = api_url(&base_url, path);
        let response = self
            .outbound_http
            .send(self.request_policy(path), || {
                self.outbound_http
                    .client()
                    .get(&url)
                    .header("Accept", "application/json")
                    .header("User-Agent", USER_AGENT)
                    .header("X-Api-Key", &api_key)
            })
            .await
            .map_err(|error| match error {
                OutboundHttpError::RateLimited(rate_limited) => ProwlarrRequestError::RateLimited(
                    match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                        Some(delay) => {
                            format!(
                                "Prowlarr rate limited the request (retry after {}s)",
                                delay.as_secs()
                            )
                        }
                        None => "Prowlarr rate limited the request".to_string(),
                    },
                    rate_limited.retry_after.map(|delay| delay.as_secs() as i64),
                    RateLimitCooldownAction::AlreadyRecorded,
                ),
                OutboundHttpError::Transport { source, .. } => {
                    ProwlarrRequestError::Unreachable(format!("request failed: {source}"))
                }
            })?;

        let status = response.status();
        let retry_after_seconds = retry_after_seconds(response.headers());
        let body = response.bytes().await.map_err(|error| {
            ProwlarrRequestError::Unreachable(format!("Prowlarr response read failed: {error}"))
        })?;

        if status.is_success() {
            return serde_json::from_slice(&body).map_err(|error| {
                ProwlarrRequestError::Unsupported(format!(
                    "Prowlarr returned invalid JSON: {error}"
                ))
            });
        }

        Err(map_http_error(path, status, &body, retry_after_seconds))
    }

    async fn fetch_child_caps_snapshot(
        &self,
        config: &ProwlarrConfig,
        indexer_id: i64,
    ) -> Result<ProwlarrCapsSnapshot, ProwlarrRequestError> {
        let base_url = format!("{}/{}", config.base_url.trim_end_matches('/'), indexer_id);
        let url = format!(
            "{}{}?t=caps&apikey={}",
            base_url.trim_end_matches('/'),
            "/api",
            config.api_key
        );
        let request_path = format!("/{indexer_id}/api?t=caps");
        let response = self
            .outbound_http
            .send(self.request_policy(&request_path), || {
                self.outbound_http
                    .client()
                    .get(&url)
                    .header("Accept", "application/xml, text/xml, application/rss+xml")
                    .header("User-Agent", USER_AGENT)
            })
            .await
            .map_err(|error| match error {
                OutboundHttpError::RateLimited(rate_limited) => ProwlarrRequestError::RateLimited(
                    match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                        Some(delay) => {
                            format!(
                                "Prowlarr rate limited the child caps request (retry after {}s)",
                                delay.as_secs()
                            )
                        }
                        None => "Prowlarr rate limited the child caps request".to_string(),
                    },
                    rate_limited.retry_after.map(|delay| delay.as_secs() as i64),
                    RateLimitCooldownAction::AlreadyRecorded,
                ),
                OutboundHttpError::Transport { source, .. } => {
                    ProwlarrRequestError::Unreachable(format!("request failed: {source}"))
                }
            })?;

        let status = response.status();
        let retry_after_seconds = retry_after_seconds(response.headers());
        let body = response.bytes().await.map_err(|error| {
            ProwlarrRequestError::Unreachable(format!("Prowlarr response read failed: {error}"))
        })?;

        if status.is_success() {
            return parse_caps_snapshot(&body);
        }

        Err(map_http_error(
            &request_path,
            status,
            &body,
            retry_after_seconds,
        ))
    }

    async fn build_sync_plan(&self, fetch_caps: bool) -> AppResult<IndexerSyncPlan> {
        let config = self.config()?.clone();
        let api = self
            .ensure_supported_api_bucket()
            .await
            .map_err(ProwlarrRequestError::into_app_error)?;
        let indexers: Vec<ProwlarrIndexerResource> =
            self.get_json(api.bucket.indexer_path()).await?;
        let app_profiles: Vec<ProwlarrAppProfile> =
            self.get_json(api.bucket.app_profile_path()).await?;
        let app_profiles_by_id = app_profiles
            .into_iter()
            .map(|profile| (profile.id, profile))
            .collect::<HashMap<_, _>>();

        if !fetch_caps {
            let children = indexers
                .into_iter()
                .filter_map(|indexer| {
                    build_managed_child_plan(&config, indexer, &app_profiles_by_id, None)
                })
                .collect();
            return Ok(IndexerSyncPlan { children });
        }

        let mut planned_children = stream::iter(indexers.into_iter().enumerate())
            .map(|(position, indexer)| {
                let config = config.clone();
                async move {
                    let child_key = indexer.id.to_string();
                    let caps_snapshot = if indexer.enable {
                        match self.fetch_child_caps_snapshot(&config, indexer.id).await {
                            Ok(snapshot) => {
                                debug!(
                                    child_key,
                                    movie_params = ?snapshot.movie_search.supported_params,
                                    tv_params = ?snapshot.tv_search.supported_params,
                                    "fetched managed child caps snapshot"
                                );
                                Some(snapshot)
                            }
                            Err(error) => {
                                warn!(
                                    child_key,
                                    error = ?error,
                                    "failed to fetch managed child caps snapshot; child will fall back to query-only search"
                                );
                                None
                            }
                        }
                    } else {
                        debug!(
                            child_key,
                            "skipping managed child caps fetch because the upstream Prowlarr indexer is disabled"
                        );
                        None
                    };

                    (position, indexer, caps_snapshot)
                }
            })
            .buffer_unordered(PROWLARR_CHILD_CAPS_FETCH_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        planned_children.sort_by_key(|(position, _, _)| *position);

        let children = planned_children
            .into_iter()
            .filter_map(|(_, indexer, caps_snapshot)| {
                build_managed_child_plan(&config, indexer, &app_profiles_by_id, caps_snapshot)
            })
            .collect();

        Ok(IndexerSyncPlan { children })
    }

    fn request_policy(&self, path: &str) -> RequestPolicy {
        let base_url = self
            .config
            .as_ref()
            .map(|config| config.base_url.as_str())
            .unwrap_or("invalid-config");
        let request_namespace = self
            .api_state
            .try_read()
            .ok()
            .and_then(|guard| guard.as_ref().map(|api| api.bucket.request_namespace()))
            .unwrap_or(ProwlarrApiBucket::V2.request_namespace());
        RequestPolicy::safe_read(
            format!("prowlarr:{request_namespace}:{base_url}"),
            format!("prowlarr:{request_namespace}:{path}"),
        )
        .with_max_retries(2)
        .with_backoff(Duration::from_secs(1), Duration::from_secs(15))
        .with_host_rps_limit(
            EXTERNAL_IMPORT_HOST_RPS_LANE,
            EXTERNAL_IMPORT_HOST_RPS_PROFILE,
        )
    }
}

pub struct ProwlarrSearchStub;

#[async_trait]
impl IndexerClient for ProwlarrSearchStub {
    async fn search(
        &self,
        _query: String,
        _ids: HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<scryer_application::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        Err(AppError::Validation(
            "Prowlarr parent configs are management-only; search through synced child indexers"
                .to_string(),
        ))
    }
}

#[async_trait]
impl IndexerManagementClient for ProwlarrManagementClient {
    async fn validate_connection(&self) -> AppResult<IndexerValidationResult> {
        match self.fetch_system_status().await {
            Ok(status) => {
                if !status.app_name.trim().eq_ignore_ascii_case("Prowlarr") {
                    let message = if status.app_name.trim().is_empty() {
                        "base_url responded but did not identify itself as Prowlarr".to_string()
                    } else {
                        format!(
                            "base_url responded as '{}', not Prowlarr",
                            status.app_name.trim()
                        )
                    };
                    return Ok(ProwlarrRequestError::InvalidConfig(message).to_validation_result());
                }

                Ok(validation_result(
                    "valid",
                    Some(&format!("Connected to Prowlarr {}", status.version)),
                    None,
                ))
            }
            Err(error) => Ok(error.to_validation_result()),
        }
    }

    async fn plan_sync(&self, _parent_config_id: &str) -> AppResult<IndexerSyncPlan> {
        self.build_sync_plan(false).await
    }

    async fn enrichment_sync_plan(
        &self,
        _parent_config_id: &str,
    ) -> AppResult<Option<IndexerSyncPlan>> {
        self.build_sync_plan(true).await.map(Some)
    }

    async fn preview_sync_plan(&self, _parent_config_id: &str) -> AppResult<IndexerSyncPlan> {
        self.build_sync_plan(false).await
    }

    fn name(&self) -> &str {
        PROWLARR_PROVIDER_TYPE
    }
}

impl IndexerPluginProvider for NativeProwlarrIndexerProvider {
    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        if is_prowlarr_provider(&config.provider_type) {
            return Some(Arc::new(ProwlarrSearchStub));
        }
        self.delegate.client_for_provider(config)
    }

    fn client_for_provider_with_proxy(
        &self,
        config: &IndexerConfig,
        proxy_config: Option<&scryer_domain::IndexerProxyConfig>,
    ) -> Option<Arc<dyn IndexerClient>> {
        if is_prowlarr_provider(&config.provider_type) {
            return Some(Arc::new(ProwlarrSearchStub));
        }
        self.delegate
            .client_for_provider_with_proxy(config, proxy_config)
    }

    fn management_client_for_provider(
        &self,
        config: &IndexerConfig,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        if is_prowlarr_provider(&config.provider_type) {
            return Some(Arc::new(ProwlarrManagementClient::new(config)));
        }
        self.delegate.management_client_for_provider(config)
    }

    fn available_provider_types(&self) -> Vec<String> {
        let mut providers = self
            .delegate
            .available_provider_types()
            .into_iter()
            .filter(|provider| !is_prowlarr_provider(provider))
            .collect::<Vec<_>>();
        providers.push(PROWLARR_PROVIDER_TYPE.to_string());
        providers.sort();
        providers.dedup();
        providers
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        self.delegate
            .builtin_provider_types()
            .into_iter()
            .filter(|provider| !is_prowlarr_provider(provider))
            .collect()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        (!is_prowlarr_provider(provider_type))
            .then(|| self.delegate.plugin_version_for_provider(provider_type))
            .flatten()
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        (!is_prowlarr_provider(provider_type))
            .then(|| self.delegate.plugin_sdk_version_for_provider(provider_type))
            .flatten()
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        (!is_prowlarr_provider(provider_type))
            .then(|| {
                self.delegate
                    .plugin_sdk_constraint_for_provider(provider_type)
            })
            .flatten()
    }

    fn plugin_type_for_provider(&self, provider_type: &str) -> Option<String> {
        if is_prowlarr_provider(provider_type) {
            return Some("indexer".to_string());
        }
        self.delegate.plugin_type_for_provider(provider_type)
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        self.delegate.scoring_policies()
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        if is_prowlarr_provider(plugin.descriptor.provider_type()) {
            tracing::warn!(
                "ignoring runtime plugin that tried to claim reserved provider 'prowlarr'"
            );
            return Ok(());
        }
        self.delegate.upsert_runtime_plugin(plugin)
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        if is_prowlarr_provider(provider_type) {
            return Ok(());
        }
        self.delegate.remove_runtime_plugin(provider_type)
    }

    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        if is_prowlarr_provider(provider_type) {
            return Ok(());
        }
        self.delegate.restore_builtin_plugin(provider_type)
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let disabled_builtins = filter_reserved_prowlarr(disabled_builtins);
        self.delegate
            .reload_plugins(external_wasm_bytes, &disabled_builtins)
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let runtime_plugins = runtime_plugins
            .iter()
            .filter(|plugin| !is_prowlarr_provider(plugin.descriptor.provider_type()))
            .cloned()
            .collect::<Vec<_>>();
        let disabled_builtins = filter_reserved_prowlarr(disabled_builtins);
        self.delegate
            .reload_runtime_plugins(&runtime_plugins, &disabled_builtins)
    }

    fn config_fields_for_provider(&self, provider_type: &str) -> Vec<ConfigFieldDef> {
        if is_prowlarr_provider(provider_type) {
            return prowlarr_config_fields();
        }
        self.delegate.config_fields_for_provider(provider_type)
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        if is_prowlarr_provider(provider_type) {
            return Some("Prowlarr".to_string());
        }
        self.delegate.plugin_name_for_provider(provider_type)
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        if is_prowlarr_provider(provider_type) {
            return Some(
                "First-party Prowlarr provider that syncs managed Newznab and Torznab indexers"
                    .to_string(),
            );
        }
        self.delegate.plugin_description_for_provider(provider_type)
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        if is_prowlarr_provider(provider_type) {
            return None;
        }
        self.delegate.default_base_url_for_provider(provider_type)
    }

    fn rate_limit_seconds_for_provider(&self, provider_type: &str) -> Option<i64> {
        if is_prowlarr_provider(provider_type) {
            return None;
        }
        self.delegate.rate_limit_seconds_for_provider(provider_type)
    }

    fn management_capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerManagementCapabilities {
        if is_prowlarr_provider(provider_type) {
            return scryer_domain::IndexerManagementCapabilities {
                supports_validate_config: true,
                supports_managed_children_sync: true,
            };
        }
        self.delegate
            .management_capabilities_for_provider(provider_type)
    }

    fn capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        if is_prowlarr_provider(provider_type) {
            return scryer_domain::IndexerProviderCapabilities::default();
        }
        self.delegate.capabilities_for_provider(provider_type)
    }
}

fn is_prowlarr_provider(provider_type: &str) -> bool {
    provider_type
        .trim()
        .eq_ignore_ascii_case(PROWLARR_PROVIDER_TYPE)
}

fn filter_reserved_prowlarr(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter(|value| !is_prowlarr_provider(value))
        .cloned()
        .collect()
}

fn prowlarr_config_fields() -> Vec<ConfigFieldDef> {
    vec![
        ConfigFieldDef {
            key: "base_url".to_string(),
            label: "Base URL".to_string(),
            field_type: ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::User,
            role: Some(ConfigFieldRole::ConnectionUrl),
            host_binding: None,
            options: vec![],
            help_text: Some("Prowlarr server URL, for example http://prowlarr:9696".to_string()),
        },
        ConfigFieldDef {
            key: "api_key".to_string(),
            label: "API Key".to_string(),
            field_type: ConfigFieldType::Password,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::User,
            role: None,
            host_binding: None,
            options: vec![],
            help_text: Some("Prowlarr API key".to_string()),
        },
    ]
}

fn validation_result(
    status: &str,
    message: Option<&str>,
    retry_after_seconds: Option<i64>,
) -> IndexerValidationResult {
    IndexerValidationResult {
        status: status.to_string(),
        message: message.map(str::to_string),
        retry_after_seconds,
    }
}

fn build_managed_child_plan(
    config: &ProwlarrConfig,
    indexer: ProwlarrIndexerResource,
    app_profiles_by_id: &HashMap<i64, ProwlarrAppProfile>,
    caps_snapshot: Option<ProwlarrCapsSnapshot>,
) -> Option<ManagedIndexerChildPlan> {
    let provider_type = provider_type_for_protocol(&indexer.protocol)?;
    let app_profile = app_profiles_by_id.get(&indexer.app_profile_id);
    let enable_rss = app_profile
        .map(|profile| profile.enable_rss)
        .unwrap_or(true);
    let enable_automatic_search = app_profile
        .map(|profile| profile.enable_automatic_search)
        .unwrap_or(true);
    let enable_interactive_search = app_profile
        .map(|profile| profile.enable_interactive_search)
        .unwrap_or(true);
    let name = indexer.name.trim();
    let name = if name.is_empty() {
        format!("Prowlarr indexer {}", indexer.id)
    } else {
        name.to_string()
    };
    let routing_categories = collect_routing_categories(&indexer.capabilities.categories);
    let routing_scopes = [
        RoutingScope::Movie,
        RoutingScope::Series,
        RoutingScope::Anime,
    ]
    .into_iter()
    .filter_map(|scope| {
        routing_categories
            .get(scope.as_str())
            .map(|categories| ManagedIndexerRoutingScope {
                scope_id: scope.as_str().to_string(),
                categories: categories.clone(),
            })
    })
    .collect::<Vec<_>>();

    let config_json = serde_json::json!({
        "base_url": format!("{}/{}", config.base_url.trim_end_matches('/'), indexer.id),
        "api_key": config.api_key,
        "api_path": "/api",
        "imdb_id_format": "canonical",
    });
    let caps_snapshot_json = serialize_caps_snapshot(caps_snapshot.as_ref());
    let managed_metadata_json = serde_json::to_string(&ManagedChildMetadata {
        indexer_id: indexer.id,
        protocol: indexer.protocol.clone(),
        app_profile_id: indexer.app_profile_id,
        priority: indexer.priority,
        download_client_id: indexer.download_client_id,
        enable_rss,
        enable_automatic_search,
        caps_snapshot,
    })
    .ok();

    Some(ManagedIndexerChildPlan {
        child_key: indexer.id.to_string(),
        name,
        provider_type: provider_type.to_string(),
        config_json: serde_json::to_string(&config_json).ok()?,
        is_enabled: indexer.enable,
        enable_interactive_search,
        enable_auto_search: enable_rss || enable_automatic_search,
        managed_metadata_json,
        caps_snapshot_json,
        routing_scopes,
    })
}

fn provider_type_for_protocol(protocol: &str) -> Option<&'static str> {
    match protocol.trim().to_ascii_lowercase().as_str() {
        "usenet" => Some("newznab"),
        "torrent" => Some("torznab"),
        _ => None,
    }
}

fn collect_routing_categories(categories: &[ProwlarrCategory]) -> HashMap<String, Vec<String>> {
    let mut routing = HashMap::<RoutingScope, BTreeSet<String>>::new();
    for category in categories {
        collect_routing_category(category, None, &mut routing);
    }

    routing
        .into_iter()
        .map(|(scope, categories)| (scope.as_str().to_string(), categories.into_iter().collect()))
        .collect()
}

fn collect_routing_category(
    category: &ProwlarrCategory,
    inherited_scope: Option<RoutingScope>,
    routing: &mut HashMap<RoutingScope, BTreeSet<String>>,
) {
    let scope = classify_scope(category).or(inherited_scope);
    if let Some(scope) = scope {
        routing
            .entry(scope)
            .or_default()
            .insert(category.id.to_string());
    }

    for sub_category in &category.sub_categories {
        collect_routing_category(sub_category, scope, routing);
    }
}

fn classify_scope(category: &ProwlarrCategory) -> Option<RoutingScope> {
    let name = category.name.trim().to_ascii_lowercase();
    if name.contains("anime") {
        return Some(RoutingScope::Anime);
    }
    if (2000..3000).contains(&category.id) || name.contains("movie") {
        return Some(RoutingScope::Movie);
    }
    if (5000..6000).contains(&category.id) || name == "tv" || name.contains("series") {
        return Some(RoutingScope::Series);
    }
    None
}

fn api_url(base_url: &str, path: &str) -> String {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{}{}", base_url.trim_end_matches('/'), path)
}

fn retry_after_seconds(headers: &reqwest::header::HeaderMap) -> Option<i64> {
    headers
        .get("retry-after")
        .or_else(|| headers.get("x-retry-after"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
}

fn map_http_error(
    path: &str,
    status: StatusCode,
    body: &[u8],
    retry_after_seconds: Option<i64>,
) -> ProwlarrRequestError {
    let body_text = String::from_utf8_lossy(body).trim().to_string();
    match status.as_u16() {
        400 => ProwlarrRequestError::InvalidConfig(non_empty_or(
            body_text,
            "Prowlarr rejected the request as invalid",
        )),
        401 | 403 => ProwlarrRequestError::AuthFailed(non_empty_or(
            body_text,
            "Prowlarr rejected the API key",
        )),
        404 => ProwlarrRequestError::InvalidConfig(match path {
            SYSTEM_STATUS_PATH => "base_url does not appear to point at a Prowlarr API".to_string(),
            INDEXER_PATH | APP_PROFILE_PATH => {
                format!("Prowlarr sync endpoint '{path}' was not found")
            }
            _ => format!("Prowlarr endpoint '{path}' was not found"),
        }),
        429 => ProwlarrRequestError::RateLimited(
            non_empty_or(body_text, "Prowlarr rate limited the request"),
            retry_after_seconds,
            RateLimitCooldownAction::RecordFallback,
        ),
        500..=599 => ProwlarrRequestError::Unreachable(non_empty_or(
            body_text,
            &format!("Prowlarr returned HTTP {status}"),
        )),
        _ => ProwlarrRequestError::Unsupported(non_empty_or(
            body_text,
            &format!("Prowlarr returned HTTP {status}"),
        )),
    }
}

fn non_empty_or(value: String, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn default_true() -> bool {
    true
}

fn parse_caps_snapshot(body: &[u8]) -> Result<ProwlarrCapsSnapshot, ProwlarrRequestError> {
    let mut reader = Reader::from_reader(Cursor::new(body));
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut snapshot = ProwlarrCapsSnapshot::default();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                match element.name().as_ref() {
                    b"server" => {
                        snapshot.server_title = attr_value(&element, b"title")?;
                    }
                    b"limits" => {
                        snapshot.limits_default = attr_i64(&element, b"default")?;
                        snapshot.limits_max = attr_i64(&element, b"max")?;
                    }
                    b"search" => {
                        snapshot.search = parse_caps_node(&element)?;
                    }
                    b"tv-search" => {
                        snapshot.tv_search = parse_caps_node(&element)?;
                    }
                    b"movie-search" => {
                        snapshot.movie_search = parse_caps_node(&element)?;
                    }
                    b"music-search" => {
                        snapshot.music_search = parse_caps_node(&element)?;
                    }
                    b"audio-search" => {
                        snapshot.audio_search = parse_caps_node(&element)?;
                    }
                    b"book-search" => {
                        snapshot.book_search = parse_caps_node(&element)?;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(ProwlarrRequestError::Unsupported(format!(
                    "Prowlarr returned invalid caps XML: {error}"
                )));
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(snapshot)
}

fn parse_caps_node(
    element: &BytesStart<'_>,
) -> Result<ProwlarrCapsSearchNode, ProwlarrRequestError> {
    Ok(ProwlarrCapsSearchNode {
        available: attr_value(element, b"available")?
            .is_some_and(|value| value.eq_ignore_ascii_case("yes")),
        supported_params: attr_value(element, b"supportedParams")?
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .collect(),
        search_engine: attr_value(element, b"searchEngine")?
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    })
}

fn serialize_caps_snapshot(snapshot: Option<&ProwlarrCapsSnapshot>) -> Option<String> {
    let snapshot = snapshot?;
    serde_json::to_string(&DomainCapsSnapshot {
        server_title: snapshot.server_title.clone(),
        limits_default: snapshot.limits_default,
        limits_max: snapshot.limits_max,
        search: caps_node_to_domain(&snapshot.search),
        tv_search: caps_node_to_domain(&snapshot.tv_search),
        movie_search: caps_node_to_domain(&snapshot.movie_search),
        music_search: caps_node_to_domain(&snapshot.music_search),
        audio_search: caps_node_to_domain(&snapshot.audio_search),
        book_search: caps_node_to_domain(&snapshot.book_search),
        categories: Default::default(),
    })
    .ok()
}

fn caps_node_to_domain(node: &ProwlarrCapsSearchNode) -> Option<DomainCapsSearchNode> {
    if !node.available && node.supported_params.is_empty() && node.search_engine.is_none() {
        return None;
    }

    Some(DomainCapsSearchNode {
        available: node.available,
        supported_params: node.supported_params.clone(),
        search_engine: node.search_engine.clone(),
    })
}

fn attr_value(
    element: &BytesStart<'_>,
    key: &[u8],
) -> Result<Option<String>, ProwlarrRequestError> {
    for attribute in element.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| {
            ProwlarrRequestError::Unsupported(format!(
                "Prowlarr returned invalid caps XML attributes: {error}"
            ))
        })?;
        if attribute.key.as_ref() == key {
            let value = std::str::from_utf8(attribute.value.as_ref()).map_err(|error| {
                ProwlarrRequestError::Unsupported(format!(
                    "Prowlarr returned non-UTF8 caps attribute values: {error}"
                ))
            })?;
            return Ok(Some(value.to_string()));
        }
    }

    Ok(None)
}

fn attr_i64(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<i64>, ProwlarrRequestError> {
    attr_value(element, key)?.map_or(Ok(None), |value| {
        value.trim().parse::<i64>().map(Some).map_err(|error| {
            ProwlarrRequestError::Unsupported(format!(
                "Prowlarr returned invalid numeric caps values: {error}"
            ))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::IndexerConfig;
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[derive(Default)]
    struct ProxyRecordingProvider {
        observed_proxy_id: Arc<std::sync::Mutex<Option<String>>>,
    }

    impl IndexerPluginProvider for ProxyRecordingProvider {
        fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
            None
        }

        fn client_for_provider_with_proxy(
            &self,
            _config: &IndexerConfig,
            proxy_config: Option<&scryer_domain::IndexerProxyConfig>,
        ) -> Option<Arc<dyn IndexerClient>> {
            *self.observed_proxy_id.lock().expect("proxy observation") =
                proxy_config.map(|config| config.id.clone());
            None
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["newznab".to_string()]
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            Vec::new()
        }
    }

    #[test]
    fn non_prowlarr_clients_preserve_indexer_proxy_configuration() {
        let delegate = Arc::new(ProxyRecordingProvider::default());
        let observed_proxy_id = Arc::clone(&delegate.observed_proxy_id);
        let provider = NativeProwlarrIndexerProvider::new(delegate);
        let mut config = test_indexer_config("http://newznab:8088");
        config.provider_type = "newznab".to_string();
        let now = Utc::now();
        let proxy_config = scryer_domain::IndexerProxyConfig {
            id: "proxy-1".to_string(),
            name: "Byparr".to_string(),
            provider_type: scryer_domain::IndexerProxyProviderType::Byparr,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: "http://byparr:8191".to_string(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };

        assert!(
            provider
                .client_for_provider_with_proxy(&config, Some(&proxy_config))
                .is_none()
        );
        assert_eq!(
            observed_proxy_id
                .lock()
                .expect("proxy observation")
                .as_deref(),
            Some("proxy-1")
        );
    }

    #[test]
    fn prowlarr_management_requests_use_importer_host_quota() {
        let config = test_indexer_config("http://127.0.0.1:9696");
        let client = ProwlarrManagementClient::new(&config);
        let request_override = client
            .request_policy("/api/v1/indexer")
            .host_rps_override
            .expect("Prowlarr management requests should select an importer quota");

        assert_eq!(
            request_override.lane.as_ref(),
            EXTERNAL_IMPORT_HOST_RPS_LANE
        );
        assert_eq!(request_override.profile, EXTERNAL_IMPORT_HOST_RPS_PROFILE);
    }

    fn test_indexer_config(base_url: &str) -> IndexerConfig {
        IndexerConfig {
            id: "cfg-1".to_string(),
            name: "Prowlarr".to_string(),
            provider_type: PROWLARR_PROVIDER_TYPE.to_string(),
            base_url: base_url.to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: false,
            enable_auto_search: false,
            indexer_proxy_config_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_at: None,
            config_json: Some(
                json!({
                    "base_url": base_url,
                    "api_key": "secret",
                })
                .to_string(),
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn rate_limited_request_error_preserves_retry_after_as_temporary_unavailable() {
        let error = ProwlarrRequestError::RateLimited(
            "Prowlarr rate limited the request".to_string(),
            Some(120),
            RateLimitCooldownAction::RecordFallback,
        )
        .into_app_error();

        match error {
            AppError::TemporaryUnavailable {
                message,
                retry_after,
                ..
            } => {
                assert_eq!(message, "Prowlarr rate limited the request");
                assert_eq!(retry_after, Some(Duration::from_secs(120)));
            }
            other => panic!("expected temporary unavailable error, got {other:?}"),
        }
    }

    #[test]
    fn anime_subcategories_do_not_leak_into_series_routing() {
        let routing = collect_routing_categories(&[ProwlarrCategory {
            id: 5000,
            name: "TV".to_string(),
            sub_categories: vec![
                ProwlarrCategory {
                    id: 5030,
                    name: "TV/HD".to_string(),
                    sub_categories: vec![],
                },
                ProwlarrCategory {
                    id: 5070,
                    name: "TV/Anime".to_string(),
                    sub_categories: vec![],
                },
            ],
        }]);

        assert_eq!(
            routing.get("series"),
            Some(&vec!["5000".to_string(), "5030".to_string()])
        );
        assert_eq!(routing.get("anime"), Some(&vec!["5070".to_string()]));
    }

    #[test]
    fn managed_child_uses_proxy_path_and_app_profile_flags() {
        let config = ProwlarrConfig {
            base_url: "https://prowlarr.example".to_string(),
            api_key: "secret".to_string(),
        };
        let caps_snapshot = ProwlarrCapsSnapshot {
            server_title: Some("Prowlarr".to_string()),
            limits_default: Some(100),
            limits_max: Some(100),
            movie_search: ProwlarrCapsSearchNode {
                available: true,
                supported_params: vec!["q".to_string(), "imdbid".to_string()],
                search_engine: None,
            },
            ..ProwlarrCapsSnapshot::default()
        };
        let indexer = ProwlarrIndexerResource {
            id: 7,
            name: "Indexer Seven".to_string(),
            enable: true,
            app_profile_id: 12,
            protocol: "torrent".to_string(),
            capabilities: ProwlarrIndexerCapabilities {
                categories: vec![ProwlarrCategory {
                    id: 2000,
                    name: "Movies".to_string(),
                    sub_categories: vec![],
                }],
            },
            priority: 25,
            download_client_id: 3,
        };
        let app_profiles = HashMap::from([(
            12,
            ProwlarrAppProfile {
                id: 12,
                enable_rss: true,
                enable_automatic_search: false,
                enable_interactive_search: true,
            },
        )]);

        let child =
            build_managed_child_plan(&config, indexer, &app_profiles, Some(caps_snapshot.clone()))
                .expect("child plan");
        let config_json: Value = serde_json::from_str(&child.config_json).unwrap();
        let metadata: ManagedChildMetadata =
            serde_json::from_str(child.managed_metadata_json.as_deref().unwrap()).unwrap();

        assert_eq!(child.provider_type, "torznab");
        assert_eq!(config_json["base_url"], "https://prowlarr.example/7");
        assert_eq!(config_json["api_key"], "secret");
        assert_eq!(config_json["api_path"], "/api");
        assert!(child.is_enabled);
        assert!(child.enable_interactive_search);
        assert!(child.enable_auto_search);
        assert_eq!(child.routing_scopes.len(), 1);
        assert_eq!(child.routing_scopes[0].scope_id, "movie");
        assert_eq!(child.routing_scopes[0].categories, vec!["2000"]);
        assert_eq!(metadata.indexer_id, 7);
        assert_eq!(metadata.app_profile_id, 12);
        assert_eq!(metadata.download_client_id, 3);
        assert!(metadata.enable_rss);
        assert!(!metadata.enable_automatic_search);
        assert_eq!(metadata.caps_snapshot, Some(caps_snapshot));
    }

    #[test]
    fn managed_child_keeps_interactive_access_when_rss_is_disabled() {
        let config = ProwlarrConfig {
            base_url: "https://prowlarr.example".to_string(),
            api_key: "secret".to_string(),
        };
        let indexer = ProwlarrIndexerResource {
            id: 9,
            name: "Interactive Only".to_string(),
            enable: true,
            app_profile_id: 21,
            protocol: "torrent".to_string(),
            capabilities: ProwlarrIndexerCapabilities::default(),
            priority: 10,
            download_client_id: 0,
        };
        let app_profiles = HashMap::from([(
            21,
            ProwlarrAppProfile {
                id: 21,
                enable_rss: false,
                enable_automatic_search: false,
                enable_interactive_search: true,
            },
        )]);

        let child =
            build_managed_child_plan(&config, indexer, &app_profiles, None).expect("child plan");
        let metadata: ManagedChildMetadata =
            serde_json::from_str(child.managed_metadata_json.as_deref().unwrap()).unwrap();

        assert!(child.is_enabled);
        assert!(child.enable_interactive_search);
        assert!(!child.enable_auto_search);
        assert!(!metadata.enable_rss);
        assert!(!metadata.enable_automatic_search);
        assert!(metadata.caps_snapshot.is_none());
    }

    #[test]
    fn parse_caps_snapshot_preserves_search_nodes_and_limits() {
        let body = br#"<?xml version="1.0" encoding="UTF-8"?>
<caps>
  <server title="Prowlarr" />
  <limits default="100" max="100" />
  <searching>
    <search available="yes" supportedParams="q" />
    <tv-search available="yes" supportedParams="q,season,ep,tvdbid,rid,tvmazeid" />
    <movie-search available="yes" supportedParams="q,imdbid,genre" />
    <music-search available="yes" supportedParams="q" />
    <audio-search available="yes" supportedParams="q" />
    <book-search available="no" supportedParams="q" />
  </searching>
</caps>"#;

        let snapshot = parse_caps_snapshot(body).expect("caps snapshot");

        assert_eq!(snapshot.server_title.as_deref(), Some("Prowlarr"));
        assert_eq!(snapshot.limits_default, Some(100));
        assert_eq!(snapshot.limits_max, Some(100));
        assert_eq!(snapshot.search.supported_params, vec!["q"]);
        assert_eq!(
            snapshot.tv_search.supported_params,
            vec!["q", "season", "ep", "tvdbid", "rid", "tvmazeid"]
        );
        assert_eq!(
            snapshot.movie_search.supported_params,
            vec!["q", "imdbid", "genre"]
        );
        assert!(!snapshot.book_search.available);
    }

    #[tokio::test]
    async fn validate_connection_only_requests_system_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Prowlarr",
                "version": "2.0.0"
            })))
            .mount(&server)
            .await;

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let result = client
            .validate_connection()
            .await
            .expect("validation result");

        assert_eq!(result.status, "valid");
        let requests = server.received_requests().await.expect("recorded requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), SYSTEM_STATUS_PATH);
    }

    #[tokio::test]
    async fn validate_connection_rejects_non_prowlarr_app_name() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Sonarr",
                "version": "4.0.0"
            })))
            .mount(&server)
            .await;

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let result = client
            .validate_connection()
            .await
            .expect("validation result");

        assert_eq!(result.status, "invalid_config");
        assert_eq!(
            result.message.as_deref(),
            Some("base_url responded as 'Sonarr', not Prowlarr")
        );
    }

    #[tokio::test]
    async fn validate_connection_rejects_unsupported_major_version() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Prowlarr",
                "version": "3.0.0"
            })))
            .mount(&server)
            .await;

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let result = client
            .validate_connection()
            .await
            .expect("validation result");

        assert_eq!(result.status, "unsupported");
        assert_eq!(
            result.message.as_deref(),
            Some("unsupported Prowlarr version '3.0.0'; expected major 2")
        );
    }

    #[tokio::test]
    async fn plan_sync_uses_lowercase_appprofile_route() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Prowlarr",
                "version": "2.0.0"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(INDEXER_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(APP_PROFILE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let plan = client.plan_sync("parent").await.expect("sync plan");

        assert!(plan.children.is_empty());
    }

    #[tokio::test]
    async fn preview_sync_plan_lists_forty_one_children_without_fetching_caps() {
        let server = MockServer::start().await;
        let indexers = (1..=41)
            .map(|id| {
                json!({
                    "id": id,
                    "name": format!("Fixture Indexer {id}"),
                    "enable": true,
                    "appProfileId": 1,
                    "protocol": "usenet",
                    "priority": 3,
                    "downloadClientId": 0,
                    "capabilities": { "categories": [] }
                })
            })
            .collect::<Vec<_>>();
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Prowlarr",
                "version": "2.0.0"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(INDEXER_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(&indexers))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(APP_PROFILE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": 1,
                "enableRss": true,
                "enableAutomaticSearch": true,
                "enableInteractiveSearch": true
            }])))
            .mount(&server)
            .await;

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let plan = client
            .preview_sync_plan("parent")
            .await
            .expect("preview plan");
        let child = plan.children.first().expect("child plan");
        let metadata: ManagedChildMetadata =
            serde_json::from_str(child.managed_metadata_json.as_deref().unwrap()).unwrap();

        assert_eq!(plan.children.len(), 41);
        assert_eq!(metadata.indexer_id, 1);
        assert!(metadata.caps_snapshot.is_none());
        assert!(child.caps_snapshot_json.is_none());

        let requests = server.received_requests().await.unwrap();
        assert!(
            !requests.iter().any(|request| request
                .url
                .query()
                .unwrap_or_default()
                .contains("t=caps")),
            "preview must not fetch child caps"
        );
    }

    #[tokio::test]
    async fn enrichment_sync_plan_fetches_and_persists_child_caps_snapshot() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Prowlarr",
                "version": "2.0.0"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(INDEXER_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": 7,
                "name": "NZBGeek",
                "enable": true,
                "appProfileId": 1,
                "protocol": "usenet",
                "priority": 3,
                "downloadClientId": 0,
                "capabilities": { "categories": [] }
            }])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(APP_PROFILE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": 1,
                "enableRss": true,
                "enableAutomaticSearch": true,
                "enableInteractiveSearch": true
            }])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/7/api"))
            .and(query_param("t", "caps"))
            .and(query_param("apikey", "secret"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<caps>
  <server title="Prowlarr" />
  <limits default="100" max="100" />
  <searching>
    <search available="yes" supportedParams="q" />
    <tv-search available="yes" supportedParams="q,season,ep,tvdbid,rid,tvmazeid" />
    <movie-search available="yes" supportedParams="q,imdbid,genre" />
    <music-search available="yes" supportedParams="q" />
    <audio-search available="yes" supportedParams="q" />
    <book-search available="no" supportedParams="q" />
  </searching>
</caps>"#,
            ))
            .mount(&server)
            .await;

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let plan = client
            .enrichment_sync_plan("parent")
            .await
            .expect("enrichment plan")
            .expect("supported enrichment plan");
        let child = plan.children.first().expect("child plan");
        let metadata: ManagedChildMetadata =
            serde_json::from_str(child.managed_metadata_json.as_deref().unwrap()).unwrap();

        assert_eq!(metadata.indexer_id, 7);
        let caps = metadata.caps_snapshot.expect("caps snapshot");
        assert_eq!(caps.search.supported_params, vec!["q"]);
        assert_eq!(
            caps.tv_search.supported_params,
            vec!["q", "season", "ep", "tvdbid", "rid", "tvmazeid"]
        );
        assert_eq!(
            caps.movie_search.supported_params,
            vec!["q", "imdbid", "genre"]
        );
    }

    #[tokio::test]
    async fn enrichment_sync_plan_fetches_caps_concurrently_and_preserves_child_order() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Prowlarr",
                "version": "2.0.0"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(INDEXER_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {
                    "id": 10,
                    "name": "First",
                    "enable": true,
                    "appProfileId": 1,
                    "protocol": "usenet",
                    "priority": 3,
                    "downloadClientId": 0,
                    "capabilities": { "categories": [] }
                },
                {
                    "id": 7,
                    "name": "Second",
                    "enable": true,
                    "appProfileId": 1,
                    "protocol": "usenet",
                    "priority": 3,
                    "downloadClientId": 0,
                    "capabilities": { "categories": [] }
                },
                {
                    "id": 42,
                    "name": "Third",
                    "enable": true,
                    "appProfileId": 1,
                    "protocol": "usenet",
                    "priority": 3,
                    "downloadClientId": 0,
                    "capabilities": { "categories": [] }
                },
                {
                    "id": 3,
                    "name": "Fourth",
                    "enable": true,
                    "appProfileId": 1,
                    "protocol": "usenet",
                    "priority": 3,
                    "downloadClientId": 0,
                    "capabilities": { "categories": [] }
                }
            ])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(APP_PROFILE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": 1,
                "enableRss": true,
                "enableAutomaticSearch": true,
                "enableInteractiveSearch": true
            }])))
            .mount(&server)
            .await;

        let caps_body = r#"<?xml version="1.0" encoding="UTF-8"?>
<caps>
  <server title="Prowlarr" />
  <searching>
    <search available="yes" supportedParams="q" />
  </searching>
</caps>"#;
        for (indexer_id, delay_ms) in [(10, 1200), (7, 300), (42, 600), (3, 450)] {
            Mock::given(method("GET"))
                .and(path(format!("/{indexer_id}/api")))
                .and(query_param("t", "caps"))
                .and(query_param("apikey", "secret"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string(caps_body)
                        .set_delay(std::time::Duration::from_millis(delay_ms)),
                )
                .mount(&server)
                .await;
        }

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let started = std::time::Instant::now();
        let plan = client
            .enrichment_sync_plan("parent")
            .await
            .expect("enrichment plan")
            .expect("supported enrichment plan");
        let elapsed = started.elapsed();

        assert!(
            elapsed < std::time::Duration::from_millis(2000),
            "caps fetch should be concurrent; elapsed {elapsed:?}"
        );
        let child_ids = plan
            .children
            .iter()
            .map(|child| {
                serde_json::from_str::<ManagedChildMetadata>(
                    child.managed_metadata_json.as_deref().unwrap(),
                )
                .unwrap()
                .indexer_id
            })
            .collect::<Vec<_>>();
        assert_eq!(child_ids, vec![10, 7, 42, 3]);
    }

    #[tokio::test]
    async fn enrichment_sync_plan_skips_caps_fetch_for_upstream_disabled_children() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Prowlarr",
                "version": "2.0.0"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(INDEXER_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": 7,
                "name": "DOGnzb",
                "enable": false,
                "appProfileId": 1,
                "protocol": "usenet",
                "priority": 3,
                "downloadClientId": 0,
                "capabilities": { "categories": [] }
            }])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(APP_PROFILE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": 1,
                "enableRss": true,
                "enableAutomaticSearch": true,
                "enableInteractiveSearch": true
            }])))
            .mount(&server)
            .await;

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let plan = client
            .enrichment_sync_plan("parent")
            .await
            .expect("enrichment plan")
            .expect("supported enrichment plan");
        let child = plan.children.first().expect("child plan");
        let metadata: ManagedChildMetadata =
            serde_json::from_str(child.managed_metadata_json.as_deref().unwrap()).unwrap();

        assert_eq!(metadata.indexer_id, 7);
        assert!(metadata.caps_snapshot.is_none());
    }

    #[tokio::test]
    async fn system_status_404_keeps_base_url_message() {
        let server = MockServer::start().await;
        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let result = client
            .validate_connection()
            .await
            .expect("validation result");

        assert_eq!(result.status, "invalid_config");
        assert_eq!(
            result.message.as_deref(),
            Some("base_url does not appear to point at a Prowlarr API")
        );
    }

    #[tokio::test]
    async fn sync_endpoint_404_names_missing_path() {
        for missing_path in [INDEXER_PATH, APP_PROFILE_PATH] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(SYSTEM_STATUS_PATH))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "appName": "Prowlarr",
                    "version": "2.0.0"
                })))
                .mount(&server)
                .await;
            if missing_path != INDEXER_PATH {
                Mock::given(method("GET"))
                    .and(path(INDEXER_PATH))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
                    .mount(&server)
                    .await;
            }
            if missing_path != APP_PROFILE_PATH {
                Mock::given(method("GET"))
                    .and(path(APP_PROFILE_PATH))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
                    .mount(&server)
                    .await;
            }

            let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
            let error = client
                .plan_sync("parent")
                .await
                .expect_err("missing endpoint");

            assert_eq!(
                error.to_string(),
                format!("validation: Prowlarr sync endpoint '{missing_path}' was not found")
            );
        }
    }

    #[tokio::test]
    async fn sync_endpoint_auth_failure_maps_to_validation() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Prowlarr",
                "version": "2.0.0"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(INDEXER_PATH))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let error = client.plan_sync("parent").await.expect_err("auth failure");

        assert_eq!(
            error.to_string(),
            "validation: Prowlarr rejected the API key"
        );
    }
}
