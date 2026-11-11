use std::collections::{BTreeSet, HashMap, HashSet};
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
    IndexerAccountingContext, IndexerClient, IndexerErrorOperation, IndexerErrorRepository,
    IndexerManagementClient, IndexerRequestResult, IndexerRequestTally, IndexerRoutingPlan,
    IndexerSearchResponse, IndexerStatsTracker, IndexerSyncPlan, IndexerValidationResult,
    ManagedIndexerChildPlan, ManagedIndexerRoutingScope, NativeIndexerProvider, NewIndexerError,
    NullIndexerErrorRepository, NullIndexerStatsTracker, RateLimitCooldownAction, SearchMode,
    classify_indexer_http_response,
    external_import::{EXTERNAL_IMPORT_HOST_RPS_LANE, EXTERNAL_IMPORT_HOST_RPS_PROFILE},
    indexer_error_history_is_persistable, indexer_response_content_type, unknown_indexer_error,
};
use scryer_domain::{
    ConfigFieldDef, ConfigFieldRole, ConfigFieldType, ConfigFieldValueSource,
    IndexerCapsSearchNode as DomainCapsSearchNode, IndexerCapsSnapshot as DomainCapsSnapshot,
    IndexerCategoryDescriptor as DomainCategoryDescriptor,
    IndexerCategoryModel as DomainCategoryModel,
    IndexerCategoryValueKind as DomainCategoryValueKind, IndexerConfig, TaggedAlias,
    indexer_rate_limit_domain_key,
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

/// One generation of Prowlarr's management API.
///
/// A generation owns every request shape and response decoder that could
/// change between API generations, so the sync planner only ever talks to a
/// generation and never to a route or wire struct directly. Prowlarr 2.x and
/// 3.x both serve the `/api/v1` generation; a release that moves to a new
/// namespace adds a variant here, its own wire structs, and a mapping into the
/// internal models below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProwlarrApiGeneration {
    ApiV1,
}

/// The pieces of a child's Newznab/Torznab `t=caps` request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProwlarrChildCapsRequest {
    url: String,
    /// The request path without credentials, for policies and error messages.
    request_path: String,
    accept: &'static str,
}

impl ProwlarrApiGeneration {
    /// Every generation, oldest first.
    const ALL: &'static [Self] = &[Self::ApiV1];

    /// The generation used to read `system/status` before the product version
    /// is known, and to label requests made before a status read resolved one.
    const DISCOVERY: Self = Self::ApiV1;

    const fn system_status_path(self) -> &'static str {
        match self {
            Self::ApiV1 => "/api/v1/system/status",
        }
    }

    const fn indexer_path(self) -> &'static str {
        match self {
            Self::ApiV1 => "/api/v1/indexer",
        }
    }

    const fn app_profile_path(self) -> &'static str {
        match self {
            Self::ApiV1 => "/api/v1/appprofile",
        }
    }

    const fn request_namespace(self) -> &'static str {
        match self {
            Self::ApiV1 => "prowlarr_api_v1",
        }
    }

    /// The path a managed child's Newznab/Torznab passthrough lives under,
    /// relative to [`Self::child_base_url`].
    const fn child_api_path(self) -> &'static str {
        match self {
            Self::ApiV1 => "/api",
        }
    }

    fn child_base_url(self, base_url: &str, indexer_id: i64) -> String {
        match self {
            Self::ApiV1 => format!("{}/{}", base_url.trim_end_matches('/'), indexer_id),
        }
    }

    fn child_caps_request(
        self,
        config: &ProwlarrConfig,
        indexer_id: i64,
    ) -> ProwlarrChildCapsRequest {
        match self {
            Self::ApiV1 => ProwlarrChildCapsRequest {
                url: format!(
                    "{}{}?t=caps&apikey={}",
                    self.child_base_url(&config.base_url, indexer_id),
                    self.child_api_path(),
                    config.api_key
                ),
                request_path: format!("/{indexer_id}{}?t=caps", self.child_api_path()),
                accept: "application/xml, text/xml, application/rss+xml",
            },
        }
    }

    fn decode_system_status(self, body: &[u8]) -> serde_json::Result<ProwlarrSystemStatus> {
        match self {
            // The internal models are the `/api/v1` wire shape.
            Self::ApiV1 => serde_json::from_slice(body),
        }
    }

    fn decode_indexers(self, body: &[u8]) -> serde_json::Result<Vec<ProwlarrIndexerResource>> {
        match self {
            Self::ApiV1 => serde_json::from_slice(body),
        }
    }

    fn decode_app_profiles(self, body: &[u8]) -> serde_json::Result<Vec<ProwlarrAppProfile>> {
        match self {
            Self::ApiV1 => serde_json::from_slice(body),
        }
    }

    fn is_system_status_path(path: &str) -> bool {
        Self::ALL
            .iter()
            .any(|generation| generation.system_status_path() == path)
    }

    fn is_sync_path(path: &str) -> bool {
        Self::ALL.iter().any(|generation| {
            generation.indexer_path() == path || generation.app_profile_path() == path
        })
    }
}

/// A Prowlarr product version as `system/status` reports it, for example
/// `2.7.0.5723`. Prowlarr reports four parts; three are accepted as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ProwlarrProductVersion {
    major: u64,
    minor: u64,
    patch: u64,
    build: Option<u64>,
}

impl ProwlarrProductVersion {
    const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            build: None,
        }
    }

    fn parse(raw: &str) -> Result<Self, ProwlarrRequestError> {
        let trimmed = raw.trim();
        let unreadable = || {
            ProwlarrRequestError::Unsupported(format!(
                "could not determine Prowlarr version from '{trimmed}'"
            ))
        };
        let parts = trimmed
            .split('.')
            .map(|part| {
                (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                    .then(|| part.parse::<u64>().ok())
                    .flatten()
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(unreadable)?;
        match parts.as_slice() {
            [major, minor, patch] => Ok(Self::new(*major, *minor, *patch)),
            [major, minor, patch, build] => Ok(Self {
                build: Some(*build),
                ..Self::new(*major, *minor, *patch)
            }),
            _ => Err(unreadable()),
        }
    }

    /// The release a version belongs to; the build number never decides
    /// support.
    const fn release(self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }
}

impl std::fmt::Display for ProwlarrProductVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(build) = self.build {
            write!(formatter, ".{build}")?;
        }
        Ok(())
    }
}

/// How far a connected Prowlarr is from the releases Scryer is tested against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProwlarrSupportTier {
    /// A major Scryer is tested against.
    Tested,
    /// A major newer than any tested one. Prowlarr keeps its API generation
    /// across majors, so Scryer connects through the newest generation it knows
    /// rather than refusing.
    Newer,
}

/// Which Prowlarr releases Scryer supports and which API generation each one
/// speaks.
struct ProwlarrSupportPolicy {
    minimum: ProwlarrProductVersion,
    /// Tested majors, ascending, with the generation each one serves.
    tested_majors: &'static [(u64, ProwlarrApiGeneration)],
    /// The generation a major newer than every tested one is spoken to with.
    newest_generation: ProwlarrApiGeneration,
}

const PROWLARR_SUPPORT_POLICY: ProwlarrSupportPolicy = ProwlarrSupportPolicy {
    minimum: ProwlarrProductVersion::new(2, 0, 0),
    tested_majors: &[
        (2, ProwlarrApiGeneration::ApiV1),
        (3, ProwlarrApiGeneration::ApiV1),
    ],
    newest_generation: ProwlarrApiGeneration::ApiV1,
};

/// The generation and tier a supported Prowlarr version resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProwlarrSupport {
    generation: ProwlarrApiGeneration,
    tier: ProwlarrSupportTier,
}

impl ProwlarrSupportPolicy {
    fn highest_tested_major(&self) -> u64 {
        self.tested_majors
            .iter()
            .map(|(major, _)| *major)
            .max()
            .unwrap_or(self.minimum.major)
    }

    fn resolve(
        &self,
        version: &ProwlarrProductVersion,
    ) -> Result<ProwlarrSupport, ProwlarrRequestError> {
        if version.release() < self.minimum.release() {
            return Err(ProwlarrRequestError::VersionUnsupported {
                version: version.to_string(),
                minimum: self.minimum.to_string(),
            });
        }
        if let Some((_, generation)) = self
            .tested_majors
            .iter()
            .find(|(major, _)| *major == version.major)
        {
            return Ok(ProwlarrSupport {
                generation: *generation,
                tier: ProwlarrSupportTier::Tested,
            });
        }
        if version.major > self.highest_tested_major() {
            return Ok(ProwlarrSupport {
                generation: self.newest_generation,
                tier: ProwlarrSupportTier::Newer,
            });
        }
        // At or above the minimum but on an untested major below the newest
        // tested one: no such major exists today, so treat it as below support.
        Err(ProwlarrRequestError::VersionUnsupported {
            version: version.to_string(),
            minimum: self.minimum.to_string(),
        })
    }

    /// Identify the server and resolve the API generation its version speaks.
    fn resolve_status(
        &self,
        status: ProwlarrSystemStatus,
    ) -> Result<ResolvedProwlarrApi, ProwlarrRequestError> {
        let app_name = status.app_name.trim();
        if !app_name.eq_ignore_ascii_case("Prowlarr") {
            let message = if app_name.is_empty() {
                "base_url responded but did not identify itself as Prowlarr".to_string()
            } else {
                format!("base_url responded as '{}', not Prowlarr", app_name)
            };
            return Err(ProwlarrRequestError::InvalidConfig(message));
        }

        let version = ProwlarrProductVersion::parse(&status.version)?;
        let support = self.resolve(&version)?;
        Ok(ResolvedProwlarrApi {
            generation: support.generation,
            version,
            tier: support.tier,
            status,
        })
    }

    fn connected_message(&self, api: &ResolvedProwlarrApi) -> String {
        match api.tier {
            ProwlarrSupportTier::Tested => format!("Connected to Prowlarr {}", api.version),
            ProwlarrSupportTier::Newer => format!(
                "Connected to Prowlarr {} (newer than the tested {}.x line)",
                api.version,
                self.highest_tested_major()
            ),
        }
    }
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
    LocalQuota(AppError),
    /// The server is older than [`ProwlarrSupportPolicy::minimum`].
    VersionUnsupported {
        version: String,
        minimum: String,
    },
}

impl ProwlarrRequestError {
    /// The stable reason a client can act on, for the refusals that have one.
    fn reason(&self) -> Option<&'static str> {
        match self {
            Self::InvalidConfig(_) => {
                Some(crate::error_reasons::ACQUISITION_PROWLARR_CONFIG_INVALID)
            }
            Self::AuthFailed(_) => Some(crate::error_reasons::ACQUISITION_PROWLARR_AUTH_FAILED),
            Self::VersionUnsupported { .. } => {
                Some(crate::error_reasons::ACQUISITION_PROWLARR_VERSION_UNSUPPORTED)
            }
            Self::LocalQuota(error) => error.reason(),
            Self::RateLimited(..) | Self::Unreachable(_) | Self::Unsupported(_) => None,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::InvalidConfig(message)
            | Self::AuthFailed(message)
            | Self::RateLimited(message, _, _)
            | Self::Unreachable(message)
            | Self::Unsupported(message) => message.clone(),
            Self::LocalQuota(error) => error.to_string(),
            Self::VersionUnsupported { version, minimum } => {
                format!("Prowlarr {version} is not supported; upgrade to {minimum} or newer")
            }
        }
    }

    fn to_validation_result(&self) -> IndexerValidationResult {
        let (status, retry_after_seconds) = match self {
            Self::InvalidConfig(_) => ("invalid_config", None),
            Self::AuthFailed(_) => ("auth_failed", None),
            Self::RateLimited(_, retry_after_seconds, _) => ("rate_limited", *retry_after_seconds),
            Self::LocalQuota(error)
                if error.reason()
                    == Some(
                        scryer_application::error_reasons::indexer_monthly_quota::REQUEST_LIMIT_REACHED,
                    ) =>
            {
                ("monthly_request_limit_reached", None)
            }
            Self::LocalQuota(_) => ("monthly_quota_unavailable", None),
            Self::Unreachable(_) => ("unreachable", None),
            Self::Unsupported(_) | Self::VersionUnsupported { .. } => ("unsupported", None),
        };
        IndexerValidationResult {
            reason: self.reason(),
            ..validation_result(status, Some(&self.message()), retry_after_seconds)
        }
    }

    fn into_app_error(self) -> AppError {
        match self {
            Self::LocalQuota(error) => error,
            error => match (error.reason(), error) {
                (Some(reason), error) => AppError::validation_refused(reason, error.message()),
                (None, Self::RateLimited(message, Some(retry_after_seconds), cooldown_action)) => {
                    AppError::rate_limited_temporary_unavailable(
                        message,
                        Some(std::time::Duration::from_secs(
                            retry_after_seconds.max(1) as u64
                        )),
                        cooldown_action,
                    )
                }
                (None, Self::RateLimited(message, None, cooldown_action)) => {
                    AppError::rate_limited_temporary_unavailable(message, None, cooldown_action)
                }
                (None, error) => AppError::Repository(error.message()),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ProwlarrSystemStatus {
    #[serde(default, rename = "appName")]
    app_name: String,
    version: String,
}

/// The API generation a connected Prowlarr was resolved to, with what it
/// reported about itself.
#[derive(Debug, Clone)]
struct ResolvedProwlarrApi {
    generation: ProwlarrApiGeneration,
    version: ProwlarrProductVersion,
    tier: ProwlarrSupportTier,
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
    #[serde(default)]
    pub parser_version: u32,
    #[serde(default)]
    pub searching_present: bool,
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

/// Prowlarr as a host-native indexer provider: a management-only parent
/// that syncs child Newznab and Torznab indexers.
pub struct NativeProwlarrIndexerProvider {
    indexer_errors: Arc<dyn IndexerErrorRepository>,
    indexer_stats: Arc<dyn IndexerStatsTracker>,
}

impl Default for NativeProwlarrIndexerProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeProwlarrIndexerProvider {
    pub fn new() -> Self {
        Self::new_with_indexer_error_repository(Arc::new(NullIndexerErrorRepository))
    }

    pub fn new_with_indexer_error_repository(
        indexer_errors: Arc<dyn IndexerErrorRepository>,
    ) -> Self {
        Self {
            indexer_errors,
            indexer_stats: Arc::new(NullIndexerStatsTracker),
        }
    }

    /// Attach the tracker that counts requests to Prowlarr's own API.
    pub fn with_indexer_stats_tracker(
        mut self,
        indexer_stats: Arc<dyn IndexerStatsTracker>,
    ) -> Self {
        self.indexer_stats = indexer_stats;
        self
    }
}

pub struct ProwlarrManagementClient {
    parent_config_id: String,
    parent_indexer_name: String,
    config: Result<ProwlarrConfig, String>,
    outbound_http: OutboundHttpClient,
    api_state: Arc<RwLock<Option<ResolvedProwlarrApi>>>,
    indexer_errors: Arc<dyn IndexerErrorRepository>,
    indexer_stats: Arc<dyn IndexerStatsTracker>,
    accounting: Option<scryer_application::IndexerAccountingContext>,
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
        tally: Option<&IndexerRequestTally>,
    ) {
        match captured_response(response).await {
            Ok(response) => {
                // A 429 that the transport will retry is still one request the
                // indexer served, and this observer is the only place it is
                // visible before the retry replaces it.
                if let Some(tally) = tally {
                    tally.note_response(&response);
                }
                self.record(operation, response).await
            }
            Err(error) => {
                if let Some(tally) = tally {
                    tally.note_result(IndexerRequestResult::NoResponse);
                }
                warn!(error = %error, "failed to read Prowlarr rate-limit response")
            }
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
        Self::new_with_indexer_error_repository_and_stats(
            config,
            indexer_errors,
            Arc::new(NullIndexerStatsTracker),
        )
    }

    pub fn new_with_indexer_error_repository_and_stats(
        config: &IndexerConfig,
        indexer_errors: Arc<dyn IndexerErrorRepository>,
        indexer_stats: Arc<dyn IndexerStatsTracker>,
    ) -> Self {
        let http_client = indexer_reqwest_client();
        Self {
            parent_config_id: config.id.clone(),
            parent_indexer_name: config.name.clone(),
            config: ProwlarrConfig::from_indexer_config(config),
            outbound_http: OutboundHttpClient::new(http_client, RateLimitRegistry::indexers()),
            api_state: Arc::new(RwLock::new(None)),
            indexer_errors,
            indexer_stats,
            accounting: scryer_application::IndexerAccountingContext::for_config(config),
        }
    }

    fn with_accounting(
        mut self,
        accounting: Option<&scryer_application::IndexerAccountingContext>,
    ) -> Self {
        self.accounting = accounting.cloned();
        self
    }

    fn tally(&self, operation: IndexerErrorOperation) -> IndexerRequestTally {
        IndexerRequestTally::new(
            self.accounting
                .as_ref()
                .map(|context| context.indexer_id.clone())
                .unwrap_or_else(|| scryer_application::CONNECTION_TEST_INDEXER_ID.to_string()),
            self.accounting
                .as_ref()
                .map(|context| context.indexer_name.clone())
                .unwrap_or_else(|| self.parent_indexer_name.clone()),
            operation,
            Arc::clone(&self.indexer_stats),
        )
    }

    fn config(&self) -> AppResult<&ProwlarrConfig> {
        self.config.as_ref().map_err(|message| {
            AppError::validation_refused(
                crate::error_reasons::ACQUISITION_PROWLARR_CONFIG_INVALID,
                message.clone(),
            )
        })
    }

    /// Refresh exactly one existing managed child, using the same transport and
    /// child quota identity as parent management. No discovery or sync occurs.
    pub(crate) async fn refresh_configured_child_caps(
        mut self,
        child: &IndexerConfig,
        request_client: reqwest::Client,
        accounting: Option<&scryer_application::IndexerAccountingContext>,
    ) -> AppResult<DomainCapsSnapshot> {
        let indexer_id = child
            .managed_child_key
            .as_deref()
            .and_then(|key| key.parse::<i64>().ok())
            .filter(|id| *id > 0)
            .ok_or_else(|| {
                AppError::validation_refused(
                    crate::error_reasons::ACQUISITION_PROWLARR_CONFIG_INVALID,
                    "Managed indexer has no valid Prowlarr child id",
                )
            })?;
        self.outbound_http = OutboundHttpClient::new(request_client, RateLimitRegistry::indexers());
        self.accounting = accounting.cloned();
        let snapshot = self
            .fetch_child_caps_snapshot(self.config()?, indexer_id)
            .await
            .map_err(ProwlarrRequestError::into_app_error)?;
        let raw = serialize_caps_snapshot(Some(&snapshot), &[])
            .ok_or_else(|| AppError::Repository("Cannot serialize child capabilities".into()))?;
        let mut snapshot: DomainCapsSnapshot =
            serde_json::from_str(&raw).map_err(|error| AppError::Repository(error.to_string()))?;
        // Parent management learned the category tree from the child resource;
        // a caps-only refresh must retain it without resyncing every child.
        if let Some(previous) =
            scryer_application::cached_indexer_caps(child, chrono::Utc::now()).snapshot
        {
            snapshot.categories = previous.categories;
        }
        Ok(snapshot)
    }

    fn error_capture(&self) -> ProwlarrErrorCapture {
        ProwlarrErrorCapture {
            indexer_id: self.parent_config_id.clone(),
            indexer_name: self.parent_indexer_name.clone(),
            indexer_errors: Arc::clone(&self.indexer_errors),
        }
    }

    #[cfg(test)]
    async fn fetch_system_status(&self) -> Result<ProwlarrSystemStatus, ProwlarrRequestError> {
        Ok(self
            .resolve_api(IndexerErrorOperation::ConnectionTest)
            .await?
            .status)
    }

    /// The generation requests are labelled and shaped with: the resolved one
    /// once a status read has run, otherwise the discovery generation.
    fn current_generation(&self) -> ProwlarrApiGeneration {
        self.api_state
            .try_read()
            .ok()
            .and_then(|guard| guard.as_ref().map(|api| api.generation))
            .unwrap_or(ProwlarrApiGeneration::DISCOVERY)
    }

    /// Read `system/status` once per client and resolve the API generation the
    /// reported version speaks through [`PROWLARR_SUPPORT_POLICY`].
    async fn resolve_api(
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

        let discovery = ProwlarrApiGeneration::DISCOVERY;
        let (status, response) = self
            .get_decoded_with_response(discovery.system_status_path(), operation, |body| {
                discovery.decode_system_status(body)
            })
            .await?;
        let api = match PROWLARR_SUPPORT_POLICY.resolve_status(status) {
            Ok(api) => api,
            Err(error) => {
                self.error_capture().record(operation, response).await;
                return Err(error);
            }
        };
        debug!(
            app_name = api.status.app_name.as_str(),
            version = %api.version,
            generation = api.generation.request_namespace(),
            tier = ?api.tier,
            "resolved Prowlarr API generation"
        );
        if api.tier == ProwlarrSupportTier::Newer {
            warn!(
                version = %api.version,
                highest_tested_major = PROWLARR_SUPPORT_POLICY.highest_tested_major(),
                generation = api.generation.request_namespace(),
                "connected to a Prowlarr newer than the tested releases"
            );
        }
        *guard = Some(api.clone());
        Ok(api)
    }

    async fn get_decoded<T>(
        &self,
        path: &str,
        operation: IndexerErrorOperation,
        decode: impl FnOnce(&[u8]) -> serde_json::Result<T>,
    ) -> AppResult<T> {
        self.get_decoded_with_response(path, operation, decode)
            .await
            .map(|(value, _)| value)
            .map_err(ProwlarrRequestError::into_app_error)
    }

    /// Fetch `path` and decode a successful body with `decode`; a body that
    /// does not decode is recorded as an indexer error.
    async fn get_decoded_with_response<T>(
        &self,
        path: &str,
        operation: IndexerErrorOperation,
        decode: impl FnOnce(&[u8]) -> serde_json::Result<T>,
    ) -> Result<(T, CapturedIndexerHttpResponse), ProwlarrRequestError> {
        let captured = self.get_success_response(path, operation).await?;
        match decode(&captured.body) {
            Ok(value) => Ok((value, captured)),
            Err(error) => {
                self.error_capture().record(operation, captured).await;
                Err(ProwlarrRequestError::Unsupported(format!(
                    "Prowlarr returned invalid JSON: {error}"
                )))
            }
        }
    }

    /// Fetch `path` as JSON and return the captured body of a successful
    /// response; any other status is recorded and mapped to an error.
    async fn get_success_response(
        &self,
        path: &str,
        operation: IndexerErrorOperation,
    ) -> Result<CapturedIndexerHttpResponse, ProwlarrRequestError> {
        let config = self
            .config
            .as_ref()
            .map_err(|message| ProwlarrRequestError::InvalidConfig(message.clone()))?;
        let base_url = config.base_url.clone();
        let api_key = config.api_key.clone();
        let url = api_url(&base_url, path);
        let error_capture = self.error_capture();
        let tally = Arc::new(self.tally(operation));
        let observer_tally = Arc::clone(&tally);
        let response = self
            .outbound_http
            .send_with_rate_limit_and_async_dispatch_observer(
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
                    let observer_tally = Arc::clone(&observer_tally);
                    async move {
                        error_capture
                            .observe_rate_limited_response(
                                operation,
                                response,
                                Some(observer_tally.as_ref()),
                            )
                            .await;
                    }
                },
                |actual_url| {
                    let tally = Arc::clone(&tally);
                    let same_origin = reqwest::Url::parse(&url)
                        .is_ok_and(|url| url.origin() == actual_url.origin());
                    async move {
                        let admitted = if same_origin {
                            tally.try_note_sent_async(actual_url.as_str()).await
                        } else {
                            tally.try_note_auxiliary_sent_call("external_redirect")
                        };
                        admitted
                            .then_some(())
                            .ok_or(OutboundHttpError::DispatchRejected)
                    }
                },
            )
            .await
            .inspect_err(|error| {
                if matches!(error, OutboundHttpError::Transport { .. }) {
                    tally.note_result(IndexerRequestResult::NoResponse);
                }
            })
            .map_err(|error| match error {
                OutboundHttpError::DispatchRejected => tally
                    .take_dispatch_error()
                    .map(ProwlarrRequestError::LocalQuota)
                    .unwrap_or_else(|| {
                        ProwlarrRequestError::Unreachable("indexer dispatch is closed".to_string())
                    }),
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
        let captured = match captured_response(response).await {
            Ok(captured) => captured,
            Err(error) => {
                tally.note_result(IndexerRequestResult::NoResponse);
                return Err(captured_response_error(error));
            }
        };
        tally.note_response(&captured);
        let status = StatusCode::from_u16(captured.status).expect("reqwest status is valid");

        if status.is_success() {
            return Ok(captured);
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
        let ProwlarrChildCapsRequest {
            url,
            request_path,
            accept,
        } = self
            .current_generation()
            .child_caps_request(config, indexer_id);
        let child_key = indexer_id.to_string();
        let error_capture = self.error_capture();
        // This is an observed call made during parent management. We cannot
        // infer how many requests Prowlarr makes internally from this response.
        let tally = Arc::new(self.tally(IndexerErrorOperation::CapsRefresh));
        let observer_tally = Arc::clone(&tally);
        let response = self
            .outbound_http
            .send_with_rate_limit_and_async_dispatch_observer(
                // Caps can wait for a later pass, so a recorded cooldown of any
                // length fails fast here instead of holding the sync lock while
                // it runs out.
                self.child_request_policy(&request_path, &child_key)
                    .with_max_inline_cooldown_wait(Duration::ZERO),
                || {
                    self.outbound_http
                        .client()
                        .get(&url)
                        .header("Accept", accept)
                        .header("User-Agent", USER_AGENT)
                },
                move |response| {
                    let error_capture = error_capture.clone();
                    let observer_tally = Arc::clone(&observer_tally);
                    async move {
                        error_capture
                            .observe_rate_limited_response(
                                IndexerErrorOperation::CapsRefresh,
                                response,
                                Some(observer_tally.as_ref()),
                            )
                            .await;
                    }
                },
                |actual_url| {
                    let tally = Arc::clone(&tally);
                    let same_origin = reqwest::Url::parse(&url)
                        .is_ok_and(|url| url.origin() == actual_url.origin());
                    async move {
                        let admitted = if same_origin {
                            tally.try_note_sent_async(actual_url.as_str()).await
                        } else {
                            tally.try_note_auxiliary_sent_call("external_redirect")
                        };
                        admitted
                            .then_some(())
                            .ok_or(OutboundHttpError::DispatchRejected)
                    }
                },
            )
            .await
            .inspect_err(|error| {
                if matches!(error, OutboundHttpError::Transport { .. }) {
                    tally.note_result(IndexerRequestResult::NoResponse);
                }
            })
            .map_err(|error| match error {
                OutboundHttpError::DispatchRejected => tally
                    .take_dispatch_error()
                    .map(ProwlarrRequestError::LocalQuota)
                    .unwrap_or_else(|| {
                        ProwlarrRequestError::Unreachable("indexer dispatch is closed".to_string())
                    }),
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
        let captured = match captured_response(response).await {
            Ok(captured) => captured,
            Err(error) => {
                tally.note_result(IndexerRequestResult::NoResponse);
                return Err(captured_response_error(error));
            }
        };
        tally.note_response(&captured);
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

    /// Lists Prowlarr's indexers and app profiles. With `caps_due`, it also
    /// requests capabilities, but only for the enabled children named there.
    async fn build_sync_plan(
        &self,
        caps_due: Option<&HashSet<String>>,
    ) -> AppResult<IndexerSyncPlan> {
        let config = self.config()?.clone();
        let generation = self
            .resolve_api(IndexerErrorOperation::ManagementSync)
            .await
            .map_err(ProwlarrRequestError::into_app_error)?
            .generation;
        let indexers = self
            .get_decoded(
                generation.indexer_path(),
                IndexerErrorOperation::ManagementSync,
                |body| generation.decode_indexers(body),
            )
            .await?;
        let app_profiles = self
            .get_decoded(
                generation.app_profile_path(),
                IndexerErrorOperation::ManagementSync,
                |body| generation.decode_app_profiles(body),
            )
            .await?;
        let app_profiles_by_id = app_profiles
            .into_iter()
            .map(|profile| (profile.id, profile))
            .collect::<HashMap<_, _>>();

        let Some(caps_due) = caps_due.filter(|due| !due.is_empty()) else {
            let children = indexers
                .into_iter()
                .filter_map(|indexer| {
                    build_managed_child_plan(
                        generation,
                        &config,
                        indexer,
                        &app_profiles_by_id,
                        None,
                    )
                })
                .collect();
            return Ok(IndexerSyncPlan { children });
        };

        let mut planned_children = stream::iter(indexers.into_iter().enumerate())
            .map(|(position, indexer)| {
                let config = config.clone();
                async move {
                    let child_key = indexer.id.to_string();
                    let caps_snapshot = if !caps_due.contains(&child_key) {
                        None
                    } else if indexer.enable {
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
                            // A rate limit is already recorded on the child's
                            // cooldown and its indexer errors; repeating it at
                            // every pass until the cooldown ends adds nothing.
                            Err(ProwlarrRequestError::RateLimited(_, retry_after_seconds, _)) => {
                                debug!(
                                    child_key,
                                    retry_after_seconds,
                                    "managed child caps fetch deferred by a rate-limit cooldown"
                                );
                                None
                            }
                            Err(error) => {
                                warn!(
                                    child_key,
                                    error = ?error,
                                    "failed to fetch managed child caps snapshot; the stored snapshot is kept until a later fetch succeeds"
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
                build_managed_child_plan(
                    generation,
                    &config,
                    indexer,
                    &app_profiles_by_id,
                    caps_snapshot,
                )
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
        let request_namespace = self.current_generation().request_namespace();
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
        Err(AppError::validation_refused(
            crate::error_reasons::ACQUISITION_PROWLARR_PARENT_SEARCH_UNSUPPORTED,
            "Prowlarr parent configs are management-only; search through synced child indexers"
                .to_string(),
        ))
    }
}

#[async_trait]
impl IndexerManagementClient for ProwlarrManagementClient {
    async fn validate_connection(&self) -> AppResult<IndexerValidationResult> {
        match self
            .resolve_api(IndexerErrorOperation::ConnectionTest)
            .await
        {
            Ok(api) => Ok(IndexerValidationResult {
                server_version: Some(api.version.to_string()),
                ..validation_result(
                    "valid",
                    Some(&PROWLARR_SUPPORT_POLICY.connected_message(&api)),
                    None,
                )
            }),
            Err(error) => Ok(error.to_validation_result()),
        }
    }

    async fn plan_sync(&self, _parent_config_id: &str) -> AppResult<IndexerSyncPlan> {
        self.build_sync_plan(None).await
    }

    async fn enrichment_sync_plan(
        &self,
        _parent_config_id: &str,
        caps_due: &HashSet<String>,
    ) -> AppResult<Option<IndexerSyncPlan>> {
        self.build_sync_plan(Some(caps_due)).await.map(Some)
    }

    async fn preview_sync_plan(&self, _parent_config_id: &str) -> AppResult<IndexerSyncPlan> {
        self.build_sync_plan(None).await
    }

    fn name(&self) -> &str {
        PROWLARR_PROVIDER_TYPE
    }
}

impl NativeIndexerProvider for NativeProwlarrIndexerProvider {
    fn provider_type(&self) -> &str {
        PROWLARR_PROVIDER_TYPE
    }

    fn display_name(&self) -> String {
        "Prowlarr".to_string()
    }

    fn description(&self) -> String {
        "First-party Prowlarr provider that syncs managed Newznab and Torznab indexers".to_string()
    }

    fn config_fields(&self) -> Vec<ConfigFieldDef> {
        prowlarr_config_fields()
    }

    fn management_capabilities(&self) -> scryer_domain::IndexerManagementCapabilities {
        scryer_domain::IndexerManagementCapabilities {
            supports_validate_config: true,
            supports_managed_children_sync: true,
        }
    }

    fn capabilities(&self) -> scryer_domain::IndexerProviderCapabilities {
        scryer_domain::IndexerProviderCapabilities::default()
    }

    fn client(
        &self,
        _config: &IndexerConfig,
        _proxy_config: Option<&scryer_domain::ProxyConfig>,
        _accounting: Option<&IndexerAccountingContext>,
    ) -> Option<Arc<dyn IndexerClient>> {
        // The parent never searches; its synced children do.
        Some(Arc::new(ProwlarrSearchStub))
    }

    fn management_client(
        &self,
        config: &IndexerConfig,
        accounting: Option<&IndexerAccountingContext>,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        Some(Arc::new(
            ProwlarrManagementClient::new_with_indexer_error_repository_and_stats(
                config,
                Arc::clone(&self.indexer_errors),
                Arc::clone(&self.indexer_stats),
            )
            .with_accounting(accounting),
        ))
    }
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
            ..Default::default()
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
            ..Default::default()
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
        ..IndexerValidationResult::default()
    }
}

fn build_managed_child_plan(
    generation: ProwlarrApiGeneration,
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
        "base_url": generation.child_base_url(&config.base_url, indexer.id),
        "api_key": config.api_key,
        "api_path": generation.child_api_path(),
        "imdb_id_format": "canonical",
    });
    let caps_snapshot_json =
        serialize_caps_snapshot(caps_snapshot.as_ref(), &indexer.capabilities.categories);
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
        404 => ProwlarrRequestError::InvalidConfig(
            if ProwlarrApiGeneration::is_system_status_path(path) {
                "base_url does not appear to point at a Prowlarr API".to_string()
            } else if ProwlarrApiGeneration::is_sync_path(path) {
                format!("Prowlarr sync endpoint '{path}' was not found")
            } else {
                format!("Prowlarr endpoint '{path}' was not found")
            },
        ),
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
    let mut snapshot = ProwlarrCapsSnapshot {
        parser_version: scryer_domain::INDEXER_CAPS_PARSER_VERSION,
        ..Default::default()
    };

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                match element.name().as_ref() {
                    "searching" => snapshot.searching_present = true,
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
            .is_none_or(|value| value.eq_ignore_ascii_case("yes")),
        supported_params: attr_value(element, "supportedParams")?
            .unwrap_or_else(|| "q".into())
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

/// Serialize the child's caps snapshot. Prowlarr reports the indexer's
/// category tree on the indexer resource rather than in the caps document, so
/// it is folded in here; the routing picker reads it back through the stored
/// snapshot to offer indexer-specific categories such as a "Movies-DE" subcat.
fn serialize_caps_snapshot(
    snapshot: Option<&ProwlarrCapsSnapshot>,
    categories: &[ProwlarrCategory],
) -> Option<String> {
    let snapshot = snapshot?;
    serde_json::to_string(&DomainCapsSnapshot {
        parser_version: snapshot.parser_version,
        searching_present: snapshot.searching_present,
        server_title: snapshot.server_title.clone(),
        limits_default: snapshot.limits_default,
        limits_max: snapshot.limits_max,
        search: caps_node_to_domain(&snapshot.search),
        tv_search: caps_node_to_domain(&snapshot.tv_search),
        movie_search: caps_node_to_domain(&snapshot.movie_search),
        music_search: caps_node_to_domain(&snapshot.music_search),
        audio_search: caps_node_to_domain(&snapshot.audio_search),
        book_search: caps_node_to_domain(&snapshot.book_search),
        categories: caps_categories_to_domain(categories),
    })
    .ok()
}

fn caps_categories_to_domain(categories: &[ProwlarrCategory]) -> DomainCategoryModel {
    let mut flattened = std::collections::BTreeMap::<i64, DomainCategoryDescriptor>::new();
    for category in categories {
        flatten_caps_category(category, &mut flattened);
    }
    if flattened.is_empty() {
        return DomainCategoryModel::default();
    }
    DomainCategoryModel {
        value_kinds: vec![DomainCategoryValueKind::Numeric],
        separate_anime_categories: false,
        provider_category_metadata: true,
        categories: flattened.into_values().collect(),
    }
}

fn flatten_caps_category(
    category: &ProwlarrCategory,
    flattened: &mut std::collections::BTreeMap<i64, DomainCategoryDescriptor>,
) {
    let label = Some(category.name.trim().to_string()).filter(|name| !name.is_empty());
    flattened
        .entry(category.id)
        .and_modify(|existing| {
            if existing.label.is_none() && label.is_some() {
                existing.label = label.clone();
            }
        })
        .or_insert_with(|| DomainCategoryDescriptor {
            value: category.id.to_string(),
            label,
            value_kind: DomainCategoryValueKind::Numeric,
            facets: Vec::new(),
        });
    for sub_category in &category.sub_categories {
        flatten_caps_category(sub_category, flattened);
    }
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
    use std::sync::Mutex;
    use std::time::Instant;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    const SYSTEM_STATUS_PATH: &str = ProwlarrApiGeneration::ApiV1.system_status_path();
    const INDEXER_PATH: &str = ProwlarrApiGeneration::ApiV1.indexer_path();
    const APP_PROFILE_PATH: &str = ProwlarrApiGeneration::ApiV1.app_profile_path();

    #[derive(Default)]
    struct RecordingIndexerErrorRepository {
        errors: tokio::sync::Mutex<Vec<NewIndexerError>>,
    }

    /// Counts what the dashboard would show: one per request actually sent.
    #[derive(Default)]
    struct CountingIndexerStatsTracker {
        sent: std::sync::atomic::AtomicU32,
    }

    impl IndexerStatsTracker for CountingIndexerStatsTracker {
        fn record_query(&self, _indexer_id: &str, _indexer_name: &str, _success: bool) {}
        fn record_grab(&self, _indexer_id: &str, _indexer_name: &str) {}
        fn record_api_limits(
            &self,
            _indexer_id: &str,
            _api_current: Option<u32>,
            _api_max: Option<u32>,
            _grab_current: Option<u32>,
            _grab_max: Option<u32>,
        ) {
        }
        fn record_api_request_sent(&self, _indexer_id: &str, _indexer_name: &str) {
            self.sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        fn all_stats(&self) -> Vec<scryer_application::IndexerQueryStats> {
            Vec::new()
        }
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

    #[test]
    fn native_prowlarr_provider_returns_the_search_stub_and_a_prowlarr_management_client() {
        let provider = NativeProwlarrIndexerProvider::new();
        let config = test_indexer_config("http://127.0.0.1:9696");

        assert_eq!(provider.provider_type(), PROWLARR_PROVIDER_TYPE);
        assert!(
            provider.client(&config, None, None).is_some(),
            "a Prowlarr parent gets the management-only search stub"
        );
        let management = provider
            .management_client(&config, None)
            .expect("Prowlarr management client");
        assert_eq!(management.name(), PROWLARR_PROVIDER_TYPE);
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
            last_error_code: None,
            id: "cfg-1".to_string(),
            name: "Prowlarr".to_string(),
            provider_type: PROWLARR_PROVIDER_TYPE.to_string(),
            base_url: base_url.to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            max_requests_per_month: None,
            max_grabs_per_month: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: false,
            enable_auto_search: false,
            proxy_config_id: None,
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
        let child = build_managed_child_plan(
            ProwlarrApiGeneration::ApiV1,
            &config,
            indexer,
            &app_profiles,
            None,
        )
        .expect("child plan");
        serde_json::from_str(child.managed_metadata_json.as_deref().unwrap()).unwrap()
    }

    /// Verbatim `/api/v1/indexer` entry from a real Prowlarr container
    /// (linuxserver/prowlarr, Torznab indexer). Pins the field names and the
    /// JSON shapes Prowlarr actually emits — including the advanced fields,
    /// which it does return, and the null it sends for an unset one.
    const REAL_PROWLARR_INDEXER_JSON: &str = r#"{
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
            }"#;

    /// Verbatim `/api/v1/appprofile` entry from the same container. Prowlarr
    /// declares `MinimumSeeders` as a plain int, so a real deployment always
    /// sends one — the default profile's 1.
    const REAL_PROWLARR_APP_PROFILE_JSON: &str = r#"{
              "name": "Standard",
              "enableRss": true,
              "enableAutomaticSearch": true,
              "enableInteractiveSearch": true,
              "minimumSeeders": 1,
              "id": 1
            }"#;

    #[test]
    fn a_real_prowlarr_payload_imports_its_seed_criteria() {
        let indexer: ProwlarrIndexerResource = serde_json::from_str(REAL_PROWLARR_INDEXER_JSON)
            .expect("real Prowlarr payload should deserialize");

        let profile: ProwlarrAppProfile = serde_json::from_str(REAL_PROWLARR_APP_PROFILE_JSON)
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
            ProwlarrApiGeneration::ApiV1,
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
                    sub_categories: vec![ProwlarrCategory {
                        id: 100020,
                        name: "Movies-DE".to_string(),
                        sub_categories: vec![],
                    }],
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

        let child = build_managed_child_plan(
            ProwlarrApiGeneration::ApiV1,
            &config,
            indexer,
            &app_profiles,
            Some(caps_snapshot.clone()),
        )
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
        assert_eq!(child.routing_scopes[0].categories, vec!["100020", "2000"]);
        assert_eq!(metadata.indexer_id, 7);
        assert_eq!(metadata.app_profile_id, 12);
        assert_eq!(metadata.download_client_id, 3);
        assert!(metadata.enable_rss);
        assert!(!metadata.enable_automatic_search);
        assert_eq!(metadata.caps_snapshot, Some(caps_snapshot));

        let stored_snapshot: DomainCapsSnapshot =
            serde_json::from_str(child.caps_snapshot_json.as_deref().unwrap()).unwrap();
        assert_eq!(stored_snapshot.server_title.as_deref(), Some("Prowlarr"));
        assert_eq!(
            stored_snapshot
                .categories
                .categories
                .iter()
                .map(|category| (category.value.as_str(), category.label.as_deref()))
                .collect::<Vec<_>>(),
            vec![("2000", Some("Movies")), ("100020", Some("Movies-DE"))]
        );
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

        let child = build_managed_child_plan(
            ProwlarrApiGeneration::ApiV1,
            &config,
            indexer,
            &app_profiles,
            None,
        )
        .expect("child plan");
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
        let stats = Arc::new(CountingIndexerStatsTracker::default());
        let client = ProwlarrManagementClient::new_with_indexer_error_repository_and_stats(
            &test_indexer_config(&server.uri()),
            repository.clone(),
            stats.clone(),
        );

        let status = client
            .fetch_system_status()
            .await
            .expect("second attempt succeeds");
        assert_eq!(status.app_name, "Prowlarr");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(
            stats.sent.load(Ordering::Relaxed),
            2,
            "the retried 429 was a request Prowlarr served and must be counted"
        );

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
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("oversized response listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(tokio::time::timeout(
            std::time::Duration::from_secs(30),
            async move {
                let (mut stream, _) = listener.accept().await.expect("request connection");
                // Read the request before closing the socket to avoid resetting unread bytes.
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    assert!(request.len() < 8192, "request headers exceed fixture limit");
                    request.push(stream.read_u8().await.expect("request headers"));
                }
                let headers = format!(
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    PROWLARR_RESPONSE_MAX_BYTES + 1
                );
                stream
                    .write_all(headers.as_bytes())
                    .await
                    .expect("oversized response headers");
            },
        ));

        let response = indexer_reqwest_client()
            .get(format!("http://{address}/oversized"))
            .send()
            .await
            .expect("oversized response headers");
        let error = captured_response(response)
            .await
            .expect_err("response exceeds capture limit");

        assert!(matches!(error, CapturedProwlarrResponseError::TooLarge));
        server
            .await
            .expect("oversized response server")
            .expect("oversized response server timed out");
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

    async fn mount_status(server: &MockServer, version: &str) {
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Prowlarr",
                "version": version
            })))
            .mount(server)
            .await;
    }

    async fn validate_against_status(version: &str) -> IndexerValidationResult {
        let server = MockServer::start().await;
        mount_status(&server, version).await;
        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        client
            .validate_connection()
            .await
            .expect("validation result")
    }

    #[test]
    fn product_versions_parse_three_and_four_part_forms() {
        assert_eq!(
            ProwlarrProductVersion::parse("2.7.0.5723").unwrap(),
            ProwlarrProductVersion {
                major: 2,
                minor: 7,
                patch: 0,
                build: Some(5723),
            }
        );
        assert_eq!(
            ProwlarrProductVersion::parse("3.0.0.5737").unwrap(),
            ProwlarrProductVersion {
                major: 3,
                minor: 0,
                patch: 0,
                build: Some(5737),
            }
        );
        assert_eq!(
            ProwlarrProductVersion::parse("3.0.0").unwrap(),
            ProwlarrProductVersion::new(3, 0, 0)
        );
        let padded = ProwlarrProductVersion::parse(" 3.0.0.5737 ").unwrap();
        assert_eq!(padded, ProwlarrProductVersion::parse("3.0.0.5737").unwrap());

        assert_eq!(
            ProwlarrProductVersion::parse("2.7.0.5723")
                .unwrap()
                .to_string(),
            "2.7.0.5723"
        );
        assert_eq!(
            ProwlarrProductVersion::parse("3.0.0").unwrap().to_string(),
            "3.0.0"
        );
        assert_eq!(padded.to_string(), "3.0.0.5737");
    }

    #[test]
    fn product_versions_reject_empty_and_non_numeric_input() {
        for raw in ["", "three", "3.x"] {
            match ProwlarrProductVersion::parse(raw) {
                Err(ProwlarrRequestError::Unsupported(message)) => assert_eq!(
                    message,
                    format!("could not determine Prowlarr version from '{raw}'")
                ),
                other => panic!("'{raw}' should be refused, got {other:?}"),
            }
        }
    }

    #[test]
    fn support_policy_maps_versions_to_generations_and_tiers() {
        let resolve = |raw: &str| {
            PROWLARR_SUPPORT_POLICY.resolve(&ProwlarrProductVersion::parse(raw).unwrap())
        };

        match resolve("1.37.0.5076") {
            Err(ProwlarrRequestError::VersionUnsupported { version, minimum }) => {
                assert_eq!(version, "1.37.0.5076");
                assert_eq!(minimum, "2.0.0");
            }
            other => panic!("1.37.0.5076 should be below the minimum, got {other:?}"),
        }
        for tested in ["2.0.0", "2.7.0.5723", "3.0.0.5737"] {
            assert_eq!(
                resolve(tested).unwrap(),
                ProwlarrSupport {
                    generation: ProwlarrApiGeneration::ApiV1,
                    tier: ProwlarrSupportTier::Tested,
                },
                "{tested}"
            );
        }
        assert_eq!(
            resolve("4.0.0.1").unwrap(),
            ProwlarrSupport {
                generation: ProwlarrApiGeneration::ApiV1,
                tier: ProwlarrSupportTier::Newer,
            }
        );
    }

    #[tokio::test]
    async fn validate_connection_accepts_tested_prowlarr_releases() {
        for version in ["2.7.0.5723", "3.0.0.5737"] {
            let result = validate_against_status(version).await;
            assert_eq!(result.status, "valid", "{version}");
            assert_eq!(
                result.message.as_deref(),
                Some(format!("Connected to Prowlarr {version}").as_str())
            );
            assert_eq!(result.server_version.as_deref(), Some(version));
            assert_eq!(result.reason, None);
        }
    }

    #[tokio::test]
    async fn validate_connection_accepts_a_newer_major_through_the_newest_generation() {
        let result = validate_against_status("4.0.0.1").await;

        assert_eq!(result.status, "valid");
        assert_eq!(
            result.message.as_deref(),
            Some("Connected to Prowlarr 4.0.0.1 (newer than the tested 3.x line)")
        );
        assert_eq!(result.server_version.as_deref(), Some("4.0.0.1"));
    }

    #[tokio::test]
    async fn validate_connection_refuses_a_release_below_the_minimum() {
        let result = validate_against_status("1.37.0.5076").await;

        assert_eq!(result.status, "unsupported");
        assert_eq!(
            result.message.as_deref(),
            Some("Prowlarr 1.37.0.5076 is not supported; upgrade to 2.0.0 or newer")
        );
        assert_eq!(
            result.reason,
            Some(crate::error_reasons::ACQUISITION_PROWLARR_VERSION_UNSUPPORTED)
        );
        assert_eq!(result.server_version, None);

        let error = ProwlarrRequestError::VersionUnsupported {
            version: "1.37.0.5076".to_string(),
            minimum: "2.0.0".to_string(),
        }
        .into_app_error();
        assert_eq!(
            error.validation_reason(),
            Some(crate::error_reasons::ACQUISITION_PROWLARR_VERSION_UNSUPPORTED)
        );
        assert_eq!(
            error.to_string(),
            "validation: Prowlarr 1.37.0.5076 is not supported; upgrade to 2.0.0 or newer"
        );
    }

    #[tokio::test]
    async fn sync_refuses_a_release_below_the_minimum_with_its_reason() {
        let server = MockServer::start().await;
        mount_status(&server, "1.37.0.5076").await;
        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));

        let error = client.plan_sync("parent").await.expect_err("refused");

        assert_eq!(
            error.validation_reason(),
            Some(crate::error_reasons::ACQUISITION_PROWLARR_VERSION_UNSUPPORTED)
        );
        let requests = server.received_requests().await.expect("recorded requests");
        assert_eq!(
            requests.len(),
            1,
            "an unsupported release must not be asked for its indexers"
        );
    }

    /// Plans the verbatim `/api/v1` payloads, with a due caps fetch, against a
    /// Prowlarr reporting `version`.
    async fn plan_real_payloads_against(version: &str) -> IndexerSyncPlan {
        let server = MockServer::start().await;
        mount_status(&server, version).await;
        let indexer: Value = serde_json::from_str(REAL_PROWLARR_INDEXER_JSON).unwrap();
        let profile: Value = serde_json::from_str(REAL_PROWLARR_APP_PROFILE_JSON).unwrap();
        let mut categorized = indexer.clone();
        categorized["id"] = json!(2);
        categorized["name"] = json!("probe-newznab");
        categorized["protocol"] = json!("usenet");
        categorized["capabilities"] = json!({
            "categories": [
                { "id": 2000, "name": "Movies", "subCategories": [
                    { "id": 2040, "name": "Movies/HD", "subCategories": [] }
                ] },
                { "id": 5000, "name": "TV", "subCategories": [
                    { "id": 5070, "name": "TV/Anime", "subCategories": [] }
                ] }
            ]
        });
        Mock::given(method("GET"))
            .and(path(INDEXER_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([indexer, categorized])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(APP_PROFILE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([profile])))
            .mount(&server)
            .await;
        for indexer_id in [1, 2] {
            Mock::given(method("GET"))
                .and(path(format!("/{indexer_id}/api")))
                .and(query_param("t", "caps"))
                .and(query_param("apikey", "secret"))
                .respond_with(ResponseTemplate::new(200).set_body_string(MINIMAL_CAPS_BODY))
                .mount(&server)
                .await;
        }

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let plan = client
            .enrichment_sync_plan("parent", &HashSet::from(["1".into(), "2".into()]))
            .await
            .expect("enrichment plan")
            .expect("supported enrichment plan");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(caps_request_count(&requests, 1), 1, "{version}");
        assert_eq!(caps_request_count(&requests, 2), 1, "{version}");

        // Each run's mock listens on its own port, which the child configs
        // embed; normalise it so two runs compare on everything else.
        let uri = server.uri();
        IndexerSyncPlan {
            children: plan
                .children
                .into_iter()
                .map(|mut child| {
                    child.config_json = child.config_json.replace(&uri, "http://prowlarr");
                    child
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn prowlarr_two_and_three_produce_the_same_sync_plan() {
        let v2 = plan_real_payloads_against("2.7.0.5723").await;
        let v3 = plan_real_payloads_against("3.0.0.5737").await;

        assert_eq!(v2.children.len(), 2);
        assert!(
            v2.children
                .iter()
                .all(|child| child.caps_snapshot_json.is_some())
        );
        assert!(!v2.children[1].routing_scopes.is_empty());
        let metadata: ManagedChildMetadata =
            serde_json::from_str(v2.children[0].managed_metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata.seed_ratio, Some(1.5));
        assert_eq!(metadata.minimum_seeders, Some(1));
        assert_eq!(v2, v3);
    }

    #[test]
    fn request_scopes_carry_the_generation_namespace() {
        let client = ProwlarrManagementClient::new(&test_indexer_config("http://127.0.0.1:9696"));
        let policy = client.request_policy(INDEXER_PATH);

        assert_eq!(
            policy.scope.to_string(),
            "prowlarr:prowlarr_api_v1:http://127.0.0.1:9696"
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
            .enrichment_sync_plan("parent", &HashSet::from(["7".to_string()]))
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
    async fn observed_management_calls_include_redirects() {
        let server = MockServer::start().await;
        Mock::given(path(SYSTEM_STATUS_PATH))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", "/redirected-status"),
            )
            .mount(&server)
            .await;
        Mock::given(path("/redirected-status"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"appName":"Prowlarr", "version":"2.0.0"})),
            )
            .mount(&server)
            .await;
        let stats = Arc::new(CountingIndexerStatsTracker::default());
        let client = ProwlarrManagementClient::new_with_indexer_error_repository_and_stats(
            &test_indexer_config(&server.uri()),
            Arc::new(RecordingIndexerErrorRepository::default()),
            stats.clone(),
        );
        client.fetch_system_status().await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        assert_eq!(stats.sent.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn cancelled_management_cooldown_does_not_count_a_send() {
        let server = MockServer::start().await;
        let stats = Arc::new(CountingIndexerStatsTracker::default());
        let mut config = test_indexer_config(&server.uri());
        config.id = format!("cooldown-{}", server.address().port());
        let client = ProwlarrManagementClient::new_with_indexer_error_repository_and_stats(
            &config,
            Arc::new(RecordingIndexerErrorRepository::default()),
            stats.clone(),
        );
        let destination = DestinationKey::from(indexer_rate_limit_domain_key(&config.id, None));
        client
            .outbound_http
            .registry()
            .record_destination_cooldown(
                &destination,
                Duration::from_secs(30),
                scryer_outbound_http::RetryAfterSource::Seconds,
            )
            .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), client.fetch_system_status())
                .await
                .is_err()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(stats.sent.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn saved_management_probe_accounts_to_saved_identity_only() {
        let server = MockServer::start().await;
        Mock::given(path(SYSTEM_STATUS_PATH))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"appName":"Prowlarr", "version":"2.0.0"})),
            )
            .mount(&server)
            .await;
        let stats = Arc::new(crate::indexers::stats::InMemoryIndexerStatsTracker::default());
        let mut config = test_indexer_config(&server.uri());
        config.id = scryer_application::CONNECTION_TEST_INDEXER_ID.to_string();
        let accounting = scryer_application::IndexerAccountingContext {
            indexer_id: "saved-parent".into(),
            indexer_name: "Saved Prowlarr".into(),
        };
        for identity in [None, Some(&accounting)] {
            let client = ProwlarrManagementClient::new_with_indexer_error_repository_and_stats(
                &config,
                Arc::new(RecordingIndexerErrorRepository::default()),
                stats.clone(),
            )
            .with_accounting(identity);
            client.fetch_system_status().await.unwrap();
        }
        let rows = stats.all_stats();
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].indexer_id, "saved-parent");
        assert_eq!(rows[0].api_requests_today, 1);
    }

    /// A caps responder that records the window each fetch is held open for.
    ///
    /// wiremock arms a response's delay the moment this returns, so a fetch is
    /// outstanding from here until `now + delay` — exactly the span the server
    /// keeps that connection open.
    struct CapsProbe {
        body: &'static str,
        delay: Duration,
        windows: Arc<Mutex<Vec<(Instant, Instant)>>>,
    }

    impl Respond for CapsProbe {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let opened = Instant::now();
            self.windows
                .lock()
                .expect("caps windows")
                .push((opened, opened + self.delay));
            ResponseTemplate::new(200)
                .set_body_string(self.body)
                .set_delay(self.delay)
        }
    }

    /// The most caps fetches that were ever open at the same instant.
    ///
    /// Overlap always peaks as some window opens, so counting the live windows at
    /// each opening is exact rather than a sample.
    fn peak_in_flight(windows: &[(Instant, Instant)]) -> usize {
        windows
            .iter()
            .map(|(opened, _)| {
                windows
                    .iter()
                    .filter(|(other_opened, other_closed)| {
                        other_opened <= opened && opened < other_closed
                    })
                    .count()
            })
            .max()
            .unwrap_or(0)
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
        let caps_windows = Arc::new(Mutex::new(Vec::new()));
        // The delays differ so the fetches finish in a different order than they
        // were issued in, which is what gives the ordering assertion below its
        // teeth. They are all long enough that every fetch is still open while
        // the rest are being issued, however loaded the machine is.
        for (indexer_id, delay_ms) in [(10, 1400), (7, 1000), (42, 1300), (3, 1100)] {
            Mock::given(method("GET"))
                .and(path(format!("/{indexer_id}/api")))
                .and(query_param("t", "caps"))
                .and(query_param("apikey", "secret"))
                .respond_with(CapsProbe {
                    body: caps_body,
                    delay: Duration::from_millis(delay_ms),
                    windows: Arc::clone(&caps_windows),
                })
                .mount(&server)
                .await;
        }

        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));
        let plan = client
            .enrichment_sync_plan(
                "parent",
                &["10", "7", "42", "3"]
                    .map(String::from)
                    .into_iter()
                    .collect(),
            )
            .await
            .expect("enrichment plan")
            .expect("supported enrichment plan");

        // Concurrency is what this asserts, so measure concurrency: how long the
        // whole plan took is a proxy that a loaded machine can push past any
        // budget while the fetches were perfectly concurrent all along.
        let peak = peak_in_flight(&caps_windows.lock().expect("caps windows"));
        assert_eq!(
            peak, 4,
            "every caps fetch should be in flight at once, not issued one at a time"
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
            .enrichment_sync_plan("parent", &HashSet::from(["7".to_string()]))
            .await
            .expect("enrichment plan")
            .expect("supported enrichment plan");
        let child = plan.children.first().expect("child plan");
        let metadata: ManagedChildMetadata =
            serde_json::from_str(child.managed_metadata_json.as_deref().unwrap()).unwrap();

        assert_eq!(metadata.indexer_id, 7);
        assert!(metadata.caps_snapshot.is_none());
    }

    async fn mount_two_child_listing(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path(SYSTEM_STATUS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "appName": "Prowlarr",
                "version": "2.0.0"
            })))
            .mount(server)
            .await;
        let child = |id: i64| {
            json!({
                "id": id,
                "name": format!("Synthetic child {id}"),
                "enable": true,
                "appProfileId": 1,
                "protocol": "usenet",
                "priority": 3,
                "downloadClientId": 0,
                "capabilities": { "categories": [] }
            })
        };
        Mock::given(method("GET"))
            .and(path(INDEXER_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([child(7), child(10)])))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path(APP_PROFILE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": 1,
                "enableRss": true,
                "enableAutomaticSearch": true,
                "enableInteractiveSearch": true
            }])))
            .mount(server)
            .await;
    }

    const MINIMAL_CAPS_BODY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<caps>
  <searching>
    <search available="yes" supportedParams="q" />
  </searching>
</caps>"#;

    fn caps_request_count(requests: &[Request], indexer_id: i64) -> usize {
        requests
            .iter()
            .filter(|request| {
                request.url.path() == format!("/{indexer_id}/api")
                    && request.url.query().unwrap_or_default().contains("t=caps")
            })
            .count()
    }

    #[tokio::test]
    async fn enrichment_fetches_caps_only_for_due_children() {
        let server = MockServer::start().await;
        mount_two_child_listing(&server).await;
        for indexer_id in [7, 10] {
            Mock::given(method("GET"))
                .and(path(format!("/{indexer_id}/api")))
                .and(query_param("t", "caps"))
                .respond_with(ResponseTemplate::new(200).set_body_string(MINIMAL_CAPS_BODY))
                .mount(&server)
                .await;
        }
        let client = ProwlarrManagementClient::new(&test_indexer_config(&server.uri()));

        let plan = client
            .enrichment_sync_plan("parent", &HashSet::from(["10".to_string()]))
            .await
            .unwrap()
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(caps_request_count(&requests, 10), 1);
        assert_eq!(caps_request_count(&requests, 7), 0);
        let snapshots = plan
            .children
            .iter()
            .map(|child| (child.child_key.as_str(), child.caps_snapshot_json.is_some()))
            .collect::<Vec<_>>();
        assert_eq!(snapshots, vec![("7", false), ("10", true)]);

        let before = server.received_requests().await.unwrap().len();
        let plan = client
            .enrichment_sync_plan("parent", &HashSet::new())
            .await
            .unwrap()
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            caps_request_count(&requests[before..], 7)
                + caps_request_count(&requests[before..], 10),
            0,
            "a pass with nothing due must not request caps"
        );
        assert!(
            plan.children
                .iter()
                .all(|child| child.caps_snapshot_json.is_none())
        );
    }

    #[tokio::test]
    async fn enrichment_does_not_request_caps_for_a_child_under_a_recorded_cooldown() {
        let server = MockServer::start().await;
        mount_two_child_listing(&server).await;
        for indexer_id in [7, 10] {
            Mock::given(method("GET"))
                .and(path(format!("/{indexer_id}/api")))
                .and(query_param("t", "caps"))
                .respond_with(ResponseTemplate::new(200).set_body_string(MINIMAL_CAPS_BODY))
                .mount(&server)
                .await;
        }
        let mut config = test_indexer_config(&server.uri());
        config.id = format!("caps-cooldown-{}", server.address().port());
        let client = ProwlarrManagementClient::new(&config);
        // A short cooldown is inside the in-line wait budget other requests
        // use; caps must not wait it out either.
        for (child_key, seconds) in [("7", 120), ("10", 37_291)] {
            client
                .outbound_http
                .registry()
                .record_destination_cooldown(
                    &DestinationKey::from(indexer_rate_limit_domain_key(
                        &config.id,
                        Some(child_key),
                    )),
                    Duration::from_secs(seconds),
                    scryer_outbound_http::RetryAfterSource::Seconds,
                )
                .await;
        }

        let plan = tokio::time::timeout(
            Duration::from_secs(60),
            client.enrichment_sync_plan(
                "parent",
                &HashSet::from(["7".to_string(), "10".to_string()]),
            ),
        )
        .await
        .expect("a recorded cooldown must not be waited out in-line")
        .unwrap()
        .unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(caps_request_count(&requests, 7), 0);
        assert_eq!(caps_request_count(&requests, 10), 0);
        assert!(
            plan.children
                .iter()
                .all(|child| child.caps_snapshot_json.is_none())
        );
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
