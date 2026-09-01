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
    AppError, AppResult, CapturedIndexerHttpHeader, CapturedIndexerHttpResponse,
    ExternalPluginWasm, IndexerClient, IndexerErrorOperation, IndexerErrorRepository,
    IndexerManagementClient, IndexerPluginProvider, IndexerRoutingPlan, IndexerSearchResponse,
    IndexerSyncPlan, IndexerValidationResult, ManagedIndexerChildPlan, ManagedIndexerRoutingScope,
    NewIndexerError, NullIndexerErrorRepository, RateLimitCooldownAction, RuntimePluginLoad,
    SearchMode, classify_indexer_http_response,
    external_import::{EXTERNAL_IMPORT_HOST_RPS_LANE, EXTERNAL_IMPORT_HOST_RPS_PROFILE},
    indexer_error_history_is_persistable, indexer_response_content_type, unknown_indexer_error,
};
use scryer_domain::{
    ConfigFieldDef, ConfigFieldRole, ConfigFieldType, ConfigFieldValueSource,
    IndexerCapsSearchNode as DomainCapsSearchNode, IndexerCapsSnapshot as DomainCapsSnapshot,
    IndexerConfig, TaggedAlias, indexer_rate_limit_domain_key,
};
use scryer_outbound_http::{
    DestinationKey, OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy,
    indexer_reqwest_client,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, warn};

pub const PROWLARR_PROVIDER_TYPE: &str = "prowlarr";

const USER_AGENT: &str = "scryer-prowlarr/0.1";
const PROWLARR_CHILD_CAPS_FETCH_CONCURRENCY: usize = 8;
/// Keep Prowlarr diagnostics aligned with the plugin host's accepted-response limit.
const PROWLARR_RESPONSE_MAX_BYTES: usize = 50 * 1024 * 1024;

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
    /// Prowlarr keeps per-indexer settings in a flat name/value list. Nested
    /// settings objects are prefixed with their camelCased property name
    /// (`SchemaBuilder.GetFieldMapping`), so `IndexerTorrentBaseSettings`
    /// arrives as `torrentBaseSettings.*`. Note this is NOT the `seedCriteria.*`
    /// spelling — that is the Sonarr-side field name Prowlarr writes *into*
    /// when it syncs an app.
    #[serde(default)]
    fields: Vec<ProwlarrIndexerField>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ProwlarrIndexerField {
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
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
    /// `AppSyncProfile.MinimumSeeders`, the app-wide fallback behind an
    /// indexer's `appMinimumSeeders`. Non-nullable upstream, so a real Prowlarr
    /// always sends it; `Option` covers a version that predates the field.
    #[serde(default, rename = "minimumSeeders")]
    minimum_seeders: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProwlarrCapsSearchNode {
    pub available: bool,
    #[serde(default)]
    pub supported_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_engine: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProwlarrCapsSnapshot {
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

// `Eq` is out because the imported seed ratio is an `f64`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// Seed criteria the operator already configured in Prowlarr for this
    /// tracker. Scryer honours them unless a seeding profile is assigned to the
    /// child, so a Prowlarr-managed setup works without restating the goals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seed_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seed_time_minutes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    season_pack_seed_time_minutes: Option<i64>,
    /// Prowlarr's `AppMinimumSeeders`, which it pushes to Sonarr as that app's
    /// `minimumSeeders`. Scryer treats it the same way it treats the goals
    /// above: honoured unless a seeding profile is assigned to the child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    minimum_seeders: Option<i32>,
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
    indexer_errors: Arc<dyn IndexerErrorRepository>,
}

impl NativeProwlarrIndexerProvider {
    pub fn new(delegate: Arc<dyn IndexerPluginProvider>) -> Self {
        Self::new_with_indexer_error_repository(delegate, Arc::new(NullIndexerErrorRepository))
    }

    pub fn new_with_indexer_error_repository(
        delegate: Arc<dyn IndexerPluginProvider>,
        indexer_errors: Arc<dyn IndexerErrorRepository>,
    ) -> Self {
        Self {
            delegate,
            indexer_errors,
        }
    }
}

pub struct ProwlarrManagementClient {
    parent_config_id: String,
    parent_indexer_name: String,
    config: Result<ProwlarrConfig, String>,
    outbound_http: OutboundHttpClient,
    api_state: Arc<RwLock<Option<ResolvedProwlarrApi>>>,
    indexer_errors: Arc<dyn IndexerErrorRepository>,
}

#[derive(Clone)]
struct ProwlarrErrorCapture {
    indexer_id: String,
    indexer_name: String,
    indexer_errors: Arc<dyn IndexerErrorRepository>,
}

impl ProwlarrErrorCapture {
    async fn record(
        &self,
        operation: IndexerErrorOperation,
        response: CapturedIndexerHttpResponse,
    ) {
        if !indexer_error_history_is_persistable(&self.indexer_id) {
            // A connection test probes under a synthetic id with no `indexers`
            // row; persisting history for it can only fail the foreign key, and
            // that failure must never stand in for the probe's own error.
            return;
        }
        let classified =
            classify_indexer_http_response(&response).unwrap_or_else(unknown_indexer_error);
        let error = NewIndexerError {
            id: uuid::Uuid::new_v4().to_string(),
            indexer_id: self.indexer_id.clone(),
            indexer_name: self.indexer_name.clone(),
            operation,
            classification: classified.classification,
            provider_error_code: classified.provider_error_code,
            message: classified.message.to_string(),
            content_type: indexer_response_content_type(&response),
            response: Some(response),
            occurred_at: chrono::Utc::now(),
        };
        if let Err(error) = self.indexer_errors.record(error).await {
            warn!(
                indexer_id = self.indexer_id.as_str(),
                error = %error,
                "failed to persist Prowlarr HTTP error response"
            );
        }
    }

    async fn observe_rate_limited_response(
        &self,
        operation: IndexerErrorOperation,
        response: reqwest::Response,
    ) {
        match captured_response(response).await {
            Ok(response) => self.record(operation, response).await,
            Err(error) => warn!(error = %error, "failed to read Prowlarr rate-limit response"),
        }
    }
}

#[derive(Debug)]
enum CapturedProwlarrResponseError {
    Read(reqwest::Error),
    TooLarge,
}

impl std::fmt::Display for CapturedProwlarrResponseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "Prowlarr response read failed: {error}"),
            Self::TooLarge => write!(
                formatter,
                "Prowlarr response exceeded the {} MiB accepted-response limit",
                PROWLARR_RESPONSE_MAX_BYTES / (1024 * 1024)
            ),
        }
    }
}

fn captured_response_error(error: CapturedProwlarrResponseError) -> ProwlarrRequestError {
    match error {
        CapturedProwlarrResponseError::Read(error) => {
            ProwlarrRequestError::Unreachable(format!("Prowlarr response read failed: {error}"))
        }
        CapturedProwlarrResponseError::TooLarge => ProwlarrRequestError::Unsupported(format!(
            "Prowlarr response exceeded the {} MiB accepted-response limit",
            PROWLARR_RESPONSE_MAX_BYTES / (1024 * 1024)
        )),
    }
}

async fn captured_response(
    mut response: reqwest::Response,
) -> Result<CapturedIndexerHttpResponse, CapturedProwlarrResponseError> {
    if response
        .content_length()
        .is_some_and(|length| length > PROWLARR_RESPONSE_MAX_BYTES as u64)
    {
        return Err(CapturedProwlarrResponseError::TooLarge);
    }
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| CapturedIndexerHttpHeader {
            name: name.as_str().to_string(),
            value: value.as_bytes().to_vec(),
        })
        .collect();
    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(PROWLARR_RESPONSE_MAX_BYTES);
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(CapturedProwlarrResponseError::Read)?
    {
        let next_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or(CapturedProwlarrResponseError::TooLarge)?;
        if next_len > PROWLARR_RESPONSE_MAX_BYTES {
            return Err(CapturedProwlarrResponseError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(CapturedIndexerHttpResponse {
        status,
        headers,
        body,
    })
}

impl ProwlarrManagementClient {
    #[cfg(test)]
    fn new(config: &IndexerConfig) -> Self {
        Self::new_with_indexer_error_repository(config, Arc::new(NullIndexerErrorRepository))
    }

    pub fn new_with_indexer_error_repository(
        config: &IndexerConfig,
        indexer_errors: Arc<dyn IndexerErrorRepository>,
    ) -> Self {
        let http_client = indexer_reqwest_client();
        Self {
            parent_config_id: config.id.clone(),
            parent_indexer_name: config.name.clone(),
            config: ProwlarrConfig::from_indexer_config(config),
            outbound_http: OutboundHttpClient::new(http_client, RateLimitRegistry::new()),
            api_state: Arc::new(RwLock::new(None)),
            indexer_errors,
        }
    }

    fn config(&self) -> AppResult<&ProwlarrConfig> {
        self.config
            .as_ref()
            .map_err(|message| AppError::Validation(message.clone()))
    }

    fn error_capture(&self) -> ProwlarrErrorCapture {
        ProwlarrErrorCapture {
            indexer_id: self.parent_config_id.clone(),
            indexer_name: self.parent_indexer_name.clone(),
            indexer_errors: Arc::clone(&self.indexer_errors),
        }
    }

    async fn fetch_system_status(&self) -> Result<ProwlarrSystemStatus, ProwlarrRequestError> {
        Ok(self
            .ensure_supported_api_bucket(IndexerErrorOperation::ConnectionTest)
            .await?
            .status)
    }

    async fn get_json<T>(&self, path: &str, operation: IndexerErrorOperation) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.get_json_with_response(path, operation)
            .await
            .map(|(value, _)| value)
            .map_err(ProwlarrRequestError::into_app_error)
    }

    async fn ensure_supported_api_bucket(
        &self,
        operation: IndexerErrorOperation,
    ) -> Result<ResolvedProwlarrApi, ProwlarrRequestError> {
        if let Some(api) = self.api_state.read().await.clone() {
            return Ok(api);
        }

        let mut guard = self.api_state.write().await;
        if let Some(api) = guard.clone() {
            return Ok(api);
        }

        let (status, response): (ProwlarrSystemStatus, CapturedIndexerHttpResponse) = self
            .get_json_with_response(SYSTEM_STATUS_PATH, operation)
            .await?;
        let api = ResolvedProwlarrApi {
            bucket: ProwlarrApiBucket::V2,
            status,
        };
        if let Err(error) = api.bucket.validate_status(&api.status) {
            self.error_capture().record(operation, response).await;
            return Err(error);
        }
        *guard = Some(api.clone());
        Ok(api)
    }

    async fn get_json_with_response<T>(
        &self,
        path: &str,
        operation: IndexerErrorOperation,
    ) -> Result<(T, CapturedIndexerHttpResponse), ProwlarrRequestError>
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
        let error_capture = self.error_capture();
        let response = self
            .outbound_http
            .send_with_rate_limit_observer(
                self.request_policy(path),
                || {
                    self.outbound_http
                        .client()
                        .get(&url)
                        .header("Accept", "application/json")
                        .header("User-Agent", USER_AGENT)
                        .header("X-Api-Key", &api_key)
                },
                move |response| {
                    let error_capture = error_capture.clone();
                    async move {
                        error_capture
                            .observe_rate_limited_response(operation, response)
                            .await;
                    }
                },
            )
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

        let retry_after_seconds = retry_after_seconds(response.headers());
        let captured = captured_response(response)
            .await
            .map_err(captured_response_error)?;
        let status = StatusCode::from_u16(captured.status).expect("reqwest status is valid");

        if status.is_success() {
            return match serde_json::from_slice(&captured.body) {
                Ok(value) => Ok((value, captured)),
                Err(error) => {
                    self.error_capture().record(operation, captured).await;
                    Err(ProwlarrRequestError::Unsupported(format!(
                        "Prowlarr returned invalid JSON: {error}"
                    )))
                }
            };
        }

        self.error_capture()
            .record(operation, captured.clone())
            .await;
        Err(map_http_error(
            path,
            status,
            &captured.body,
            retry_after_seconds,
        ))
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
        let child_key = indexer_id.to_string();
        let error_capture = self.error_capture();
        let response = self
            .outbound_http
            .send_with_rate_limit_observer(
                self.child_request_policy(&request_path, &child_key),
                || {
                    self.outbound_http
                        .client()
                        .get(&url)
                        .header("Accept", "application/xml, text/xml, application/rss+xml")
                        .header("User-Agent", USER_AGENT)
                },
                move |response| {
                    let error_capture = error_capture.clone();
                    async move {
                        error_capture
                            .observe_rate_limited_response(
                                IndexerErrorOperation::CapsRefresh,
                                response,
                            )
                            .await;
                    }
                },
            )
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

        let retry_after_seconds = retry_after_seconds(response.headers());
        let captured = captured_response(response)
            .await
            .map_err(captured_response_error)?;
        let status = StatusCode::from_u16(captured.status).expect("reqwest status is valid");

        if status.is_success() {
            return match parse_caps_snapshot(&captured.body) {
                Ok(snapshot) => Ok(snapshot),
                Err(error) => {
                    self.error_capture()
                        .record(IndexerErrorOperation::CapsRefresh, captured)
                        .await;
                    Err(error)
                }
            };
        }

        self.error_capture()
            .record(IndexerErrorOperation::CapsRefresh, captured.clone())
            .await;
        Err(map_http_error(
            &request_path,
            status,
            &captured.body,
            retry_after_seconds,
        ))
    }

    async fn build_sync_plan(&self, fetch_caps: bool) -> AppResult<IndexerSyncPlan> {
        let config = self.config()?.clone();
        let api = self
            .ensure_supported_api_bucket(IndexerErrorOperation::ManagementSync)
            .await
            .map_err(ProwlarrRequestError::into_app_error)?;
        let indexers: Vec<ProwlarrIndexerResource> = self
            .get_json(
                api.bucket.indexer_path(),
                IndexerErrorOperation::ManagementSync,
            )
            .await?;
        let app_profiles: Vec<ProwlarrAppProfile> = self
            .get_json(
                api.bucket.app_profile_path(),
                IndexerErrorOperation::ManagementSync,
            )
            .await?;
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
        self.request_policy_for_child(path, None)
    }

    fn child_request_policy(&self, path: &str, child_key: &str) -> RequestPolicy {
        self.request_policy_for_child(path, Some(child_key))
    }

    fn request_policy_for_child(&self, path: &str, child_key: Option<&str>) -> RequestPolicy {
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
        let scope = child_key.map_or_else(
            || format!("prowlarr:{request_namespace}:{base_url}"),
            |child_key| format!("prowlarr:{request_namespace}:{base_url}:child:{child_key}"),
        );
        let policy =
            RequestPolicy::safe_read(scope, format!("prowlarr:{request_namespace}:{path}"))
                .with_max_retries(2)
                .with_backoff(Duration::from_secs(1), Duration::from_secs(15))
                .with_host_rps_limit(
                    EXTERNAL_IMPORT_HOST_RPS_LANE,
                    EXTERNAL_IMPORT_HOST_RPS_PROFILE,
                );
        policy.with_destination_cooldown_key(DestinationKey::from(indexer_rate_limit_domain_key(
            &self.parent_config_id,
            child_key,
        )))
    }
}

pub struct ProwlarrSearchStub;

#[async_trait]
impl IndexerClient for ProwlarrSearchStub {
    async fn search_stream(
        &self,
        _query: String,
        _ids: HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _operation: scryer_application::IndexerErrorOperation,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<scryer_application::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
        _page_sink: scryer_application::IndexerSearchPageSink,
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
            return Some(Arc::new(
                ProwlarrManagementClient::new_with_indexer_error_repository(
                    config,
                    Arc::clone(&self.indexer_errors),
                ),
            ));
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
    let is_torrent = provider_type == "torznab";
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
        // Usenet indexers have no seeding obligation, so their fields are not
        // read even if Prowlarr happens to carry them.
        seed_ratio: is_torrent
            .then(|| prowlarr_seed_ratio(&indexer.fields))
            .flatten(),
        seed_time_minutes: is_torrent
            .then(|| prowlarr_seed_minutes(&indexer.fields, "torrentBaseSettings.seedTime"))
            .flatten(),
        season_pack_seed_time_minutes: is_torrent
            .then(|| prowlarr_seed_minutes(&indexer.fields, "torrentBaseSettings.packSeedTime"))
            .flatten(),
        // Prowlarr's own rule for the value it pushes to an app:
        // `TorrentBaseSettings.AppMinimumSeeders ?? AppProfile.MinimumSeeders`
        // (`Applications/Sonarr/Sonarr.cs:282`). A null check, not a truthiness
        // one — an indexer-level zero wins over a positive profile value.
        minimum_seeders: is_torrent
            .then(|| {
                prowlarr_minimum_seeders(&indexer.fields)
                    .or_else(|| app_profile.and_then(|profile| profile.minimum_seeders))
            })
            .flatten(),
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

/// Reads one of Prowlarr's flat `fields` entries.
///
/// Prowlarr omits a field entirely, or sends it with a null value, when the
/// operator left it blank; both mean "no goal on this axis" rather than zero.
fn prowlarr_field_value<'a>(
    fields: &'a [ProwlarrIndexerField],
    name: &str,
) -> Option<&'a serde_json::Value> {
    fields
        .iter()
        .find(|field| field.name.eq_ignore_ascii_case(name))
        .and_then(|field| field.value.as_ref())
        .filter(|value| !value.is_null())
}

fn prowlarr_seed_ratio(fields: &[ProwlarrIndexerField]) -> Option<f64> {
    let value = prowlarr_field_value(fields, "torrentBaseSettings.seedRatio")?;
    let ratio = value.as_f64().or_else(|| {
        value
            .as_str()
            .and_then(|raw| raw.trim().parse::<f64>().ok())
    })?;
    (ratio.is_finite() && ratio > 0.0).then_some(ratio)
}

/// Read Prowlarr's `AppMinimumSeeders`.
///
/// Unlike the goal readers above, this keeps a zero. Prowlarr's own validator
/// only *warns* on a non-positive value (`.AsWarning()`), so zero really is
/// storable upstream, and there it means "do not enforce a minimum" — which is
/// not the same as leaving the field unset, which means "inherit Scryer's
/// floor". Collapsing the two, as `prowlarr_seed_minutes` deliberately does for
/// goals where zero and unset both mean "no goal", would silently re-enable a
/// check the operator turned off.
fn prowlarr_minimum_seeders(fields: &[ProwlarrIndexerField]) -> Option<i32> {
    let value = prowlarr_field_value(fields, "torrentBaseSettings.appMinimumSeeders")?;
    let seeders = value
        .as_i64()
        .or_else(|| value.as_f64().map(|value| value as i64))
        .or_else(|| {
            value
                .as_str()
                .and_then(|raw| raw.trim().parse::<i64>().ok())
        })?;
    (seeders >= 0).then_some(seeders.clamp(0, i64::from(i32::MAX)) as i32)
}

fn prowlarr_seed_minutes(fields: &[ProwlarrIndexerField], name: &str) -> Option<i64> {
    let value = prowlarr_field_value(fields, name)?;
    let minutes = value
        .as_i64()
        .or_else(|| value.as_f64().map(|value| value as i64))
        .or_else(|| {
            value
                .as_str()
                .and_then(|raw| raw.trim().parse::<i64>().ok())
        })?;
    (minutes > 0).then_some(minutes)
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
                    "server" => {
                        snapshot.server_title = attr_value(&element, "title")?;
                    }
                    "limits" => {
                        snapshot.limits_default = attr_i64(&element, "default")?;
                        snapshot.limits_max = attr_i64(&element, "max")?;
                    }
                    "search" => {
                        snapshot.search = parse_caps_node(&element)?;
                    }
                    "tv-search" => {
                        snapshot.tv_search = parse_caps_node(&element)?;
                    }
                    "movie-search" => {
                        snapshot.movie_search = parse_caps_node(&element)?;
                    }
                    "music-search" => {
                        snapshot.music_search = parse_caps_node(&element)?;
                    }
                    "audio-search" => {
                        snapshot.audio_search = parse_caps_node(&element)?;
                    }
                    "book-search" => {
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
        available: attr_value(element, "available")?
            .is_some_and(|value| value.eq_ignore_ascii_case("yes")),
        supported_params: attr_value(element, "supportedParams")?
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .collect(),
        search_engine: attr_value(element, "searchEngine")?
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

fn attr_value(element: &BytesStart<'_>, key: &str) -> Result<Option<String>, ProwlarrRequestError> {
    for attribute in element.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| {
            ProwlarrRequestError::Unsupported(format!(
                "Prowlarr returned invalid caps XML attributes: {error}"
            ))
        })?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.value.into_owned()));
        }
    }

    Ok(None)
}

fn attr_i64(element: &BytesStart<'_>, key: &str) -> Result<Option<i64>, ProwlarrRequestError> {
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

    #[derive(Default)]
    struct RecordingIndexerErrorRepository {
        errors: tokio::sync::Mutex<Vec<NewIndexerError>>,
    }

    #[async_trait]
    impl IndexerErrorRepository for RecordingIndexerErrorRepository {
        async fn record(&self, error: NewIndexerError) -> AppResult<()> {
            self.errors.lock().await.push(error);
            Ok(())
        }

        async fn list(
            &self,
            _indexer_id: Option<&str>,
            _first: usize,
            _after: Option<&str>,
        ) -> AppResult<scryer_application::IndexerErrorPage> {
            Ok(scryer_application::IndexerErrorPage {
                items: Vec::new(),
                next_cursor: None,
            })
        }

        async fn get_detail(
            &self,
            _id: &str,
        ) -> AppResult<Option<scryer_application::IndexerErrorDetail>> {
            Ok(None)
        }

        async fn delete_older_than(&self, _cutoff: chrono::DateTime<Utc>) -> AppResult<u32> {
            Ok(0)
        }
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
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
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
        assert_eq!(
            client
                .request_policy("/api/v1/indexer")
                .destination_cooldown_override
                .expect("parent requests should override destination cooldown identity")
                .as_str(),
            "cfg-1"
        );

        let child_policy = client.child_request_policy("/42/api?t=caps", "42");
        assert!(child_policy.scope.to_string().ends_with(":child:42"));
        assert_eq!(
            child_policy
                .destination_cooldown_override
                .expect("child requests should override destination cooldown identity")
                .as_str(),
            "cfg-1:42"
        );
        assert!(child_policy.host_rps_override.is_some());
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
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
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

    fn seed_field(name: &str, value: serde_json::Value) -> ProwlarrIndexerField {
        ProwlarrIndexerField {
            name: name.to_string(),
            value: Some(value),
        }
    }

    fn indexer_with_fields(
        protocol: &str,
        fields: Vec<ProwlarrIndexerField>,
    ) -> ProwlarrIndexerResource {
        ProwlarrIndexerResource {
            fields,
            id: 7,
            name: "Indexer Seven".to_string(),
            enable: true,
            app_profile_id: 12,
            protocol: protocol.to_string(),
            capabilities: ProwlarrIndexerCapabilities::default(),
            priority: 25,
            download_client_id: 3,
        }
    }

    /// An app profile that carries no minimum of its own, so a child's own
    /// `appMinimumSeeders` is the only source in play.
    fn app_profile(id: i64, minimum_seeders: Option<i32>) -> ProwlarrAppProfile {
        ProwlarrAppProfile {
            id,
            enable_rss: true,
            enable_automatic_search: true,
            enable_interactive_search: true,
            minimum_seeders,
        }
    }

    fn child_metadata(indexer: ProwlarrIndexerResource) -> ManagedChildMetadata {
        child_metadata_with_app_profile(indexer, app_profile(12, None))
    }

    fn child_metadata_with_app_profile(
        indexer: ProwlarrIndexerResource,
        profile: ProwlarrAppProfile,
    ) -> ManagedChildMetadata {
        let config = ProwlarrConfig {
            base_url: "https://prowlarr.example".to_string(),
            api_key: "secret".to_string(),
        };
        let app_profiles = HashMap::from([(profile.id, profile)]);
        let child =
            build_managed_child_plan(&config, indexer, &app_profiles, None).expect("child plan");
        serde_json::from_str(child.managed_metadata_json.as_deref().unwrap()).unwrap()
    }

    /// Verbatim `/api/v1/indexer` payload from a real Prowlarr container
    /// (linuxserver/prowlarr, Torznab indexer). Pins the field names and the
    /// JSON shapes Prowlarr actually emits — including the advanced fields,
    /// which it does return, and the null it sends for an unset one.
    #[test]
    fn a_real_prowlarr_payload_imports_its_seed_criteria() {
        let indexer: ProwlarrIndexerResource = serde_json::from_str(
            r#"{
              "enable": true,
              "appProfileId": 1,
              "protocol": "torrent",
              "priority": 25,
              "downloadClientId": 0,
              "name": "probe-torznab",
              "id": 1,
              "fields": [
                { "name": "torrentBaseSettings.appMinimumSeeders", "value": null },
                { "name": "torrentBaseSettings.seedRatio", "value": 1.5 },
                { "name": "torrentBaseSettings.seedTime", "value": 4320 },
                { "name": "torrentBaseSettings.packSeedTime", "value": 10080 },
                { "name": "torrentBaseSettings.preferMagnetUrl", "value": false }
              ]
            }"#,
        )
        .expect("real Prowlarr payload should deserialize");

        // Verbatim `/api/v1/appprofile` entry from the same container. Prowlarr
        // declares `MinimumSeeders` as a plain int, so a real deployment always
        // sends one — the default profile's 1.
        let profile: ProwlarrAppProfile = serde_json::from_str(
            r#"{
              "name": "Standard",
              "enableRss": true,
              "enableAutomaticSearch": true,
              "enableInteractiveSearch": true,
              "minimumSeeders": 1,
              "id": 1
            }"#,
        )
        .expect("real Prowlarr app profile should deserialize");
        assert_eq!(profile.minimum_seeders, Some(1));

        let metadata = child_metadata_with_app_profile(indexer, profile);
        assert_eq!(metadata.seed_ratio, Some(1.5));
        assert_eq!(metadata.seed_time_minutes, Some(4320));
        assert_eq!(metadata.season_pack_seed_time_minutes, Some(10080));
        assert_eq!(
            metadata.minimum_seeders,
            Some(1),
            "the probe left appMinimumSeeders unset, so the app profile answers"
        );
    }

    /// Prowlarr computes the minimum it pushes to an app as
    /// `TorrentBaseSettings.AppMinimumSeeders ?? AppProfile.MinimumSeeders`
    /// (`Applications/Sonarr/Sonarr.cs:282`). Scryer synthesizes the same value
    /// so a Prowlarr-managed child admits releases the way Prowlarr's own Sonarr
    /// sync would.
    #[test]
    fn the_app_profile_minimum_stands_in_when_the_indexer_leaves_the_field_blank() {
        let metadata = child_metadata_with_app_profile(
            indexer_with_fields(
                "torrent",
                vec![seed_field(
                    "torrentBaseSettings.appMinimumSeeders",
                    serde_json::Value::Null,
                )],
            ),
            app_profile(12, Some(5)),
        );
        assert_eq!(metadata.minimum_seeders, Some(5));

        // The field omitted entirely reads the same way as an explicit null.
        let omitted = child_metadata_with_app_profile(
            indexer_with_fields("torrent", Vec::new()),
            app_profile(12, Some(5)),
        );
        assert_eq!(omitted.minimum_seeders, Some(5));
    }

    #[test]
    fn the_indexer_field_overrides_the_app_profile_minimum() {
        let metadata = child_metadata_with_app_profile(
            indexer_with_fields(
                "torrent",
                vec![seed_field(
                    "torrentBaseSettings.appMinimumSeeders",
                    serde_json::json!(3),
                )],
            ),
            app_profile(12, Some(5)),
        );
        assert_eq!(metadata.minimum_seeders, Some(3));

        // Including when the indexer's answer is "do not enforce": Prowlarr's
        // `??` is a null check, not a truthiness check.
        let disabled = child_metadata_with_app_profile(
            indexer_with_fields(
                "torrent",
                vec![seed_field(
                    "torrentBaseSettings.appMinimumSeeders",
                    serde_json::json!(0),
                )],
            ),
            app_profile(12, Some(5)),
        );
        assert_eq!(disabled.minimum_seeders, Some(0));
    }

    #[test]
    fn an_app_profile_minimum_of_zero_also_survives_as_an_explicit_disable() {
        let metadata = child_metadata_with_app_profile(
            indexer_with_fields("torrent", Vec::new()),
            app_profile(12, Some(0)),
        );
        assert_eq!(metadata.minimum_seeders, Some(0));
    }

    #[test]
    fn a_child_whose_app_profile_is_unknown_falls_back_to_scryers_own_floor() {
        // The profile list did not contain this child's `appProfileId`, so there
        // is nothing to inherit and the metadata stays silent — which reads
        // downstream as "use the Scryer floor".
        let config = ProwlarrConfig {
            base_url: "https://prowlarr.example".to_string(),
            api_key: "secret".to_string(),
        };
        let child = build_managed_child_plan(
            &config,
            indexer_with_fields("torrent", Vec::new()),
            &HashMap::from([(99, app_profile(99, Some(5)))]),
            None,
        )
        .expect("child plan");
        let metadata: ManagedChildMetadata =
            serde_json::from_str(child.managed_metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata.minimum_seeders, None);
    }

    #[test]
    fn a_prowlarr_minimum_of_zero_imports_as_an_explicit_disable() {
        // Prowlarr's validator only warns on a non-positive value, so zero is
        // storable upstream and means "do not enforce". Unlike the goal fields,
        // it must not collapse into "unset".
        let disabled = child_metadata(indexer_with_fields(
            "torrent",
            vec![seed_field(
                "torrentBaseSettings.appMinimumSeeders",
                serde_json::json!(0),
            )],
        ));
        assert_eq!(disabled.minimum_seeders, Some(0));

        let configured = child_metadata(indexer_with_fields(
            "torrent",
            vec![seed_field(
                "torrentBaseSettings.appMinimumSeeders",
                serde_json::json!(3),
            )],
        ));
        assert_eq!(configured.minimum_seeders, Some(3));

        let absent = child_metadata(indexer_with_fields(
            "torrent",
            vec![seed_field(
                "torrentBaseSettings.appMinimumSeeders",
                serde_json::Value::Null,
            )],
        ));
        assert_eq!(absent.minimum_seeders, None);
    }

    #[test]
    fn usenet_children_ignore_a_stray_minimum_seeders_field() {
        // Neither source applies: usenet has no swarm, so an indexer field and
        // an app-profile fallback are both ignored.
        let metadata = child_metadata_with_app_profile(
            indexer_with_fields(
                "usenet",
                vec![seed_field(
                    "torrentBaseSettings.appMinimumSeeders",
                    serde_json::json!(5),
                )],
            ),
            app_profile(12, Some(7)),
        );
        assert_eq!(metadata.minimum_seeders, None);
    }

    #[test]
    fn torrent_children_import_the_seed_criteria_configured_in_prowlarr() {
        let metadata = child_metadata(indexer_with_fields(
            "torrent",
            vec![
                seed_field("torrentBaseSettings.seedRatio", serde_json::json!(1.5)),
                seed_field("torrentBaseSettings.seedTime", serde_json::json!(4320)),
                seed_field("torrentBaseSettings.packSeedTime", serde_json::json!(10080)),
            ],
        ));
        assert_eq!(metadata.seed_ratio, Some(1.5));
        assert_eq!(metadata.seed_time_minutes, Some(4320));
        assert_eq!(metadata.season_pack_seed_time_minutes, Some(10080));
    }

    #[test]
    fn blank_zero_and_stringified_prowlarr_seed_criteria_are_handled() {
        // Prowlarr sends numbers as strings on some field types.
        let stringified = child_metadata(indexer_with_fields(
            "torrent",
            vec![
                seed_field("torrentBaseSettings.seedRatio", serde_json::json!("2.0")),
                seed_field("torrentBaseSettings.seedTime", serde_json::json!("60")),
            ],
        ));
        assert_eq!(stringified.seed_ratio, Some(2.0));
        assert_eq!(stringified.seed_time_minutes, Some(60));

        // Null, absent, and zero all mean "no goal on this axis", never zero.
        let blank = child_metadata(indexer_with_fields(
            "torrent",
            vec![
                seed_field("torrentBaseSettings.seedRatio", serde_json::Value::Null),
                seed_field("torrentBaseSettings.seedTime", serde_json::json!(0)),
            ],
        ));
        assert_eq!(blank.seed_ratio, None);
        assert_eq!(blank.seed_time_minutes, None);
        assert_eq!(blank.season_pack_seed_time_minutes, None);
    }

    #[test]
    fn usenet_children_carry_no_seed_criteria() {
        let metadata = child_metadata(indexer_with_fields(
            "usenet",
            vec![seed_field(
                "torrentBaseSettings.seedRatio",
                serde_json::json!(1.5),
            )],
        ));
        assert_eq!(metadata.seed_ratio, None);
        assert_eq!(metadata.seed_time_minutes, None);
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
            fields: Vec::new(),
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
                minimum_seeders: None,
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
            fields: Vec::new(),
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
                minimum_seeders: None,
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
    async fn records_non_success_and_invalid_success_responses() {
        let server = MockServer::start().await;
        let repository = Arc::new(RecordingIndexerErrorRepository::default());
        let config = test_indexer_config(&server.uri());
        let client = ProwlarrManagementClient::new_with_indexer_error_repository(
            &config,
            repository.clone(),
        );

        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(
                ResponseTemplate::new(500)
                    .insert_header("content-type", "application/octet-stream")
                    .set_body_bytes(vec![0, 255, 1, 254]),
            )
            .expect(1)
            .mount(&server)
            .await;

        let error = client
            .fetch_system_status()
            .await
            .expect_err("500 response");
        assert!(matches!(error, ProwlarrRequestError::Unreachable(_)));
        let errors = repository.errors.lock().await;
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].operation, IndexerErrorOperation::ConnectionTest);
        assert_eq!(errors[0].response.as_ref().unwrap().status, 500);
        assert_eq!(
            errors[0].response.as_ref().unwrap().body,
            vec![0, 255, 1, 254]
        );
        assert_eq!(
            errors[0].classification,
            scryer_application::IndexerErrorClassification::HttpServerError
        );
        drop(errors);

        server.reset().await;
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_string("not-json"))
            .expect(1)
            .mount(&server)
            .await;

        let error = client
            .fetch_system_status()
            .await
            .expect_err("invalid JSON response");
        assert!(matches!(error, ProwlarrRequestError::Unsupported(_)));
        let errors = repository.errors.lock().await;
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[1].response.as_ref().unwrap().status, 200);
        assert_eq!(errors[1].response.as_ref().unwrap().body, b"not-json");
        assert_eq!(
            errors[1].classification,
            scryer_application::IndexerErrorClassification::Unknown
        );
    }

    #[tokio::test]
    async fn records_rate_limited_attempt_before_retrying_successfully() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let server = MockServer::start().await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let responder_attempts = attempts.clone();
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(move |_request: &wiremock::Request| {
                if responder_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(429)
                        .insert_header("retry-after", "0")
                        .set_body_string("rate limited body")
                } else {
                    ResponseTemplate::new(200).set_body_json(json!({
                        "appName": "Prowlarr",
                        "version": "2.0.0"
                    }))
                }
            })
            .expect(2)
            .mount(&server)
            .await;

        let repository = Arc::new(RecordingIndexerErrorRepository::default());
        let client = ProwlarrManagementClient::new_with_indexer_error_repository(
            &test_indexer_config(&server.uri()),
            repository.clone(),
        );

        let status = client
            .fetch_system_status()
            .await
            .expect("second attempt succeeds");
        assert_eq!(status.app_name, "Prowlarr");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);

        let errors = repository.errors.lock().await;
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].response.as_ref().unwrap().status, 429);
        assert_eq!(
            errors[0].response.as_ref().unwrap().body,
            b"rate limited body"
        );
        assert_eq!(errors[0].operation, IndexerErrorOperation::ConnectionTest);
        assert_eq!(
            errors[0].classification,
            scryer_application::IndexerErrorClassification::HttpRateLimited
        );
    }

    #[tokio::test]
    async fn rejects_declared_responses_larger_than_the_capture_limit() {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("oversized response listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("request connection");
            let headers = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                PROWLARR_RESPONSE_MAX_BYTES + 1
            );
            stream
                .write_all(headers.as_bytes())
                .await
                .expect("oversized response headers");
        });

        let response = indexer_reqwest_client()
            .get(format!("http://{address}/oversized"))
            .send()
            .await
            .expect("oversized response headers");
        let error = captured_response(response)
            .await
            .expect_err("response exceeds capture limit");

        assert!(matches!(error, CapturedProwlarrResponseError::TooLarge));
        server.await.expect("oversized response server");
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
