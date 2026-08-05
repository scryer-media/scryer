use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::future::Future;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use aws_lc_rs::digest;
use reqwest::Client;
use scryer_application::{
    AnimeEpisodeMapping, AnimeMapping, AnimeMovie, AppError, AppResult, BulkArtworkUrlResult,
    BulkMetadataResult, DiscoveryCollectionCompletionInput, DiscoveryCollectionCompletionResult,
    DiscoveryContextChangesInput, DiscoveryContextChangesResult, DiscoveryContextSnapshotAckResult,
    DiscoveryContextSnapshotPageResult, DiscoveryContextSnapshotStatusResult,
    DiscoveryContextSnapshotSubmitInput, DiscoveryContextSnapshotSubmitResult,
    DiscoveryDashboardResult, DiscoveryPublicFeedInput, DiscoveryRelatedResult, EpisodeArtworkUrls,
    EpisodeMetadata, MetadataGateway, MetadataSearchItem, MetadataSearchQuery, MovieMetadata,
    MultiMetadataSearchResult, RateLimitCooldownAction, RichMetadataSearchItem, SeasonMetadata,
    SeriesArtworkUrls, SeriesMetadata, SettingsRepository, SmgScryerUpdateNotice, TitleArtworkUrls,
    TitleExternalRating, TitleRatingSummary, TitleRecommendationsInput,
};
use scryer_domain::CanonicalMediaTag;
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, OutboundRequestError, RateLimitRegistry, RequestPolicy,
    smg_reqwest_client,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info, warn};

struct ApqCacheEntry {
    etag: String,
    body: String,
}

struct ApqCache {
    map: HashMap<String, ApqCacheEntry>,
    order: VecDeque<String>,
}

impl ApqCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, key: &str) -> Option<&ApqCacheEntry> {
        self.map.get(key)
    }

    #[expect(clippy::map_entry)] // entry API borrows map, conflicting with eviction logic
    fn insert(&mut self, key: String, entry: ApqCacheEntry) {
        if self.map.contains_key(&key) {
            self.map.insert(key, entry);
            return;
        }
        if self.map.len() >= 1000
            && let Some(oldest) = self.order.pop_front()
        {
            self.map.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.map.insert(key, entry);
    }
}

use crate::metadata::response_body::{ResponseBodyPreview, read_response_body_preview};
use crate::{graphql::metadata_gateway as graphql_docs, smg_enrollment};

fn sha256_hex(input: &str) -> String {
    let hash = digest::digest(&digest::SHA256, input.as_bytes());
    hash.as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

fn blake3_digest(input: &str) -> String {
    format!("blake3:{}", blake3::hash(input.as_bytes()).to_hex())
}

fn apq_cache_key(operation_name: &str, hash: &str, variables_str: &str) -> String {
    format!("{operation_name}:{hash}:{}", blake3_digest(variables_str))
}

/// Precompute the SHA-256 hash for a static query string (APQ registration).
fn apq_hash(query: &str) -> String {
    sha256_hex(query)
}

const OP_SEARCH_TVDB: &str = "SearchTvdb";
const OP_SEARCH_TVDB_RICH: &str = "SearchTvdbRich";
const OP_SEARCH_TVDB_MULTI: &str = "SearchTvdbMulti";
const OP_GET_MOVIE: &str = "GetMovie";
const OP_GET_SERIES: &str = "GetSeries";
const OP_METADATA_BULK: &str = "MetadataBulk";
const OP_TITLE_RECOMMENDATIONS: &str = "TitleRecommendations";
const OP_DISCOVER_PUBLIC_FEED: &str = "DiscoverPublicFeed";
const OP_COLLECTION_COMPLETIONS: &str = "CollectionCompletions";
const OP_SUBMIT_DISCOVERY_CONTEXT_SNAPSHOT: &str = "SubmitDiscoveryContextSnapshot";
const OP_DISCOVERY_CONTEXT_SNAPSHOT_STATUS: &str = "DiscoveryContextSnapshotStatus";
const OP_DISCOVERY_CONTEXT_SNAPSHOT_PAGE: &str = "DiscoveryContextSnapshotPage";
const OP_DISCOVERY_CONTEXT_CHANGES: &str = "DiscoveryContextChanges";
const OP_ACKNOWLEDGE_DISCOVERY_CONTEXT_SNAPSHOT: &str = "AcknowledgeDiscoveryContextSnapshot";

/// Configuration for SMG enrollment and application-layer instance auth.
pub struct SmgEnrollmentConfig {
    pub registration_secret: Option<String>,
}

/// Signing materials for application-layer instance authentication.
#[derive(Clone)]
enum InstanceAuth {
    Pq {
        instance_id: Arc<String>,
        seed_b64: Arc<String>,
        key_id: Arc<String>,
        enrollment_generation: Option<i64>,
    },
}

/// Tracks the state of mTLS enrollment to prevent rapid-fire retries on failure.
enum MtlsState {
    /// Enrollment hasn't been attempted yet.
    NotAttempted,
    /// Enrollment succeeded; use this client and auth materials.
    Enrolled { client: Client, auth: InstanceAuth },
    /// Enrollment failed; don't retry until `retry_after`.
    Failed { retry_after: Instant, attempts: u32 },
}

/// SHA-256 hex digest of a byte slice (for request body hashing).
fn sha256_hex_bytes(data: &[u8]) -> String {
    let hash = digest::digest(&digest::SHA256, data);
    hash.as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

/// Attach PQ instance auth headers by signing the full request target.
async fn apply_instance_auth_headers(
    req: reqwest::RequestBuilder,
    auth: &InstanceAuth,
    method: &str,
    url: &reqwest::Url,
    body_bytes: &[u8],
) -> AppResult<reqwest::RequestBuilder> {
    apply_instance_auth_headers_with_nonce(req, auth, method, url, body_bytes, None).await
}

async fn apply_instance_auth_headers_with_nonce(
    req: reqwest::RequestBuilder,
    auth: &InstanceAuth,
    method: &str,
    url: &reqwest::Url,
    body_bytes: &[u8],
    nonce_override: Option<String>,
) -> AppResult<reqwest::RequestBuilder> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| AppError::Repository(format!("system clock before UNIX_EPOCH: {e}")))?
        .as_secs() as i64;
    match auth {
        InstanceAuth::Pq {
            seed_b64, key_id, ..
        } => {
            let body_hash = sha256_hex_bytes(body_bytes);
            let host = canonical_request_host(url)?;
            let path_and_query = canonical_request_path_and_query(url);
            let auth_version = smg_enrollment::configured_pq_auth_version();
            let nonce = Some(match nonce_override {
                Some(nonce) => nonce,
                None => smg_enrollment::generate_pq_auth_nonce().map_err(|e| {
                    AppError::Repository(format!("failed to generate PQ auth nonce: {e}"))
                })?,
            });
            let signature = smg_enrollment::sign_pq_request(
                seed_b64,
                auth_version,
                method,
                &host,
                &path_and_query,
                timestamp,
                nonce.as_deref(),
                &body_hash,
            )
            .await
            .map_err(|e| AppError::Repository(format!("failed to sign PQ request: {e}")))?;
            debug!(
                timestamp,
                key_id = %key_id,
                auth_version = auth_version.header_value(),
                has_nonce = nonce.is_some(),
                sig_len = signature.len(),
                body_hash,
                "attaching PQ X-Scryer-* instance auth headers"
            );
            let mut req = req
                .header("X-Scryer-Auth-Version", auth_version.header_value())
                .header("X-Scryer-Key-Id", &**key_id)
                .header("X-Scryer-Timestamp", timestamp.to_string())
                .header("X-Scryer-Signature", signature);
            if let Some(nonce) = nonce {
                req = req.header("X-Scryer-Nonce", nonce);
            }
            Ok(req)
        }
    }
}

fn canonical_request_host(url: &reqwest::Url) -> AppResult<String> {
    let host = url
        .host()
        .ok_or_else(|| AppError::Repository("metadata gateway URL missing host".into()))?;
    let host = match host {
        url::Host::Domain(domain) => domain.to_string(),
        url::Host::Ipv4(addr) => addr.to_string(),
        url::Host::Ipv6(addr) => format!("[{addr}]"),
    };
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

fn canonical_request_path_and_query(url: &reqwest::Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    }
}

/// Minimum interval between cert-rejection re-enrollment attempts.
const REENROLLMENT_COOLDOWN: Duration = Duration::from_secs(60);
const METADATA_GATEWAY_MAX_RETRIES: u32 = 3;
const METADATA_GATEWAY_RATE_LIMIT_BASE_DELAY: Duration = Duration::from_secs(2);
const METADATA_GATEWAY_RATE_LIMIT_MAX_DELAY: Duration = Duration::from_secs(30);
const METADATA_GATEWAY_TRANSIENT_BASE_DELAY: Duration = Duration::from_secs(1);
const METADATA_GATEWAY_TRANSIENT_MAX_DELAY: Duration = Duration::from_secs(5);
const METADATA_GATEWAY_MAX_SEARCH_BATCH: usize = 50;
const METADATA_GATEWAY_MAX_METADATA_BULK_BATCH: usize = 50;
const METADATA_GATEWAY_MAX_BULK_METADATA_ALIAS_BATCH: usize = 100;
const METADATA_GATEWAY_COMPATIBILITY_POLL_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const METADATA_GATEWAY_COMPATIBILITY_STARTUP_GUARD: Duration = Duration::from_secs(30 * 60);
const METADATA_GATEWAY_VERSION_COMPATIBILITY_PATH: &str = "/api/version-compatibility";
const SCRYER_RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize)]
struct VersionCompatibilitySuccessResponse {
    compatibility: Option<VersionCompatibilityDecisionPayload>,
    update: Option<VersionCompatibilityUpdatePayload>,
}

#[derive(Deserialize)]
struct VersionCompatibilityErrorResponse {
    compatibility: Option<VersionCompatibilityDecisionPayload>,
    update: Option<VersionCompatibilityUpdatePayload>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    minimum_version: Option<String>,
    #[serde(default)]
    your_version: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    upgrade_deadline: Option<String>,
}

#[derive(Deserialize)]
struct VersionCompatibilityDecisionPayload {
    status: String,
    #[serde(default)]
    minimum_version: String,
    #[serde(default)]
    your_version: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    upgrade_deadline: Option<String>,
}

impl VersionCompatibilityDecisionPayload {
    fn into_notice(self) -> Option<smg_enrollment::VersionIncompatible> {
        if self.status.eq_ignore_ascii_case("supported") {
            return None;
        }

        Some(smg_enrollment::VersionIncompatible {
            status: self.status,
            minimum_version: self.minimum_version,
            your_version: self.your_version,
            message: self.message,
            upgrade_deadline: self
                .upgrade_deadline
                .filter(|value| !value.trim().is_empty()),
        })
    }
}

#[derive(Deserialize)]
struct VersionCompatibilityUpdatePayload {
    #[serde(default)]
    available: bool,
    #[serde(default)]
    current_version: String,
    #[serde(default)]
    latest_version: String,
    #[serde(default)]
    latest_tag: String,
    #[serde(default)]
    release_url: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    checked_at: String,
}

impl VersionCompatibilityUpdatePayload {
    fn into_notice(self) -> Option<SmgScryerUpdateNotice> {
        if !self.available || self.latest_version.trim().is_empty() {
            return None;
        }

        Some(SmgScryerUpdateNotice {
            available: true,
            current_version: self.current_version,
            latest_version: self.latest_version,
            latest_tag: self.latest_tag,
            release_url: self.release_url.filter(|value| !value.trim().is_empty()),
            published_at: self.published_at.filter(|value| !value.trim().is_empty()),
            checked_at: self.checked_at,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct VersionCompatibilityCheckResult {
    compatibility_notice: Option<smg_enrollment::VersionIncompatible>,
    update_notice: Option<SmgScryerUpdateNotice>,
}

fn parse_version_compatibility_success(body: &[u8]) -> AppResult<VersionCompatibilityCheckResult> {
    let parsed: VersionCompatibilitySuccessResponse =
        serde_json::from_slice(body).map_err(|error| {
            AppError::Repository(format!(
                "failed to decode SMG version compatibility response: {error}"
            ))
        })?;
    Ok(VersionCompatibilityCheckResult {
        compatibility_notice: parsed
            .compatibility
            .and_then(VersionCompatibilityDecisionPayload::into_notice),
        update_notice: parsed
            .update
            .and_then(VersionCompatibilityUpdatePayload::into_notice),
    })
}

fn parse_version_compatibility_incompatible(
    body: &[u8],
) -> AppResult<VersionCompatibilityCheckResult> {
    let parsed: VersionCompatibilityErrorResponse =
        serde_json::from_slice(body).map_err(|error| {
            AppError::Repository(format!(
                "failed to decode SMG version compatibility error response: {error}"
            ))
        })?;
    let compatibility =
        parsed
            .compatibility
            .unwrap_or_else(|| VersionCompatibilityDecisionPayload {
                status: parsed.status.unwrap_or_else(|| "blocked".to_string()),
                minimum_version: parsed
                    .minimum_version
                    .unwrap_or_else(|| "unknown".to_string()),
                your_version: parsed.your_version.unwrap_or_else(|| "unknown".to_string()),
                message: parsed.message.unwrap_or_default(),
                upgrade_deadline: parsed.upgrade_deadline,
            });
    let compatibility_notice = compatibility.into_notice().ok_or_else(|| {
        AppError::Repository("SMG version compatibility error did not include a notice".to_string())
    })?;

    Ok(VersionCompatibilityCheckResult {
        compatibility_notice: Some(compatibility_notice),
        update_notice: parsed
            .update
            .and_then(VersionCompatibilityUpdatePayload::into_notice),
    })
}

fn is_version_incompatible_response(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|parsed| {
            parsed
                .get("error")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .is_some_and(|error| error == "version_incompatible")
}

fn compatibility_poll_phase(instance_id: &str) -> Duration {
    let digest = digest::digest(&digest::SHA256, instance_id.as_bytes());
    let mut raw = [0_u8; 8];
    raw.copy_from_slice(&digest.as_ref()[..8]);
    let offset_secs =
        u64::from_be_bytes(raw) % METADATA_GATEWAY_COMPATIBILITY_POLL_INTERVAL.as_secs();
    Duration::from_secs(offset_secs)
}

fn next_version_compatibility_poll_delay_at(
    now: SystemTime,
    phase: Duration,
    minimum_delay: Duration,
) -> Duration {
    let now_secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let interval_secs = METADATA_GATEWAY_COMPATIBILITY_POLL_INTERVAL.as_secs();
    let phase_secs = phase.as_secs() % interval_secs;

    let mut next_slot = (now_secs / interval_secs) * interval_secs + phase_secs;
    if next_slot <= now_secs {
        next_slot = next_slot.saturating_add(interval_secs);
    }

    let earliest = now_secs.saturating_add(minimum_delay.as_secs());
    if next_slot < earliest {
        let delta = earliest - next_slot;
        let skips = delta.div_ceil(interval_secs);
        next_slot = next_slot.saturating_add(skips * interval_secs);
    }

    Duration::from_secs(next_slot.saturating_sub(now_secs))
}

pub struct MetadataGatewayClient {
    http: Client,
    outbound_http: OutboundHttpClient,
    endpoint: String,
    registration_url: String,
    enrollment_config: SmgEnrollmentConfig,
    enrollment_store: Option<Arc<dyn SettingsRepository>>,
    mtls_state: tokio::sync::RwLock<MtlsState>,
    last_reenrollment: tokio::sync::Mutex<Option<Instant>>,
    pq_rotation: tokio::sync::Mutex<()>,
    compatibility_refresh: tokio::sync::Mutex<()>,
    version_incompatible: tokio::sync::Mutex<Option<smg_enrollment::VersionIncompatible>>,
    search_hash: String,
    search_rich_hash: String,
    search_multi_hash: String,
    movie_hash: String,
    series_hash: String,
    title_recommendations_hash: String,
    collection_completions_hash: String,
    submit_discovery_context_snapshot_hash: String,
    discovery_context_snapshot_status_hash: String,
    discovery_context_snapshot_page_hash: String,
    discovery_context_changes_hash: String,
    acknowledge_discovery_context_snapshot_hash: String,
    apq_cache: RwLock<ApqCache>,
}

impl MetadataGatewayClient {
    pub fn new(
        endpoint: String,
        db: crate::SqliteServices,
        enrollment_config: SmgEnrollmentConfig,
    ) -> Self {
        Self::new_with_enrollment_store(
            endpoint,
            Arc::new(crate::SettingsStore::new(
                db.datastore(),
                db.encryption_key_state(),
            )),
            enrollment_config,
        )
    }

    pub fn new_with_enrollment_store(
        endpoint: String,
        enrollment_store: Arc<dyn SettingsRepository>,
        enrollment_config: SmgEnrollmentConfig,
    ) -> Self {
        let search_hash = apq_hash(graphql_docs::SEARCH_TVDB_QUERY);
        let search_rich_hash = apq_hash(graphql_docs::SEARCH_TVDB_RICH_QUERY);
        let search_multi_hash = apq_hash(graphql_docs::SEARCH_TVDB_MULTI_QUERY);
        let movie_hash = apq_hash(graphql_docs::GET_MOVIE_QUERY);
        let series_hash = apq_hash(graphql_docs::GET_SERIES_QUERY);
        let title_recommendations_hash = apq_hash(graphql_docs::TITLE_RECOMMENDATIONS_QUERY);
        let collection_completions_hash = apq_hash(graphql_docs::COLLECTION_COMPLETIONS_QUERY);
        let submit_discovery_context_snapshot_hash =
            apq_hash(graphql_docs::SUBMIT_DISCOVERY_CONTEXT_SNAPSHOT_QUERY);
        let discovery_context_snapshot_status_hash =
            apq_hash(graphql_docs::DISCOVERY_CONTEXT_SNAPSHOT_STATUS_QUERY);
        let discovery_context_snapshot_page_hash =
            apq_hash(graphql_docs::DISCOVERY_CONTEXT_SNAPSHOT_PAGE_QUERY);
        let discovery_context_changes_hash =
            apq_hash(graphql_docs::DISCOVERY_CONTEXT_CHANGES_QUERY);
        let acknowledge_discovery_context_snapshot_hash =
            apq_hash(graphql_docs::ACKNOWLEDGE_DISCOVERY_CONTEXT_SNAPSHOT_QUERY);

        // Derive registration URL from GraphQL endpoint
        let registration_url = if endpoint.ends_with("/graphql") {
            format!(
                "{}/api/register",
                &endpoint[..endpoint.len() - "/graphql".len()]
            )
        } else {
            format!("{}/api/register", endpoint.trim_end_matches('/'))
        };

        debug!(
            endpoint = %endpoint,
            has_registration_secret = enrollment_config.registration_secret.is_some(),
            %search_hash,
            %search_rich_hash,
            %search_multi_hash,
            %movie_hash,
            %series_hash,
            %title_recommendations_hash,
            %collection_completions_hash,
            %submit_discovery_context_snapshot_hash,
            %discovery_context_snapshot_status_hash,
            %discovery_context_snapshot_page_hash,
            %discovery_context_changes_hash,
            %acknowledge_discovery_context_snapshot_hash,
            "metadata gateway client initialized (APQ enabled)"
        );

        let http = smg_reqwest_client();

        Self {
            outbound_http: OutboundHttpClient::new(http.clone(), RateLimitRegistry::new()),
            http,
            endpoint,
            registration_url,
            enrollment_config,
            last_reenrollment: tokio::sync::Mutex::new(None),
            pq_rotation: tokio::sync::Mutex::new(()),
            compatibility_refresh: tokio::sync::Mutex::new(()),
            version_incompatible: tokio::sync::Mutex::new(None),
            enrollment_store: Some(enrollment_store),
            mtls_state: tokio::sync::RwLock::new(MtlsState::NotAttempted),
            search_hash,
            search_rich_hash,
            search_multi_hash,
            movie_hash,
            series_hash,
            title_recommendations_hash,
            collection_completions_hash,
            submit_discovery_context_snapshot_hash,
            discovery_context_snapshot_status_hash,
            discovery_context_snapshot_page_hash,
            discovery_context_changes_hash,
            acknowledge_discovery_context_snapshot_hash,
            apq_cache: RwLock::new(ApqCache::new()),
        }
    }

    pub fn new_without_enrollment_store(
        endpoint: String,
        enrollment_config: SmgEnrollmentConfig,
    ) -> Self {
        if enrollment_config.registration_secret.is_some() {
            warn!(
                "SMG enrollment is not available for this datastore engine in the PostgreSQL blank-install slice"
            );
        }

        let search_hash = apq_hash(graphql_docs::SEARCH_TVDB_QUERY);
        let search_rich_hash = apq_hash(graphql_docs::SEARCH_TVDB_RICH_QUERY);
        let search_multi_hash = apq_hash(graphql_docs::SEARCH_TVDB_MULTI_QUERY);
        let movie_hash = apq_hash(graphql_docs::GET_MOVIE_QUERY);
        let series_hash = apq_hash(graphql_docs::GET_SERIES_QUERY);
        let title_recommendations_hash = apq_hash(graphql_docs::TITLE_RECOMMENDATIONS_QUERY);
        let collection_completions_hash = apq_hash(graphql_docs::COLLECTION_COMPLETIONS_QUERY);
        let submit_discovery_context_snapshot_hash =
            apq_hash(graphql_docs::SUBMIT_DISCOVERY_CONTEXT_SNAPSHOT_QUERY);
        let discovery_context_snapshot_status_hash =
            apq_hash(graphql_docs::DISCOVERY_CONTEXT_SNAPSHOT_STATUS_QUERY);
        let discovery_context_snapshot_page_hash =
            apq_hash(graphql_docs::DISCOVERY_CONTEXT_SNAPSHOT_PAGE_QUERY);
        let discovery_context_changes_hash =
            apq_hash(graphql_docs::DISCOVERY_CONTEXT_CHANGES_QUERY);
        let acknowledge_discovery_context_snapshot_hash =
            apq_hash(graphql_docs::ACKNOWLEDGE_DISCOVERY_CONTEXT_SNAPSHOT_QUERY);
        let registration_url = if endpoint.ends_with("/graphql") {
            format!(
                "{}/api/register",
                &endpoint[..endpoint.len() - "/graphql".len()]
            )
        } else {
            format!("{}/api/register", endpoint.trim_end_matches('/'))
        };
        let http = smg_reqwest_client();

        Self {
            outbound_http: OutboundHttpClient::new(http.clone(), RateLimitRegistry::new()),
            http,
            endpoint,
            registration_url,
            enrollment_config,
            last_reenrollment: tokio::sync::Mutex::new(None),
            pq_rotation: tokio::sync::Mutex::new(()),
            compatibility_refresh: tokio::sync::Mutex::new(()),
            version_incompatible: tokio::sync::Mutex::new(None),
            enrollment_store: None,
            mtls_state: tokio::sync::RwLock::new(MtlsState::NotAttempted),
            search_hash,
            search_rich_hash,
            search_multi_hash,
            movie_hash,
            series_hash,
            title_recommendations_hash,
            collection_completions_hash,
            submit_discovery_context_snapshot_hash,
            discovery_context_snapshot_status_hash,
            discovery_context_snapshot_page_hash,
            discovery_context_changes_hash,
            acknowledge_discovery_context_snapshot_hash,
            apq_cache: RwLock::new(ApqCache::new()),
        }
    }

    /// Get the mTLS HTTP client and optional signing materials, enrolling lazily on first call.
    ///
    /// If no registration secret is configured, returns the plain HTTP client with no auth.
    /// If enrollment fails, returns an error with exponential backoff on retries.
    async fn get_http_client(&self) -> AppResult<(Client, Option<InstanceAuth>)> {
        let secret = match &self.enrollment_config.registration_secret {
            Some(s) => s,
            None => return Ok((self.http.clone(), None)),
        };

        // Fast path: check current state under read lock
        {
            let guard = self.mtls_state.read().await;
            match &*guard {
                MtlsState::Enrolled { client, auth } => {
                    return Ok((client.clone(), Some(auth.clone())));
                }
                MtlsState::Failed { retry_after, .. } if Instant::now() < *retry_after => {
                    return Err(AppError::Repository(
                        "SMG instance auth enrollment pending retry (backoff)".into(),
                    ));
                }
                _ => {}
            }
        }

        // Slow path: need to attempt enrollment
        let mut guard = self.mtls_state.write().await;
        // Double-check after acquiring write lock
        match &*guard {
            MtlsState::Enrolled { client, auth } => {
                return Ok((client.clone(), Some(auth.clone())));
            }
            MtlsState::Failed { retry_after, .. } if Instant::now() < *retry_after => {
                return Err(AppError::Repository(
                    "SMG instance auth enrollment pending retry (backoff)".into(),
                ));
            }
            _ => {}
        }

        let attempts = match &*guard {
            MtlsState::Failed { attempts, .. } => *attempts,
            _ => 0,
        };

        match self.try_build_mtls_client(secret).await {
            Ok((client, auth)) => {
                info!(
                    "SMG instance auth enrollment successful, using instance authentication for metadata requests"
                );
                let result = (client.clone(), Some(auth.clone()));
                *guard = MtlsState::Enrolled { client, auth };
                Ok(result)
            }
            Err(e) => {
                let next_attempts = attempts + 1;
                let retry_after = enrollment_retry_delay(&e, attempts);
                warn!(
                    error = %e,
                    attempt = next_attempts,
                    retry_in_secs = retry_after.as_secs(),
                    "SMG instance auth enrollment failed"
                );
                *guard = MtlsState::Failed {
                    retry_after: Instant::now() + retry_after,
                    attempts: next_attempts,
                };
                Err(AppError::Repository(format!(
                    "SMG instance auth enrollment failed: {e}"
                )))
            }
        }
    }

    async fn try_build_mtls_client(
        &self,
        registration_secret: &str,
    ) -> Result<(Client, InstanceAuth), smg_enrollment::EnrollmentError> {
        let db = self.enrollment_store.as_ref().ok_or_else(|| {
            smg_enrollment::EnrollmentError::Other(
                "SMG enrollment persistence is not implemented for this datastore engine"
                    .to_string(),
            )
        })?;
        let state = match smg_enrollment::ensure_enrolled(
            &**db,
            &self.registration_url,
            registration_secret,
        )
        .await
        {
            Ok(state) => state,
            Err(error) => {
                if let smg_enrollment::EnrollmentError::VersionIncompatible(ref incompatibility) =
                    error
                {
                    self.remember_enrollment_version_incompatible(incompatibility)
                        .await;
                }
                return Err(error);
            }
        };

        if let (Some(seed_b64), Some(key_id)) =
            (state.pq_seed_b64.as_ref(), state.pq_key_id.as_ref())
        {
            return Ok((
                self.http.clone(),
                InstanceAuth::Pq {
                    instance_id: Arc::new(state.instance_id.clone()),
                    seed_b64: Arc::new(seed_b64.clone()),
                    key_id: Arc::new(key_id.clone()),
                    enrollment_generation: state.pq_enrollment_generation,
                },
            ));
        }
        Err(smg_enrollment::EnrollmentError::Other(
            "SMG enrollment completed without PQ request-signing state".to_string(),
        ))
    }

    /// Invalidate cached enrollment after a cert rejection (401) from SMG.
    /// Clears SQLite cache and resets state so the next request triggers fresh enrollment.
    /// Returns `true` if invalidation happened, `false` if still within cooldown.
    async fn invalidate_enrollment(&self) -> bool {
        let mut last = self.last_reenrollment.lock().await;
        if let Some(prev) = *last
            && prev.elapsed() < REENROLLMENT_COOLDOWN
        {
            debug!(
                cooldown_remaining_secs = (REENROLLMENT_COOLDOWN - prev.elapsed()).as_secs(),
                "skipping re-enrollment (cooldown active)"
            );
            return false;
        }
        *last = Some(Instant::now());
        drop(last);

        warn!("SMG rejected instance auth — clearing cached enrollment for re-registration");
        if let Some(db) = self.enrollment_store.as_ref() {
            if let Err(e) = smg_enrollment::clear_enrollment_cache(&**db).await {
                warn!(error = %e, "failed to clear enrollment cache from SQLite");
            }
        } else {
            warn!(
                "SMG enrollment cache clear skipped because this datastore engine has no enrollment store"
            );
        }
        let mut guard = self.mtls_state.write().await;
        *guard = MtlsState::NotAttempted;
        true
    }

    async fn store_version_compatibility_state(
        &self,
        notice: Option<smg_enrollment::VersionIncompatible>,
        update_notice: Option<SmgScryerUpdateNotice>,
    ) -> AppResult<()> {
        let db = self.enrollment_store.as_ref().ok_or_else(|| {
            AppError::Repository(
                "SMG compatibility notice persistence is not implemented for this datastore engine"
                    .to_string(),
            )
        })?;
        smg_enrollment::persist_version_compatibility_notice(&**db, notice.as_ref())
            .await
            .map_err(AppError::Repository)?;
        smg_enrollment::persist_scryer_update_notice(&**db, update_notice.as_ref())
            .await
            .map_err(AppError::Repository)?;
        *self.version_incompatible.lock().await = notice;
        Ok(())
    }

    async fn remember_enrollment_version_incompatible(
        &self,
        incompatibility: &smg_enrollment::VersionIncompatible,
    ) {
        if let Some(db) = self.enrollment_store.as_ref()
            && let Err(error) =
                smg_enrollment::persist_version_compatibility_notice(&**db, Some(incompatibility))
                    .await
        {
            warn!(
                error = %error,
                "failed to persist SMG version compatibility notice from enrollment"
            );
        }
        if let Ok(mut guard) = self.version_incompatible.try_lock() {
            *guard = Some(incompatibility.clone());
        }
    }

    /// Eagerly trigger enrollment in a background task so the mTLS client is ready before
    /// the first real metadata query arrives. Call this once after construction; it is
    /// safe to call concurrently with any other method.
    pub async fn warm_enrollment(&self) -> Option<smg_enrollment::VersionIncompatible> {
        let _ = self.get_http_client().await;
        if self.compatibility_polling_enabled()
            && let Err(error) = self.refresh_version_compatibility(false).await
        {
            warn!(error = %error, "SMG version compatibility warmup failed");
        }
        self.version_incompatible.lock().await.clone()
    }

    pub fn compatibility_polling_enabled(&self) -> bool {
        self.enrollment_config.registration_secret.is_some()
    }

    pub async fn version_compatibility_poll_phase(&self) -> AppResult<Duration> {
        let db = self.enrollment_store.as_ref().ok_or_else(|| {
            AppError::Repository(
                "SMG compatibility polling is not implemented for this datastore engine"
                    .to_string(),
            )
        })?;
        let instance_id = smg_enrollment::ensure_instance_id(&**db)
            .await
            .map_err(AppError::Repository)?;
        Ok(compatibility_poll_phase(&instance_id))
    }

    pub fn next_version_compatibility_poll_delay(
        phase: Duration,
        minimum_delay: Duration,
    ) -> Duration {
        next_version_compatibility_poll_delay_at(SystemTime::now(), phase, minimum_delay)
    }

    pub fn version_compatibility_startup_guard() -> Duration {
        METADATA_GATEWAY_COMPATIBILITY_STARTUP_GUARD
    }

    pub async fn refresh_version_compatibility(
        &self,
        skip_if_busy: bool,
    ) -> AppResult<Option<smg_enrollment::VersionIncompatible>> {
        let _guard = if skip_if_busy {
            match self.compatibility_refresh.try_lock() {
                Ok(guard) => guard,
                Err(_) => return Ok(None),
            }
        } else {
            self.compatibility_refresh.lock().await
        };

        if !self.compatibility_polling_enabled() {
            return Ok(None);
        }

        let url = smg_enrollment::derive_registration_endpoint(
            &self.registration_url,
            METADATA_GATEWAY_VERSION_COMPATIBILITY_PATH,
        )
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let payload = json!({ "version": SCRYER_RUNTIME_VERSION });
        let body_bytes = serde_json::to_vec(&payload).map_err(|error| {
            AppError::Repository(format!("failed to serialize payload: {error}"))
        })?;
        let endpoint_url = reqwest::Url::parse(&url)
            .map_err(|error| AppError::Repository(format!("invalid compatibility URL: {error}")))?;

        let mut retried_after_reenrollment = false;
        loop {
            let (client, auth) = self.get_http_client().await?;
            let build_req = || {
                let client = client.clone();
                let auth = auth.clone();
                let url = url.clone();
                let body_bytes = body_bytes.clone();
                let endpoint_url = endpoint_url.clone();
                async move {
                    let mut req = client
                        .post(url)
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(body_bytes.clone());
                    if let Some(ref auth) = auth {
                        req = apply_instance_auth_headers(
                            req,
                            auth,
                            reqwest::Method::POST.as_str(),
                            &endpoint_url,
                            &body_bytes,
                        )
                        .await?;
                    }
                    Ok(req)
                }
            };

            let response = self
                .send_request_with_retry(build_req, "SMG version compatibility check")
                .await?;
            self.reconcile_pq_enrollment_generation(auth.as_ref(), response.headers())
                .await;

            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED
                && !retried_after_reenrollment
                && self.enrollment_config.registration_secret.is_some()
            {
                let preview = read_response_body_preview(
                    response,
                    "SMG version compatibility response read failed",
                )
                .await?;
                retried_after_reenrollment = true;
                if !self.invalidate_enrollment().await {
                    warn!(
                        status = %status,
                        body_preview = %preview.escaped_text(),
                        body_preview_bytes = preview.preview_bytes,
                        content_length = ?preview.content_length,
                        content_type = ?preview.content_type,
                        body_truncated = preview.truncated,
                        "SMG version compatibility auth rejected during re-enrollment cooldown"
                    );
                    return Err(AppError::Repository(format!(
                        "SMG version compatibility check auth rejected ({status}), re-enrollment on cooldown"
                    )));
                }
                info!("retrying SMG version compatibility check after re-enrollment");
                continue;
            }

            if status.is_success() {
                let body = response.bytes().await.map_err(|error| {
                    AppError::Repository(format!(
                        "SMG version compatibility response read failed: {error}"
                    ))
                })?;
                let check = parse_version_compatibility_success(&body)?;
                self.store_version_compatibility_state(
                    check.compatibility_notice.clone(),
                    check.update_notice,
                )
                .await?;
                return Ok(check.compatibility_notice);
            }

            if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
                let preview = read_response_body_preview(
                    response,
                    "SMG version compatibility response read failed",
                )
                .await?;
                if preview.truncated || !is_version_incompatible_response(preview.text.as_bytes()) {
                    warn!(
                        status = %status,
                        body_preview = %preview.escaped_text(),
                        body_preview_bytes = preview.preview_bytes,
                        content_length = ?preview.content_length,
                        content_type = ?preview.content_type,
                        body_truncated = preview.truncated,
                        "SMG version compatibility check returned unexpected response"
                    );
                    return Err(AppError::Repository(format!(
                        "SMG version compatibility check failed (HTTP {status})"
                    )));
                }
                let check = parse_version_compatibility_incompatible(preview.text.as_bytes())?;
                self.store_version_compatibility_state(
                    check.compatibility_notice.clone(),
                    check.update_notice,
                )
                .await?;
                return Ok(check.compatibility_notice);
            }

            let error = smg_enrollment::registration_response_error(
                response,
                "SMG version compatibility check",
            )
            .await;
            match error {
                smg_enrollment::EnrollmentError::VersionIncompatible(incompatibility) => {
                    self.store_version_compatibility_state(Some(incompatibility.clone()), None)
                        .await?;
                    return Ok(Some(incompatibility));
                }
                smg_enrollment::EnrollmentError::RateLimited(rate_limited) => {
                    return Err(AppError::Repository(format!(
                        "SMG version compatibility check rate limited: {}",
                        rate_limited.message
                    )));
                }
                smg_enrollment::EnrollmentError::Other(message) => {
                    return Err(AppError::Repository(message));
                }
            }
        }
    }

    /// Execute a GraphQL query using APQ (Automatic Persisted Queries).
    ///
    /// 1. Send GET with hash only (no query body) — cache-friendly.
    ///    Sends `If-None-Match` if we have a cached ETag; on 304 returns cached body.
    /// 2. If the server returns `PersistedQueryNotFound`, POST with full query + hash to register.
    /// 3. Subsequent GETs for the same hash will hit Cloudflare edge cache.
    async fn execute_graphql_apq<T: serde::de::DeserializeOwned>(
        &self,
        operation_name: &'static str,
        query: &str,
        hash: &str,
        variables: serde_json::Value,
    ) -> AppResult<T> {
        let extensions = json!({
            "persistedQuery": {
                "version": 1,
                "sha256Hash": hash
            }
        });

        let variables_str = serde_json::to_string(&variables)
            .map_err(|e| AppError::Repository(format!("failed to serialize variables: {e}")))?;
        let extensions_str = serde_json::to_string(&extensions)
            .map_err(|e| AppError::Repository(format!("failed to serialize extensions: {e}")))?;

        let cache_key = apq_cache_key(operation_name, hash, &variables_str);

        // Check for a cached ETag to send If-None-Match
        let cached_etag = self
            .apq_cache
            .read()
            .unwrap()
            .get(&cache_key)
            .map(|e| e.etag.clone());

        debug!(endpoint = %self.endpoint, operation_name, hash, has_etag = cached_etag.is_some(), "APQ GET request");

        let (client, auth) = self.get_http_client().await?;

        // Build URL with query params so the GET request target is complete before signing.
        let mut url = reqwest::Url::parse(&self.endpoint)
            .map_err(|e| AppError::Repository(format!("invalid endpoint URL: {e}")))?;
        url.query_pairs_mut()
            .append_pair("operationName", operation_name)
            .append_pair("extensions", &extensions_str)
            .append_pair("variables", &variables_str);

        let get_result = self
            .send_request_with_retry(
                || {
                    let client = client.clone();
                    let cached_etag = cached_etag.clone();
                    let auth = auth.clone();
                    let url = url.clone();
                    async move {
                        let mut req = client.get(url.clone());
                        if let Some(ref etag) = cached_etag {
                            req = req.header(reqwest::header::IF_NONE_MATCH, etag);
                        }
                        if let Some(ref auth) = auth {
                            req = apply_instance_auth_headers(
                                req,
                                auth,
                                reqwest::Method::GET.as_str(),
                                &url,
                                &[],
                            )
                            .await?;
                        }
                        Ok(req)
                    }
                },
                "metadata gateway APQ GET",
            )
            .await;

        match get_result {
            Ok(resp) if resp.status() == reqwest::StatusCode::NOT_MODIFIED => {
                self.reconcile_pq_enrollment_generation(auth.as_ref(), resp.headers())
                    .await;
                // 304: serve from our local cache
                let body = self
                    .apq_cache
                    .read()
                    .unwrap()
                    .get(&cache_key)
                    .map(|e| e.body.clone())
                    .ok_or_else(|| AppError::Repository("APQ 304 but no cached body".into()))?;
                debug!(hash, "APQ 304 — serving from ETag cache");
                let parsed: GraphqlResponse<T> = serde_json::from_str(&body)
                    .map_err(|e| AppError::Repository(format!("APQ cache: invalid JSON: {e}")))?;
                parsed
                    .data
                    .ok_or_else(|| AppError::Repository("APQ cache: empty data".into()))
            }
            Ok(resp) if resp.status().is_success() => {
                self.reconcile_pq_enrollment_generation(auth.as_ref(), resp.headers())
                    .await;
                let etag = resp
                    .headers()
                    .get(reqwest::header::ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                let raw = resp
                    .text()
                    .await
                    .map_err(|e| AppError::Repository(e.to_string()))?;

                let parsed: GraphqlResponse<T> = serde_json::from_str(&raw)
                    .map_err(|e| AppError::Repository(format!("APQ GET: invalid JSON: {e}")))?;

                // Check for PersistedQueryNotFound before caching
                if let Some(ref errors) = parsed.errors {
                    let is_not_found = errors
                        .iter()
                        .any(|e| e.message.contains("PersistedQueryNotFound"));
                    if is_not_found {
                        debug!(hash, "APQ cache miss, registering via POST");
                        return self
                            .execute_graphql_apq_register(
                                operation_name,
                                query,
                                &extensions,
                                &variables,
                            )
                            .await;
                    }
                    let msg = errors
                        .first()
                        .map(|e| e.message.as_str())
                        .unwrap_or("metadata gateway returned errors");
                    return Err(AppError::Repository(msg.to_string()));
                }

                // Store ETag + body for future conditional requests (evicts oldest beyond 1000)
                if let Some(etag) = etag {
                    self.apq_cache
                        .write()
                        .unwrap()
                        .insert(cache_key, ApqCacheEntry { etag, body: raw });
                }

                parsed
                    .data
                    .ok_or_else(|| AppError::Repository("APQ GET: empty data".into()))
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED => {
                // Cert rejection — invalidate before falling through to POST retry
                // (execute_graphql will handle the actual re-enrollment + retry)
                self.invalidate_enrollment().await;
                self.execute_graphql_apq_register(operation_name, query, &extensions, &variables)
                    .await
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED => {
                let preview =
                    read_response_body_preview(resp, "APQ GET response read failed").await?;
                debug!(
                    hash,
                    body_preview = %preview.escaped_text(),
                    "APQ GET not allowed, retrying via POST"
                );
                self.execute_graphql_apq_register(operation_name, query, &extensions, &variables)
                    .await
            }
            Ok(resp) => {
                let status = resp.status();
                let preview =
                    read_response_body_preview(resp, "APQ GET response read failed").await?;
                warn!(
                    status = %status,
                    hash,
                    body_preview = %preview.escaped_text(),
                    body_preview_bytes = preview.preview_bytes,
                    content_length = ?preview.content_length,
                    content_type = ?preview.content_type,
                    body_truncated = preview.truncated,
                    "APQ GET failed"
                );
                Err(AppError::Repository(format!(
                    "metadata gateway request failed ({status}): {}",
                    preview.escaped_text()
                )))
            }
            Err(error) => {
                debug!(error = %error, hash, "APQ GET request failed");
                Err(error)
            }
        }
    }

    /// POST with full query + extensions to register the hash, then return the result.
    async fn execute_graphql_apq_register<T: serde::de::DeserializeOwned>(
        &self,
        operation_name: &'static str,
        query: &str,
        extensions: &serde_json::Value,
        variables: &serde_json::Value,
    ) -> AppResult<T> {
        let payload = json!({
            "operationName": operation_name,
            "query": query,
            "variables": variables,
            "extensions": extensions,
        });

        self.execute_graphql(payload).await
    }

    async fn execute_graphql_apq_post<T: serde::de::DeserializeOwned>(
        &self,
        operation_name: &'static str,
        query: &str,
        hash: &str,
        variables: serde_json::Value,
    ) -> AppResult<T> {
        let extensions = json!({
            "persistedQuery": {
                "version": 1,
                "sha256Hash": hash
            }
        });

        self.execute_graphql_apq_register(operation_name, query, &extensions, &variables)
            .await
    }

    async fn execute_public_graphql_get<T: serde::de::DeserializeOwned>(
        &self,
        operation_name: &'static str,
        query: &str,
        variables: serde_json::Value,
    ) -> AppResult<T> {
        let variables_str = serde_json::to_string(&variables)
            .map_err(|e| AppError::Repository(format!("failed to serialize variables: {e}")))?;
        let mut url = reqwest::Url::parse(&self.endpoint)
            .map_err(|e| AppError::Repository(format!("invalid endpoint URL: {e}")))?;
        url.query_pairs_mut()
            .append_pair("operationName", operation_name)
            .append_pair("query", query)
            .append_pair("variables", &variables_str);

        debug!(endpoint = %self.endpoint, operation_name, "sending public metadata gateway GET");

        let client = self.http.clone();
        let response = self
            .send_request_with_retry(
                || {
                    let client = client.clone();
                    let url = url.clone();
                    async move { Ok(client.get(url)) }
                },
                "metadata gateway public GraphQL GET",
            )
            .await?;

        let status = response.status();
        if !status.is_success() {
            let preview = read_response_body_preview(
                response,
                "metadata gateway public response read failed",
            )
            .await?;
            warn!(
                status = %status,
                body_preview = %preview.escaped_text(),
                body_preview_bytes = preview.preview_bytes,
                content_length = ?preview.content_length,
                content_type = ?preview.content_type,
                body_truncated = preview.truncated,
                "metadata gateway public request failed"
            );
            return Err(AppError::Repository(format!(
                "metadata gateway public request failed ({status})"
            )));
        }

        let raw_text = response
            .text()
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        debug!(status = %status, body_len = raw_text.len(), "metadata gateway public response");

        self.parse_graphql_response(&raw_text)
    }

    async fn execute_graphql<T: serde::de::DeserializeOwned>(
        &self,
        payload: serde_json::Value,
    ) -> AppResult<T> {
        debug!(endpoint = %self.endpoint, "sending metadata gateway request");
        let response = self.send_with_retry(&payload).await?;

        let status = response.status();

        // On instance-auth rejection, invalidate enrollment and retry once with fresh creds.
        if status == reqwest::StatusCode::UNAUTHORIZED
            && self.enrollment_config.registration_secret.is_some()
        {
            let preview = read_response_body_preview(
                response,
                "metadata gateway auth rejection response read failed",
            )
            .await?;
            if !self.invalidate_enrollment().await {
                warn!(
                    status = %status,
                    body_preview = %preview.escaped_text(),
                    body_preview_bytes = preview.preview_bytes,
                    content_length = ?preview.content_length,
                    content_type = ?preview.content_type,
                    body_truncated = preview.truncated,
                    "metadata gateway instance auth rejected during re-enrollment cooldown"
                );
                return Err(AppError::Repository(format!(
                    "metadata gateway instance auth rejected ({status}), re-enrollment on cooldown"
                )));
            }
            info!("retrying metadata request after re-enrollment");
            let retry_resp = self.send_with_retry(&payload).await?;
            let retry_status = retry_resp.status();
            if !retry_status.is_success() {
                let preview = read_response_body_preview(
                    retry_resp,
                    "metadata gateway retry response read failed",
                )
                .await?;
                warn!(
                    status = %retry_status,
                    body_preview = %preview.escaped_text(),
                    body_preview_bytes = preview.preview_bytes,
                    content_length = ?preview.content_length,
                    content_type = ?preview.content_type,
                    body_truncated = preview.truncated,
                    "metadata gateway request failed after re-enrollment"
                );
                return Err(AppError::Repository(format!(
                    "metadata gateway request failed ({retry_status}): {}",
                    preview.escaped_text()
                )));
            }
            let retry_text = retry_resp
                .text()
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;
            return self.parse_graphql_response(&retry_text);
        }

        if !status.is_success() {
            let preview =
                read_response_body_preview(response, "metadata gateway response read failed")
                    .await?;
            warn!(
                status = %status,
                body_preview = %preview.escaped_text(),
                body_preview_bytes = preview.preview_bytes,
                content_length = ?preview.content_length,
                content_type = ?preview.content_type,
                body_truncated = preview.truncated,
                "metadata gateway request failed"
            );
            return Err(AppError::Repository(format!(
                "metadata gateway request failed ({status}): {}",
                preview.escaped_text()
            )));
        }

        let raw_text = response
            .text()
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        debug!(status = %status, body_len = raw_text.len(), "metadata gateway response");

        self.parse_graphql_response(&raw_text)
    }

    fn parse_graphql_response<T: serde::de::DeserializeOwned>(
        &self,
        raw_text: &str,
    ) -> AppResult<T> {
        let parsed: GraphqlResponse<T> = serde_json::from_str(raw_text).map_err(|err| {
            let preview = ResponseBodyPreview::from_text(raw_text);
            warn!(
                body_preview = %preview.escaped_text(),
                body_preview_bytes = preview.preview_bytes,
                content_length = ?preview.content_length,
                content_type = ?preview.content_type,
                body_truncated = preview.truncated,
                error = %err,
                "metadata gateway returned invalid JSON"
            );
            AppError::Repository(format!("metadata gateway returned invalid JSON: {err}"))
        })?;

        if let Some(errors) = parsed.errors {
            let message = errors
                .first()
                .map(|error| error.message.as_str())
                .unwrap_or("metadata gateway returned errors");
            warn!(error = %message, "metadata gateway returned GraphQL errors");
            return Err(AppError::Repository(message.to_string()));
        }

        if parsed.data.is_none() {
            let preview = ResponseBodyPreview::from_text(raw_text);
            warn!(
                body_preview = %preview.escaped_text(),
                body_preview_bytes = preview.preview_bytes,
                content_length = ?preview.content_length,
                content_type = ?preview.content_type,
                body_truncated = preview.truncated,
                "metadata gateway returned empty data"
            );
        }

        parsed
            .data
            .ok_or_else(|| AppError::Repository("metadata gateway returned empty data".into()))
    }

    async fn send_request_with_retry<F, Fut>(
        &self,
        build_req: F,
        request_label: &'static str,
    ) -> AppResult<reqwest::Response>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = AppResult<reqwest::RequestBuilder>>,
    {
        for retry_index in 0..=METADATA_GATEWAY_MAX_RETRIES {
            match self
                .outbound_http
                .send_async(self.request_policy(request_label), &build_req)
                .await
            {
                Ok(resp) if resp.status().is_server_error() => {
                    if retry_index == METADATA_GATEWAY_MAX_RETRIES {
                        return Ok(resp);
                    }

                    let retry_after = metadata_gateway_transient_delay(retry_index);
                    warn!(
                        request = request_label,
                        status = %resp.status(),
                        retry_attempt = retry_index + 1,
                        retry_after_ms = retry_after.as_millis(),
                        "metadata gateway returned server error, retrying"
                    );
                    tokio::time::sleep(retry_after).await;
                }
                Ok(resp) => return Ok(resp),
                Err(OutboundRequestError::Build(error)) => return Err(error),
                Err(OutboundRequestError::Http(error)) => {
                    return Err(map_metadata_gateway_outbound_error(request_label, error));
                }
            }
        }

        Err(AppError::Repository(format!(
            "{request_label} exhausted retries"
        )))
    }

    fn request_policy(&self, request_label: &'static str) -> RequestPolicy {
        RequestPolicy::safe_read("metadata_gateway", request_label)
            .with_max_retries(METADATA_GATEWAY_MAX_RETRIES)
            .with_backoff(
                METADATA_GATEWAY_RATE_LIMIT_BASE_DELAY,
                METADATA_GATEWAY_RATE_LIMIT_MAX_DELAY,
            )
    }

    async fn reconcile_pq_enrollment_generation(
        &self,
        auth: Option<&InstanceAuth>,
        headers: &reqwest::header::HeaderMap,
    ) {
        let Some(InstanceAuth::Pq {
            instance_id,
            seed_b64,
            key_id,
            enrollment_generation,
        }) = auth
        else {
            return;
        };

        let Some(server_generation) = headers
            .get("X-SMG-Enrollment-Generation")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<i64>().ok())
        else {
            return;
        };

        let local_generation = enrollment_generation.unwrap_or(0);
        if server_generation <= local_generation {
            return;
        }

        let _rotation_guard = self.pq_rotation.lock().await;
        let Some(db) = self.enrollment_store.as_ref() else {
            warn!(
                "SMG PQ enrollment rotation skipped because this datastore engine has no enrollment store"
            );
            return;
        };

        if enrollment_generation.is_none() && server_generation == 1 {
            if let Err(error) =
                smg_enrollment::persist_pq_enrollment_generation(&**db, server_generation).await
            {
                warn!(
                    error = %error,
                    server_generation,
                    key_id = %key_id,
                    "failed to persist initial SMG PQ enrollment generation"
                );
                return;
            }
            let mut state = self.mtls_state.write().await;
            *state = MtlsState::NotAttempted;
            return;
        }

        match smg_enrollment::rotate_pq_enrollment(
            &**db,
            instance_id,
            seed_b64,
            key_id,
            &self.registration_url,
        )
        .await
        {
            Ok(_) => {
                info!(
                    instance_id = %instance_id,
                    key_id = %key_id,
                    old_generation = local_generation,
                    new_generation = server_generation,
                    "rotated SMG PQ enrollment after generation advance"
                );
                let mut state = self.mtls_state.write().await;
                *state = MtlsState::NotAttempted;
            }
            Err(error) => {
                warn!(
                    error = %error,
                    instance_id = %instance_id,
                    key_id = %key_id,
                    old_generation = local_generation,
                    new_generation = server_generation,
                    "failed to rotate SMG PQ enrollment after generation advance"
                );
            }
        }
    }

    async fn send_with_retry(&self, payload: &serde_json::Value) -> AppResult<reqwest::Response> {
        let (client, auth) = self.get_http_client().await?;
        let body_bytes = serde_json::to_vec(payload)
            .map_err(|e| AppError::Repository(format!("failed to serialize payload: {e}")))?;
        let endpoint_url = reqwest::Url::parse(&self.endpoint)
            .map_err(|e| AppError::Repository(format!("invalid endpoint URL: {e}")))?;

        let build_req = || {
            let client = client.clone();
            let auth = auth.clone();
            let endpoint_url = endpoint_url.clone();
            let body_bytes = body_bytes.clone();
            let endpoint = self.endpoint.clone();
            async move {
                let mut req = client
                    .post(endpoint)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body_bytes.clone());
                if let Some(ref auth) = auth {
                    req = apply_instance_auth_headers(
                        req,
                        auth,
                        reqwest::Method::POST.as_str(),
                        &endpoint_url,
                        &body_bytes,
                    )
                    .await?;
                }
                Ok(req)
            }
        };

        let response = self
            .send_request_with_retry(build_req, "metadata gateway request")
            .await?;
        self.reconcile_pq_enrollment_generation(auth.as_ref(), response.headers())
            .await;
        Ok(response)
    }

    /// POST a batched GraphQL query directly and return the `data` field as raw JSON.
    ///
    /// Batched alias-heavy requests intentionally bypass APQ. The variable entropy on
    /// these requests makes persisted-query cache hits unlikely enough that the GET +
    /// register dance is wasted overhead.
    ///
    /// Tolerates partial errors (some aliases may resolve while others fail).
    async fn post_batched_graphql_partial(&self, query: &str) -> AppResult<serde_json::Value> {
        let payload = json!({ "query": query });
        self.post_batched_graphql_partial_payload(&payload, "bulk metadata request")
            .await
    }

    async fn post_batched_graphql_partial_payload(
        &self,
        payload: &serde_json::Value,
        request_label: &'static str,
    ) -> AppResult<serde_json::Value> {
        let (client, auth) = self.get_http_client().await?;
        let body_bytes = serde_json::to_vec(payload)
            .map_err(|e| AppError::Repository(format!("failed to serialize payload: {e}")))?;
        let endpoint_url = reqwest::Url::parse(&self.endpoint)
            .map_err(|e| AppError::Repository(format!("invalid endpoint URL: {e}")))?;
        let build_req = || {
            let client = client.clone();
            let auth = auth.clone();
            let endpoint_url = endpoint_url.clone();
            let body_bytes = body_bytes.clone();
            let endpoint = self.endpoint.clone();
            async move {
                let mut req = client
                    .post(endpoint)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body_bytes.clone());
                if let Some(ref auth) = auth {
                    req = apply_instance_auth_headers(
                        req,
                        auth,
                        reqwest::Method::POST.as_str(),
                        &endpoint_url,
                        &body_bytes,
                    )
                    .await?;
                }
                Ok(req)
            }
        };
        let resp = self
            .send_request_with_retry(build_req, request_label)
            .await?;
        self.reconcile_pq_enrollment_generation(auth.as_ref(), resp.headers())
            .await;

        let status = resp.status();

        // On instance-auth rejection, invalidate and retry with fresh creds.
        if status == reqwest::StatusCode::UNAUTHORIZED
            && self.enrollment_config.registration_secret.is_some()
        {
            let preview = read_response_body_preview(
                resp,
                "bulk metadata auth rejection response read failed",
            )
            .await?;
            if !self.invalidate_enrollment().await {
                warn!(
                    request = request_label,
                    status = %status,
                    body_preview = %preview.escaped_text(),
                    body_preview_bytes = preview.preview_bytes,
                    content_length = ?preview.content_length,
                    content_type = ?preview.content_type,
                    body_truncated = preview.truncated,
                    "bulk metadata instance auth rejected during re-enrollment cooldown"
                );
                return Err(AppError::Repository(format!(
                    "bulk metadata instance auth rejected ({status}), re-enrollment on cooldown"
                )));
            }
            info!(
                request = request_label,
                "retrying metadata gateway request after re-enrollment"
            );
            let (client2, auth2) = self.get_http_client().await?;
            let build_retry_req = || {
                let client2 = client2.clone();
                let auth2 = auth2.clone();
                let endpoint_url = endpoint_url.clone();
                let body_bytes = body_bytes.clone();
                let endpoint = self.endpoint.clone();
                async move {
                    let mut req = client2
                        .post(endpoint)
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(body_bytes.clone());
                    if let Some(ref auth2) = auth2 {
                        req = apply_instance_auth_headers(
                            req,
                            auth2,
                            reqwest::Method::POST.as_str(),
                            &endpoint_url,
                            &body_bytes,
                        )
                        .await?;
                    }
                    Ok(req)
                }
            };
            let resp2 = self
                .send_request_with_retry(build_retry_req, request_label)
                .await?;
            let status2 = resp2.status();
            if !status2.is_success() {
                let preview =
                    read_response_body_preview(resp2, "bulk metadata retry response read failed")
                        .await?;
                warn!(
                    request = request_label,
                    status = %status2,
                    body_preview = %preview.escaped_text(),
                    body_preview_bytes = preview.preview_bytes,
                    content_length = ?preview.content_length,
                    content_type = ?preview.content_type,
                    body_truncated = preview.truncated,
                    "bulk metadata request failed after re-enrollment"
                );
                return Err(AppError::Repository(format!(
                    "bulk metadata request failed after re-enrollment ({status2}): {}",
                    preview.escaped_text()
                )));
            }
            let body2 = resp2
                .text()
                .await
                .map_err(|e| AppError::Repository(format!("bulk metadata read body: {e}")))?;
            return self.parse_partial_response(&body2);
        }

        if !status.is_success() {
            let preview =
                read_response_body_preview(resp, "bulk metadata response read failed").await?;
            warn!(
                request = request_label,
                status = %status,
                body_preview = %preview.escaped_text(),
                body_preview_bytes = preview.preview_bytes,
                content_length = ?preview.content_length,
                content_type = ?preview.content_type,
                body_truncated = preview.truncated,
                "bulk metadata request failed"
            );
            return Err(AppError::Repository(format!(
                "bulk metadata request failed ({status}): {}",
                preview.escaped_text()
            )));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| AppError::Repository(format!("bulk metadata read body: {e}")))?;
        self.parse_partial_response(&body)
    }

    fn parse_partial_response(&self, body: &str) -> AppResult<serde_json::Value> {
        let parsed: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| AppError::Repository(format!("bulk metadata invalid JSON: {e}")))?;

        if let Some(errors) = parsed.get("errors")
            && let Some(arr) = errors.as_array()
        {
            if parsed.get("data").is_none_or(serde_json::Value::is_null) {
                let msg = arr
                    .first()
                    .and_then(|err| err.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("bulk metadata returned errors without data");
                return Err(AppError::Repository(msg.to_string()));
            }
            for err in arr {
                let msg = err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                debug!("bulk metadata partial error: {msg}");
            }
        }

        parsed
            .get("data")
            .cloned()
            .ok_or_else(|| AppError::Repository("bulk metadata: no data in response".into()))
    }

    async fn get_metadata_bulk_via_metadata_bulk(
        &self,
        unique_movies: &[i64],
        unique_series: &[i64],
        language: &str,
    ) -> AppResult<BulkMetadataResult> {
        let request_started_at = Instant::now();
        let mut movies = HashMap::new();
        let mut series = HashMap::new();
        let bulk_requests = build_bulk_metadata_alias_requests(unique_movies, unique_series);
        let mut request_count = 0usize;

        for chunk in bulk_requests.chunks(METADATA_GATEWAY_MAX_METADATA_BULK_BATCH) {
            request_count = request_count.saturating_add(1);
            let chunk_started_at = Instant::now();
            let mut chunk_movie_ids = Vec::new();
            let mut chunk_series_ids = Vec::new();
            for request in chunk {
                match request {
                    BulkMetadataAliasRequest::Movie(tvdb_id) => chunk_movie_ids.push(*tvdb_id),
                    BulkMetadataAliasRequest::Series(tvdb_id) => chunk_series_ids.push(*tvdb_id),
                }
            }
            let movies_requested = chunk_movie_ids.len();
            let series_requested = chunk_series_ids.len();

            let data: MetadataBulkResponse = self
                .execute_graphql(json!({
                    "operationName": OP_METADATA_BULK,
                    "query": graphql_docs::METADATA_BULK_QUERY,
                    "variables": {
                        "movieTvdbIds": chunk_movie_ids,
                        "seriesTvdbIds": chunk_series_ids,
                        "language": language,
                        "includeEpisodes": true,
                    },
                }))
                .await?;
            let movie_count = data.metadata_bulk.movies.len();
            let series_count = data.metadata_bulk.series.len();
            for movie in data.metadata_bulk.movies {
                let metadata = movie_metadata_from_item(movie);
                movies.insert(metadata.tvdb_id, metadata);
            }
            for series_item in data.metadata_bulk.series {
                let metadata = series_metadata_from_item(series_item);
                series.insert(metadata.tvdb_id, metadata);
            }
            info!(
                request = request_count,
                ids = chunk.len(),
                movies_requested,
                series_requested,
                movies_resolved = movie_count,
                series_resolved = series_count,
                elapsed_ms = chunk_started_at.elapsed().as_millis() as u64,
                "metadata gateway metadataBulk request complete"
            );
        }

        info!(
            requests = request_count,
            movies_requested = unique_movies.len(),
            series_requested = unique_series.len(),
            movies_resolved = movies.len(),
            series_resolved = series.len(),
            elapsed_ms = request_started_at.elapsed().as_millis() as u64,
            "metadata gateway metadataBulk complete"
        );
        Ok(BulkMetadataResult { movies, series })
    }
}

// ---------------------------------------------------------------------------
// Bulk query builders (GraphQL aliases)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum BulkMetadataAliasRequest {
    Movie(i64),
    Series(i64),
}

fn build_search_tvdb_batch_query(queries: &[MetadataSearchQuery]) -> Vec<MetadataSearchQuery> {
    let mut normalized = Vec::with_capacity(queries.len());
    let mut seen = HashSet::with_capacity(queries.len());

    for query in queries {
        let trimmed_query = query.query.trim();
        let trimmed_type = query.type_hint.trim();
        let has_external_id =
            query.imdb_id.is_some() || query.tmdb_id.is_some() || query.tvdb_id.is_some();
        if trimmed_type.is_empty() || (trimmed_query.is_empty() && !has_external_id) {
            continue;
        }

        let normalized_query = MetadataSearchQuery {
            query: trimmed_query.to_string(),
            type_hint: trimmed_type.to_string(),
            year: query.year,
            imdb_id: query.imdb_id.clone(),
            tmdb_id: query.tmdb_id.clone(),
            tvdb_id: query.tvdb_id.clone(),
        };

        if seen.insert(normalized_query.clone()) {
            normalized.push(normalized_query);
        }
    }

    normalized
}

#[derive(Serialize)]
struct SearchTvdbBatchRequestInput {
    query: String,
    #[serde(rename = "type")]
    type_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    year: Option<i32>,
    #[serde(rename = "imdbId", skip_serializing_if = "Option::is_none")]
    imdb_id: Option<String>,
    #[serde(rename = "tmdbId", skip_serializing_if = "Option::is_none")]
    tmdb_id: Option<String>,
    #[serde(rename = "tvdbId", skip_serializing_if = "Option::is_none")]
    tvdb_id: Option<String>,
    limit: i32,
}

fn build_bulk_metadata_alias_requests(
    movie_ids: &[i64],
    series_ids: &[i64],
) -> Vec<BulkMetadataAliasRequest> {
    movie_ids
        .iter()
        .copied()
        .map(BulkMetadataAliasRequest::Movie)
        .chain(
            series_ids
                .iter()
                .copied()
                .map(BulkMetadataAliasRequest::Series),
        )
        .collect()
}

fn merge_bulk_artwork_url_partial(
    data: &serde_json::Value,
    movies: &mut HashMap<i64, TitleArtworkUrls>,
    series: &mut HashMap<i64, SeriesArtworkUrls>,
) {
    let Some(obj) = data.as_object() else {
        return;
    };

    for (alias, value) in obj {
        if value.is_null() {
            continue;
        }
        if alias.starts_with('m') {
            if let Ok(movie_result) = serde_json::from_value::<ArtworkMovieResult>(value.clone()) {
                let movie = movie_result.movie;
                movies.insert(
                    movie.tvdb_id,
                    TitleArtworkUrls {
                        poster_url: normalize_optional_artwork_url(Some(movie.poster_url)),
                        background_url: pick_artwork_url(&movie.artworks, "background"),
                    },
                );
            }
        } else if alias.starts_with('s')
            && let Ok(series_result) = serde_json::from_value::<ArtworkSeriesResult>(value.clone())
        {
            let item = series_result.series;
            series.insert(
                item.tvdb_id,
                SeriesArtworkUrls {
                    poster_url: normalize_optional_artwork_url(Some(item.poster_url)),
                    background_url: pick_artwork_url(&item.artworks, "background"),
                    episodes: item
                        .episodes
                        .into_iter()
                        .map(|episode| EpisodeArtworkUrls {
                            tvdb_id: episode.tvdb_id,
                            season_number: episode.season_number,
                            episode_number: episode.episode_number,
                            image_url: normalize_optional_artwork_url(episode.image_url),
                        })
                        .collect(),
                },
            );
        }
    }
}

fn movie_metadata_from_item(m: MovieItem) -> MovieMetadata {
    MovieMetadata {
        target_key: None,
        tvdb_id: m.tvdb_id,
        name: m.name,
        slug: m.slug,
        year: m.year,
        content_status: m.status,
        overview: m.overview,
        poster_url: normalize_artwork_url(&m.poster_url),
        background_url: pick_artwork_url(&m.artworks, "background"),
        language: m.language,
        runtime_minutes: m.runtime_minutes,
        sort_title: m.sort_title,
        imdb_id: m.imdb_id,
        tmdb_id: m.tmdb_id,
        popularity: m.tmdb_popularity,
        anidb_id: m.anidb_id,
        canonical_tags: canonical_tags_from_gateway(m.canonical_tags),
        studio: m.studio,
        tmdb_release_date: m.tmdb_release_date,
        ratings: rating_summary_from_gateway(m.rating, m.rating_sources, m.external_ratings),
    }
}

fn series_metadata_from_item(s: SeriesItem) -> SeriesMetadata {
    SeriesMetadata {
        target_key: None,
        tvdb_id: s.tvdb_id,
        name: s.name,
        sort_name: s.sort_name,
        slug: s.slug,
        year: s.year,
        content_status: s.status,
        first_aired: s.first_aired,
        overview: s.overview,
        network: s.network,
        runtime_minutes: s.runtime_minutes,
        poster_url: normalize_artwork_url(&s.poster_url),
        background_url: pick_artwork_url(&s.artworks, "background"),
        country: s.country,
        canonical_tags: canonical_tags_from_gateway(s.canonical_tags),
        aliases: s.aliases,
        tagged_aliases: s
            .tagged_aliases
            .into_iter()
            .map(|ta| scryer_domain::TaggedAlias {
                name: ta.name,
                language: ta.language,
            })
            .collect(),
        seasons: s
            .seasons
            .into_iter()
            .map(|season| SeasonMetadata {
                tvdb_id: season.tvdb_id,
                number: season.number,
                label: season.label,
                episode_type: season.episode_type,
            })
            .collect(),
        episodes: s
            .episodes
            .into_iter()
            .map(|ep| EpisodeMetadata {
                tvdb_id: ep.tvdb_id,
                episode_number: ep.episode_number,
                name: ep.name,
                aired: ep.aired,
                runtime_minutes: ep.runtime_minutes,
                is_filler: ep.is_filler,
                is_recap: ep.is_recap,
                overview: ep.overview,
                absolute_number: ep.absolute_number,
                season_number: ep.season_number,
                image_url: ep.image_url,
            })
            .collect(),
        anime_mappings: s
            .anime_mappings
            .into_iter()
            .map(|m| AnimeMapping {
                mal_id: m.mal_id,
                mal_dub_id: m.mal_dub_id,
                anilist_id: m.anilist_id,
                anidb_id: m.anidb_id,
                kitsu_id: m.kitsu_id,
                simkl_id: m.simkl_id,
                thetvdb_id: m.thetvdb_id,
                themoviedb_id: m.themoviedb_id,
                imdb_id: m.imdb_id,
                trakt_id: m.trakt_id,
                alt_tvdb_id: m.alt_tvdb_id,
                thetvdb_season: m.thetvdb_season,
                thetvdb_part: m.thetvdb_part,
                score: m.score,
                anime_media_type: m.anime_media_type.unwrap_or_default(),
                global_media_type: m.global_media_type.unwrap_or_default(),
                status: m.status.unwrap_or_default(),
                mapping_type: m.mapping_type.unwrap_or_default(),
                episode_mappings: m
                    .episode_mappings
                    .into_iter()
                    .map(|e| AnimeEpisodeMapping {
                        tvdb_season: e.tvdb_season,
                        episode_start: e.episode_start,
                        episode_end: e.episode_end,
                    })
                    .collect(),
            })
            .collect(),
        anime_movies: s
            .anime_movies
            .into_iter()
            .map(|movie| AnimeMovie {
                movie_tvdb_id: movie.movie_tvdb_id,
                movie_tmdb_id: movie.movie_tmdb_id,
                movie_imdb_id: movie.movie_imdb_id,
                movie_mal_id: movie.movie_mal_id,
                movie_anidb_id: movie.movie_anidb_id,
                name: movie.name,
                slug: movie.slug,
                year: movie.year,
                content_status: movie.content_status,
                overview: movie.overview,
                poster_url: movie.poster_url,
                language: movie.language,
                runtime_minutes: movie.runtime_minutes,
                sort_title: movie.sort_title,
                imdb_id: movie.imdb_id,
                studio: movie.studio,
                digital_release_date: movie.digital_release_date,
                association_confidence: movie.association_confidence,
                continuity_status: movie.continuity_status,
                movie_form: movie.movie_form,
                placement: movie.placement,
                confidence: movie.confidence,
                signal_summary: movie.signal_summary,
            })
            .collect(),
        ratings: rating_summary_from_gateway(s.rating, s.rating_sources, s.external_ratings),
    }
}

fn build_bulk_artwork_url_query(movie_ids: &[i64], series_ids: &[i64], language: &str) -> String {
    let mut q = String::from("query {\n");
    for (i, &id) in movie_ids.iter().enumerate() {
        let _ = writeln!(
            q,
            "  m{i}: movie(tvdbId: {id}, language: \"{language}\") {{ movie {{ tvdb_id poster_url artworks {{ kind url }} }} }}"
        );
    }
    for (i, &id) in series_ids.iter().enumerate() {
        let _ = writeln!(
            q,
            "  s{i}: series(id: \"{id}\", includeEpisodes: true, language: \"{language}\") {{ series {{ tvdb_id poster_url artworks {{ kind url }} episodes {{ tvdb_id season_number episode_number image_url }} }} }}"
        );
    }
    q.push_str("}\n");
    q
}

#[cfg(test)]
mod tests {
    use super::{
        ArtworkItem, InstanceAuth, MetadataGatewayClient, MetadataSearchQuery, MtlsState,
        OP_DISCOVER_PUBLIC_FEED, OP_DISCOVERY_CONTEXT_CHANGES, OP_GET_MOVIE, OP_GET_SERIES,
        OP_METADATA_BULK, OP_SEARCH_TVDB, SearchTvdbBatchResult, SearchTvdbResponse,
        SmgEnrollmentConfig, apply_instance_auth_headers_with_nonce, apq_cache_key, apq_hash,
        build_bulk_artwork_url_query, build_search_tvdb_batch_query, canonical_request_host,
        canonical_request_path_and_query, compatibility_poll_phase, enrollment_retry_delay,
        is_version_incompatible_response, map_metadata_gateway_outbound_error,
        next_version_compatibility_poll_delay_at, normalize_artwork_url,
        normalize_optional_artwork_url, parse_version_compatibility_incompatible,
        parse_version_compatibility_success, pick_artwork_url, sha256_hex,
        validate_search_tvdb_batch_echo,
    };
    use base64::Engine as _;
    use scryer_application::{
        AppError, DiscoveryContextChangeType, DiscoveryContextChangedSubjectInput,
        DiscoveryContextChangesInput, DiscoveryPublicFeedInput, DiscoverySubjectInput,
        MetadataGateway,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime};

    use crate::{
        graphql::metadata_gateway as graphql_docs,
        smg_enrollment::{EnrollmentError, RateLimited},
    };
    use wiremock::matchers::{body_string_contains, method, path, query_param};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn search_tvdb_payload() -> serde_json::Value {
        json!({
            "data": {
                "searchTvdb": {
                    "results": []
                }
            }
        })
    }

    #[test]
    fn metadata_gateway_outbound_rate_limit_preserves_retry_after() {
        let error = scryer_outbound_http::OutboundHttpError::RateLimited(
            scryer_outbound_http::RateLimitedError {
                scope: scryer_outbound_http::RateLimitScopeKey::from("metadata:gateway"),
                retry_after: Some(Duration::from_secs(90)),
                attempts: 1,
                retry_after_source: scryer_outbound_http::RetryAfterSource::Seconds,
                request_label: std::borrow::Cow::Borrowed("metadata gateway"),
            },
        );
        let error = map_metadata_gateway_outbound_error("metadata gateway", error);

        match error {
            AppError::TemporaryUnavailable {
                message,
                retry_after,
                ..
            } => {
                assert!(
                    message.contains("retry after 90s"),
                    "unexpected message: {message}"
                );
                assert_eq!(retry_after, Some(Duration::from_secs(90)));
            }
            other => panic!("expected temporary unavailable error, got {other:?}"),
        }
    }

    fn empty_metadata_bulk_payload() -> serde_json::Value {
        json!({
            "data": {
                "metadataBulk": {
                    "movies": [],
                    "series": [],
                    "missing_movie_tvdb_ids": [],
                    "missing_series_tvdb_ids": []
                }
            }
        })
    }

    fn movie_item_payload(tvdb_id: i64) -> serde_json::Value {
        json!({
            "tvdb_id": tvdb_id,
            "name": "Fixture Movie",
            "slug": "fixture-movie",
            "year": 2026,
            "status": "released",
            "overview": "Fixture overview",
            "poster_url": "",
            "language": "eng",
            "runtime_minutes": 90,
            "sort_title": "fixture movie",
            "imdb_id": "tt1234567",
            "tmdb_id": 123,
            "studio": "",
            "tmdb_release_date": null,
            "rating": null,
            "rating_sources": [],
            "external_ratings": [],
            "artworks": []
        })
    }

    fn series_item_payload(tvdb_id: i64) -> serde_json::Value {
        json!({
            "tvdb_id": tvdb_id,
            "name": "Fixture Series",
            "sort_name": "fixture series",
            "slug": "fixture-series",
            "status": "continuing",
            "year": 2026,
            "first_aired": "",
            "overview": "Fixture overview",
            "network": "",
            "runtime_minutes": 45,
            "poster_url": "",
            "country": "",
            "canonical_tags": [],
            "rating": null,
            "rating_sources": [],
            "external_ratings": [],
            "aliases": [],
            "tagged_aliases": [],
            "artworks": [],
            "seasons": [],
            "episodes": [],
            "anime_mappings": [],
            "anime_movies": []
        })
    }

    fn movie_payload(tvdb_id: i64) -> serde_json::Value {
        json!({
            "data": {
                "movie": {
                    "movie": movie_item_payload(tvdb_id)
                }
            }
        })
    }

    fn series_payload(tvdb_id: i64) -> serde_json::Value {
        json!({
            "data": {
                "series": {
                    "series": series_item_payload(tvdb_id)
                }
            }
        })
    }

    fn persisted_query_not_found_payload() -> serde_json::Value {
        json!({
            "errors": [
                {
                    "message": "PersistedQueryNotFound"
                }
            ]
        })
    }

    fn unsigned_gateway_client(endpoint: String) -> MetadataGatewayClient {
        MetadataGatewayClient::new_without_enrollment_store(
            endpoint,
            SmgEnrollmentConfig {
                registration_secret: None,
            },
        )
    }

    async fn signed_gateway_client(endpoint: String) -> MetadataGatewayClient {
        let client = MetadataGatewayClient::new_without_enrollment_store(
            endpoint,
            SmgEnrollmentConfig {
                registration_secret: Some("fixture-secret".to_string()),
            },
        );
        let seed_b64 = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        *client.mtls_state.write().await = MtlsState::Enrolled {
            client: scryer_outbound_http::smg_reqwest_client(),
            auth: InstanceAuth::Pq {
                instance_id: Arc::new("fixture-instance".to_string()),
                seed_b64: Arc::new(seed_b64),
                key_id: Arc::new("fixture-key".to_string()),
                enrollment_generation: Some(1),
            },
        };
        client
    }

    fn test_instance_auth() -> InstanceAuth {
        InstanceAuth::Pq {
            instance_id: Arc::new("fixture-instance".to_string()),
            seed_b64: Arc::new(base64::engine::general_purpose::STANDARD.encode([7u8; 32])),
            key_id: Arc::new("fixture-key".to_string()),
            enrollment_generation: Some(1),
        }
    }

    fn header_value<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
        request
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    }

    fn assert_v2_signed_request(request: &Request) {
        assert_eq!(
            header_value(request, "x-scryer-auth-version"),
            Some("pqsig-v2")
        );
        assert_eq!(
            header_value(request, "x-scryer-key-id"),
            Some("fixture-key")
        );
        assert!(header_value(request, "x-scryer-timestamp").is_some());
        assert!(header_value(request, "x-scryer-signature").is_some());
        assert!(header_value(request, "x-scryer-nonce").is_some());
    }

    #[tokio::test]
    async fn get_metadata_bulk_uses_metadata_bulk_when_available() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains(format!(
                "\"operationName\":\"{OP_METADATA_BULK}\""
            )))
            .and(body_string_contains("\"movieTvdbIds\":[101]"))
            .and(body_string_contains("\"seriesTvdbIds\":[202]"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_metadata_bulk_payload()))
            .expect(1)
            .mount(&server)
            .await;
        let client = unsigned_gateway_client(format!("{}/graphql", server.uri()));

        let result = client
            .get_metadata_bulk(&[101], &[202], "eng")
            .await
            .expect("metadataBulk request should succeed");

        assert!(result.movies.is_empty());
        assert!(result.series.is_empty());

        let requests = server
            .received_requests()
            .await
            .expect("metadataBulk request should be captured");
        let payload: serde_json::Value = serde_json::from_slice(&requests[0].body)
            .expect("metadataBulk request should contain JSON");
        assert!(
            payload.get("extensions").is_none(),
            "metadataBulk must not use APQ"
        );
    }

    #[test]
    fn current_smg_metadata_queries_omit_target_key() {
        let queries = [
            graphql_docs::METADATA_BULK_QUERY,
            graphql_docs::GET_MOVIE_QUERY,
            graphql_docs::GET_SERIES_QUERY,
        ];

        assert!(graphql_docs::METADATA_BULK_QUERY.contains("metadataBulk"));
        assert!(graphql_docs::METADATA_BULK_QUERY.contains("external_ratings"));
        assert!(graphql_docs::GET_SERIES_QUERY.contains("tagged_aliases"));
        assert!(queries.iter().all(|query| !query.contains("target_key")));
    }

    #[tokio::test]
    async fn get_metadata_bulk_caps_metadata_bulk_requests_at_fifty_combined_ids() {
        let server = MockServer::start().await;
        let request_sizes = Arc::new(Mutex::new(Vec::new()));
        let request_sizes_for_mock = Arc::clone(&request_sizes);
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("metadataBulk"))
            .respond_with(move |request: &Request| {
                let body: serde_json::Value =
                    serde_json::from_slice(&request.body).expect("request json");
                let variables = body
                    .get("variables")
                    .expect("variables")
                    .as_object()
                    .expect("variables object");
                let movie_count = variables
                    .get("movieTvdbIds")
                    .and_then(|value| value.as_array())
                    .map_or(0, Vec::len);
                let series_count = variables
                    .get("seriesTvdbIds")
                    .and_then(|value| value.as_array())
                    .map_or(0, Vec::len);
                request_sizes_for_mock
                    .lock()
                    .expect("request sizes lock")
                    .push((movie_count, series_count));
                ResponseTemplate::new(200).set_body_json(empty_metadata_bulk_payload())
            })
            .expect(2)
            .mount(&server)
            .await;
        let client = unsigned_gateway_client(format!("{}/graphql", server.uri()));
        let movie_ids = (1..=30).collect::<Vec<_>>();
        let series_ids = (101..=130).collect::<Vec<_>>();

        client
            .get_metadata_bulk(&movie_ids, &series_ids, "eng")
            .await
            .expect("metadataBulk chunks should succeed");

        let request_sizes = request_sizes.lock().expect("request sizes lock");
        assert_eq!(request_sizes.len(), 2);
        assert!(
            request_sizes
                .iter()
                .all(|(movies, series)| movies + series <= 50)
        );
        assert_eq!(
            request_sizes
                .iter()
                .map(|(movies, series)| movies + series)
                .sum::<usize>(),
            60
        );
    }

    #[tokio::test]
    async fn get_movie_still_uses_single_title_query() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/graphql"))
            .and(query_param("operationName", OP_GET_MOVIE))
            .respond_with(ResponseTemplate::new(200).set_body_json(movie_payload(909)))
            .expect(1)
            .mount(&server)
            .await;
        let client = unsigned_gateway_client(format!("{}/graphql", server.uri()));

        let movie = client
            .get_movie(909, "eng")
            .await
            .expect("single movie request should still work");

        assert_eq!(movie.tvdb_id, 909);
        assert_eq!(movie.name, "Fixture Movie");
        assert_eq!(movie.target_key, None);
    }

    #[tokio::test]
    async fn get_series_still_uses_single_title_query() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/graphql"))
            .and(query_param("operationName", OP_GET_SERIES))
            .respond_with(ResponseTemplate::new(200).set_body_json(series_payload(424536)))
            .expect(1)
            .mount(&server)
            .await;
        let client = unsigned_gateway_client(format!("{}/graphql", server.uri()));

        let series = client
            .get_series(424536, "eng")
            .await
            .expect("single series request should still work");

        assert_eq!(series.tvdb_id, 424536);
        assert_eq!(series.name, "Fixture Series");
        assert_eq!(series.target_key, None);
    }

    #[tokio::test]
    async fn apq_get_includes_operation_name() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/graphql"))
            .and(query_param("operationName", OP_SEARCH_TVDB))
            .respond_with(ResponseTemplate::new(200).set_body_json(search_tvdb_payload()))
            .expect(1)
            .mount(&server)
            .await;
        let client = unsigned_gateway_client(format!("{}/graphql", server.uri()));

        let data: SearchTvdbResponse = client
            .execute_graphql_apq(
                OP_SEARCH_TVDB,
                graphql_docs::SEARCH_TVDB_QUERY,
                &client.search_hash,
                json!({
                    "query": "Fixture Query",
                    "type": "movie",
                    "limit": 10,
                    "year": null,
                }),
            )
            .await
            .expect("APQ GET should succeed");

        assert!(data.search_tvdb.results.is_empty());
    }

    #[tokio::test]
    async fn apq_registration_post_includes_operation_name() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/graphql"))
            .and(query_param("operationName", OP_SEARCH_TVDB))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(persisted_query_not_found_payload()),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("\"operationName\":\"SearchTvdb\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(search_tvdb_payload()))
            .expect(1)
            .mount(&server)
            .await;
        let client = unsigned_gateway_client(format!("{}/graphql", server.uri()));

        let data: SearchTvdbResponse = client
            .execute_graphql_apq(
                OP_SEARCH_TVDB,
                graphql_docs::SEARCH_TVDB_QUERY,
                &client.search_hash,
                json!({
                    "query": "Fixture Query",
                    "type": "movie",
                    "limit": 10,
                    "year": null,
                }),
            )
            .await
            .expect("APQ registration should succeed");

        assert!(data.search_tvdb.results.is_empty());
    }

    #[tokio::test]
    async fn discover_public_feed_uses_full_query_public_get() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/graphql"))
            .and(query_param("operationName", OP_DISCOVER_PUBLIC_FEED))
            .and(query_param(
                "query",
                graphql_docs::DISCOVER_PUBLIC_FEED_QUERY,
            ))
            .respond_with(|request: &Request| {
                let params = request
                    .url
                    .query_pairs()
                    .map(|(key, _)| key.into_owned())
                    .collect::<Vec<_>>();
                assert!(
                    !params.iter().any(|key| key == "extensions"),
                    "public feed GET must not use hash-only APQ"
                );
                ResponseTemplate::new(200).set_body_json(json!({
                    "data": {
                        "discoverPublicFeed": {
                            "subject_keys": [],
                            "generated_at": "2026-06-25T00:00:00Z",
                            "sections": []
                        }
                    }
                }))
            })
            .expect(1)
            .mount(&server)
            .await;
        let client = unsigned_gateway_client(format!("{}/graphql", server.uri()));

        let data = client
            .discover_public_feed(&DiscoveryPublicFeedInput {
                region: "US".to_string(),
                language: "eng".to_string(),
                section_types: Vec::new(),
                limit_per_section: 25,
                include_unresolved: false,
                full_sections: true,
            })
            .await
            .expect("discovery feed should succeed through public GET");

        assert!(data.sections.is_empty());
    }

    #[tokio::test]
    async fn discovery_context_changes_uses_post_apq_and_serializes_change_type() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains(
                "\"operationName\":\"DiscoveryContextChanges\"",
            ))
            .and(body_string_contains("\"changeType\":\"ADDED\""))
            .and(body_string_contains(
                "\"contextSubjectKeys\":[\"tvdb:series:1\"]",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "discoveryContextChanges": {
                        "status": "COMPLETE",
                        "retry_after_seconds": 5,
                        "generated_at": "2026-06-25T00:00:00Z",
                        "context_fingerprint": "current",
                        "previous_context_fingerprint": "previous",
                        "discovery_index_watermark": "test",
                        "context_subject_count": 1,
                        "changed_subject_count": 1,
                        "resolved_changed_subject_keys": ["tvdb:series:1"],
                        "removed_subject_keys": [],
                        "affected_target_keys": [],
                        "items": []
                    }
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = unsigned_gateway_client(format!("{}/graphql", server.uri()));

        let data = client
            .discovery_context_changes(&DiscoveryContextChangesInput {
                context_subject_keys: vec!["tvdb:series:1".to_string()],
                changed_subjects: vec![DiscoveryContextChangedSubjectInput {
                    subject: DiscoverySubjectInput {
                        key: Some("tvdb:series:1".to_string()),
                        ..Default::default()
                    },
                    change_type: DiscoveryContextChangeType::Added,
                    previous_subject: None,
                }],
                previous_context_fingerprint: Some("previous".to_string()),
                region: "US".to_string(),
                language: "eng".to_string(),
                max_items: 50,
                include_owned: true,
                include_unresolved: false,
                context_fingerprint: Some("current".to_string()),
            })
            .await
            .expect("incremental discovery reload should succeed through APQ POST");

        assert_eq!(data.status, "COMPLETE");
        assert_eq!(data.resolved_changed_subject_keys, vec!["tvdb:series:1"]);
    }

    #[test]
    fn apq_cache_key_uses_blake3_variables_digest() {
        let variables_str = r#"{"query":"Fixture Query","type":"movie","limit":10,"year":null}"#;
        let variables_digest = blake3::hash(variables_str.as_bytes()).to_hex().to_string();

        let key = apq_cache_key(OP_SEARCH_TVDB, "query-hash", variables_str);

        assert_eq!(
            key,
            format!("{OP_SEARCH_TVDB}:query-hash:blake3:{variables_digest}")
        );
        assert!(!key.contains(variables_str));
    }

    #[test]
    fn apq_hash_uses_query_string_only() {
        let query = graphql_docs::SEARCH_TVDB_QUERY;

        assert_eq!(apq_hash(query), sha256_hex(query));
        assert_ne!(
            apq_hash(query),
            sha256_hex(&format!("{OP_SEARCH_TVDB}:{query}"))
        );
        assert_ne!(
            apq_hash(query),
            sha256_hex(&format!("{query}:{{\"query\":\"Fixture Query\"}}"))
        );
    }

    #[test]
    fn apq_get_request_target_preserves_encoded_query() {
        let mut url = reqwest::Url::parse("https://smg.example/graphql").expect("url");
        url.query_pairs_mut()
            .append_pair("operationName", OP_SEARCH_TVDB)
            .append_pair(
                "extensions",
                "{\"persistedQuery\":{\"sha256Hash\":\"abc\"}}",
            )
            .append_pair("variables", "{\"query\":\"Fixture Query\"}");

        let target = canonical_request_path_and_query(&url);

        assert!(target.starts_with("/graphql?"));
        assert!(target.contains("operationName=SearchTvdb"));
        assert!(target.contains("extensions=%7B%22persistedQuery%22%3A"));
        assert!(target.contains("variables=%7B%22query%22%3A%22Fixture+Query%22%7D"));
    }

    #[test]
    fn canonical_request_host_includes_final_request_port() {
        let url = reqwest::Url::parse("http://127.0.0.1:43210/graphql").expect("url");
        let ipv6_url = reqwest::Url::parse("http://[::1]:43210/graphql").expect("ipv6 url");

        assert_eq!(canonical_request_host(&url).unwrap(), "127.0.0.1:43210");
        assert_eq!(canonical_request_host(&ipv6_url).unwrap(), "[::1]:43210");
    }

    #[tokio::test]
    async fn v2_signed_graphql_post_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(|request: &Request| {
                assert_v2_signed_request(request);
                ResponseTemplate::new(200).set_body_json(search_tvdb_payload())
            })
            .expect(1)
            .mount(&server)
            .await;
        let client = signed_gateway_client(format!("{}/graphql", server.uri())).await;

        let data: SearchTvdbResponse = client
            .execute_graphql(json!({
                "query": graphql_docs::SEARCH_TVDB_QUERY,
                "variables": {
                    "query": "Fixture Query",
                    "type": "movie",
                    "limit": 10,
                    "year": null,
                },
            }))
            .await
            .expect("signed GraphQL POST should succeed");

        assert!(data.search_tvdb.results.is_empty());
    }

    #[tokio::test]
    async fn v2_signed_apq_get_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/graphql"))
            .and(query_param("operationName", OP_SEARCH_TVDB))
            .respond_with(|request: &Request| {
                assert_v2_signed_request(request);
                ResponseTemplate::new(200).set_body_json(search_tvdb_payload())
            })
            .expect(1)
            .mount(&server)
            .await;
        let client = signed_gateway_client(format!("{}/graphql", server.uri())).await;

        let data: SearchTvdbResponse = client
            .execute_graphql_apq(
                OP_SEARCH_TVDB,
                graphql_docs::SEARCH_TVDB_QUERY,
                &client.search_hash,
                json!({
                    "query": "Fixture Query",
                    "type": "movie",
                    "limit": 10,
                    "year": null,
                }),
            )
            .await
            .expect("signed APQ GET should succeed");

        assert!(data.search_tvdb.results.is_empty());
    }

    #[tokio::test]
    async fn v2_signed_version_compatibility_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/version-compatibility"))
            .respond_with(|request: &Request| {
                assert_v2_signed_request(request);
                ResponseTemplate::new(200).set_body_json(json!({
                    "compatibility": null,
                    "update": null,
                }))
            })
            .expect(1)
            .mount(&server)
            .await;

        let auth = test_instance_auth();
        let http = scryer_outbound_http::smg_reqwest_client();
        let url = reqwest::Url::parse(&format!("{}/api/version-compatibility", server.uri()))
            .expect("version compatibility URL");
        let body_bytes =
            serde_json::to_vec(&json!({ "version": "fixture" })).expect("compatibility body");
        let response = apply_instance_auth_headers_with_nonce(
            http.post(url.clone())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body_bytes.clone()),
            &auth,
            reqwest::Method::POST.as_str(),
            &url,
            &body_bytes,
            None,
        )
        .await
        .expect("signed version compatibility request")
        .send()
        .await
        .expect("version compatibility response");

        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    #[tokio::test]
    async fn reused_v2_nonce_is_rejected_by_mock() {
        let server = MockServer::start().await;
        let seen = Arc::new(Mutex::new(std::collections::HashSet::<String>::new()));
        let seen_for_mock = seen.clone();
        Mock::given(method("GET"))
            .and(path("/graphql"))
            .respond_with(move |request: &Request| {
                assert_v2_signed_request(request);
                let key_id = header_value(request, "x-scryer-key-id").unwrap_or_default();
                let nonce = header_value(request, "x-scryer-nonce").unwrap_or_default();
                let key = format!("{key_id}:{nonce}");
                let mut seen = seen_for_mock.lock().expect("seen nonce lock");
                if seen.insert(key) {
                    ResponseTemplate::new(200).set_body_json(search_tvdb_payload())
                } else {
                    ResponseTemplate::new(401).set_body_string("reused nonce")
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let auth = test_instance_auth();
        let client = scryer_outbound_http::smg_reqwest_client();
        let url = reqwest::Url::parse(&format!("{}/graphql", server.uri())).expect("url");

        let first = apply_instance_auth_headers_with_nonce(
            client.get(url.clone()),
            &auth,
            reqwest::Method::GET.as_str(),
            &url,
            &[],
            Some("fixed-nonce".to_string()),
        )
        .await
        .expect("first signed request")
        .send()
        .await
        .expect("first response");
        let second = apply_instance_auth_headers_with_nonce(
            client.get(url.clone()),
            &auth,
            reqwest::Method::GET.as_str(),
            &url,
            &[],
            Some("fixed-nonce".to_string()),
        )
        .await
        .expect("second signed request")
        .send()
        .await
        .expect("second response");

        assert_eq!(first.status(), reqwest::StatusCode::OK);
        assert_eq!(second.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_v2_nonce_is_rejected_by_mock() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/graphql"))
            .respond_with(|request: &Request| {
                if header_value(request, "x-scryer-auth-version") == Some("pqsig-v2")
                    && header_value(request, "x-scryer-nonce").is_none()
                {
                    ResponseTemplate::new(401).set_body_string("missing nonce")
                } else {
                    ResponseTemplate::new(200).set_body_json(search_tvdb_payload())
                }
            })
            .expect(1)
            .mount(&server)
            .await;

        let response = scryer_outbound_http::smg_reqwest_client()
            .get(format!("{}/graphql", server.uri()))
            .header("X-Scryer-Auth-Version", "pqsig-v2")
            .header("X-Scryer-Key-Id", "fixture-key")
            .header("X-Scryer-Timestamp", "123")
            .header("X-Scryer-Signature", "fixture-signature")
            .send()
            .await
            .expect("mock response");

        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn title_recommendations_query_uses_related_title_rating_provenance() {
        assert!(graphql_docs::TITLE_RECOMMENDATIONS_QUERY.contains("rating_sources"));
        assert!(graphql_docs::TITLE_RECOMMENDATIONS_QUERY.contains("rating_provenance"));
        assert!(graphql_docs::TITLE_RECOMMENDATIONS_QUERY.contains("rating_source"));
        assert!(graphql_docs::TITLE_RECOMMENDATIONS_QUERY.contains("metadata_source"));
        assert!(graphql_docs::TITLE_RECOMMENDATIONS_QUERY.contains("external_ids"));
        assert!(!graphql_docs::TITLE_RECOMMENDATIONS_QUERY.contains("external_ratings"));
    }

    #[test]
    fn discovery_queries_use_discovery_rating_provenance_shape() {
        let queries = [
            graphql_docs::DISCOVER_PUBLIC_FEED_QUERY,
            graphql_docs::DISCOVERY_CONTEXT_SNAPSHOT_PAGE_QUERY,
            graphql_docs::DISCOVERY_CONTEXT_CHANGES_QUERY,
            graphql_docs::COLLECTION_COMPLETIONS_QUERY,
            graphql_docs::TITLE_RECOMMENDATIONS_QUERY,
        ];

        for query in queries {
            assert!(query.contains("rating_sources"));
            assert!(query.contains("rating_provenance"));
            assert!(query.contains("rating_source"));
            assert!(query.contains("metadata_source"));
            assert!(query.contains("external_ids"));
            assert!(query.contains("source"));
            assert!(query.contains("kind"));
            assert!(query.contains("key"));
            assert!(!query.contains("external_ratings"));
        }
    }

    #[test]
    fn discovery_queries_request_canonical_tags() {
        let queries = [
            graphql_docs::DISCOVER_PUBLIC_FEED_QUERY,
            graphql_docs::DISCOVERY_CONTEXT_SNAPSHOT_PAGE_QUERY,
            graphql_docs::DISCOVERY_CONTEXT_CHANGES_QUERY,
            graphql_docs::COLLECTION_COMPLETIONS_QUERY,
            graphql_docs::TITLE_RECOMMENDATIONS_QUERY,
        ];

        for query in queries {
            assert!(query.contains("canonical_tags"));
            assert!(query.contains("source_tag_keys"));
        }
    }

    #[test]
    fn discovery_title_applies_rating_provenance_as_external_ratings() {
        let mut item = scryer_application::DiscoveryTitle {
            rating_provenance: vec![scryer_application::DiscoveryRatingProvenance {
                metadata_source: "mdblist".to_string(),
                rating_source: "imdb".to_string(),
                value: Some(8.2),
                score: Some(82.0),
                normalized: 0.82,
                votes: Some(12_345),
                url: "https://example.test/imdb".to_string(),
            }],
            ..scryer_application::DiscoveryTitle::default()
        };

        item.apply_rating_provenance();

        assert!(item.rating_provenance.is_empty());
        assert_eq!(item.external_ratings.len(), 1);
        assert_eq!(item.external_ratings[0].source, "imdb");
        assert_eq!(item.external_ratings[0].value, Some(8.2));
        assert_eq!(item.external_ratings[0].score, Some(82.0));
        assert_eq!(item.external_ratings[0].normalized, 0.82);
        assert_eq!(item.external_ratings[0].votes, Some(12_345));
        assert_eq!(item.external_ratings[0].url, "https://example.test/imdb");
    }

    #[test]
    fn bulk_artwork_url_query_uses_narrow_projection() {
        let query = build_bulk_artwork_url_query(&[11], &[22], "eng");

        assert!(query.contains("movie(tvdbId: 11"));
        assert!(query.contains("series(id: \"22\""));
        assert!(query.contains("tvdb_id poster_url artworks { kind url }"));
        assert!(query.contains("episodes { tvdb_id season_number episode_number image_url }"));
        assert!(!query.contains("...MovieFields"));
        assert!(!query.contains("...SeriesFields"));
        assert!(!query.contains("tagged_aliases"));
    }

    #[test]
    fn search_tvdb_batch_queries_trim_dedupe_and_preserve_first_seen_order() {
        let queries = vec![
            MetadataSearchQuery {
                query: "  Lantern Tide  ".to_string(),
                type_hint: "movie".to_string(),
                year: Some(2001),
                imdb_id: Some("tt1234567".to_string()),
                tmdb_id: None,
                tvdb_id: None,
            },
            MetadataSearchQuery {
                query: "Lantern Tide".to_string(),
                type_hint: "movie".to_string(),
                year: Some(2001),
                imdb_id: Some("tt1234567".to_string()),
                tmdb_id: None,
                tvdb_id: None,
            },
            MetadataSearchQuery {
                query: "   ".to_string(),
                type_hint: "series".to_string(),
                year: None,
                imdb_id: None,
                tmdb_id: None,
                tvdb_id: None,
            },
            MetadataSearchQuery {
                query: "   ".to_string(),
                type_hint: "movie".to_string(),
                year: None,
                imdb_id: None,
                tmdb_id: Some("2502".to_string()),
                tvdb_id: None,
            },
            MetadataSearchQuery {
                query: "Velvet Comet".to_string(),
                type_hint: "anime".to_string(),
                year: None,
                imdb_id: None,
                tmdb_id: None,
                tvdb_id: Some("999".to_string()),
            },
            MetadataSearchQuery {
                query: "Lantern Tide".to_string(),
                type_hint: "movie".to_string(),
                year: Some(2002),
                imdb_id: None,
                tmdb_id: None,
                tvdb_id: None,
            },
        ];

        let normalized = build_search_tvdb_batch_query(&queries);

        assert_eq!(normalized.len(), 4);
        assert_eq!(normalized[0].query, "Lantern Tide");
        assert_eq!(normalized[0].type_hint, "movie");
        assert_eq!(normalized[0].year, Some(2001));
        assert_eq!(normalized[0].imdb_id.as_deref(), Some("tt1234567"));
        assert_eq!(normalized[1].query, "");
        assert_eq!(normalized[1].type_hint, "movie");
        assert_eq!(normalized[1].year, None);
        assert_eq!(normalized[1].tmdb_id.as_deref(), Some("2502"));
        assert_eq!(normalized[2].query, "Velvet Comet");
        assert_eq!(normalized[2].type_hint, "anime");
        assert_eq!(normalized[2].year, None);
        assert_eq!(normalized[2].tvdb_id.as_deref(), Some("999"));
        assert_eq!(normalized[3].query, "Lantern Tide");
        assert_eq!(normalized[3].type_hint, "movie");
        assert_eq!(normalized[3].year, Some(2002));
    }

    #[test]
    fn search_tvdb_batch_query_uses_dedicated_field() {
        assert!(graphql_docs::SEARCH_TVDB_BATCH_QUERY.contains("searchTvdbBatch"));
        assert!(!graphql_docs::SEARCH_TVDB_BATCH_QUERY.contains("searchTvdb(query:"));
        assert!(graphql_docs::SEARCH_TVDB_BATCH_QUERY.contains("query"));
        assert!(graphql_docs::SEARCH_TVDB_BATCH_QUERY.contains("type"));
        assert!(graphql_docs::SEARCH_TVDB_BATCH_QUERY.contains("year"));
        assert!(graphql_docs::SEARCH_TVDB_BATCH_QUERY.contains("auto_match_safe"));
        assert!(graphql_docs::SEARCH_TVDB_BATCH_QUERY.contains("auto_match_signals"));
    }

    #[test]
    fn search_tvdb_batch_echo_validation_accepts_matching_response() {
        let expected = MetadataSearchQuery {
            query: "Lantern Tide".to_string(),
            type_hint: "movie".to_string(),
            year: Some(2001),
            imdb_id: Some("tt1234567".to_string()),
            tmdb_id: None,
            tvdb_id: None,
        };
        let actual = SearchTvdbBatchResult {
            query: "Lantern Tide".to_string(),
            type_hint: Some("movie".to_string()),
            year: Some(2001),
            results: Vec::new(),
        };

        validate_search_tvdb_batch_echo(&expected, &actual).expect("matching echo");
    }

    #[test]
    fn search_tvdb_batch_echo_validation_rejects_mismatched_response() {
        let expected = MetadataSearchQuery {
            query: "Lantern Tide".to_string(),
            type_hint: "movie".to_string(),
            year: Some(2001),
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
        };
        let actual = SearchTvdbBatchResult {
            query: "Wrong Tide".to_string(),
            type_hint: Some("movie".to_string()),
            year: Some(2001),
            results: Vec::new(),
        };

        assert!(validate_search_tvdb_batch_echo(&expected, &actual).is_err());
    }

    #[test]
    fn normalize_artwork_url_collapses_duplicate_path_separators() {
        let url = "https://artworks.thetvdb.com/banners/movies/147325/backgrounds//5vyMUvxy6W0xU9Unnh5M7WXkh4l.jpg";

        assert_eq!(
            normalize_artwork_url(url),
            "https://artworks.thetvdb.com/banners/movies/147325/backgrounds/5vyMUvxy6W0xU9Unnh5M7WXkh4l.jpg"
        );
    }

    #[test]
    fn normalize_optional_artwork_url_preserves_missing_and_existing_urls() {
        assert_eq!(normalize_optional_artwork_url(None), None);
        assert_eq!(normalize_optional_artwork_url(Some("".to_string())), None);
        assert_eq!(
            normalize_optional_artwork_url(Some("   ".to_string())),
            None
        );
        assert_eq!(
            normalize_optional_artwork_url(Some(
                "https://artworks.thetvdb.com/banners/posters/example.jpg".to_string()
            )),
            Some("https://artworks.thetvdb.com/banners/posters/example.jpg".to_string())
        );
    }

    #[test]
    fn pick_artwork_url_skips_blank_matching_artwork_urls() {
        let artworks = vec![
            ArtworkItem {
                kind: "background".to_string(),
                url: "   ".to_string(),
            },
            ArtworkItem {
                kind: "background".to_string(),
                url: "https://artworks.thetvdb.com/banners/backgrounds//usable.jpg".to_string(),
            },
        ];

        assert_eq!(
            pick_artwork_url(&artworks, "background"),
            Some("https://artworks.thetvdb.com/banners/backgrounds/usable.jpg".to_string())
        );
    }

    #[test]
    fn enrollment_retry_delay_prefers_rate_limit_header_delay() {
        let delay = enrollment_retry_delay(
            &EnrollmentError::RateLimited(RateLimited {
                retry_after: Some(Duration::from_secs(75)),
                message: "cloudflare rate limit".to_string(),
            }),
            0,
        );

        assert_eq!(delay, Duration::from_secs(75));
    }

    #[test]
    fn enrollment_retry_delay_falls_back_when_header_is_missing() {
        let delay = enrollment_retry_delay(
            &EnrollmentError::RateLimited(RateLimited {
                retry_after: None,
                message: "cloudflare rate limit".to_string(),
            }),
            1,
        );

        assert_eq!(delay, Duration::from_secs(60));
    }

    #[test]
    fn compatibility_poll_phase_is_stable_and_bounded() {
        let first = compatibility_poll_phase("instance-a");
        let second = compatibility_poll_phase("instance-a");
        let different = compatibility_poll_phase("instance-b");

        assert_eq!(first, second);
        assert!(first < Duration::from_secs(6 * 60 * 60));
        assert!(different < Duration::from_secs(6 * 60 * 60));
    }

    #[test]
    fn next_version_compatibility_poll_delay_skips_slots_inside_startup_guard() {
        let now = std::time::UNIX_EPOCH + Duration::from_secs(6 * 60 * 60 + 5 * 60);
        let phase = Duration::from_secs(10 * 60);

        let delay =
            next_version_compatibility_poll_delay_at(now, phase, Duration::from_secs(30 * 60));

        assert_eq!(delay, Duration::from_secs(6 * 60 * 60 + 5 * 60));
    }

    #[test]
    fn next_version_compatibility_poll_delay_uses_next_ring_slot_without_guard() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(6 * 60 * 60 + 5 * 60);
        let phase = Duration::from_secs(10 * 60);

        let delay = next_version_compatibility_poll_delay_at(now, phase, Duration::from_secs(0));

        assert_eq!(delay, Duration::from_secs(5 * 60));
    }

    #[test]
    fn parse_version_compatibility_success_returns_none_for_supported() {
        let body = br#"{
            "compatibility": {
                "status": "supported",
                "minimum_version": "",
                "your_version": "0.12.0",
                "message": ""
            }
        }"#;

        let check = parse_version_compatibility_success(body).expect("parse supported response");

        assert!(check.compatibility_notice.is_none());
        assert!(check.update_notice.is_none());
    }

    #[test]
    fn parse_version_compatibility_success_preserves_deprecated_notice() {
        let body = br#"{
            "compatibility": {
                "status": "deprecated",
                "minimum_version": "0.12.1",
                "your_version": "0.12.0",
                "message": "Upgrade recommended soon.",
                "upgrade_deadline": "2026-05-31"
            }
        }"#;

        let check = parse_version_compatibility_success(body).expect("parse deprecated response");
        let notice = check.compatibility_notice.expect("deprecated notice");

        assert_eq!(notice.status, "deprecated");
        assert_eq!(notice.minimum_version, "0.12.1");
        assert_eq!(notice.your_version, "0.12.0");
        assert_eq!(notice.message, "Upgrade recommended soon.");
        assert_eq!(notice.upgrade_deadline.as_deref(), Some("2026-05-31"));
        assert!(check.update_notice.is_none());
    }

    #[test]
    fn parse_version_compatibility_success_preserves_available_update() {
        let body = br#"{
            "compatibility": {
                "status": "supported",
                "minimum_version": "",
                "your_version": "0.16.0",
                "message": ""
            },
            "update": {
                "available": true,
                "current_version": "0.16.0",
                "latest_version": "0.16.1",
                "latest_tag": "v0.16.1",
                "release_url": "https://github.com/scryer-media/scryer/releases/tag/v0.16.1",
                "published_at": "2026-06-14T12:00:00Z",
                "checked_at": "2026-06-15T12:00:00Z"
            }
        }"#;

        let check = parse_version_compatibility_success(body).expect("parse update response");
        let update = check.update_notice.expect("update notice");

        assert!(check.compatibility_notice.is_none());
        assert!(update.available);
        assert_eq!(update.current_version, "0.16.0");
        assert_eq!(update.latest_version, "0.16.1");
        assert_eq!(update.latest_tag, "v0.16.1");
        assert_eq!(
            update.release_url.as_deref(),
            Some("https://github.com/scryer-media/scryer/releases/tag/v0.16.1")
        );
        assert_eq!(update.published_at.as_deref(), Some("2026-06-14T12:00:00Z"));
        assert_eq!(update.checked_at, "2026-06-15T12:00:00Z");
    }

    #[test]
    fn parse_version_compatibility_success_clears_unavailable_update() {
        let body = br#"{
            "compatibility": {
                "status": "supported",
                "your_version": "0.16.1"
            },
            "update": {
                "available": false,
                "current_version": "0.16.1",
                "latest_version": "0.16.1",
                "latest_tag": "v0.16.1",
                "checked_at": "2026-06-15T12:00:00Z"
            }
        }"#;

        let check = parse_version_compatibility_success(body).expect("parse update response");

        assert!(check.compatibility_notice.is_none());
        assert!(check.update_notice.is_none());
    }

    #[test]
    fn parse_version_compatibility_incompatible_preserves_notice_and_update() {
        let body = br#"{
            "error": "version_incompatible",
            "status": "blocked",
            "minimum_version": "0.16.1",
            "your_version": "0.15.0",
            "message": "Upgrade required.",
            "upgrade_deadline": "2026-06-30",
            "update": {
                "available": true,
                "current_version": "0.15.0",
                "latest_version": "0.16.1",
                "latest_tag": "v0.16.1",
                "checked_at": "2026-06-15T12:00:00Z"
            }
        }"#;

        let check =
            parse_version_compatibility_incompatible(body).expect("parse incompatible response");
        let notice = check.compatibility_notice.expect("compatibility notice");
        let update = check.update_notice.expect("update notice");

        assert_eq!(notice.status, "blocked");
        assert_eq!(notice.minimum_version, "0.16.1");
        assert_eq!(notice.your_version, "0.15.0");
        assert_eq!(notice.message, "Upgrade required.");
        assert_eq!(notice.upgrade_deadline.as_deref(), Some("2026-06-30"));
        assert_eq!(update.latest_version, "0.16.1");
    }

    #[test]
    fn version_compatibility_error_kind_rejects_generic_validation_errors() {
        let body = br#"{
            "error": "invalid_request",
            "message": "version is required"
        }"#;

        assert!(!is_version_incompatible_response(body));
        assert!(is_version_incompatible_response(
            br#"{ "error": "version_incompatible" }"#
        ));
    }

    #[test]
    fn movie_response_deserializes_tmdb_id() {
        let data: super::MovieResponse = serde_json::from_value(json!({
            "movie": {
                "movie": {
                    "tvdb_id": 91_001,
                    "name": "External Movie",
                    "slug": "external-movie",
                    "year": 2026,
                    "status": "Released",
                    "overview": "External Movie overview",
                    "poster_url": "https://example.com/poster.jpg",
                    "language": "eng",
                    "runtime_minutes": 100,
                    "sort_title": "External Movie",
                    "imdb_id": "tt9100100",
                    "tmdb_id": 810_010,
                    "anidb_id": null,
                    "studio": "Test Studio",
                    "tmdb_release_date": "2026-01-01"
                }
            }
        }))
        .expect("movie response should deserialize");

        assert_eq!(data.movie.movie.tmdb_id, Some(810_010));
    }
}

#[derive(Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

// --- Discovery types ---

#[derive(Deserialize)]
struct DiscoverPublicFeedResponse {
    #[serde(rename = "discoverPublicFeed")]
    discover_public_feed: DiscoveryDashboardResult,
}

#[derive(Deserialize)]
struct TitleRecommendationsResponse {
    #[serde(rename = "titleRecommendations")]
    title_recommendations: DiscoveryRelatedResult,
}

#[derive(Deserialize)]
struct CollectionCompletionsResponse {
    #[serde(rename = "collectionCompletions")]
    collection_completions: DiscoveryCollectionCompletionResult,
}

#[derive(Deserialize)]
struct SubmitDiscoveryContextSnapshotResponse {
    #[serde(rename = "submitDiscoveryContextSnapshot")]
    submit_discovery_context_snapshot: DiscoveryContextSnapshotSubmitResult,
}

#[derive(Deserialize)]
struct DiscoveryContextSnapshotStatusResponse {
    #[serde(rename = "discoveryContextSnapshotStatus")]
    discovery_context_snapshot_status: DiscoveryContextSnapshotStatusResult,
}

#[derive(Deserialize)]
struct DiscoveryContextSnapshotPageResponse {
    #[serde(rename = "discoveryContextSnapshotPage")]
    discovery_context_snapshot_page: DiscoveryContextSnapshotPageResult,
}

#[derive(Deserialize)]
struct DiscoveryContextChangesResponse {
    #[serde(rename = "discoveryContextChanges")]
    discovery_context_changes: DiscoveryContextChangesResult,
}

#[derive(Deserialize)]
struct AcknowledgeDiscoveryContextSnapshotResponse {
    #[serde(rename = "acknowledgeDiscoveryContextSnapshot")]
    acknowledge_discovery_context_snapshot: DiscoveryContextSnapshotAckResult,
}

// --- Search types ---

#[derive(Deserialize)]
struct SearchTvdbResponse {
    #[serde(rename = "searchTvdb")]
    search_tvdb: SearchTvdbResult,
}

#[derive(Deserialize)]
struct SearchTvdbBatchResponse {
    #[serde(rename = "searchTvdbBatch")]
    search_tvdb_batch: Vec<SearchTvdbBatchResult>,
}

#[derive(Deserialize)]
struct SearchTvdbBatchResult {
    query: String,
    #[serde(rename = "type")]
    type_hint: Option<String>,
    year: Option<i32>,
    results: Vec<SearchTvdbItem>,
}

#[derive(Deserialize)]
struct SearchTvdbResult {
    results: Vec<SearchTvdbItem>,
}

#[derive(Deserialize)]
struct SearchTvdbItem {
    #[serde(rename = "tvdb_id")]
    tvdb_id: i64,
    name: String,
    year: Option<i32>,
    #[serde(default)]
    auto_match_safe: bool,
    #[serde(default)]
    auto_match_signals: Vec<String>,
}

fn validate_search_tvdb_batch_echo(
    expected: &MetadataSearchQuery,
    actual: &SearchTvdbBatchResult,
) -> AppResult<()> {
    if actual.query != expected.query
        || actual.type_hint.as_deref() != Some(expected.type_hint.as_str())
        || actual.year != expected.year
    {
        return Err(AppError::Repository(format!(
            "metadata gateway batch response mismatch: expected query={:?} type={:?} year={:?}, got query={:?} type={:?} year={:?}",
            expected.query,
            expected.type_hint,
            expected.year,
            actual.query,
            actual.type_hint,
            actual.year
        )));
    }

    Ok(())
}

#[derive(Deserialize)]
struct SearchTvdbRichItem {
    tvdb_id: i64,
    name: String,
    imdb_id: Option<String>,
    slug: Option<String>,
    #[serde(rename = "type")]
    type_hint: Option<String>,
    year: Option<i32>,
    status: Option<String>,
    overview: Option<String>,
    popularity: Option<f64>,
    poster_url: Option<String>,
    language: Option<String>,
    runtime_minutes: Option<i32>,
    sort_title: Option<String>,
}

#[derive(Deserialize)]
struct SearchTvdbRichResponse {
    #[serde(rename = "searchTvdb")]
    search_tvdb: SearchTvdbRichResult,
}

#[derive(Deserialize)]
struct SearchTvdbRichResult {
    results: Vec<SearchTvdbRichItem>,
}

// --- Multi-search types ---

#[derive(Deserialize)]
struct SearchTvdbMultiResponse {
    #[serde(rename = "searchTvdbMulti")]
    search_tvdb_multi: SearchTvdbMultiResult,
}

#[derive(Deserialize)]
struct SearchTvdbMultiResult {
    movies: Vec<SearchTvdbRichItem>,
    series: Vec<SearchTvdbRichItem>,
    anime: Vec<SearchTvdbRichItem>,
}

// --- Movie types ---

#[derive(Deserialize)]
struct ArtworkMovieResult {
    movie: ArtworkTitleItem,
}

#[derive(Deserialize)]
struct ArtworkTitleItem {
    tvdb_id: i64,
    poster_url: String,
    #[serde(default)]
    artworks: Vec<ArtworkItem>,
}

#[derive(Deserialize)]
struct MetadataBulkResponse {
    #[serde(rename = "metadataBulk")]
    metadata_bulk: MetadataBulkResult,
}

#[derive(Deserialize)]
struct MetadataBulkResult {
    movies: Vec<MovieItem>,
    series: Vec<SeriesItem>,
}

#[derive(Deserialize)]
struct MovieResponse {
    movie: MovieResult,
}

#[derive(Deserialize)]
struct MovieResult {
    movie: MovieItem,
}

#[derive(Deserialize)]
struct MovieItem {
    tvdb_id: i64,
    name: String,
    slug: String,
    year: Option<i32>,
    status: String,
    overview: String,
    poster_url: String,
    language: String,
    runtime_minutes: i32,
    sort_title: String,
    imdb_id: String,
    #[serde(default)]
    tmdb_id: Option<i64>,
    #[serde(default)]
    tmdb_popularity: Option<f64>,
    #[serde(default)]
    anidb_id: Option<i64>,
    #[serde(default)]
    canonical_tags: Vec<CanonicalTagItem>,
    studio: String,
    tmdb_release_date: Option<String>,
    #[serde(default)]
    rating: Option<f64>,
    #[serde(default)]
    rating_sources: Vec<String>,
    #[serde(default)]
    external_ratings: Vec<ExternalRatingItem>,
    #[serde(default)]
    artworks: Vec<ArtworkItem>,
}

#[derive(Clone, Debug, Deserialize)]
struct ExternalRatingItem {
    source: String,
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    score: Option<f64>,
    normalized: f64,
    #[serde(default)]
    votes: Option<i32>,
    url: String,
}

#[derive(Clone, Debug, Deserialize)]
struct CanonicalTagItem {
    key: String,
    category: String,
    name: String,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default)]
    source_tag_keys: Vec<String>,
    #[serde(default)]
    is_adult: bool,
    #[serde(default)]
    is_spoiler: bool,
}

fn rating_summary_from_gateway(
    rating: Option<f64>,
    rating_sources: Vec<String>,
    external_ratings: Vec<ExternalRatingItem>,
) -> TitleRatingSummary {
    let rating_sources = rating_sources
        .into_iter()
        .filter_map(|source| {
            let source = source.trim();
            (!source.is_empty()).then(|| source.to_string())
        })
        .collect();
    let external_ratings = external_ratings
        .into_iter()
        .filter_map(|rating| {
            let source = rating.source.trim();
            if source.is_empty() {
                return None;
            }
            Some(TitleExternalRating {
                source: source.to_string(),
                value: rating.value,
                score: rating.score,
                normalized: rating.normalized,
                votes: rating.votes,
                url: rating.url.trim().to_string(),
            })
        })
        .collect();
    TitleRatingSummary {
        rating,
        rating_sources,
        external_ratings,
    }
}

fn canonical_tags_from_gateway(items: Vec<CanonicalTagItem>) -> Vec<CanonicalMediaTag> {
    items
        .into_iter()
        .filter_map(|item| {
            let key = item.key.trim();
            let category = item.category.trim();
            let name = item.name.trim();
            if key.is_empty() || category.is_empty() || name.is_empty() {
                return None;
            }

            Some(CanonicalMediaTag {
                key: key.to_string(),
                category: category.to_string(),
                name: name.to_string(),
                confidence: item.confidence.filter(|value| value.is_finite()),
                sources: item.sources,
                source_tag_keys: item.source_tag_keys,
                is_adult: item.is_adult,
                is_spoiler: item.is_spoiler,
            })
        })
        .collect()
}

// --- Artwork helper ---

#[derive(Deserialize)]
struct ArtworkItem {
    kind: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct TaggedAliasItem {
    name: String,
    language: String,
}

fn pick_artwork_url(artworks: &[ArtworkItem], kind: &str) -> Option<String> {
    artworks
        .iter()
        .filter(|a| a.kind == kind)
        .find_map(|a| normalize_optional_artwork_url(Some(a.url.clone())))
}

fn normalize_optional_artwork_url(url: Option<String>) -> Option<String> {
    url.and_then(|value| {
        let normalized = normalize_artwork_url(&value);
        (!normalized.trim().is_empty()).then_some(normalized)
    })
}

fn bounded_exponential_backoff(attempt: u32, base: Duration, max: Duration) -> Duration {
    let multiplier = 1u32 << attempt.min(4);
    let delay = base.saturating_mul(multiplier);
    if delay > max { max } else { delay }
}

fn enrollment_retry_delay(error: &smg_enrollment::EnrollmentError, attempt: u32) -> Duration {
    if let smg_enrollment::EnrollmentError::RateLimited(rate_limited) = error
        && let Some(retry_after) = rate_limited.retry_after
        && !retry_after.is_zero()
    {
        return retry_after;
    }

    bounded_exponential_backoff(attempt, Duration::from_secs(30), Duration::from_secs(300))
}

fn metadata_gateway_transient_delay(attempt: u32) -> Duration {
    bounded_exponential_backoff(
        attempt,
        METADATA_GATEWAY_TRANSIENT_BASE_DELAY,
        METADATA_GATEWAY_TRANSIENT_MAX_DELAY,
    )
}

fn map_metadata_gateway_outbound_error(request_label: &str, error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => {
            let retry_after_seconds = rate_limited
                .retry_after
                .filter(|delay| !delay.is_zero())
                .map(|delay| delay.as_secs());
            let message = match retry_after_seconds {
                Some(seconds) if seconds > 0 => format!(
                    "{request_label} was rate limited by the metadata gateway; retry after {seconds}s"
                ),
                _ => format!("{request_label} was rate limited by the metadata gateway"),
            };
            AppError::rate_limited_temporary_unavailable(
                message,
                rate_limited.retry_after.filter(|delay| !delay.is_zero()),
                RateLimitCooldownAction::AlreadyRecorded,
            )
        }
        OutboundHttpError::Transport { source, .. } => AppError::Repository(format!(
            "{request_label} failed: {}",
            outbound_transport_error_message(&source)
        )),
    }
}

fn outbound_transport_error_message(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(error) = source {
        let _ = write!(message, ": {error}");
        source = error.source();
    }
    message
}

fn normalize_artwork_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let Ok(mut parsed) = reqwest::Url::parse(trimmed) else {
        return trimmed.to_string();
    };

    let normalized_path = parsed
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    parsed.set_path(&format!("/{normalized_path}"));

    parsed.to_string()
}

// --- Series types ---

#[derive(Deserialize)]
struct ArtworkSeriesResult {
    series: ArtworkSeriesItem,
}

#[derive(Deserialize)]
struct ArtworkSeriesItem {
    tvdb_id: i64,
    poster_url: String,
    #[serde(default)]
    artworks: Vec<ArtworkItem>,
    #[serde(default)]
    episodes: Vec<ArtworkEpisodeItem>,
}

#[derive(Deserialize)]
struct ArtworkEpisodeItem {
    tvdb_id: i64,
    season_number: i32,
    episode_number: i32,
    #[serde(default)]
    image_url: Option<String>,
}

#[derive(Deserialize)]
struct SeriesResponse {
    series: SeriesResult,
}

#[derive(Deserialize)]
struct SeriesResult {
    series: SeriesItem,
}

#[derive(Deserialize)]
struct SeriesItem {
    tvdb_id: i64,
    name: String,
    sort_name: String,
    slug: String,
    status: String,
    year: Option<i32>,
    first_aired: String,
    overview: String,
    network: String,
    runtime_minutes: i32,
    poster_url: String,
    country: String,
    #[serde(default)]
    canonical_tags: Vec<CanonicalTagItem>,
    #[serde(default)]
    rating: Option<f64>,
    #[serde(default)]
    rating_sources: Vec<String>,
    #[serde(default)]
    external_ratings: Vec<ExternalRatingItem>,
    aliases: Vec<String>,
    #[serde(default)]
    tagged_aliases: Vec<TaggedAliasItem>,
    #[serde(default)]
    artworks: Vec<ArtworkItem>,
    seasons: Vec<SeriesSeasonItem>,
    episodes: Vec<SeriesEpisodeItem>,
    #[serde(default)]
    anime_mappings: Vec<AnimeMappingItem>,
    #[serde(default)]
    anime_movies: Vec<AnimeMovieItem>,
}

#[derive(Deserialize)]
struct SeriesSeasonItem {
    tvdb_id: i64,
    number: i32,
    label: String,
    episode_type: String,
}

#[derive(Deserialize)]
struct SeriesEpisodeItem {
    tvdb_id: i64,
    episode_number: i32,
    season_number: i32,
    name: String,
    aired: String,
    runtime_minutes: i32,
    is_filler: bool,
    is_recap: bool,
    overview: String,
    absolute_number: String,
    #[serde(default)]
    image_url: String,
}

#[derive(Deserialize)]
struct AnimeMappingItem {
    mal_id: Option<i64>,
    mal_dub_id: Option<i64>,
    anilist_id: Option<i64>,
    anidb_id: Option<i64>,
    kitsu_id: Option<i64>,
    simkl_id: Option<i64>,
    thetvdb_id: Option<i64>,
    themoviedb_id: Option<i64>,
    imdb_id: Option<i64>,
    trakt_id: Option<i64>,
    alt_tvdb_id: Option<i64>,
    thetvdb_season: Option<i32>,
    thetvdb_part: Option<i32>,
    score: Option<f64>,
    anime_media_type: Option<String>,
    global_media_type: Option<String>,
    status: Option<String>,
    #[serde(default)]
    mapping_type: Option<String>,
    #[serde(default)]
    episode_mappings: Vec<AnimeEpisodeMappingItem>,
}

#[derive(Deserialize)]
struct AnimeEpisodeMappingItem {
    tvdb_season: i32,
    episode_start: i32,
    episode_end: i32,
}

#[derive(Deserialize)]
struct AnimeMovieItem {
    movie_tvdb_id: Option<i64>,
    movie_tmdb_id: Option<i64>,
    movie_imdb_id: Option<String>,
    movie_mal_id: Option<i64>,
    #[serde(default)]
    movie_anidb_id: Option<i64>,
    name: String,
    slug: String,
    year: Option<i32>,
    content_status: String,
    overview: String,
    poster_url: String,
    language: String,
    runtime_minutes: i32,
    sort_title: String,
    imdb_id: String,
    studio: String,
    digital_release_date: Option<String>,
    association_confidence: String,
    continuity_status: String,
    movie_form: String,
    placement: String,
    confidence: String,
    signal_summary: String,
}

#[async_trait]
impl MetadataGateway for MetadataGatewayClient {
    async fn search_tvdb(
        &self,
        query: &str,
        type_hint: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        let variables = json!({
            "query": query,
            "type": type_hint,
            "limit": 10,
            "year": year,
        });

        let data: SearchTvdbResponse = self
            .execute_graphql_apq(
                OP_SEARCH_TVDB,
                graphql_docs::SEARCH_TVDB_QUERY,
                &self.search_hash,
                variables,
            )
            .await?;

        Ok(data
            .search_tvdb
            .results
            .into_iter()
            .map(|item| MetadataSearchItem {
                tvdb_id: item.tvdb_id.to_string(),
                name: item.name,
                year: item.year,
                auto_match_safe: item.auto_match_safe,
                auto_match_signals: item.auto_match_signals,
            })
            .collect())
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        let deduped_queries = build_search_tvdb_batch_query(queries);

        if deduped_queries.is_empty() {
            return Ok(HashMap::new());
        }

        let mut results = HashMap::new();

        for chunk in deduped_queries.chunks(METADATA_GATEWAY_MAX_SEARCH_BATCH) {
            let request_started_at = Instant::now();
            debug!(
                query_count = chunk.len(),
                "metadata gateway batched search request"
            );
            let request_inputs = chunk
                .iter()
                .map(|query| SearchTvdbBatchRequestInput {
                    query: query.query.clone(),
                    type_hint: query.type_hint.clone(),
                    year: query.year,
                    imdb_id: query.imdb_id.clone(),
                    tmdb_id: query.tmdb_id.clone(),
                    tvdb_id: query.tvdb_id.clone(),
                    limit: 10,
                })
                .collect::<Vec<_>>();
            let payload = json!({
                "query": graphql_docs::SEARCH_TVDB_BATCH_QUERY,
                "variables": {
                    "requests": request_inputs,
                    "language": language,
                },
            });
            let data: SearchTvdbBatchResponse = self.execute_graphql(payload).await?;
            let elapsed_ms = request_started_at.elapsed().as_millis() as u64;
            debug!(
                query_count = chunk.len(),
                elapsed_ms, "metadata gateway batched search complete"
            );
            if data.search_tvdb_batch.len() != chunk.len() {
                return Err(AppError::Repository(format!(
                    "metadata gateway batch response length mismatch: requested {}, got {}",
                    chunk.len(),
                    data.search_tvdb_batch.len()
                )));
            }

            let exact_id_count = chunk
                .iter()
                .filter(|query| {
                    query.query.trim().is_empty()
                        && (query.imdb_id.is_some()
                            || query.tmdb_id.is_some()
                            || query.tvdb_id.is_some())
                })
                .count();
            let mut hit_count = 0usize;
            let mut safe_hit_count = 0usize;
            for (query_spec, item) in chunk.iter().cloned().zip(data.search_tvdb_batch) {
                validate_search_tvdb_batch_echo(&query_spec, &item)?;
                let items = item
                    .results
                    .into_iter()
                    .map(|entry| MetadataSearchItem {
                        tvdb_id: entry.tvdb_id.to_string(),
                        name: entry.name,
                        year: entry.year,
                        auto_match_safe: entry.auto_match_safe,
                        auto_match_signals: entry.auto_match_signals,
                    })
                    .collect::<Vec<_>>();
                if !items.is_empty() {
                    hit_count = hit_count.saturating_add(1);
                }
                if items.iter().any(|item| item.auto_match_safe) {
                    safe_hit_count = safe_hit_count.saturating_add(1);
                }
                results.insert(query_spec, items);
            }

            tracing::info!(
                target: "import_scan_hint_debug",
                request_count = chunk.len(),
                exact_id_count,
                fuzzy_count = chunk.len().saturating_sub(exact_id_count),
                hit_count,
                safe_hit_count,
                elapsed_ms,
                "smg batch search result",
            );

            for query in chunk {
                results.entry(query.clone()).or_default();
            }
        }

        Ok(results)
    }

    async fn search_tvdb_rich(
        &self,
        query: &str,
        type_hint: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        let variables = json!({
            "query": query,
            "type": type_hint,
            "limit": limit,
            "language": language,
            "year": year,
        });

        let data: SearchTvdbRichResponse = self
            .execute_graphql_apq(
                OP_SEARCH_TVDB_RICH,
                graphql_docs::SEARCH_TVDB_RICH_QUERY,
                &self.search_rich_hash,
                variables,
            )
            .await?;

        Ok(data
            .search_tvdb
            .results
            .into_iter()
            .map(|item| RichMetadataSearchItem {
                tvdb_id: item.tvdb_id.to_string(),
                name: item.name,
                imdb_id: item.imdb_id,
                slug: item.slug,
                type_hint: item.type_hint,
                year: item.year,
                status: item.status,
                overview: item.overview,
                popularity: item.popularity,
                poster_url: normalize_optional_artwork_url(item.poster_url),
                language: item.language,
                runtime_minutes: item.runtime_minutes,
                sort_title: item.sort_title,
            })
            .collect())
    }

    async fn search_tvdb_multi(
        &self,
        query: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        let variables = json!({
            "query": query,
            "limit": limit,
            "language": language,
        });

        let data: SearchTvdbMultiResponse = self
            .execute_graphql_apq(
                OP_SEARCH_TVDB_MULTI,
                graphql_docs::SEARCH_TVDB_MULTI_QUERY,
                &self.search_multi_hash,
                variables,
            )
            .await?;

        let convert = |items: Vec<SearchTvdbRichItem>| -> Vec<RichMetadataSearchItem> {
            items
                .into_iter()
                .map(|item| RichMetadataSearchItem {
                    tvdb_id: item.tvdb_id.to_string(),
                    name: item.name,
                    imdb_id: item.imdb_id,
                    slug: item.slug,
                    type_hint: item.type_hint,
                    year: item.year,
                    status: item.status,
                    overview: item.overview,
                    popularity: item.popularity,
                    poster_url: normalize_optional_artwork_url(item.poster_url),
                    language: item.language,
                    runtime_minutes: item.runtime_minutes,
                    sort_title: item.sort_title,
                })
                .collect()
        };

        Ok(MultiMetadataSearchResult {
            movies: convert(data.search_tvdb_multi.movies),
            series: convert(data.search_tvdb_multi.series),
            anime: convert(data.search_tvdb_multi.anime),
        })
    }

    async fn get_movie(&self, tvdb_id: i64, language: &str) -> AppResult<MovieMetadata> {
        let variables = json!({
            "tvdbId": tvdb_id,
            "language": language,
        });

        let data: MovieResponse = self
            .execute_graphql_apq(
                OP_GET_MOVIE,
                graphql_docs::GET_MOVIE_QUERY,
                &self.movie_hash,
                variables,
            )
            .await?;
        let m = data.movie.movie;

        Ok(MovieMetadata {
            target_key: None,
            tvdb_id: m.tvdb_id,
            name: m.name,
            slug: m.slug,
            year: m.year,
            content_status: m.status,
            overview: m.overview,
            poster_url: normalize_artwork_url(&m.poster_url),
            background_url: pick_artwork_url(&m.artworks, "background"),
            language: m.language,
            runtime_minutes: m.runtime_minutes,
            sort_title: m.sort_title,
            imdb_id: m.imdb_id,
            tmdb_id: m.tmdb_id,
            popularity: m.tmdb_popularity,
            anidb_id: m.anidb_id,
            canonical_tags: canonical_tags_from_gateway(m.canonical_tags),
            studio: m.studio,
            tmdb_release_date: m.tmdb_release_date,
            ratings: rating_summary_from_gateway(m.rating, m.rating_sources, m.external_ratings),
        })
    }

    async fn get_series(&self, tvdb_id: i64, language: &str) -> AppResult<SeriesMetadata> {
        let variables = json!({
            "id": tvdb_id.to_string(),
            "includeEpisodes": true,
            "language": language,
        });

        let data: SeriesResponse = self
            .execute_graphql_apq(
                OP_GET_SERIES,
                graphql_docs::GET_SERIES_QUERY,
                &self.series_hash,
                variables,
            )
            .await?;
        let s = data.series.series;

        Ok(SeriesMetadata {
            target_key: None,
            tvdb_id: s.tvdb_id,
            name: s.name,
            sort_name: s.sort_name,
            slug: s.slug,
            year: s.year,
            content_status: s.status,
            first_aired: s.first_aired,
            overview: s.overview,
            network: s.network,
            runtime_minutes: s.runtime_minutes,
            poster_url: normalize_artwork_url(&s.poster_url),
            background_url: pick_artwork_url(&s.artworks, "background"),
            country: s.country,
            canonical_tags: canonical_tags_from_gateway(s.canonical_tags),
            aliases: s.aliases,
            tagged_aliases: s
                .tagged_aliases
                .into_iter()
                .map(|ta| scryer_domain::TaggedAlias {
                    name: ta.name,
                    language: ta.language,
                })
                .collect(),
            seasons: s
                .seasons
                .into_iter()
                .map(|season| SeasonMetadata {
                    tvdb_id: season.tvdb_id,
                    number: season.number,
                    label: season.label,
                    episode_type: season.episode_type,
                })
                .collect(),
            episodes: s
                .episodes
                .into_iter()
                .map(|ep| EpisodeMetadata {
                    tvdb_id: ep.tvdb_id,
                    episode_number: ep.episode_number,
                    name: ep.name,
                    aired: ep.aired,
                    runtime_minutes: ep.runtime_minutes,
                    is_filler: ep.is_filler,
                    is_recap: ep.is_recap,
                    overview: ep.overview,
                    absolute_number: ep.absolute_number,
                    season_number: ep.season_number,
                    image_url: ep.image_url,
                })
                .collect(),
            anime_mappings: s
                .anime_mappings
                .into_iter()
                .map(|m| AnimeMapping {
                    mal_id: m.mal_id,
                    mal_dub_id: m.mal_dub_id,
                    anilist_id: m.anilist_id,
                    anidb_id: m.anidb_id,
                    kitsu_id: m.kitsu_id,
                    simkl_id: m.simkl_id,
                    thetvdb_id: m.thetvdb_id,
                    themoviedb_id: m.themoviedb_id,
                    imdb_id: m.imdb_id,
                    trakt_id: m.trakt_id,
                    alt_tvdb_id: m.alt_tvdb_id,
                    thetvdb_season: m.thetvdb_season,
                    thetvdb_part: m.thetvdb_part,
                    score: m.score,
                    anime_media_type: m.anime_media_type.unwrap_or_default(),
                    global_media_type: m.global_media_type.unwrap_or_default(),
                    status: m.status.unwrap_or_default(),
                    mapping_type: m.mapping_type.unwrap_or_default(),
                    episode_mappings: m
                        .episode_mappings
                        .into_iter()
                        .map(|e| AnimeEpisodeMapping {
                            tvdb_season: e.tvdb_season,
                            episode_start: e.episode_start,
                            episode_end: e.episode_end,
                        })
                        .collect(),
                })
                .collect(),
            anime_movies: s
                .anime_movies
                .into_iter()
                .map(|movie| AnimeMovie {
                    movie_tvdb_id: movie.movie_tvdb_id,
                    movie_tmdb_id: movie.movie_tmdb_id,
                    movie_imdb_id: movie.movie_imdb_id,
                    movie_mal_id: movie.movie_mal_id,
                    movie_anidb_id: movie.movie_anidb_id,
                    name: movie.name,
                    slug: movie.slug,
                    year: movie.year,
                    content_status: movie.content_status,
                    overview: movie.overview,
                    poster_url: movie.poster_url,
                    language: movie.language,
                    runtime_minutes: movie.runtime_minutes,
                    sort_title: movie.sort_title,
                    imdb_id: movie.imdb_id,
                    studio: movie.studio,
                    digital_release_date: movie.digital_release_date,
                    association_confidence: movie.association_confidence,
                    continuity_status: movie.continuity_status,
                    movie_form: movie.movie_form,
                    placement: movie.placement,
                    confidence: movie.confidence,
                    signal_summary: movie.signal_summary,
                })
                .collect(),
            ratings: rating_summary_from_gateway(s.rating, s.rating_sources, s.external_ratings),
        })
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        series_tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<BulkMetadataResult> {
        if movie_tvdb_ids.is_empty() && series_tvdb_ids.is_empty() {
            return Ok(BulkMetadataResult::default());
        }

        let unique_movies: Vec<i64> = movie_tvdb_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let unique_series: Vec<i64> = series_tvdb_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let request_started_at = Instant::now();

        debug!(
            movies = unique_movies.len(),
            series = unique_series.len(),
            "bulk metadata request"
        );

        let result = self
            .get_metadata_bulk_via_metadata_bulk(&unique_movies, &unique_series, language)
            .await?;
        debug!(
            movies_resolved = result.movies.len(),
            series_resolved = result.series.len(),
            elapsed_ms = request_started_at.elapsed().as_millis() as u64,
            "bulk metadata complete"
        );
        Ok(result)
    }

    async fn discover_public_feed(
        &self,
        input: &DiscoveryPublicFeedInput,
    ) -> AppResult<DiscoveryDashboardResult> {
        let data: DiscoverPublicFeedResponse = self
            .execute_public_graphql_get(
                OP_DISCOVER_PUBLIC_FEED,
                graphql_docs::DISCOVER_PUBLIC_FEED_QUERY,
                json!({ "input": input }),
            )
            .await?;
        let mut result = data.discover_public_feed;
        for section in &mut result.sections {
            for item in &mut section.items {
                item.apply_rating_provenance();
            }
        }
        Ok(result)
    }

    async fn title_recommendations(
        &self,
        input: &TitleRecommendationsInput,
    ) -> AppResult<DiscoveryRelatedResult> {
        let data: TitleRecommendationsResponse = self
            .execute_graphql_apq_post(
                OP_TITLE_RECOMMENDATIONS,
                graphql_docs::TITLE_RECOMMENDATIONS_QUERY,
                &self.title_recommendations_hash,
                json!({
                    "subject": input.subject,
                    "query": input.query,
                    "limit": input.limit,
                    "language": input.language,
                    "includeUnresolved": input.include_unresolved,
                }),
            )
            .await?;
        let mut title_recommendations = data.title_recommendations;
        for result in &mut title_recommendations.results {
            result.apply_rating_provenance();
        }
        Ok(title_recommendations)
    }

    async fn collection_completions(
        &self,
        input: &DiscoveryCollectionCompletionInput,
    ) -> AppResult<DiscoveryCollectionCompletionResult> {
        let data: CollectionCompletionsResponse = self
            .execute_graphql_apq_post(
                OP_COLLECTION_COMPLETIONS,
                graphql_docs::COLLECTION_COMPLETIONS_QUERY,
                &self.collection_completions_hash,
                json!({ "input": input }),
            )
            .await?;
        let mut result = data.collection_completions;
        for item in &mut result.results {
            item.apply_rating_provenance();
        }
        Ok(result)
    }

    async fn submit_discovery_context_snapshot(
        &self,
        input: &DiscoveryContextSnapshotSubmitInput,
    ) -> AppResult<DiscoveryContextSnapshotSubmitResult> {
        let data: SubmitDiscoveryContextSnapshotResponse = self
            .execute_graphql_apq_post(
                OP_SUBMIT_DISCOVERY_CONTEXT_SNAPSHOT,
                graphql_docs::SUBMIT_DISCOVERY_CONTEXT_SNAPSHOT_QUERY,
                &self.submit_discovery_context_snapshot_hash,
                json!({ "input": input }),
            )
            .await?;
        Ok(data.submit_discovery_context_snapshot)
    }

    async fn discovery_context_snapshot_status(
        &self,
        request_id: &str,
    ) -> AppResult<DiscoveryContextSnapshotStatusResult> {
        let data: DiscoveryContextSnapshotStatusResponse = self
            .execute_graphql_apq_post(
                OP_DISCOVERY_CONTEXT_SNAPSHOT_STATUS,
                graphql_docs::DISCOVERY_CONTEXT_SNAPSHOT_STATUS_QUERY,
                &self.discovery_context_snapshot_status_hash,
                json!({ "requestId": request_id }),
            )
            .await?;
        Ok(data.discovery_context_snapshot_status)
    }

    async fn discovery_context_snapshot_page(
        &self,
        request_id: &str,
        page: i32,
    ) -> AppResult<DiscoveryContextSnapshotPageResult> {
        let data: DiscoveryContextSnapshotPageResponse = self
            .execute_graphql_apq_post(
                OP_DISCOVERY_CONTEXT_SNAPSHOT_PAGE,
                graphql_docs::DISCOVERY_CONTEXT_SNAPSHOT_PAGE_QUERY,
                &self.discovery_context_snapshot_page_hash,
                json!({ "requestId": request_id, "page": page }),
            )
            .await?;
        let mut result = data.discovery_context_snapshot_page;
        for item in &mut result.items {
            item.apply_rating_provenance();
        }
        Ok(result)
    }

    async fn discovery_context_changes(
        &self,
        input: &DiscoveryContextChangesInput,
    ) -> AppResult<DiscoveryContextChangesResult> {
        let data: DiscoveryContextChangesResponse = self
            .execute_graphql_apq_post(
                OP_DISCOVERY_CONTEXT_CHANGES,
                graphql_docs::DISCOVERY_CONTEXT_CHANGES_QUERY,
                &self.discovery_context_changes_hash,
                json!({ "input": input }),
            )
            .await?;
        let mut result = data.discovery_context_changes;
        for item in &mut result.items {
            item.apply_rating_provenance();
        }
        Ok(result)
    }

    async fn acknowledge_discovery_context_snapshot(
        &self,
        request_id: &str,
    ) -> AppResult<DiscoveryContextSnapshotAckResult> {
        let data: AcknowledgeDiscoveryContextSnapshotResponse = self
            .execute_graphql_apq_post(
                OP_ACKNOWLEDGE_DISCOVERY_CONTEXT_SNAPSHOT,
                graphql_docs::ACKNOWLEDGE_DISCOVERY_CONTEXT_SNAPSHOT_QUERY,
                &self.acknowledge_discovery_context_snapshot_hash,
                json!({ "requestId": request_id }),
            )
            .await?;
        Ok(data.acknowledge_discovery_context_snapshot)
    }

    async fn get_artwork_urls_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        series_tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<BulkArtworkUrlResult> {
        if movie_tvdb_ids.is_empty() && series_tvdb_ids.is_empty() {
            return Ok(BulkArtworkUrlResult::default());
        }

        let unique_movies: Vec<i64> = movie_tvdb_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let unique_series: Vec<i64> = series_tvdb_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let request_started_at = Instant::now();
        debug!(
            movies = unique_movies.len(),
            series = unique_series.len(),
            "bulk artwork url request"
        );

        let mut movies = HashMap::new();
        let mut series = HashMap::new();
        let bulk_requests = build_bulk_metadata_alias_requests(&unique_movies, &unique_series);
        for chunk in bulk_requests.chunks(METADATA_GATEWAY_MAX_BULK_METADATA_ALIAS_BATCH) {
            let mut chunk_movie_ids = Vec::new();
            let mut chunk_series_ids = Vec::new();
            for request in chunk {
                match request {
                    BulkMetadataAliasRequest::Movie(tvdb_id) => chunk_movie_ids.push(*tvdb_id),
                    BulkMetadataAliasRequest::Series(tvdb_id) => chunk_series_ids.push(*tvdb_id),
                }
            }

            let query = build_bulk_artwork_url_query(&chunk_movie_ids, &chunk_series_ids, language);
            let data = self.post_batched_graphql_partial(&query).await?;
            merge_bulk_artwork_url_partial(&data, &mut movies, &mut series);
        }

        debug!(
            movies_resolved = movies.len(),
            series_resolved = series.len(),
            elapsed_ms = request_started_at.elapsed().as_millis() as u64,
            "bulk artwork url request complete"
        );

        Ok(BulkArtworkUrlResult { movies, series })
    }
}
