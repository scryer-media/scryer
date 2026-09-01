use async_graphql::http::ALL_WEBSOCKET_PROTOCOLS;
use async_graphql::{
    Data, ErrorExtensionValues, Executor, Request as GraphQLRequest, Response as GraphQLResponse,
    ServerError,
};
use async_graphql_axum::{GraphQLProtocol, GraphQLWebSocket};
use aws_lc_rs::hmac;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path as AxumPath, State, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use futures_util::{
    Stream, StreamExt,
    stream::{self, BoxStream},
};
use scryer_application::{
    API_KEY_PREFIX, AppError, AppResult, AppUseCase, AuthenticatedTokenClaims, JwtSessionScope,
    OAuthAuthorizationSource,
};
use scryer_domain::{ActorCapabilityMask, AppPermissionMask, Id};
use scryer_interface::RequestLoaders;
use scryer_interface::context::{
    ApiKeyManagementSession, AuthRuntimeStateHandle, AuthlessDefaultSession, ConnectionAuthEpoch,
    InteractiveSession, LoginAttemptLimiter, MfaVerification, OAuthActorSession,
    RequestSessionPersistence,
};
use scryer_logging::{ActorContext, LogContext, RequestContext, context_span, update_context};
use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, RwLock as StdRwLock};
use std::task::{Context as TaskContext, Poll};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{Instrument, warn};
use uuid::Uuid;

use crate::base_path::BasePath;
use crate::http_error::ErrorResponse;
use crate::rate_limit::{
    GraphqlRateLimitClass, HttpRateLimitClass, RateLimitKey, ScryerRateLimiter,
    analyze_authentication_request, classify_graphql, classify_graphql_request,
    rate_limited_graphql_error, rate_limited_graphql_response,
    rate_limited_graphql_single_response,
};

const X_FORWARDED_PROTO: &str = "x-forwarded-proto";
pub(crate) const GRAPHQL_MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const GRAPHQL_MAX_BATCH_OPERATIONS: usize = 10;
const GRAPHQL_POST_EXECUTION_TIMEOUT_CODE: &str = "GRAPHQL_EXECUTION_TIMEOUT";
const AUTHENTICATION_REQUIRED_CODE: &str = "AUTHENTICATION_REQUIRED";
const MFA_STEP_UP_REQUIRED_CODE: &str = "MFA_STEP_UP_REQUIRED";
const MFA_STEP_UP_REQUIRED_STATUS_CODE: u16 = 460;
const INTERNAL_SERVER_ERROR_MESSAGE: &str = "Internal server error";
const CORS_ALLOWED_ORIGINS_ENV: &str = "SCRYER_CORS_ALLOWED_ORIGINS";
const WS_ALLOWED_ORIGINS_ENV: &str = "SCRYER_WS_ALLOWED_ORIGINS";
#[cfg(test)]
const PRODUCTION_CORS_OPT_IN_ENV: &str = "SCRYER_ENABLE_PRODUCTION_CORS";
const WEB_UI_URL_ENV: &str = "SCRYER_WEB_UI_URL";
pub(crate) const UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST_ENV: &str =
    "SCRYER_UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST";
const AUTHLESS_WEB_CLIENT_HEADER: &str = "x-scryer-web-client";
const AUTHLESS_WEB_CLIENT_COOKIE: &str = "scryer_authless_client";
const AUTHLESS_WEB_CLIENT_TTL_SECONDS: u64 = 5 * 60;
const AUTHLESS_ACCESS_ALLOWLIST_DNS_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const AUTHLESS_ACCESS_ALLOWLIST_DNS_NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AuthlessAccessPolicy {
    pub(crate) allow_unauthenticated_public_access: bool,
}

#[derive(Clone)]
pub(crate) struct AuthlessAccessGuardState {
    pub(crate) auth_runtime: AuthRuntimeStateHandle,
    pub(crate) policy: AuthlessAccessPolicy,
    pub(crate) allowlist: AuthlessAccessAllowlist,
}

#[derive(Clone)]
pub(crate) struct AuthlessWebClientProofState {
    secret: Arc<Vec<u8>>,
}

#[derive(Clone)]
pub(crate) struct AuthlessWebClientProofRouteState {
    pub(crate) auth_runtime: AuthRuntimeStateHandle,
    pub(crate) policy: AuthlessAccessPolicy,
    pub(crate) proof: AuthlessWebClientProofState,
    pub(crate) allowlist: AuthlessAccessAllowlist,
}

#[derive(Clone)]
pub(crate) struct AuthlessAccessAllowlist {
    inner: Arc<AuthlessAccessAllowlistInner>,
}

struct AuthlessAccessAllowlistInner {
    ip_matchers: Vec<AuthlessIpMatcher>,
    dns_hosts: Vec<String>,
    dns_cache: RwLock<HashMap<String, CachedDnsAllowlistEntry>>,
}

#[derive(Clone)]
struct CachedDnsAllowlistEntry {
    ips: Vec<IpAddr>,
    expires_at: Instant,
}

#[derive(Clone, Copy)]
enum AuthlessIpMatcher {
    Exact(IpAddr),
    Cidr(IpAddr, u8),
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthlessWebClientProofResponse {
    proof: String,
    expires_at: u64,
}

impl AuthlessAccessAllowlist {
    pub(crate) fn parse(raw: &str) -> Self {
        let mut ip_matchers = Vec::new();
        let mut dns_hosts = Vec::new();

        for entry in raw
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            if let Some(matcher) = parse_authless_ip_matcher(entry) {
                ip_matchers.push(matcher);
            } else if let Some(host) = parse_authless_allowlist_host(entry) {
                dns_hosts.push(host);
            } else {
                warn!(
                    env = UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST_ENV,
                    value = entry,
                    "ignoring invalid unauthenticated public access allowlist entry"
                );
            }
        }

        dns_hosts.sort();
        dns_hosts.dedup();

        Self {
            inner: Arc::new(AuthlessAccessAllowlistInner {
                ip_matchers,
                dns_hosts,
                dns_cache: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub(crate) fn is_configured(&self) -> bool {
        !self.inner.ip_matchers.is_empty() || !self.inner.dns_hosts.is_empty()
    }

    #[cfg(test)]
    async fn cache_host_for_test(&self, host: &str, ips: Vec<IpAddr>) {
        self.inner.dns_cache.write().await.insert(
            host.to_string(),
            CachedDnsAllowlistEntry {
                ips,
                expires_at: Instant::now() + AUTHLESS_ACCESS_ALLOWLIST_DNS_CACHE_TTL,
            },
        );
    }

    async fn allows_public_ip(&self, ip: IpAddr) -> bool {
        if self
            .inner
            .ip_matchers
            .iter()
            .any(|matcher| matcher.matches(ip))
        {
            return true;
        }

        let dns_hosts = self.inner.dns_hosts.clone();
        for host in dns_hosts {
            if self.dns_host_matches_ip(&host, ip).await {
                return true;
            }
        }

        false
    }

    async fn dns_host_matches_ip(&self, host: &str, ip: IpAddr) -> bool {
        if let Some(ips) = self.cached_dns_ips(host).await {
            return ips.contains(&ip);
        }

        let ips = self.resolve_dns_host(host).await;
        let ttl = if ips.is_empty() {
            AUTHLESS_ACCESS_ALLOWLIST_DNS_NEGATIVE_CACHE_TTL
        } else {
            AUTHLESS_ACCESS_ALLOWLIST_DNS_CACHE_TTL
        };
        self.inner.dns_cache.write().await.insert(
            host.to_string(),
            CachedDnsAllowlistEntry {
                ips: ips.clone(),
                expires_at: Instant::now() + ttl,
            },
        );
        ips.contains(&ip)
    }

    async fn cached_dns_ips(&self, host: &str) -> Option<Vec<IpAddr>> {
        let cache = self.inner.dns_cache.read().await;
        let entry = cache.get(host)?;
        if entry.expires_at > Instant::now() {
            Some(entry.ips.clone())
        } else {
            None
        }
    }

    async fn resolve_dns_host(&self, host: &str) -> Vec<IpAddr> {
        match tokio::net::lookup_host((host, 0)).await {
            Ok(addrs) => {
                let mut ips: Vec<IpAddr> = addrs.map(|addr| addr.ip()).collect();
                ips.sort();
                ips.dedup();
                ips
            }
            Err(error) => {
                warn!(
                    env = UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST_ENV,
                    host,
                    error = %error,
                    "failed to resolve unauthenticated public access allowlist host"
                );
                Vec::new()
            }
        }
    }
}

impl Default for AuthlessAccessAllowlist {
    fn default() -> Self {
        Self::parse("")
    }
}

impl AuthlessIpMatcher {
    fn matches(self, ip: IpAddr) -> bool {
        match self {
            Self::Exact(exact) => exact == ip,
            Self::Cidr(base, prefix) => cidr_contains(base, prefix, ip),
        }
    }
}

fn parse_authless_ip_matcher(raw: &str) -> Option<AuthlessIpMatcher> {
    let Some((ip, prefix)) = raw.split_once('/') else {
        return raw.parse::<IpAddr>().ok().map(AuthlessIpMatcher::Exact);
    };
    let ip = ip.trim().parse::<IpAddr>().ok()?;
    let prefix = prefix.trim().parse::<u8>().ok()?;
    match ip {
        IpAddr::V4(_) if prefix <= 32 => Some(AuthlessIpMatcher::Cidr(ip, prefix)),
        IpAddr::V6(_) if prefix <= 128 => Some(AuthlessIpMatcher::Cidr(ip, prefix)),
        _ => None,
    }
}

fn parse_authless_allowlist_host(raw: &str) -> Option<String> {
    let host = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || host.len() > 253
        || host.contains("://")
        || host.contains('/')
        || host.contains('\\')
        || host.contains(':')
    {
        return None;
    }

    if host.split('.').all(valid_dns_label) {
        Some(host)
    } else {
        None
    }
}

fn valid_dns_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn cidr_contains(base: IpAddr, prefix: u8, ip: IpAddr) -> bool {
    match (base, ip) {
        (IpAddr::V4(base), IpAddr::V4(ip)) => {
            let mask = ipv4_mask(prefix);
            u32::from(base) & mask == u32::from(ip) & mask
        }
        (IpAddr::V6(base), IpAddr::V6(ip)) => {
            let mask = ipv6_mask(prefix);
            u128::from(base) & mask == u128::from(ip) & mask
        }
        _ => false,
    }
}

fn ipv4_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    }
}

fn ipv6_mask(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    }
}

impl AuthlessWebClientProofState {
    pub(crate) fn new() -> Self {
        let rng = SystemRandom::new();
        let mut secret = vec![0_u8; 32];
        if rng.fill(&mut secret).is_err() {
            secret = Id::new().0.into_bytes();
        }
        Self {
            secret: Arc::new(secret),
        }
    }

    fn issue(&self) -> AppResult<(String, String, u64)> {
        let mut nonce_bytes = [0_u8; 16];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| AppError::Repository("failed to create web client proof".into()))?;
        let nonce = hex_encode(&nonce_bytes);
        Ok(self.issue_for_nonce(nonce))
    }

    fn issue_for_nonce(&self, nonce: String) -> (String, String, u64) {
        let expires_at = unix_now() + AUTHLESS_WEB_CLIENT_TTL_SECONDS;
        let signature = self.sign(&nonce, expires_at);
        (
            nonce.clone(),
            format!("{nonce}.{expires_at}.{signature}"),
            expires_at,
        )
    }

    fn validate_headers(&self, headers: &HeaderMap, proof_override: Option<&str>) -> bool {
        let proof = proof_override.or_else(|| {
            headers
                .get(AUTHLESS_WEB_CLIENT_HEADER)
                .and_then(|value| value.to_str().ok())
        });
        let Some(proof) = proof else {
            return false;
        };
        let Some(cookie_nonce) = authless_cookie_nonce(headers) else {
            return false;
        };
        self.validate(proof, &cookie_nonce)
    }

    fn validate(&self, proof: &str, cookie_nonce: &str) -> bool {
        let mut parts = proof.split('.');
        let Some(nonce) = parts.next() else {
            return false;
        };
        let Some(expires_at) = parts.next().and_then(|value| value.parse::<u64>().ok()) else {
            return false;
        };
        let Some(signature) = parts.next() else {
            return false;
        };
        if parts.next().is_some() || nonce != cookie_nonce || expires_at < unix_now() {
            return false;
        }
        constant_time_eq(
            signature.as_bytes(),
            self.sign(nonce, expires_at).as_bytes(),
        )
    }

    fn sign(&self, nonce: &str, expires_at: u64) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.secret);
        let message = format!("scryer-authless-web-client:v1:{nonce}:{expires_at}");
        hex_encode(hmac::sign(&key, message.as_bytes()).as_ref())
    }
}

pub(crate) async fn authless_web_client_proof_handler(
    State(state): State<AuthlessWebClientProofRouteState>,
    request: Request<Body>,
) -> Response {
    let (parts, _) = request.into_parts();
    let remote_addr = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0);
    let headers = parts.headers;
    let snapshot = state.auth_runtime.snapshot();

    if let AuthlessAccessDecision::Reject(reason) = authless_web_client_proof_decision(
        &snapshot,
        state.policy,
        &state.allowlist,
        &headers,
        remote_addr,
    )
    .await
    {
        // A rejected proof is a routine access-control outcome, not an error:
        // the logged-out web client probes for authless (public) access on its
        // GraphQL requests and the server declines per policy; the client
        // handles the 403 gracefully. `AuthRequired` fires on every
        // unauthenticated request to a form-login instance, so keep it at debug
        // to avoid flooding logs. Rarer reasons (cross-site, missing/malformed
        // forwarding, non-local peer) stay at warn — they can signal a proxy
        // misconfiguration or cross-site probing worth noticing.
        if matches!(reason, AuthlessAccessRejectReason::AuthRequired) {
            tracing::debug!("Authless web client proof unavailable: {reason}");
        } else {
            warn!("Rejecting authless web client proof request: {reason}");
        }
        let mut response = (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new(
                "Scryer web client proof is not available for this request".to_string(),
            )),
        )
            .into_response();
        apply_authless_web_client_response_headers(&mut response);
        return response;
    }

    let proof_result = authless_cookie_nonce(&headers)
        .map(|nonce| Ok(state.proof.issue_for_nonce(nonce)))
        .unwrap_or_else(|| state.proof.issue());

    match proof_result {
        Ok((nonce, proof, expires_at)) => {
            let mut response =
                Json(AuthlessWebClientProofResponse { proof, expires_at }).into_response();
            apply_authless_web_client_response_headers(&mut response);
            let cookie = authless_web_client_cookie(&nonce, &headers);
            if let Ok(value) = http::HeaderValue::from_str(&cookie) {
                response.headers_mut().append(header::SET_COOKIE, value);
            }
            response
        }
        Err(err) => {
            let mut response = map_app_error(err);
            apply_authless_web_client_response_headers(&mut response);
            response
        }
    }
}

async fn authless_web_client_proof_decision(
    snapshot: &scryer_interface::context::AuthRuntimeStateSnapshot,
    policy: AuthlessAccessPolicy,
    allowlist: &AuthlessAccessAllowlist,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> AuthlessAccessDecision {
    if headers.contains_key(header::AUTHORIZATION) {
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::AuthorizationCredential);
    }

    if request_is_cross_site(headers) {
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::CrossSiteRequest);
    }

    if local_ip_bypass_active(snapshot, headers, remote_addr) {
        return AuthlessAccessDecision::Allow;
    }

    if snapshot.effective_form_login_enabled {
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::AuthRequired);
    }

    authless_access_decision_with_allowlist(snapshot, policy, allowlist, headers, remote_addr).await
}

fn request_is_cross_site(headers: &HeaderMap) -> bool {
    headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("cross-site"))
}

fn authless_web_client_cookie(nonce: &str, headers: &HeaderMap) -> String {
    let mut cookie = format!(
        "{AUTHLESS_WEB_CLIENT_COOKIE}={nonce}; Path=/; Max-Age={AUTHLESS_WEB_CLIENT_TTL_SECONDS}; HttpOnly; SameSite=Strict"
    );
    if request_is_secure(headers) {
        cookie.push_str("; Secure");
    }
    cookie
}

fn apply_authless_web_client_response_headers(response: &mut Response) {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-store, max-age=0"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, http::HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(header::EXPIRES, http::HeaderValue::from_static("0"));
}

fn request_is_secure(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .any(|proto| proto.trim().eq_ignore_ascii_case("https"))
        })
        .unwrap_or(false)
        || headers
            .get("forwarded")
            .and_then(|value| value.to_str().ok())
            .map(forwarded_header_has_https_proto)
            .unwrap_or(false)
}

fn forwarded_header_has_https_proto(value: &str) -> bool {
    value.split(',').any(|entry| {
        entry.split(';').any(|part| {
            let Some((name, value)) = part.split_once('=') else {
                return false;
            };
            name.trim().eq_ignore_ascii_case("proto")
                && value.trim_matches('"').trim().eq_ignore_ascii_case("https")
        })
    })
}

fn authless_cookie_nonce(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(name, value)| {
            let value = value.trim();
            (name == AUTHLESS_WEB_CLIENT_COOKIE && is_authless_cookie_nonce(value))
                .then(|| value.to_string())
        })
}

fn is_authless_cookie_nonce(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .fold(0_u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

#[derive(Clone, Debug)]
pub(crate) struct CorsConfig {
    pub(crate) allow_all: bool,
    pub(crate) allowed_origins: Vec<String>,
}

impl CorsConfig {
    pub(crate) fn from_env() -> Self {
        Self::from_env_for_mode(cfg!(debug_assertions))
    }

    fn from_env_for_mode(debug_assertions: bool) -> Self {
        let configured_origins = std::env::var(CORS_ALLOWED_ORIGINS_ENV).ok();
        let origins = match configured_origins {
            Some(raw) if debug_assertions => parse_cors_allowed_origins(&raw),
            Some(_) => {
                tracing::warn!(
                    env = CORS_ALLOWED_ORIGINS_ENV,
                    "ignoring CORS origins because CORS is dev-mode only"
                );
                Vec::new()
            }
            None => default_cors_allowed_origins_for_mode(debug_assertions),
        };

        Self {
            allow_all: false,
            allowed_origins: origins,
        }
    }

    fn is_allowed(&self, origin: &str) -> bool {
        if self.allow_all {
            return true;
        }
        self.allowed_origins.iter().any(|allowed| allowed == origin)
    }
}

fn parse_cors_allowed_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(cors_allowed_origin)
        .collect()
}

fn cors_allowed_origin(origin: &str) -> Option<String> {
    if matches!(origin.trim(), "*" | "http://*" | "https://*") {
        tracing::warn!(
            origin,
            "ignoring wildcard CORS Origin; configure exact origins instead"
        );
        return None;
    }

    canonical_origin(origin)
}

#[derive(Clone, Debug, Default)]
pub(crate) struct WebSocketOriginPolicy {
    allowed_origins: Vec<String>,
}

impl WebSocketOriginPolicy {
    pub(crate) fn from_env(cors: &CorsConfig) -> Self {
        Self::from_env_for_mode(cors, cfg!(debug_assertions))
    }

    fn from_env_for_mode(cors: &CorsConfig, debug_assertions: bool) -> Self {
        let origins = match std::env::var(WS_ALLOWED_ORIGINS_ENV) {
            Ok(raw) if debug_assertions => parse_websocket_allowed_origins(&raw),
            Ok(_) => {
                tracing::warn!(
                    env = WS_ALLOWED_ORIGINS_ENV,
                    "ignoring WebSocket origins because CORS is dev-mode only"
                );
                Vec::new()
            }
            Err(_) => cors
                .allowed_origins
                .iter()
                .filter_map(|origin| websocket_allowed_origin(origin))
                .collect(),
        };

        Self {
            allowed_origins: origins,
        }
    }

    fn check(&self, headers: &HeaderMap) -> Result<(), String> {
        let Some(origin) = headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(());
        };

        let Some(origin) = websocket_allowed_origin(origin) else {
            return Err("invalid WebSocket Origin".to_string());
        };

        if request_is_same_origin(headers, &origin)
            || self
                .allowed_origins
                .iter()
                .any(|allowed| allowed == &origin)
        {
            return Ok(());
        }

        Err(format!("WebSocket Origin is not allowed: {origin}"))
    }
}

fn parse_websocket_allowed_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(websocket_allowed_origin)
        .collect()
}

fn websocket_allowed_origin(origin: &str) -> Option<String> {
    if matches!(origin.trim(), "*" | "http://*" | "https://*") {
        tracing::warn!(
            origin,
            "ignoring wildcard WebSocket Origin; configure exact origins instead"
        );
        return None;
    }
    canonical_origin(origin)
}

fn request_is_same_origin(headers: &HeaderMap, origin: &str) -> bool {
    let Some((origin_scheme, origin_authority)) = split_origin(origin) else {
        return false;
    };
    let Some(request_authority) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(normalize_authority)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if origin_authority != request_authority {
        return false;
    }

    forwarded_proto(headers)
        .is_none_or(|proto| origin_scheme_matches_forwarded_proto(&origin_scheme, &proto))
}

fn split_origin(origin: &str) -> Option<(String, String)> {
    let (scheme, authority) = origin.split_once("://")?;
    Some((scheme.to_ascii_lowercase(), normalize_authority(authority)))
}

fn normalize_authority(authority: &str) -> String {
    authority.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn forwarded_proto(headers: &HeaderMap) -> Option<String> {
    headers
        .get(X_FORWARDED_PROTO)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn origin_scheme_matches_forwarded_proto(origin_scheme: &str, forwarded_proto: &str) -> bool {
    matches!(
        (origin_scheme, forwarded_proto),
        ("http", "http") | ("http", "ws") | ("https", "https") | ("https", "wss")
    )
}

fn default_cors_allowed_origins_for_mode(debug_assertions: bool) -> Vec<String> {
    let mut origins = if debug_assertions {
        vec![
            "http://localhost:3000".to_string(),
            "http://127.0.0.1:3000".to_string(),
            "http://0.0.0.0:3000".to_string(),
            "http://host.docker.internal:3000".to_string(),
            "http://nodejs:3000".to_string(),
        ]
    } else {
        Vec::new()
    };

    if debug_assertions
        && let Ok(web_ui_url) = std::env::var(WEB_UI_URL_ENV)
        && let Some(web_ui_origin) = canonical_origin(&web_ui_url)
    {
        push_origin_if_missing(&mut origins, web_ui_origin.clone());
        add_docker_loopback_aliases(&web_ui_origin, &mut origins);
    }

    origins
}

fn push_origin_if_missing(origins: &mut Vec<String>, candidate: String) {
    if !origins.iter().any(|origin| origin == &candidate) {
        origins.push(candidate);
    }
}

fn canonical_origin(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if matches!(trimmed, "*" | "http://*" | "https://*") {
        return None;
    }

    let uri = trimmed.parse::<Uri>().ok()?;
    let scheme = uri.scheme_str()?;
    let authority = uri.authority()?;
    Some(format!("{scheme}://{authority}"))
}

fn add_docker_loopback_aliases(origin: &str, origins: &mut Vec<String>) {
    let Ok(uri) = origin.parse::<Uri>() else {
        return;
    };
    let Some(scheme) = uri.scheme_str() else {
        return;
    };
    let Some(authority) = uri.authority() else {
        return;
    };

    let host = authority.host();
    let port = authority.port_u16();
    if !matches!(
        host,
        "localhost" | "127.0.0.1" | "0.0.0.0" | "host.docker.internal" | "nodejs"
    ) {
        return;
    }

    for alias in [
        "localhost",
        "127.0.0.1",
        "0.0.0.0",
        "host.docker.internal",
        "nodejs",
    ] {
        let authority = match port {
            Some(port) => format!("{alias}:{port}"),
            None => alias.to_string(),
        };
        push_origin_if_missing(origins, format!("{scheme}://{authority}"));
    }
}

pub(crate) async fn cors_handler(
    request: Request<Body>,
    next: Next,
    policy: CorsConfig,
) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let requested_headers = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    if request.method() == Method::OPTIONS && origin.as_deref().is_some() {
        let origin = origin.expect("checked above");
        if !policy.is_allowed(&origin) {
            return StatusCode::FORBIDDEN.into_response();
        }

        let mut response = StatusCode::NO_CONTENT.into_response();
        apply_cors_headers(
            response.headers_mut(),
            &origin,
            requested_headers.as_deref(),
        );
        return response;
    }

    let mut response = next.run(request).await;
    if let Some(origin) = origin
        && policy.is_allowed(&origin)
    {
        apply_cors_headers(
            response.headers_mut(),
            &origin,
            requested_headers.as_deref(),
        );
    }

    response
}

pub(crate) fn apply_cors_headers(
    headers: &mut http::HeaderMap,
    origin: &str,
    requested_headers: Option<&str>,
) {
    use http::HeaderValue;

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_str(origin).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
    );

    let mut allow_headers = "Content-Type, Authorization, X-Scryer-Language".to_string();
    if let Some(requested_headers) = requested_headers {
        let requested_headers = requested_headers.trim();
        if !requested_headers.is_empty() {
            allow_headers = format!("{}, {}", allow_headers, requested_headers);
        }
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_str(&allow_headers).unwrap_or_else(|_| {
            HeaderValue::from_static("Content-Type, Authorization, X-Scryer-Language")
        }),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
}

pub(crate) async fn index_page() -> impl IntoResponse {
    let web_url =
        std::env::var("SCRYER_WEB_UI_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let base_path = BasePath::from_env();
    let graphql_url = base_path.join("/graphql");
    Html(format!(
        r#"
<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>scryer</title>
    <style>
      :root {{
        color-scheme: dark;
      }}
      body {{
        margin: 0;
        min-height: 100vh;
        font-family: Inter, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif;
        background: #0f1224;
        color: #e6edff;
        display: grid;
        place-items: center;
      }}
      main {{
        width: min(780px, 100% - 2rem);
      }}
      a {{
        color: #9fb2ff;
      }}
    </style>
  </head>
  <body>
    <main>
      <h1>scryer web UI</h1>
      <p>The SPA has moved to Next.js.</p>
      <p>
        Start the web app in <code>apps/scryer-web</code> and open
        <a href="{web_url}">{web_url}</a>.
      </p>
      <p>
        Backend endpoint: <code>{graphql_url}</code> is still served by this service.
      </p>
    </main>
  </body>
</html>
    "#,
    ))
}

pub(crate) async fn graphql_ws_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    protocol: GraphQLProtocol,
    ws: WebSocketUpgrade,
) -> Response {
    let schema = state.schema.clone();
    let app = state.app.clone();
    let rate_limiter = state.rate_limiter.clone();
    let client_ip = request_client_ip(&headers, Some(remote_addr)).unwrap_or(remote_addr.ip());
    let peer_ip = remote_addr.ip();
    let auth_runtime = state.auth_runtime.clone();
    let auth_snapshot = auth_runtime.snapshot();
    let auth_enabled = auth_snapshot.effective_form_login_enabled;
    let local_bypass_active = local_ip_bypass_active(&auth_snapshot, &headers, Some(remote_addr));
    let connection_epoch = auth_snapshot.epoch;
    if let Err(error) = state.ws_origin_policy.check(&headers) {
        tracing::warn!(
            remote_addr = %remote_addr,
            error = %error,
            "rejecting GraphQL WebSocket connection because browser Origin is not allowed"
        );
        return (StatusCode::FORBIDDEN, error).into_response();
    }
    if authorization_token_from_headers(&headers)
        .ok()
        .flatten()
        .is_some_and(is_api_key_bearer)
    {
        return (
            StatusCode::UNAUTHORIZED,
            "API keys are not supported for WebSocket connections",
        )
            .into_response();
    }

    let initial_actor = match resolve_actor(&state, &headers, Some(remote_addr)).await {
        Ok(actor) => actor,
        Err(AppError::Unauthorized(_)) => return oauth_access_revoked_http_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to resolve GraphQL WebSocket actor");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let authless_proof_required = initial_actor
        .as_ref()
        .is_some_and(ResolvedActor::requires_authless_web_client_proof);
    let connection_context = graphql_websocket_log_context(
        if authless_proof_required {
            None
        } else {
            initial_actor.as_ref()
        },
        request_client_ip(&headers, Some(remote_addr)).unwrap_or(remote_addr.ip()),
    );
    let connection_context = Arc::new(StdRwLock::new(connection_context));
    let connection_span = context_span(
        connection_context
            .read()
            .expect("connection log context lock must not be poisoned")
            .clone(),
    );
    let initial_data = graphql_ws_connection_data(
        connection_epoch,
        if authless_proof_required {
            None
        } else {
            initial_actor.clone()
        },
    );
    let oauth_session = Arc::new(StdRwLock::new(if authless_proof_required {
        None
    } else {
        initial_actor
            .as_ref()
            .and_then(ResolvedActor::oauth_session)
    }));
    let rate_limit_user_id = Arc::new(StdRwLock::new(
        initial_actor.as_ref().map(|actor| actor.user.id.clone()),
    ));
    let authless_web_client_proof = state.authless_web_client_proof.clone();
    let ws_headers = headers.clone();
    let connection_span_for_init = connection_span.clone();
    let connection_context_for_init = connection_context.clone();
    let oauth_session_for_init = oauth_session.clone();
    let rate_limit_user_id_for_init = rate_limit_user_id.clone();

    ws.max_message_size(GRAPHQL_MAX_MESSAGE_BYTES)
        .max_frame_size(GRAPHQL_MAX_MESSAGE_BYTES)
        .protocols(ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| async move {
            let app_for_init = app.clone();
            let initial_actor = initial_actor.clone();
            let proof_state = authless_web_client_proof.clone();
            let headers_for_init = ws_headers.clone();
            let connection_span_for_init = connection_span_for_init.clone();
            let connection_context_for_init = connection_context_for_init.clone();
            let oauth_session_for_init = oauth_session_for_init.clone();
            let rate_limit_user_id_for_init = rate_limit_user_id_for_init.clone();
            let rate_limiter = rate_limiter.clone();
            let executor = ContextualGraphqlExecutor::new(ContextualGraphqlExecutorConfig {
                schema,
                app: app.clone(),
                connection_context: connection_context.clone(),
                oauth_session,
                rate_limit_user_id,
                rate_limiter,
                client_ip,
                peer_ip,
            });
            GraphQLWebSocket::new(stream, executor, protocol)
                .with_data(initial_data)
                .on_connection_init(move |value: serde_json::Value| {
                    let connection_span = connection_span_for_init.clone();
                    let connection_context = connection_context_for_init.clone();
                    let oauth_session = oauth_session_for_init.clone();
                    let rate_limit_user_id = rate_limit_user_id_for_init.clone();
                    let app_for_init = app_for_init.clone();
                    let initial_actor = initial_actor.clone();
                    let proof_state = proof_state.clone();
                    let headers_for_init = headers_for_init.clone();
                    async move {
                        let auth_value = value.get("Authorization").and_then(|v| v.as_str());
                        let proof_value = value
                            .get("authlessWebClientProof")
                            .or_else(|| value.get("X-Scryer-Web-Client"))
                            .and_then(|v| v.as_str());
                        let actor = resolve_ws_connection_init_actor(
                            &app_for_init,
                            WsConnectionInitActorRequest {
                                auth_enabled,
                                local_bypass_active,
                                initial_actor,
                                auth_value,
                                authless_proof_required,
                                proof_state: &proof_state,
                                headers: &headers_for_init,
                                proof_value,
                            },
                        )
                        .await?;
                        *oauth_session
                            .write()
                            .expect("OAuth session lock must not be poisoned") =
                            actor.as_ref().and_then(ResolvedActor::oauth_session);
                        *rate_limit_user_id
                            .write()
                            .expect("rate limit user lock must not be poisoned") =
                            actor.as_ref().map(|actor| actor.user.id.clone());
                        if let Some(actor) = actor.as_ref() {
                            let mut updated_context = connection_context
                                .read()
                                .expect("connection log context lock must not be poisoned")
                                .clone();
                            updated_context.actor = Some(actor_log_context(actor));
                            *connection_context
                                .write()
                                .expect("connection log context lock must not be poisoned") =
                                updated_context.clone();
                            update_context(&connection_span, updated_context);
                        }
                        Ok(graphql_ws_connection_data(connection_epoch, actor))
                    }
                })
                .serve()
                .instrument(connection_span)
                .await;
        })
}

#[derive(Clone)]
struct ContextualGraphqlExecutor {
    schema: scryer_interface::ApiSchema,
    app: AppUseCase,
    connection_context: Arc<StdRwLock<LogContext>>,
    oauth_session: Arc<StdRwLock<Option<OAuthActorSession>>>,
    rate_limit_user_id: Arc<StdRwLock<Option<String>>>,
    rate_limiter: ScryerRateLimiter,
    client_ip: IpAddr,
    peer_ip: IpAddr,
}

struct ContextualGraphqlExecutorConfig {
    schema: scryer_interface::ApiSchema,
    app: AppUseCase,
    connection_context: Arc<StdRwLock<LogContext>>,
    oauth_session: Arc<StdRwLock<Option<OAuthActorSession>>>,
    rate_limit_user_id: Arc<StdRwLock<Option<String>>>,
    rate_limiter: ScryerRateLimiter,
    client_ip: IpAddr,
    peer_ip: IpAddr,
}

impl ContextualGraphqlExecutor {
    fn new(config: ContextualGraphqlExecutorConfig) -> Self {
        Self {
            schema: config.schema,
            app: config.app,
            connection_context: config.connection_context,
            oauth_session: config.oauth_session,
            rate_limit_user_id: config.rate_limit_user_id,
            rate_limiter: config.rate_limiter,
            client_ip: config.client_ip,
            peer_ip: config.peer_ip,
        }
    }

    fn operation_context(&self, request: &GraphQLRequest) -> LogContext {
        let mut context = self
            .connection_context
            .read()
            .expect("connection log context lock must not be poisoned")
            .clone();
        let (operation_name, operation_type) = graphql_request_operation_metadata(request);
        if let Some(request_context) = context.request.as_mut() {
            request_context.operation_name = operation_name;
            request_context.operation_type = operation_type;
        }
        context
    }
}

fn login_attempt_limiter(
    rate_limiter: ScryerRateLimiter,
    rate_limit_key: RateLimitKey,
) -> LoginAttemptLimiter {
    let check_limiter = rate_limiter.clone();
    let check_key = rate_limit_key.clone();
    let record_limiter = rate_limiter.clone();
    let record_key = rate_limit_key;
    LoginAttemptLimiter::new(
        move |principal| {
            check_limiter
                .check_login_principal(&check_key, principal)
                .map_err(|decision| rate_limited_graphql_error(&decision))
        },
        move |principal| record_limiter.record_login_principal_failure(&record_key, principal),
        move |principal| rate_limiter.clear_login_principal_failures(principal),
    )
}

impl Executor for ContextualGraphqlExecutor {
    fn execute(&self, request: GraphQLRequest) -> impl Future<Output = GraphQLResponse> + Send {
        let span = context_span(self.operation_context(&request));
        let schema = self.schema.clone();
        let app = self.app.clone();
        let oauth_session = self.oauth_session.clone();
        let rate_limiter = self.rate_limiter.clone();
        let rate_limit_user_id = self
            .rate_limit_user_id
            .read()
            .expect("rate limit user lock must not be poisoned")
            .clone();
        let rate_limit_key = RateLimitKey::for_client_and_peer(
            self.client_ip,
            self.peer_ip,
            rate_limit_user_id.as_deref(),
        );
        async move {
            let authentication = analyze_authentication_request(&request);
            let class = if authentication.rejected {
                GraphqlRateLimitClass::Login
            } else {
                authentication
                    .class
                    .unwrap_or_else(|| classify_graphql_request(&request))
            };
            if let Err(decision) = rate_limiter.check_graphql(class, &rate_limit_key) {
                return rate_limited_graphql_single_response(&decision);
            }
            if authentication.rejected {
                return authentication_mutation_single_field_response();
            }
            let principal = authentication.principal;
            if let Some(principal) = principal.as_ref()
                && let Err(decision) =
                    rate_limiter.check_login_principal(&rate_limit_key, principal)
            {
                return rate_limited_graphql_single_response(&decision);
            }
            if validate_ws_oauth_session(&app, &oauth_session)
                .await
                .is_err()
            {
                return oauth_access_revoked_response();
            }
            schema
                .execute(request.data(login_attempt_limiter(rate_limiter, rate_limit_key)))
                .await
        }
        .instrument(span)
    }

    fn execute_stream(
        &self,
        request: GraphQLRequest,
        session_data: Option<Arc<Data>>,
    ) -> BoxStream<'static, GraphQLResponse> {
        let span = context_span(self.operation_context(&request));
        let rate_limit_user_id = self
            .rate_limit_user_id
            .read()
            .expect("rate limit user lock must not be poisoned")
            .clone();
        let authentication = analyze_authentication_request(&request);
        let rate_limit_key = RateLimitKey::for_client_and_peer(
            self.client_ip,
            self.peer_ip,
            rate_limit_user_id.as_deref(),
        );
        let class = if authentication.rejected {
            GraphqlRateLimitClass::Login
        } else {
            authentication
                .class
                .unwrap_or_else(|| classify_graphql_request(&request))
        };
        if let Err(decision) = self.rate_limiter.check_graphql(class, &rate_limit_key) {
            return Box::pin(stream::once(async move {
                rate_limited_graphql_single_response(&decision)
            }));
        }
        if authentication.rejected {
            return Box::pin(stream::once(async {
                authentication_mutation_single_field_response()
            }));
        }
        if let Some(principal) = authentication.principal.as_ref()
            && let Err(decision) = self
                .rate_limiter
                .check_login_principal(&rate_limit_key, principal)
        {
            return Box::pin(stream::once(async move {
                rate_limited_graphql_single_response(&decision)
            }));
        }
        let request = request.data(login_attempt_limiter(
            self.rate_limiter.clone(),
            rate_limit_key,
        ));
        let app = self.app.clone();
        let oauth_session = self.oauth_session.clone();
        Box::pin(ContextualGraphqlResponseStream {
            inner: Box::pin(stream::unfold(
                (
                    true,
                    Executor::execute_stream(&self.schema, request, session_data),
                    app,
                    oauth_session,
                ),
                |(active, mut inner, app, oauth_session)| async move {
                    if !active
                        || validate_ws_oauth_session(&app, &oauth_session)
                            .await
                            .is_err()
                    {
                        return active.then_some((
                            oauth_access_revoked_response(),
                            (false, inner, app, oauth_session),
                        ));
                    }

                    let response = inner.next().await?;
                    let response = if validate_ws_oauth_session(&app, &oauth_session)
                        .await
                        .is_ok()
                    {
                        response
                    } else {
                        oauth_access_revoked_response()
                    };
                    let keep_active =
                        !response_has_error_code(&response, AUTHENTICATION_REQUIRED_CODE);
                    Some((response, (keep_active, inner, app, oauth_session)))
                },
            )),
            span,
        })
    }
}

async fn validate_ws_oauth_session(
    app: &AppUseCase,
    oauth_session: &Arc<StdRwLock<Option<OAuthActorSession>>>,
) -> AppResult<()> {
    let oauth_session = oauth_session
        .read()
        .expect("OAuth session lock must not be poisoned")
        .clone();
    if let Some(oauth_session) = oauth_session {
        app.validate_oauth_access_token(&oauth_session.client_id, &oauth_session.grant_id)
            .await?;
    }
    Ok(())
}

fn oauth_access_revoked_response() -> GraphQLResponse {
    let mut extensions = ErrorExtensionValues::default();
    extensions.set("code", AUTHENTICATION_REQUIRED_CODE);
    let mut error = ServerError::new("OAuth access is no longer authorized", None);
    error.extensions = Some(extensions);
    GraphQLResponse::from_errors(vec![error])
}

fn oauth_access_revoked_graphql_response() -> Response {
    let batch = async_graphql::BatchResponse::Single(oauth_access_revoked_response());
    let body = serde_json::to_vec(&batch).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn api_key_unauthorized_graphql_response() -> Response {
    let mut extensions = ErrorExtensionValues::default();
    extensions.set("code", AUTHENTICATION_REQUIRED_CODE);
    let mut error = ServerError::new("API key is invalid or no longer authorized", None);
    error.extensions = Some(extensions);
    let batch = async_graphql::BatchResponse::Single(GraphQLResponse::from_errors(vec![error]));
    let body = serde_json::to_vec(&batch).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn rate_limited_graphql_http_response(decision: &crate::rate_limit::RateLimitDecision) -> Response {
    let batch_response = rate_limited_graphql_response(decision);
    let body = serde_json::to_vec(&batch_response).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn authentication_mutation_single_field_response() -> GraphQLResponse {
    let mut extensions = ErrorExtensionValues::default();
    extensions.set("code", "AUTHENTICATION_MUTATION_SINGLE_FIELD_REQUIRED");
    let mut error = ServerError::new(
        "authentication-sensitive mutations must contain exactly one top-level field",
        None,
    );
    error.extensions = Some(extensions);
    GraphQLResponse::from_errors(vec![error])
}

fn authentication_mutation_single_field_http_response() -> Response {
    let batch =
        async_graphql::BatchResponse::Single(authentication_mutation_single_field_response());
    let body = serde_json::to_vec(&batch).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn oauth_access_revoked_http_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse::new(
            "OAuth access is no longer authorized".to_string(),
        )),
    )
        .into_response()
}

struct ContextualGraphqlResponseStream {
    inner: BoxStream<'static, GraphQLResponse>,
    span: tracing::Span,
}

impl Stream for ContextualGraphqlResponseStream {
    type Item = GraphQLResponse;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.span.in_scope(|| this.inner.as_mut().poll_next(cx))
    }
}

#[derive(Clone)]
pub(crate) struct AuthState {
    pub(crate) app: AppUseCase,
    pub(crate) schema: scryer_interface::ApiSchema,
    pub(crate) auth_runtime: AuthRuntimeStateHandle,
    pub(crate) rate_limiter: ScryerRateLimiter,
    pub(crate) ws_origin_policy: WebSocketOriginPolicy,
    pub(crate) authless_web_client_proof: AuthlessWebClientProofState,
}

/// GraphQL handler that returns a streaming response body.
///
/// When the client disconnects (e.g. via `AbortController.abort()` in the browser),
/// hyper stops polling this body stream, which drops the `execute_batch` future.
/// This cancels the entire resolver chain — including any outbound reqwest call to
/// SMG — so the cancellation propagates all the way through to the database query.
pub(crate) async fn graphql_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    body: async_graphql_axum::GraphQLBatchRequest,
) -> Response {
    let batch = body.into_inner();
    if batch.iter().count() > GRAPHQL_MAX_BATCH_OPERATIONS {
        return graphql_batch_limit_response();
    }
    let client_ip = request_client_ip(&headers, Some(remote_addr)).unwrap_or(remote_addr.ip());
    let peer_ip = remote_addr.ip();
    let rate_limit_class = classify_graphql(&batch);
    let authentication_requests = batch
        .iter()
        .map(analyze_authentication_request)
        .collect::<Vec<_>>();
    let rate_limit_key = RateLimitKey::for_client_and_peer(client_ip, peer_ip, None);
    for (request, authentication) in batch.iter().zip(&authentication_requests) {
        let request_rate_limit_class = if authentication.rejected {
            GraphqlRateLimitClass::Login
        } else {
            authentication
                .class
                .unwrap_or_else(|| classify_graphql_request(request))
        };
        if let Err(decision) = state
            .rate_limiter
            .check_graphql(request_rate_limit_class, &rate_limit_key)
        {
            return rate_limited_graphql_http_response(&decision);
        }
        if let Some(principal) = authentication.principal.as_ref()
            && let Err(decision) = state
                .rate_limiter
                .check_login_principal(&rate_limit_key, principal)
        {
            return rate_limited_graphql_http_response(&decision);
        }
    }
    if authentication_requests
        .iter()
        .any(|analysis| analysis.rejected)
    {
        return authentication_mutation_single_field_http_response();
    }
    let has_authentication_request = authentication_requests
        .iter()
        .any(|analysis| analysis.class.is_some());

    let api_key_bearer = authorization_token_from_headers(&headers)
        .ok()
        .flatten()
        .is_some_and(is_api_key_bearer);
    let actor = match resolve_actor(&state, &headers, Some(remote_addr)).await {
        Ok(actor) => actor,
        Err(AppError::Unauthorized(_)) if api_key_bearer => {
            return api_key_unauthorized_graphql_response();
        }
        Err(AppError::Unauthorized(_)) => return oauth_access_revoked_graphql_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to resolve GraphQL actor");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if actor
        .as_ref()
        .is_some_and(ResolvedActor::requires_authless_web_client_proof)
        && !state
            .authless_web_client_proof
            .validate_headers(&headers, None)
    {
        return authless_web_client_forbidden_response(
            "Scryer web client proof is required for unauthenticated access",
        );
    }
    let rate_limit_key = RateLimitKey::for_client_and_peer(
        client_ip,
        peer_ip,
        actor.as_ref().map(|actor| actor.user.id.as_str()),
    );
    let request_span = context_span(graphql_request_log_context(
        &batch,
        actor.as_ref(),
        client_ip,
    ));
    let session_persistence = RequestSessionPersistence {
        default_persist_session: default_persist_session_for_request(&headers, Some(remote_addr)),
    };
    let login_attempt_limiter = login_attempt_limiter(state.rate_limiter.clone(), rate_limit_key);
    if let Some(actor) = actor.as_ref()
        && !has_authentication_request
        && let Err(decision) = state.rate_limiter.check_graphql(
            rate_limit_class,
            &RateLimitKey::for_client_and_peer(client_ip, peer_ip, Some(&actor.user.id)),
        )
    {
        return rate_limited_graphql_http_response(&decision);
    }
    touch_oauth_grant_last_used(&state.app, actor.as_ref()).await;
    let batch = if let Some(actor) = actor {
        let oauth_session = actor.oauth_session();
        let interactive_session = actor.is_interactive_session();
        let authless_default_session = actor.is_authless_default_session();
        let api_key_management_session = actor.can_manage_api_keys();
        // Request-scoped dataloaders: the batch cache lives for exactly this
        // HTTP request (shared across a batched request's entries — same actor,
        // same snapshot). The WebSocket path intentionally gets none; resolvers
        // fall back to direct application calls when loaders are absent.
        let loaders = RequestLoaders::new(state.app.clone(), actor.user.clone());
        match batch {
            async_graphql::BatchRequest::Single(req) => {
                let mut req = req
                    .data(actor.mfa_verification())
                    .data(actor.user)
                    .data(loaders);
                if interactive_session {
                    req = req.data(InteractiveSession);
                }
                if authless_default_session {
                    req = req.data(AuthlessDefaultSession);
                }
                if api_key_management_session {
                    req = req.data(ApiKeyManagementSession);
                }
                if let Some(oauth_session) = oauth_session {
                    req = req.data(oauth_session);
                }
                async_graphql::BatchRequest::Single(req)
            }
            async_graphql::BatchRequest::Batch(reqs) => async_graphql::BatchRequest::Batch(
                reqs.into_iter()
                    .map(|req| {
                        let mut req = req
                            .data(actor.mfa_verification())
                            .data(actor.user.clone())
                            .data(loaders.clone());
                        if interactive_session {
                            req = req.data(InteractiveSession);
                        }
                        if authless_default_session {
                            req = req.data(AuthlessDefaultSession);
                        }
                        if api_key_management_session {
                            req = req.data(ApiKeyManagementSession);
                        }
                        if let Some(oauth_session) = actor.oauth_session() {
                            req = req.data(oauth_session);
                        }
                        req
                    })
                    .collect(),
            ),
        }
    } else {
        batch
    };

    let batch = match batch {
        async_graphql::BatchRequest::Single(req) => async_graphql::BatchRequest::Single(
            req.data(session_persistence).data(login_attempt_limiter),
        ),
        async_graphql::BatchRequest::Batch(reqs) => async_graphql::BatchRequest::Batch(
            reqs.into_iter()
                .map(|req| {
                    req.data(session_persistence)
                        .data(login_attempt_limiter.clone())
                })
                .collect(),
        ),
    };

    let schema = state.schema.clone();
    let execution_timeout = graphql_post_execution_timeout();
    let batch_response = match tokio::time::timeout(
        execution_timeout,
        schema.execute_batch(batch).instrument(request_span.clone()),
    )
    .instrument(request_span.clone())
    .await
    {
        Ok(response) => response,
        Err(_) => {
            request_span.in_scope(|| {
                tracing::warn!(
                    timeout_seconds = execution_timeout.as_secs(),
                    "graphql POST execution timed out"
                );
            });
            graphql_execution_timeout_response()
        }
    };
    if let Err(error) = state
        .app
        .image_proxy_repository()
        .flush_image_proxy_sources()
        .instrument(request_span)
        .await
    {
        tracing::error!(error = %error, "failed to durably register GraphQL image proxy sources");
        let body = serde_json::to_vec(&serde_json::json!({
            "errors": [{"message": "failed to register image sources"}]
        }))
        .unwrap_or_else(|_| b"{}".to_vec());
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
    }
    let response_status = graphql_response_status(&batch_response);
    let body = serde_json::to_vec(&batch_response).unwrap_or_else(|_| b"{}".to_vec());

    Response::builder()
        .status(response_status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

pub(crate) async fn emby_avatar_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    AxumPath((connection_id, user_id, image_tag)): AxumPath<(String, String, String)>,
) -> Response {
    let Ok(Some(actor)) = resolve_actor(&state, &headers, Some(remote_addr)).await else {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .unwrap();
    };
    if actor.token_claims.session_scope != JwtSessionScope::Full {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .unwrap();
    }
    if connection_id.len() > 256
        || user_id.len() > 256
        || image_tag.len() > 256
        || connection_id.is_empty()
        || user_id.is_empty()
        || image_tag.is_empty()
    {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    }
    let avatar = match state
        .app
        .fetch_emby_server_user_avatar(&actor.user, &connection_id, &user_id, &image_tag)
        .await
    {
        Ok(Some(avatar)) => avatar,
        Err(AppError::Unauthorized(_)) => {
            return Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::empty())
                .unwrap();
        }
        Ok(None) | Err(AppError::NotFound(_)) => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .unwrap();
        }
        Err(error) => {
            tracing::warn!(connection_id, operation = "emby_avatar", error = %error, "Emby avatar proxy failed");
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::empty())
                .unwrap();
        }
    };
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, avatar.content_type)
        .header(header::CACHE_CONTROL, "private, max-age=300");
    if let Some(etag) = avatar.etag {
        response = response.header(header::ETAG, etag);
    }
    if let Some(last_modified) = avatar.last_modified {
        response = response.header(header::LAST_MODIFIED, last_modified);
    }
    response.body(Body::from(avatar.bytes)).unwrap()
}

pub(crate) async fn enforce_authless_access_guard(
    State(state): State<AuthlessAccessGuardState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let decision = authless_access_decision_with_allowlist(
        &state.auth_runtime.snapshot(),
        state.policy,
        &state.allowlist,
        request.headers(),
        Some(remote_addr),
    )
    .await;

    match decision {
        AuthlessAccessDecision::Allow => next.run(request).await,
        AuthlessAccessDecision::Reject(reason) => {
            let method = request.method().clone();
            let path = request.uri().path().to_string();
            tracing::warn!(
                remote_addr = %remote_addr,
                method = %method,
                path = %path,
                reason = %reason,
                "rejecting auth-disabled request from non-local client"
            );
            (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse::new(
                    "Scryer authentication is disabled; public unauthenticated access is blocked"
                        .to_string(),
                )),
            )
                .into_response()
        }
    }
}

fn graphql_response_status(response: &async_graphql::BatchResponse) -> StatusCode {
    if graphql_response_has_error_code(response, AUTHENTICATION_REQUIRED_CODE) {
        return StatusCode::UNAUTHORIZED;
    }

    if graphql_response_has_error_code(response, MFA_STEP_UP_REQUIRED_CODE) {
        return StatusCode::from_u16(MFA_STEP_UP_REQUIRED_STATUS_CODE)
            .expect("MFA step-up status code is a valid HTTP status code");
    }

    StatusCode::OK
}

fn authless_web_client_forbidden_response(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse::new(message.to_string())),
    )
        .into_response()
}

fn graphql_response_has_error_code(response: &async_graphql::BatchResponse, code: &str) -> bool {
    match response {
        async_graphql::BatchResponse::Single(response) => response_has_error_code(response, code),
        async_graphql::BatchResponse::Batch(responses) => responses
            .iter()
            .any(|response| response_has_error_code(response, code)),
    }
}

fn response_has_error_code(response: &GraphQLResponse, code: &str) -> bool {
    response.errors.iter().any(|error| {
        let Some(extensions) = &error.extensions else {
            return false;
        };
        matches!(extensions.get("code"), Some(async_graphql::Value::String(value)) if value == code)
    })
}

fn graphql_execution_timeout_response() -> async_graphql::BatchResponse {
    let execution_timeout = graphql_post_execution_timeout();
    let mut extensions = ErrorExtensionValues::default();
    extensions.set("code", GRAPHQL_POST_EXECUTION_TIMEOUT_CODE);
    extensions.set("timeoutSeconds", execution_timeout.as_secs());

    let mut error = ServerError::new(
        format!(
            "GraphQL request timed out after {} seconds",
            execution_timeout.as_secs()
        ),
        None,
    );
    error.extensions = Some(extensions);
    async_graphql::BatchResponse::Single(GraphQLResponse::from_errors(vec![error]))
}

fn graphql_batch_limit_response() -> Response {
    let mut extensions = ErrorExtensionValues::default();
    extensions.set("code", "GRAPHQL_BATCH_LIMIT_EXCEEDED");
    extensions.set("maxOperations", GRAPHQL_MAX_BATCH_OPERATIONS);
    let mut error = ServerError::new(
        format!("GraphQL batches may contain at most {GRAPHQL_MAX_BATCH_OPERATIONS} operations"),
        None,
    );
    error.extensions = Some(extensions);
    let batch_response =
        async_graphql::BatchResponse::Single(GraphQLResponse::from_errors(vec![error]));
    let body = serde_json::to_vec(&batch_response).unwrap_or_else(|_| b"{}".to_vec());

    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn graphql_post_execution_timeout() -> Duration {
    // Resolver-specific deadlines still fail faster. This transport guard must
    // remain above both the longest valid indexer operation and an operator's
    // configured download-client feedback window, otherwise it silently wins.
    graphql_post_execution_timeout_for(
        scryer_infrastructure_acquisition::downloads::clients::download_client_feedback_timeout(),
    )
}

fn graphql_post_execution_timeout_for(download_feedback_timeout: Duration) -> Duration {
    scryer_outbound_http::LONG_RUNNING_HTTP_OPERATION_TIMEOUT
        .max(download_feedback_timeout.saturating_add(Duration::from_secs(5)))
}

#[derive(Clone)]
struct ResolvedActor {
    user: scryer_domain::User,
    token_claims: AuthenticatedTokenClaims,
    source: ResolvedActorSource,
    api_key_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResolvedActorSource {
    AuthenticatedToken,
    ApiKey,
    AuthlessDefault,
}

impl ResolvedActorSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::AuthenticatedToken => "authenticated_token",
            Self::ApiKey => "api_key",
            Self::AuthlessDefault => "authless_default",
        }
    }
}

fn graphql_request_log_context(
    batch: &async_graphql::BatchRequest,
    actor: Option<&ResolvedActor>,
    client_ip: IpAddr,
) -> LogContext {
    let (operation_name, operation_type) = graphql_operation_metadata(batch);
    let request = RequestContext {
        id: Uuid::new_v4().to_string(),
        transport: "graphql_http".to_owned(),
        operation_name,
        operation_type,
        client_ip: Some(client_ip.to_string()),
    };
    let mut context = LogContext::request(request);
    if let Some(actor) = actor {
        context = context.with_actor(actor_log_context(actor));
    }
    context
}

fn graphql_websocket_log_context(actor: Option<&ResolvedActor>, client_ip: IpAddr) -> LogContext {
    let request = RequestContext {
        id: Uuid::new_v4().to_string(),
        transport: "graphql_ws".to_owned(),
        operation_name: None,
        operation_type: None,
        client_ip: Some(client_ip.to_string()),
    };
    let mut context = LogContext::request(request);
    if let Some(actor) = actor {
        context = context.with_actor(actor_log_context(actor));
    }
    context
}

fn actor_log_context(actor: &ResolvedActor) -> ActorContext {
    ActorContext {
        kind: if actor.user.is_system_execution_actor() {
            "system".to_owned()
        } else {
            "user".to_owned()
        },
        id: Some(actor.user.id.clone()),
        display_name: Some(actor.audit_display_name()),
        source: Some(
            actor
                .api_key_id
                .as_ref()
                .map(|key_id| format!("api_key:{key_id}"))
                .unwrap_or_else(|| actor.source.as_str().to_owned()),
        ),
    }
}

fn graphql_operation_metadata(
    batch: &async_graphql::BatchRequest,
) -> (Option<String>, Option<String>) {
    let mut requests = batch.iter();
    let Some(request) = requests.next() else {
        return (None, None);
    };
    if requests.next().is_some() {
        return (None, Some("batch".to_owned()));
    }

    graphql_request_operation_metadata(request)
}

fn graphql_request_operation_metadata(
    request: &GraphQLRequest,
) -> (Option<String>, Option<String>) {
    let requested_name = request.operation_name.clone();
    let Ok(document) = async_graphql::parser::parse_query(&request.query) else {
        return (requested_name, None);
    };

    let selected_operation = match request.operation_name.as_deref() {
        Some(name) => document
            .operations
            .iter()
            .find(|(operation_name, _)| operation_name.is_some_and(|value| value.as_str() == name)),
        None => {
            let mut operations = document.operations.iter();
            let first = operations.next();
            if operations.next().is_none() {
                first
            } else {
                None
            }
        }
    };
    let Some((document_name, operation)) = selected_operation else {
        return (requested_name, None);
    };

    (
        requested_name.or_else(|| document_name.map(ToString::to_string)),
        Some(operation.node.ty.to_string()),
    )
}

impl ResolvedActor {
    fn requires_authless_web_client_proof(&self) -> bool {
        self.source == ResolvedActorSource::AuthlessDefault
    }

    fn is_interactive_session(&self) -> bool {
        self.source == ResolvedActorSource::AuthenticatedToken
            && !self.token_claims.is_oauth_access_token()
    }

    fn is_authless_default_session(&self) -> bool {
        self.source == ResolvedActorSource::AuthlessDefault
    }

    fn can_manage_api_keys(&self) -> bool {
        self.is_interactive_session() || self.source == ResolvedActorSource::AuthlessDefault
    }

    fn audit_display_name(&self) -> String {
        self.user.username.clone()
    }

    fn mfa_verification(&self) -> MfaVerification {
        MfaVerification {
            verified_until: self.token_claims.mfa_verified_until,
            step_up_verified_until: self.token_claims.mfa_step_up_verified_until,
            security_action_verified_until: self.token_claims.security_action_verified_until,
            session_scope: self.token_claims.session_scope,
            persist_session: self.token_claims.persist_session,
            auth_session_version: self.token_claims.auth_session_version.clone(),
            password_change_required_after_enrollment: self
                .token_claims
                .password_change_required_after_enrollment,
            oauth_authorization_source: self.token_claims.oauth_authorization_source,
        }
    }

    fn oauth_session(&self) -> Option<OAuthActorSession> {
        if !self.token_claims.is_oauth_access_token() {
            return None;
        }
        Some(OAuthActorSession {
            client_id: self.token_claims.oauth_client_id.clone()?,
            grant_id: self.token_claims.oauth_grant_id.clone()?,
        })
    }
}

fn graphql_ws_connection_data(connection_epoch: u64, actor: Option<ResolvedActor>) -> Data {
    let mut data = Data::default();
    data.insert(ConnectionAuthEpoch(connection_epoch));
    if let Some(actor) = actor {
        data.insert(actor.mfa_verification());
        if actor.is_interactive_session() {
            data.insert(InteractiveSession);
        }
        if actor.is_authless_default_session() {
            data.insert(AuthlessDefaultSession);
        }
        if actor.can_manage_api_keys() {
            data.insert(ApiKeyManagementSession);
        }
        if let Some(oauth_session) = actor.oauth_session() {
            data.insert(oauth_session);
        }
        data.insert(actor.user);
    }
    data
}

async fn touch_oauth_grant_last_used(app: &AppUseCase, actor: Option<&ResolvedActor>) {
    let Some(actor) = actor else {
        return;
    };
    let Some(session) = actor.oauth_session() else {
        return;
    };
    if let Err(error) = app
        .touch_oauth_refresh_grant_last_used(&session.client_id, &session.grant_id)
        .await
    {
        tracing::debug!(
            error = %error,
            client_id = %session.client_id,
            grant_id = %session.grant_id,
            "failed to update OAuth grant last-used timestamp"
        );
    }
}

fn touch_oauth_grant_last_used_background(app: &AppUseCase, actor: &ResolvedActor) {
    let Some(session) = actor.oauth_session() else {
        return;
    };
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(error) = app
            .touch_oauth_refresh_grant_last_used(&session.client_id, &session.grant_id)
            .await
        {
            tracing::debug!(
                error = %error,
                client_id = %session.client_id,
                grant_id = %session.grant_id,
                "failed to update OAuth grant last-used timestamp"
            );
        }
    });
}

struct WsConnectionInitActorRequest<'a> {
    auth_enabled: bool,
    local_bypass_active: bool,
    initial_actor: Option<ResolvedActor>,
    auth_value: Option<&'a str>,
    authless_proof_required: bool,
    proof_state: &'a AuthlessWebClientProofState,
    headers: &'a HeaderMap,
    proof_value: Option<&'a str>,
}

async fn resolve_ws_connection_init_actor(
    app: &AppUseCase,
    request: WsConnectionInitActorRequest<'_>,
) -> Result<Option<ResolvedActor>, async_graphql::Error> {
    let WsConnectionInitActorRequest {
        auth_enabled,
        local_bypass_active,
        initial_actor,
        auth_value,
        authless_proof_required,
        proof_state,
        headers,
        proof_value,
    } = request;

    if let Some(raw) = auth_value {
        return match parse_bearer_token(raw) {
            Some(token) if is_api_key_bearer(token) => Err(async_graphql::Error::new(
                "API keys are not supported for WebSocket connections",
            )),
            Some(token) => match app.authenticate_token_with_claims(token).await {
                Ok((user, token_claims)) => attach_resolved_actor(
                    app,
                    user,
                    token_claims,
                    ResolvedActorSource::AuthenticatedToken,
                    None,
                )
                .await
                .map(|actor| {
                    touch_oauth_grant_last_used_background(app, &actor);
                    Some(actor)
                })
                .map_err(|e| async_graphql::Error::new(format!("authentication failed: {e}"))),
                Err(e) => Err(async_graphql::Error::new(format!(
                    "authentication failed: {e}"
                ))),
            },
            None => Err(async_graphql::Error::new("invalid authorization header")),
        };
    }

    if !authless_proof_required
        && let Some(actor) = initial_actor.as_ref()
        && actor.source == ResolvedActorSource::AuthenticatedToken
    {
        touch_oauth_grant_last_used(app, initial_actor.as_ref()).await;
        return Ok(initial_actor);
    }

    if authless_proof_required && !proof_state.validate_headers(headers, proof_value) {
        return Err(async_graphql::Error::new(
            "Scryer web client proof is required for unauthenticated websocket access",
        ));
    }

    if !auth_enabled || local_bypass_active {
        return Ok(initial_actor);
    }

    Ok(None)
}

async fn resolve_actor(
    state: &AuthState,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> AppResult<Option<ResolvedActor>> {
    let snapshot = state.auth_runtime.snapshot();
    let local_bypass = local_ip_bypass_active(&snapshot, headers, remote_addr);
    let actor = match authorization_token_from_headers(headers) {
        Ok(Some(token)) if is_api_key_bearer(token) => {
            match state.app.authenticate_api_key(token).await {
                Ok(authentication) => {
                    let mut user = authentication.user;
                    user.username =
                        format!("api ({}) obo {}", authentication.key_label, user.username);
                    Some((
                        user,
                        AuthenticatedTokenClaims::default(),
                        ResolvedActorSource::ApiKey,
                        Some(authentication.key_id),
                    ))
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Some(token)) => match state.app.authenticate_token_with_claims(token).await {
            Ok((user, token_claims)) => {
                if snapshot.effective_form_login_enabled
                    && token_claims.oauth_authorization_source == OAuthAuthorizationSource::Authless
                {
                    None
                } else {
                    Some((
                        user,
                        token_claims,
                        ResolvedActorSource::AuthenticatedToken,
                        None,
                    ))
                }
            }
            Err(error) => return Err(error),
        },
        Ok(None) if !snapshot.effective_form_login_enabled => {
            resolve_default_user(&state.app, true).await.map(|user| {
                (
                    anonymous_user(user),
                    AuthenticatedTokenClaims::default(),
                    ResolvedActorSource::AuthlessDefault,
                    None,
                )
            })
        }
        Ok(None) if local_bypass => resolve_default_user(&state.app, false).await.map(|user| {
            (
                anonymous_user(user),
                mfa_bypass_token_claims(),
                ResolvedActorSource::AuthlessDefault,
                None,
            )
        }),
        Ok(None) => None,
        Err(error) => return Err(error),
    };

    match actor {
        Some((user, token_claims, source, api_key_id)) => {
            attach_resolved_actor(&state.app, user, token_claims, source, api_key_id)
                .await
                .map(Some)
        }
        None => Ok(None),
    }
}

async fn attach_resolved_actor(
    app: &AppUseCase,
    user: scryer_domain::User,
    token_claims: AuthenticatedTokenClaims,
    source: ResolvedActorSource,
    api_key_id: Option<String>,
) -> AppResult<ResolvedActor> {
    if token_claims.is_oauth_access_token() {
        app.validate_oauth_access_token(
            token_claims
                .oauth_client_id
                .as_deref()
                .expect("OAuth token includes a client ID"),
            token_claims
                .oauth_grant_id
                .as_deref()
                .expect("OAuth token includes a grant ID"),
        )
        .await?;
    }
    let mut user = app.attach_user_authorization(user).await?;
    user.authorization.actor_capabilities = match source {
        ResolvedActorSource::AuthenticatedToken => token_claims.actor_capabilities,
        ResolvedActorSource::ApiKey => ActorCapabilityMask::NONE,
        ResolvedActorSource::AuthlessDefault => ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
    };
    if token_claims.is_oauth_access_token() {
        if token_claims.oauth_authorization_source == OAuthAuthorizationSource::Authless {
            user = anonymous_user(user);
        }
        user.authorization.app = AppPermissionMask::NONE;
        user.authorization.actor_capabilities = ActorCapabilityMask::NONE;
    }
    Ok(ResolvedActor {
        user,
        token_claims,
        source,
        api_key_id,
    })
}

async fn resolve_default_user(
    app_use_case: &AppUseCase,
    create_if_missing: bool,
) -> Option<scryer_domain::User> {
    match app_use_case.find_default_user().await {
        Ok(Some(user)) => Some(user),
        Ok(None) if create_if_missing => app_use_case.find_or_create_default_user().await.ok(),
        Ok(None) => None,
        Err(_) => None,
    }
}

fn anonymous_user(mut user: scryer_domain::User) -> scryer_domain::User {
    user.username = "Anonymous".to_string();
    user
}

fn mfa_bypass_token_claims() -> AuthenticatedTokenClaims {
    AuthenticatedTokenClaims {
        mfa_verified_until: Some(i64::MAX),
        mfa_step_up_verified_until: Some(i64::MAX),
        ..AuthenticatedTokenClaims::default()
    }
}

fn authorization_token_from_headers(headers: &HeaderMap) -> Result<Option<&str>, AppError> {
    let Some(auth_header) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };

    let raw = auth_header
        .to_str()
        .map_err(|_| AppError::Unauthorized("invalid authorization header".into()))?;
    let token = parse_bearer_token(raw)
        .ok_or_else(|| AppError::Unauthorized("invalid authorization header".into()))?;

    Ok(Some(token))
}

fn is_api_key_bearer(token: &str) -> bool {
    token
        .strip_prefix(API_KEY_PREFIX)
        .is_some_and(|suffix| suffix.starts_with('_'))
}

pub(crate) fn parse_bearer_token(raw: &str) -> Option<&str> {
    let mut parts = raw.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if scheme.eq_ignore_ascii_case("bearer") {
        Some(token)
    } else {
        None
    }
}

#[cfg(test)]
mod ws_origin_tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ws_headers(host: &str, origin: Option<&str>, forwarded_proto: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
        }
        if let Some(forwarded_proto) = forwarded_proto {
            headers.insert(
                X_FORWARDED_PROTO,
                HeaderValue::from_str(forwarded_proto).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn websocket_origin_policy_allows_no_origin_clients() {
        let policy = WebSocketOriginPolicy::default();
        let headers = ws_headers("192.168.1.25:8080", None, None);

        assert!(policy.check(&headers).is_ok());
    }

    #[test]
    fn websocket_origin_policy_allows_same_origin_lan_host() {
        let policy = WebSocketOriginPolicy::default();
        let headers = ws_headers("192.168.1.25:8080", Some("http://192.168.1.25:8080"), None);

        assert!(policy.check(&headers).is_ok());
    }

    #[test]
    fn websocket_origin_policy_allows_configured_origin() {
        let policy = WebSocketOriginPolicy {
            allowed_origins: vec!["https://scryer.example.test".to_string()],
        };
        let headers = ws_headers(
            "127.0.0.1:8080",
            Some("https://scryer.example.test"),
            Some("https"),
        );

        assert!(policy.check(&headers).is_ok());
    }

    #[test]
    fn websocket_origin_policy_rejects_cross_site_browser_origin() {
        let policy = WebSocketOriginPolicy::default();
        let headers = ws_headers("192.168.1.25:8080", Some("https://evil.example.test"), None);

        assert!(policy.check(&headers).is_err());
    }

    #[test]
    fn websocket_origin_policy_rejects_malformed_browser_origin() {
        let policy = WebSocketOriginPolicy::default();
        let headers = ws_headers("192.168.1.25:8080", Some("not an origin"), None);

        assert!(policy.check(&headers).is_err());
    }

    #[test]
    fn websocket_origin_policy_requires_forwarded_proto_match_when_present() {
        let policy = WebSocketOriginPolicy::default();
        let headers = ws_headers(
            "scryer.example.test",
            Some("http://scryer.example.test"),
            Some("https"),
        );

        assert!(policy.check(&headers).is_err());
    }
}

fn local_ip_bypass_active(
    snapshot: &scryer_interface::context::AuthRuntimeStateSnapshot,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> bool {
    if !snapshot.effective_form_login_enabled || !snapshot.skip_login_for_local_ips {
        return false;
    }

    request_has_trusted_local_provenance(headers, remote_addr)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AuthlessAccessDecision {
    Allow,
    Reject(AuthlessAccessRejectReason),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AuthlessAccessRejectReason {
    AuthRequired,
    AuthorizationCredential,
    CrossSiteRequest,
    MissingRemoteAddress,
    PublicPeer(IpAddr),
    PublicForwardedClient(IpAddr),
    MalformedForwardedClient,
}

impl std::fmt::Display for AuthlessAccessRejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthRequired => f.write_str("authentication is required"),
            Self::AuthorizationCredential => {
                f.write_str("authenticated credentials cannot request an authless web client proof")
            }
            Self::CrossSiteRequest => {
                f.write_str("request fetch metadata identifies a cross-site request")
            }
            Self::MissingRemoteAddress => f.write_str("missing remote address"),
            Self::PublicPeer(ip) => write!(f, "peer address {ip} is not private/local"),
            Self::PublicForwardedClient(ip) => {
                write!(f, "forwarded client address {ip} is not private/local")
            }
            Self::MalformedForwardedClient => {
                f.write_str("forwarding headers are present but no valid client IP was found")
            }
        }
    }
}

#[cfg(test)]
fn authless_access_decision(
    snapshot: &scryer_interface::context::AuthRuntimeStateSnapshot,
    policy: AuthlessAccessPolicy,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> AuthlessAccessDecision {
    if snapshot.effective_form_login_enabled {
        return AuthlessAccessDecision::Allow;
    }

    if policy.allow_unauthenticated_public_access {
        return AuthlessAccessDecision::Allow;
    }

    let Some(peer_ip) = remote_addr.map(|addr| addr.ip()) else {
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MissingRemoteAddress);
    };

    if !is_local_network_ip(peer_ip) {
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(peer_ip));
    }

    if has_proxy_forwarding_headers(headers) {
        return match forwarded_client_ip_chain(headers) {
            Ok(client_ips) => client_ips
                .into_iter()
                .find(|client_ip| !is_local_network_ip(*client_ip))
                .map_or(AuthlessAccessDecision::Allow, |public_ip| {
                    AuthlessAccessDecision::Reject(
                        AuthlessAccessRejectReason::PublicForwardedClient(public_ip),
                    )
                }),
            Err(_) => {
                AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MalformedForwardedClient)
            }
        };
    }

    AuthlessAccessDecision::Allow
}

async fn authless_access_decision_with_allowlist(
    snapshot: &scryer_interface::context::AuthRuntimeStateSnapshot,
    policy: AuthlessAccessPolicy,
    allowlist: &AuthlessAccessAllowlist,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> AuthlessAccessDecision {
    if snapshot.effective_form_login_enabled {
        return AuthlessAccessDecision::Allow;
    }

    let allowlist_configured = allowlist.is_configured();
    if policy.allow_unauthenticated_public_access && !allowlist_configured {
        return AuthlessAccessDecision::Allow;
    }

    let Some(peer_ip) = remote_addr.map(|addr| addr.ip()) else {
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MissingRemoteAddress);
    };

    if !is_local_network_ip(peer_ip) {
        if allowlist_configured && allowlist.allows_public_ip(peer_ip).await {
            return AuthlessAccessDecision::Allow;
        }
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(peer_ip));
    }

    if has_proxy_forwarding_headers(headers) {
        return match forwarded_client_ip_chain(headers) {
            Ok(client_ips) => {
                for public_ip in client_ips
                    .into_iter()
                    .filter(|client_ip| !is_local_network_ip(*client_ip))
                {
                    if !allowlist_configured || !allowlist.allows_public_ip(public_ip).await {
                        return AuthlessAccessDecision::Reject(
                            AuthlessAccessRejectReason::PublicForwardedClient(public_ip),
                        );
                    }
                }

                AuthlessAccessDecision::Allow
            }
            Err(()) => {
                AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MalformedForwardedClient)
            }
        };
    }

    AuthlessAccessDecision::Allow
}

fn request_client_ip(headers: &HeaderMap, remote_addr: Option<SocketAddr>) -> Option<IpAddr> {
    let peer_ip = remote_addr?.ip();
    if is_trusted_proxy_ip(peer_ip)
        && let Some(forwarded_ip) = forwarded_client_ip(headers)
    {
        return Some(forwarded_ip);
    }
    Some(peer_ip)
}

fn default_persist_session_for_request(
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> bool {
    request_has_trusted_local_provenance(headers, remote_addr)
}

fn request_has_trusted_local_provenance(
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> bool {
    let Some(peer_ip) = remote_addr.map(|addr| addr.ip()) else {
        return false;
    };
    if has_proxy_forwarding_headers(headers) {
        return is_trusted_proxy_ip(peer_ip)
            && forwarded_client_ip_chain(headers)
                .is_ok_and(|client_ips| client_ips.into_iter().all(is_local_network_ip));
    }
    is_local_network_ip(peer_ip)
}

fn forwarded_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    x_forwarded_for_client_ip(headers)
        .or_else(|| x_real_ip_client_ip(headers))
        .or_else(|| forwarded_header_client_ip(headers))
}

fn forwarded_client_ip_chain(headers: &HeaderMap) -> Result<Vec<IpAddr>, ()> {
    let mut ips = Vec::new();
    collect_x_forwarded_for_ips(headers, &mut ips)?;
    collect_x_real_ip_ips(headers, &mut ips)?;
    collect_forwarded_header_ips(headers, &mut ips)?;

    if ips.is_empty() { Err(()) } else { Ok(ips) }
}

fn has_proxy_forwarding_headers(headers: &HeaderMap) -> bool {
    headers.contains_key("x-forwarded-for")
        || headers.contains_key("x-real-ip")
        || headers.contains_key(header::FORWARDED)
        || headers.contains_key("x-forwarded-host")
        || headers.contains_key(X_FORWARDED_PROTO)
}

fn x_forwarded_for_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').find_map(parse_forwarded_ip_token))
}

fn x_real_ip_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_forwarded_ip_token)
}

fn forwarded_header_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(header::FORWARDED)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value.split(',').find_map(|entry| {
                entry.split(';').find_map(|part| {
                    let (name, raw_value) = part.split_once('=')?;
                    if !name.trim().eq_ignore_ascii_case("for") {
                        return None;
                    }
                    parse_forwarded_ip_token(raw_value)
                })
            })
        })
}

fn collect_x_forwarded_for_ips(headers: &HeaderMap, ips: &mut Vec<IpAddr>) -> Result<(), ()> {
    for value in headers.get_all("x-forwarded-for") {
        let value = value.to_str().map_err(|_| ())?;
        for token in value.split(',') {
            ips.push(parse_forwarded_ip_token(token).ok_or(())?);
        }
    }
    Ok(())
}

fn collect_x_real_ip_ips(headers: &HeaderMap, ips: &mut Vec<IpAddr>) -> Result<(), ()> {
    for value in headers.get_all("x-real-ip") {
        let value = value.to_str().map_err(|_| ())?;
        ips.push(parse_forwarded_ip_token(value).ok_or(())?);
    }
    Ok(())
}

fn collect_forwarded_header_ips(headers: &HeaderMap, ips: &mut Vec<IpAddr>) -> Result<(), ()> {
    for value in headers.get_all(header::FORWARDED) {
        let value = value.to_str().map_err(|_| ())?;
        for entry in value.split(',') {
            for part in entry.split(';') {
                let Some((name, raw_value)) = part.split_once('=') else {
                    continue;
                };
                if name.trim().eq_ignore_ascii_case("for") {
                    ips.push(parse_forwarded_ip_token(raw_value).ok_or(())?);
                }
            }
        }
    }
    Ok(())
}

fn parse_forwarded_ip_token(raw: &str) -> Option<IpAddr> {
    let token = raw.trim().trim_matches('"');
    if token.is_empty() || token.eq_ignore_ascii_case("unknown") {
        return None;
    }

    token
        .parse::<IpAddr>()
        .ok()
        .or_else(|| token.parse::<SocketAddr>().ok().map(|addr| addr.ip()))
        .or_else(|| {
            let bracketed = token.strip_prefix('[')?;
            let end = bracketed.find(']')?;
            bracketed[..end].parse::<IpAddr>().ok()
        })
}

fn is_trusted_proxy_ip(ip: IpAddr) -> bool {
    ip.is_loopback() || is_local_network_ip(ip)
}

fn is_local_network_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => ipv4.is_private() || ipv4.is_loopback() || ipv4.is_link_local(),
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unique_local()
                || ipv6.is_unicast_link_local()
                || ipv6
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_local_network_ip(IpAddr::V4(mapped)))
        }
    }
}

pub(crate) async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok", "ready": true}))
}

pub(crate) async fn rate_limit_http_api(
    State(auth_state): State<AuthState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(rate_limit_class) = classify_http_rate_limit(request.method(), request.uri().path())
    else {
        return next.run(request).await;
    };

    let client_ip =
        request_client_ip(request.headers(), Some(remote_addr)).unwrap_or(remote_addr.ip());
    let key = RateLimitKey::for_client_and_peer(client_ip, remote_addr.ip(), None);
    match auth_state.rate_limiter.check_http(rate_limit_class, &key) {
        Ok(()) => next.run(request).await,
        Err(decision) => {
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse::new(decision.message)),
            )
                .into_response();
            if let Some(retry_after) = decision.retry_after
                && let Ok(value) = http::HeaderValue::from_str(&retry_after.as_secs().to_string())
            {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            response
        }
    }
}

#[cfg(test)]
fn skip_http_rate_limit(_method: &Method, path: &str) -> bool {
    classify_http_rate_limit(_method, path).is_none()
}

fn classify_http_rate_limit(_method: &Method, path: &str) -> Option<HttpRateLimitClass> {
    if path == "/authless-client" {
        return Some(HttpRateLimitClass::AuthlessClient);
    }
    if path == "/oauth/token" || path == "/oauth/authorize/decision" {
        return Some(HttpRateLimitClass::OAuth);
    }
    if path.starts_with("/backups/") || path == "/api" || path.starts_with("/api/") {
        return Some(HttpRateLimitClass::Api);
    }
    None
}

pub(crate) fn map_app_error(error: AppError) -> Response {
    match error {
        AppError::Unauthorized(message) => {
            (StatusCode::UNAUTHORIZED, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::Validation(message) => {
            (StatusCode::BAD_REQUEST, Json(ErrorResponse::new(message))).into_response()
        }
        // The refusal code is a GraphQL-side contract; over REST this stays the
        // validation failure it is.
        AppError::LocationPlanRefused { message, .. } => {
            (StatusCode::BAD_REQUEST, Json(ErrorResponse::new(message))).into_response()
        }
        // The retired direct root write is a GraphQL-side contract too; over
        // REST it stays the validation failure it is.
        AppError::DirectRootWriteRetired { message, .. } => {
            (StatusCode::BAD_REQUEST, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::NoAutoEligibleRelease { .. } => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "no auto-eligible release found".to_string(),
            )),
        )
            .into_response(),
        AppError::PluginInstallInProgress(message) => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::NotFound(message) => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::DownloadFeedbackTimeout(message) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse::new(message)),
        )
            .into_response(),
        AppError::DownloadSubmitAmbiguous(message) => {
            (StatusCode::BAD_GATEWAY, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::DownloadSubmitAmbiguousWithClient { message, .. } => {
            (StatusCode::BAD_GATEWAY, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::DownloadSubmitRejected(message) => {
            (StatusCode::BAD_REQUEST, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::DownloadSourceGone(message)
        | AppError::DownloadSubmitUnavailable(message)
        | AppError::DownloadSubmitFailoverExhausted(message) => {
            (StatusCode::BAD_GATEWAY, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::ArchiveExtractionPluginRequired { message, .. } => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::ArchiveExtractionTimedOut { message } => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse::new(message)),
        )
            .into_response(),
        AppError::TemporaryUnavailable {
            message,
            retry_after,
            ..
        } => {
            let mut response = (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::new(message)),
            )
                .into_response();
            if let Some(delay) = retry_after
                && let Ok(value) = HeaderValue::from_str(&delay.as_secs().to_string())
            {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            response
        }
        AppError::MfaStepUpRequired(message)
        | AppError::ReauthenticationRequired(message)
        | AppError::TotpEnrollmentRequired(message)
        | AppError::MfaEnrollmentRequired(message)
        | AppError::PasswordChangeRequired(message)
        | AppError::TotpInvalidCode(message)
        | AppError::TotpRecoveryCodeUsed(message) => {
            (StatusCode::UNAUTHORIZED, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::Canceled(message) => (
            StatusCode::REQUEST_TIMEOUT,
            Json(ErrorResponse::new(message)),
        )
            .into_response(),
        AppError::ManualReconciliationRequired(message) => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new(message))).into_response()
        }
        error @ (AppError::ImportSourceInspection { .. }
        | AppError::UnsupportedImportSource { .. }
        | AppError::ImportSourceChanged { .. }) => {
            let error_id = Id::new().0;
            tracing::error!(
                error_id = %error_id,
                error_kind = "ImportSource",
                error = %error,
                "masked internal import source error"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::with_error_id(
                    INTERNAL_SERVER_ERROR_MESSAGE.to_string(),
                    error_id,
                )),
            )
                .into_response()
        }
        AppError::ImportEvidenceUnavailable(message) | AppError::Repository(message) => {
            let error_id = Id::new().0;
            tracing::error!(
                error_id = %error_id,
                error_kind = "Repository",
                error = %message,
                "masked internal repository error"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::with_error_id(
                    INTERNAL_SERVER_ERROR_MESSAGE.to_string(),
                    error_id,
                )),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod integration_test_common;

#[cfg(test)]
mod tests {
    use super::integration_test_common as common;
    use super::*;

    use axum::Router;
    use axum::body::to_bytes;
    use axum::http::HeaderValue;
    use axum::routing::{get, post};
    use serde_json::Value;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::{
        LazyLock, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tower::ServiceExt;

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct AvatarCountingVerifier {
        fetch_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl scryer_application::ExternalIdentityVerifier for AvatarCountingVerifier {
        async fn verify_plex(
            &self,
            _: &str,
            _: Option<&str>,
            _: &str,
        ) -> AppResult<scryer_application::VerifiedExternalIdentity> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn discover_plex_servers(
            &self,
            _: &str,
        ) -> AppResult<Vec<scryer_application::PlexServerDiscovery>> {
            Ok(Vec::new())
        }

        async fn verify_jellyfin(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> AppResult<scryer_application::VerifiedExternalIdentity> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn test_jellyfin_connection(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn test_jellyfin_api_key(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn exchange_jellyfin_admin_api_key(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> AppResult<String> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn list_jellyfin_users(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> AppResult<Vec<scryer_application::JellyfinServerUser>> {
            Ok(Vec::new())
        }

        async fn fetch_emby_user_avatar(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> AppResult<Option<scryer_application::EmbyAvatar>> {
            self.fetch_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(scryer_application::EmbyAvatar {
                content_type: "image/png".into(),
                bytes: vec![1, 2, 3],
                etag: None,
                last_modified: None,
            }))
        }

        async fn list_plex_users(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> AppResult<Vec<scryer_application::PlexServerUser>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn emby_avatar_handler_requires_full_scope_and_manage_users_before_upstream() {
        use scryer_application::MediaServerConnectionRepository as _;
        use scryer_application::testing::AppUseCaseTestExt as _;

        let context = common::TestContext::new().await;
        let fetch_calls = Arc::new(AtomicUsize::new(0));
        let connection_store = Arc::new(
            scryer_infrastructure_library::media::servers::MediaServerConnectionStore::new(
                context.db.datastore(),
                context.db.encryption_key_state(),
            ),
        );
        let app = context.app.with_test_overrides(|builder| {
            builder
                .with_media_server_connection_store(connection_store.clone())
                .with_external_identity_verifier(Arc::new(AvatarCountingVerifier {
                    fetch_calls: Arc::clone(&fetch_calls),
                }))
        });
        let now = chrono::Utc::now();
        connection_store
            .create(scryer_domain::MediaServerConnection {
                id: "emby-main".into(),
                provider: scryer_domain::MediaServerProvider::Emby,
                display_name: "Emby".into(),
                base_url: "https://emby.example.test".into(),
                external_url: None,
                enabled: true,
                login_enabled: true,
                linking_enabled: false,
                auto_add_enabled: false,
                default_app_permissions: AppPermissionMask::NONE,
                default_library_grants: Vec::new(),
                machine_id: None,
                api_key: Some("emby-admin-key".into()),
                emby_server_id: Some("emby-server".into()),
                emby_connect_enabled: false,
                path_mappings: Vec::new(),
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("create Emby connection");

        let admin = context
            .app
            .find_or_create_default_user()
            .await
            .expect("default administrator");
        let ordinary = context
            .app
            .create_user(
                &admin,
                "avatar-ordinary".into(),
                "ordinary-password".into(),
                AppPermissionMask::NONE,
                Vec::new(),
            )
            .await
            .expect("create ordinary actor");
        let manager = context
            .app
            .create_user(
                &admin,
                "avatar-manager".into(),
                "manager-password".into(),
                AppPermissionMask::MANAGE_USERS,
                Vec::new(),
            )
            .await
            .expect("create ManageUsers actor");
        let ordinary_token = app
            .issue_access_token(&ordinary)
            .await
            .expect("issue ordinary token");
        let mfa_enrollment_token = app
            .issue_mfa_enrollment_token(&ordinary, false, false, None)
            .await
            .expect("issue MFA enrollment token");
        let manager_token = app
            .issue_access_token(&manager)
            .await
            .expect("issue ManageUsers token");

        let state = AuthState {
            app,
            schema: context.schema.clone(),
            auth_runtime: AuthRuntimeStateHandle::new(auth_enabled_snapshot()),
            rate_limiter: ScryerRateLimiter::from_env(),
            ws_origin_policy: WebSocketOriginPolicy::default(),
            authless_web_client_proof: AuthlessWebClientProofState::new(),
        };
        let router = Router::new()
            .route(
                "/api/media-server-avatars/{connection_id}/{user_id}/{image_tag}",
                get(emby_avatar_handler),
            )
            .with_state(state);
        let request = |token: Option<&str>| {
            let mut request = Request::builder()
                .uri("/api/media-server-avatars/emby-main/external-user/avatar-tag")
                .body(Body::empty())
                .expect("avatar request");
            if let Some(token) = token {
                request.headers_mut().insert(
                    header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {token}")).expect("authorization"),
                );
            }
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))));
            request
        };

        for token in [
            None,
            Some("invalid-token"),
            Some(mfa_enrollment_token.as_str()),
        ] {
            let response = router
                .clone()
                .oneshot(request(token))
                .await
                .expect("avatar response");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(fetch_calls.load(Ordering::SeqCst), 0);
        }

        let response = router
            .clone()
            .oneshot(request(Some(&ordinary_token)))
            .await
            .expect("avatar response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(fetch_calls.load(Ordering::SeqCst), 0);

        let response = router
            .oneshot(request(Some(&manager_token)))
            .await
            .expect("avatar response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(fetch_calls.load(Ordering::SeqCst), 1);
        let avatar_bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("avatar bytes");
        assert_eq!(avatar_bytes.as_ref(), &[1, 2, 3]);
    }

    #[tokio::test]
    async fn api_keys_cannot_manage_accounts_and_revoked_keys_are_rejected() {
        let context = common::TestContext::new().await;
        let admin = context
            .app
            .find_or_create_default_user()
            .await
            .expect("default administrator");
        let created = context
            .app
            .create_api_key(
                &admin,
                scryer_application::CreateApiKey {
                    label: "local integration".into(),
                    expiry: scryer_application::ApiKeyExpiryPreset::Never,
                },
            )
            .await
            .expect("create API key");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", created.raw_key))
                .expect("API key authorization header"),
        );
        let state = AuthState {
            app: context.app.clone(),
            schema: context.schema.clone(),
            auth_runtime: context.auth_runtime.clone(),
            rate_limiter: ScryerRateLimiter::from_env(),
            ws_origin_policy: WebSocketOriginPolicy::default(),
            authless_web_client_proof: AuthlessWebClientProofState::new(),
        };

        let actor = resolve_actor(
            &state,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))),
        )
        .await
        .expect("authenticate API key")
        .expect("API-key actor");
        assert_eq!(actor.user.username, "api (local integration) obo admin");
        assert_eq!(
            actor.user.authorization.actor_capabilities,
            ActorCapabilityMask::NONE
        );
        assert!(matches!(
            context.app.totp_enrollment_start(&actor.user).await,
            Err(AppError::Unauthorized(_))
        ));

        context
            .settings_store
            .batch_ensure_setting_definitions(vec![
                scryer_infrastructure_sql::types::SettingDefinitionSeed {
                    category: "security".into(),
                    scope: "system".into(),
                    key_name: "auth.mfa.require_config_step_up".into(),
                    data_type: "boolean".into(),
                    default_value_json: "false".into(),
                    is_sensitive: false,
                    validation_json: None,
                },
            ])
            .await
            .expect("seed configuration MFA step-up setting");
        context
            .settings_store
            .upsert_setting_value(
                "system",
                "auth.mfa.require_config_step_up",
                None,
                "true",
                "test",
                None,
            )
            .await
            .expect("enable configuration MFA step-up");
        context
            .auth_runtime
            .apply_saved_security_settings(true, false);
        let router = Router::new()
            .route("/graphql", post(graphql_handler))
            .with_state(state.clone());
        let mut request = Request::builder()
            .method("POST")
            .uri("/graphql")
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", created.raw_key))
                    .expect("API key authorization header"),
            )
            .body(Body::from(
                r#"{"query":"mutation { createUser(input: { username: \"blocked\", password: \"testpass123\", appPermissions: [], libraryPermissions: [] }) { id } }"}"#,
            ))
            .expect("GraphQL request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))));
        let response = router.oneshot(request).await.expect("GraphQL response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("GraphQL response body");
        let body: Value = serde_json::from_slice(&body).expect("GraphQL JSON response");
        assert_eq!(
            body["errors"][0]["extensions"]["code"], "TOTP_ENROLLMENT_REQUIRED",
            "API key must not satisfy configuration MFA step-up: {body}"
        );

        assert!(
            context
                .app
                .revoke_api_key(&admin, &created.api_key.id)
                .await
                .expect("revoke API key")
        );
        assert!(matches!(
            resolve_actor(
                &state,
                &headers,
                Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))),
            )
            .await,
            Err(AppError::Unauthorized(_))
        ));

        let router = Router::new()
            .route("/graphql", post(graphql_handler))
            .with_state(state);
        let mut request = Request::builder()
            .method("POST")
            .uri("/graphql")
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", created.raw_key))
                    .expect("API key authorization header"),
            )
            .body(Body::from(r#"{"query":"{ __typename }"}"#))
            .expect("GraphQL request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))));
        let response = router.oneshot(request).await.expect("GraphQL response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("GraphQL response body");
        assert!(
            String::from_utf8_lossy(&body).contains("API key is invalid or no longer authorized")
        );
    }

    #[tokio::test]
    async fn invalid_bearer_never_falls_back_to_the_authless_default_actor() {
        let context = common::TestContext::new().await;
        let state = AuthState {
            app: context.app.clone(),
            schema: context.schema.clone(),
            auth_runtime: AuthRuntimeStateHandle::new(auth_disabled_snapshot()),
            rate_limiter: ScryerRateLimiter::from_env(),
            ws_origin_policy: WebSocketOriginPolicy::default(),
            authless_web_client_proof: AuthlessWebClientProofState::new(),
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer invalid-session-token"),
        );

        let result = resolve_actor(
            &state,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))),
        )
        .await;

        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn authless_browser_session_can_create_an_admin_api_key() {
        let context = common::TestContext::new().await;
        let proof_state = AuthlessWebClientProofState::new();
        let state = AuthState {
            app: context.app.clone(),
            schema: context.schema.clone(),
            auth_runtime: AuthRuntimeStateHandle::new(auth_disabled_snapshot()),
            rate_limiter: ScryerRateLimiter::from_env(),
            ws_origin_policy: WebSocketOriginPolicy::default(),
            authless_web_client_proof: proof_state.clone(),
        };
        let router = Router::new()
            .route("/graphql", post(graphql_handler))
            .with_state(state);
        let mut request = request_with_peer_and_authless_proof(
            "/graphql",
            SocketAddr::from((Ipv4Addr::LOCALHOST, 3000)),
            &proof_state,
        );
        *request.method_mut() = Method::POST;
        request.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        *request.body_mut() = Body::from(
            r#"{"query":"mutation { createMyApiKey(input: { label: \"browser\", expiry: DAYS_90 }) { apiKey } }"}"#,
        );

        let response = router.oneshot(request).await.expect("GraphQL response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("GraphQL response body");
        let body: Value = serde_json::from_slice(&body).expect("GraphQL JSON response");
        let raw_key = body["data"]["createMyApiKey"]["apiKey"]
            .as_str()
            .unwrap_or_else(|| panic!("unexpected authless API-key response: {body}"));
        let authenticated = context
            .app
            .authenticate_api_key(raw_key)
            .await
            .expect("created API key authenticates");
        let admin = context
            .app
            .find_or_create_default_user()
            .await
            .expect("default administrator");
        assert_eq!(authenticated.user.id, admin.id);
        assert_eq!(authenticated.user.username, admin.username);
    }

    #[tokio::test]
    async fn websocket_invalid_bearer_never_falls_back_to_authless_access() {
        let context = common::TestContext::new().await;
        let proof_state = AuthlessWebClientProofState::new();
        let (headers, proof) = authless_ws_proof_headers(&proof_state);

        let result = resolve_ws_connection_init_actor(
            &context.app,
            WsConnectionInitActorRequest {
                auth_enabled: false,
                local_bypass_active: false,
                initial_actor: Some(authless_ws_test_actor()),
                auth_value: Some("Bearer invalid-session-token"),
                authless_proof_required: true,
                proof_state: &proof_state,
                headers: &headers,
                proof_value: Some(&proof),
            },
        )
        .await;

        let error = match result {
            Ok(_) => panic!("invalid WebSocket credentials must not fall back to authless access"),
            Err(error) => error,
        };
        assert!(error.message.contains("authentication failed"));
    }

    fn clear_cors_env() {
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::remove_var(CORS_ALLOWED_ORIGINS_ENV);
            std::env::remove_var(WS_ALLOWED_ORIGINS_ENV);
            std::env::remove_var(PRODUCTION_CORS_OPT_IN_ENV);
            std::env::remove_var(WEB_UI_URL_ENV);
        }
    }

    #[test]
    fn default_cors_origins_match_runtime_mode() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();

        let origins = default_cors_allowed_origins_for_mode(cfg!(debug_assertions));
        let dev_origins = default_cors_allowed_origins_for_mode(true);
        let release_origins = default_cors_allowed_origins_for_mode(false);

        if cfg!(debug_assertions) {
            assert!(
                origins
                    .iter()
                    .any(|origin| origin == "http://localhost:3000")
            );
        } else {
            assert!(origins.is_empty());
        }
        assert!(
            dev_origins
                .iter()
                .any(|origin| origin == "http://localhost:3000")
        );
        assert!(release_origins.is_empty());
    }

    #[test]
    fn web_ui_origin_only_extends_dev_mode_defaults() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(WEB_UI_URL_ENV, "http://127.0.0.1:4545/app");
        }

        let origins = default_cors_allowed_origins_for_mode(cfg!(debug_assertions));
        let dev_origins = default_cors_allowed_origins_for_mode(true);
        let release_origins = default_cors_allowed_origins_for_mode(false);

        if cfg!(debug_assertions) {
            assert!(
                origins
                    .iter()
                    .any(|origin| origin == "http://127.0.0.1:4545")
            );
            assert!(
                origins
                    .iter()
                    .any(|origin| origin == "http://localhost:4545")
            );
            assert!(
                origins
                    .iter()
                    .any(|origin| origin == "http://host.docker.internal:4545")
            );
        } else {
            assert!(origins.is_empty());
        }

        assert!(
            dev_origins
                .iter()
                .any(|origin| origin == "http://127.0.0.1:4545")
        );
        assert!(
            dev_origins
                .iter()
                .any(|origin| origin == "http://localhost:4545")
        );
        assert!(
            dev_origins
                .iter()
                .any(|origin| origin == "http://host.docker.internal:4545")
        );
        assert!(release_origins.is_empty());
        clear_cors_env();
    }

    #[test]
    fn cors_env_rejects_wildcard_origins() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(
                CORS_ALLOWED_ORIGINS_ENV,
                "*, http://*, https://*, http://localhost:3000",
            );
        }

        let config = CorsConfig::from_env_for_mode(true);

        assert!(!config.allow_all);
        assert!(config.is_allowed("http://localhost:3000"));
        assert!(!config.is_allowed("http://evil.example"));
        assert!(
            !config
                .allowed_origins
                .iter()
                .any(|origin| matches!(origin.as_str(), "*" | "http://*" | "https://*"))
        );
        clear_cors_env();
    }

    #[test]
    fn release_mode_ignores_cors_env_without_opt_in() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(CORS_ALLOWED_ORIGINS_ENV, "http://localhost:3000");
        }

        let config = CorsConfig::from_env_for_mode(false);

        assert!(!config.allow_all);
        assert!(config.allowed_origins.is_empty());
        assert!(!config.is_allowed("http://localhost:3000"));
        clear_cors_env();
    }

    #[test]
    fn release_mode_ignores_cors_env_even_with_legacy_opt_in() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(CORS_ALLOWED_ORIGINS_ENV, "http://localhost:3000/app");
            std::env::set_var(PRODUCTION_CORS_OPT_IN_ENV, "1");
        }

        let config = CorsConfig::from_env_for_mode(false);

        assert!(!config.allow_all);
        assert!(config.allowed_origins.is_empty());
        assert!(!config.is_allowed("http://localhost:3000"));
        assert!(!config.is_allowed("http://127.0.0.1:3000"));
        clear_cors_env();
    }

    #[test]
    fn websocket_origin_parser_rejects_wildcards() {
        let origins =
            parse_websocket_allowed_origins("*, http://*, https://*, http://localhost:3000/app");

        assert_eq!(origins, vec!["http://localhost:3000"]);
    }

    #[test]
    fn release_mode_ignores_websocket_env_without_opt_in() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(WS_ALLOWED_ORIGINS_ENV, "http://localhost:3000");
        }

        let cors = CorsConfig {
            allow_all: false,
            allowed_origins: Vec::new(),
        };
        let policy = WebSocketOriginPolicy::from_env_for_mode(&cors, false);

        assert!(policy.allowed_origins.is_empty());
        clear_cors_env();
    }

    #[test]
    fn release_mode_ignores_websocket_env_even_with_legacy_opt_in() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(WS_ALLOWED_ORIGINS_ENV, "http://localhost:3000/app");
            std::env::set_var(PRODUCTION_CORS_OPT_IN_ENV, "1");
        }

        let cors = CorsConfig {
            allow_all: false,
            allowed_origins: Vec::new(),
        };
        let policy = WebSocketOriginPolicy::from_env_for_mode(&cors, false);

        assert!(policy.allowed_origins.is_empty());
        clear_cors_env();
    }

    fn graphql_error_response_with_code(code: &str) -> async_graphql::BatchResponse {
        let mut extensions = ErrorExtensionValues::default();
        extensions.set("code", code);
        let mut error = ServerError::new("graphQL error", None);
        error.extensions = Some(extensions);
        async_graphql::BatchResponse::Single(GraphQLResponse::from_errors(vec![error]))
    }

    #[test]
    fn graphql_authentication_required_response_uses_unauthorized_status() {
        let response = graphql_error_response_with_code(AUTHENTICATION_REQUIRED_CODE);

        assert_eq!(graphql_response_status(&response), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn graphql_mfa_step_up_response_uses_step_up_status() {
        let response = graphql_error_response_with_code(MFA_STEP_UP_REQUIRED_CODE);

        assert_eq!(
            graphql_response_status(&response),
            StatusCode::from_u16(MFA_STEP_UP_REQUIRED_STATUS_CODE).unwrap()
        );
    }

    #[test]
    fn graphql_non_mfa_error_response_keeps_ok_status() {
        let response = graphql_error_response_with_code("VALIDATION_FAILED");

        assert_eq!(graphql_response_status(&response), StatusCode::OK);
    }

    #[test]
    fn graphql_batched_mfa_step_up_response_uses_step_up_status() {
        let mut extensions = ErrorExtensionValues::default();
        extensions.set("code", MFA_STEP_UP_REQUIRED_CODE);
        let mut error = ServerError::new("MFA step-up required", None);
        error.extensions = Some(extensions);
        let response = async_graphql::BatchResponse::Batch(vec![
            GraphQLResponse::new(async_graphql::Value::Null),
            GraphQLResponse::from_errors(vec![error]),
        ]);

        assert_eq!(
            graphql_response_status(&response),
            StatusCode::from_u16(MFA_STEP_UP_REQUIRED_STATUS_CODE).unwrap()
        );
    }

    #[test]
    fn graphql_batched_authentication_required_takes_precedence_over_step_up() {
        let mut auth_extensions = ErrorExtensionValues::default();
        auth_extensions.set("code", AUTHENTICATION_REQUIRED_CODE);
        let mut auth_error = ServerError::new("authentication required", None);
        auth_error.extensions = Some(auth_extensions);

        let mut mfa_extensions = ErrorExtensionValues::default();
        mfa_extensions.set("code", MFA_STEP_UP_REQUIRED_CODE);
        let mut mfa_error = ServerError::new("MFA step-up required", None);
        mfa_error.extensions = Some(mfa_extensions);

        let response = async_graphql::BatchResponse::Batch(vec![
            GraphQLResponse::from_errors(vec![mfa_error]),
            GraphQLResponse::from_errors(vec![auth_error]),
        ]);

        assert_eq!(graphql_response_status(&response), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn graphql_timeout_response_reports_execution_timeout() {
        let response = graphql_execution_timeout_response();
        let body = serde_json::to_value(&response).expect("timeout response serializes");
        let timeout_seconds = graphql_post_execution_timeout().as_secs();

        assert_eq!(
            body["errors"][0]["message"],
            format!("GraphQL request timed out after {timeout_seconds} seconds")
        );
        assert_eq!(
            body["errors"][0]["extensions"]["code"],
            GRAPHQL_POST_EXECUTION_TIMEOUT_CODE
        );
        assert_eq!(
            body["errors"][0]["extensions"]["timeoutSeconds"],
            timeout_seconds
        );
    }

    #[test]
    fn graphql_timeout_covers_default_and_configured_feedback_windows() {
        assert_eq!(
            graphql_post_execution_timeout_for(
                scryer_outbound_http::DEFAULT_DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT,
            ),
            Duration::from_secs(310)
        );
        assert_eq!(
            graphql_post_execution_timeout_for(Duration::from_secs(600)),
            Duration::from_secs(605)
        );
    }

    #[tokio::test]
    async fn repository_error_response_masks_details_and_includes_error_id() {
        let response = map_app_error(AppError::Repository(
            "database password leaked in upstream detail".into(),
        ));

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body_text = String::from_utf8(body.to_vec()).expect("response body is utf8");
        let body: Value = serde_json::from_str(&body_text).expect("response body is json");

        assert_eq!(body["error"], INTERNAL_SERVER_ERROR_MESSAGE);
        assert!(
            body["error_id"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(!body_text.contains("database password"));
        assert!(!body_text.contains("upstream detail"));
    }

    #[tokio::test]
    async fn validation_error_response_omits_error_id() {
        let response = map_app_error(AppError::Validation("bad request".into()));

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body: Value = serde_json::from_slice(&body).expect("response body is json");

        assert_eq!(body["error"], "bad request");
        assert!(body.get("error_id").is_none());
    }

    #[tokio::test]
    async fn temporary_unavailable_response_preserves_retry_after() {
        let response = map_app_error(AppError::temporary_unavailable(
            "subtitle provider is temporarily deferred",
            Some(Duration::from_secs(120)),
        ));

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.headers().get(header::RETRY_AFTER),
            Some(&HeaderValue::from_static("120"))
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body: Value = serde_json::from_slice(&body).expect("response body is json");
        assert_eq!(body["error"], "subtitle provider is temporarily deferred");
        assert!(body.get("error_id").is_none());
    }

    #[test]
    fn local_network_ip_ranges_match_expected_blocks() {
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            172, 16, 0, 1
        ))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            172, 31, 255, 254
        ))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            192, 168, 5, 10
        ))));
        assert!(!is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            172, 15, 0, 1
        ))));
        assert!(!is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            172, 32, 0, 1
        ))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            169, 254, 10, 20
        ))));
        assert!(is_local_network_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_local_network_ip(IpAddr::V6(Ipv6Addr::new(
            0xfc00, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(is_local_network_ip(IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(!is_local_network_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_local_network_ip(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0x4860, 0, 0, 0, 0, 0, 0x8888
        ))));
    }

    fn auth_disabled_snapshot() -> scryer_interface::context::AuthRuntimeStateSnapshot {
        scryer_interface::context::AuthRuntimeStateSnapshot {
            form_login_enabled: false,
            skip_login_for_local_ips: false,
            effective_form_login_enabled: false,
            webauthn_configured: false,
            passkey_enabled: false,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        }
    }

    fn auth_enabled_snapshot() -> scryer_interface::context::AuthRuntimeStateSnapshot {
        scryer_interface::context::AuthRuntimeStateSnapshot {
            form_login_enabled: true,
            skip_login_for_local_ips: false,
            effective_form_login_enabled: true,
            webauthn_configured: false,
            passkey_enabled: false,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        }
    }

    fn protected_authless_policy() -> AuthlessAccessPolicy {
        AuthlessAccessPolicy {
            allow_unauthenticated_public_access: false,
        }
    }

    fn public_authless_policy() -> AuthlessAccessPolicy {
        AuthlessAccessPolicy {
            allow_unauthenticated_public_access: true,
        }
    }

    #[test]
    fn authless_guard_allows_auth_enabled_requests() {
        let headers = HeaderMap::new();
        let decision = authless_access_decision(
            &auth_enabled_snapshot(),
            protected_authless_policy(),
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
        );

        assert_eq!(decision, AuthlessAccessDecision::Allow);
    }

    #[test]
    fn authless_guard_allows_private_and_loopback_clients() {
        let headers = HeaderMap::new();
        for addr in [
            SocketAddr::from((Ipv4Addr::LOCALHOST, 3000)),
            SocketAddr::from((Ipv4Addr::new(10, 1, 2, 3), 3000)),
            SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000)),
            SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
            SocketAddr::from((Ipv4Addr::new(169, 254, 10, 20), 3000)),
            SocketAddr::from((Ipv6Addr::LOCALHOST, 3000)),
            SocketAddr::from((Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 1), 3000)),
            SocketAddr::from((Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1), 3000)),
        ] {
            assert_eq!(
                authless_access_decision(
                    &auth_disabled_snapshot(),
                    protected_authless_policy(),
                    &headers,
                    Some(addr),
                ),
                AuthlessAccessDecision::Allow,
                "{addr} should be allowed"
            );
        }
    }

    #[test]
    fn authless_guard_rejects_public_clients() {
        let headers = HeaderMap::new();

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(IpAddr::V4(
                Ipv4Addr::new(8, 8, 8, 8)
            )))
        );
        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((
                    Ipv6Addr::new(0x2001, 0x4860, 0, 0, 0, 0, 0, 0x8888),
                    3000,
                ))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(IpAddr::V6(
                Ipv6Addr::new(0x2001, 0x4860, 0, 0, 0, 0, 0, 0x8888)
            )))
        );
    }

    #[test]
    fn authless_guard_public_override_allows_public_clients() {
        let headers = HeaderMap::new();
        let decision = authless_access_decision(
            &auth_disabled_snapshot(),
            public_authless_policy(),
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
        );

        assert_eq!(decision, AuthlessAccessDecision::Allow);
    }

    #[tokio::test]
    async fn authless_guard_public_override_allowlist_allows_matching_ip() {
        let headers = HeaderMap::new();
        let allowlist = AuthlessAccessAllowlist::parse("8.8.8.8");

        assert_eq!(
            authless_access_decision_with_allowlist(
                &auth_disabled_snapshot(),
                public_authless_policy(),
                &allowlist,
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            )
            .await,
            AuthlessAccessDecision::Allow
        );
    }

    #[tokio::test]
    async fn authless_guard_allowlist_implies_public_access_for_matching_ip() {
        let headers = HeaderMap::new();
        let allowlist = AuthlessAccessAllowlist::parse("8.8.8.8");

        assert_eq!(
            authless_access_decision_with_allowlist(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &allowlist,
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            )
            .await,
            AuthlessAccessDecision::Allow
        );
    }

    #[tokio::test]
    async fn authless_guard_public_override_allowlist_ignores_invalid_entries_when_valid_remains() {
        let headers = HeaderMap::new();
        let allowlist = AuthlessAccessAllowlist::parse("https://bad.example, 8.8.8.8");

        assert!(allowlist.is_configured());
        assert_eq!(
            authless_access_decision_with_allowlist(
                &auth_disabled_snapshot(),
                public_authless_policy(),
                &allowlist,
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            )
            .await,
            AuthlessAccessDecision::Allow
        );
        assert_eq!(
            authless_access_decision_with_allowlist(
                &auth_disabled_snapshot(),
                public_authless_policy(),
                &allowlist,
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 4, 4), 3000))),
            )
            .await,
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(IpAddr::V4(
                Ipv4Addr::new(8, 8, 4, 4)
            )))
        );
    }

    #[tokio::test]
    async fn authless_guard_public_override_allowlist_rejects_unlisted_ip() {
        let headers = HeaderMap::new();
        let allowlist = AuthlessAccessAllowlist::parse("203.0.113.10");

        assert_eq!(
            authless_access_decision_with_allowlist(
                &auth_disabled_snapshot(),
                public_authless_policy(),
                &allowlist,
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            )
            .await,
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(IpAddr::V4(
                Ipv4Addr::new(8, 8, 8, 8)
            )))
        );
    }

    #[tokio::test]
    async fn authless_guard_public_override_allowlist_matches_ipv4_and_ipv6_cidr() {
        let headers = HeaderMap::new();
        let allowlist = AuthlessAccessAllowlist::parse("8.8.8.0/24,2001:4860::/32");

        assert_eq!(
            authless_access_decision_with_allowlist(
                &auth_disabled_snapshot(),
                public_authless_policy(),
                &allowlist,
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            )
            .await,
            AuthlessAccessDecision::Allow
        );
        assert_eq!(
            authless_access_decision_with_allowlist(
                &auth_disabled_snapshot(),
                public_authless_policy(),
                &allowlist,
                &headers,
                Some(SocketAddr::from((
                    Ipv6Addr::new(0x2001, 0x4860, 0, 0, 0, 0, 0, 0x8888),
                    3000,
                ))),
            )
            .await,
            AuthlessAccessDecision::Allow
        );
    }

    #[tokio::test]
    async fn authless_guard_public_override_allowlist_matches_cached_dns_host() {
        let headers = HeaderMap::new();
        let allowlist = AuthlessAccessAllowlist::parse("home.example.test");
        allowlist
            .cache_host_for_test(
                "home.example.test",
                vec![IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))],
            )
            .await;

        assert_eq!(
            authless_access_decision_with_allowlist(
                &auth_disabled_snapshot(),
                public_authless_policy(),
                &allowlist,
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            )
            .await,
            AuthlessAccessDecision::Allow
        );
    }

    #[test]
    fn authless_guard_public_override_allows_public_peer() {
        let headers = HeaderMap::new();

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                public_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            ),
            AuthlessAccessDecision::Allow
        );
    }

    #[test]
    fn authless_guard_does_not_trust_forwarded_headers_from_public_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("192.168.1.25"));

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(IpAddr::V4(
                Ipv4Addr::new(8, 8, 8, 8)
            )))
        );
    }

    #[test]
    fn authless_guard_rejects_forwarded_proto_without_client_ip() {
        let mut headers = HeaderMap::new();
        headers.insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MalformedForwardedClient)
        );
    }

    #[test]
    fn authless_guard_rejects_public_forwarded_client_through_private_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicForwardedClient(
                IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))
            ))
        );
    }

    #[tokio::test]
    async fn authless_guard_public_override_allowlist_allows_matching_forwarded_client() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));
        let allowlist = AuthlessAccessAllowlist::parse("8.8.8.8");

        assert_eq!(
            authless_access_decision_with_allowlist(
                &auth_disabled_snapshot(),
                public_authless_policy(),
                &allowlist,
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            )
            .await,
            AuthlessAccessDecision::Allow
        );
    }

    #[test]
    fn authless_guard_rejects_public_ip_anywhere_in_forwarded_for_chain() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 8.8.8.8"),
        );

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicForwardedClient(
                IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))
            ))
        );
    }

    #[test]
    fn authless_guard_rejects_public_ip_anywhere_in_forwarded_header_chain() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("for=192.168.1.25;proto=https, for=8.8.8.8"),
        );

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicForwardedClient(
                IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))
            ))
        );
    }

    #[test]
    fn authless_guard_allows_private_forwarded_client_through_private_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("192.168.1.25"));

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Allow
        );
    }

    #[test]
    fn authless_guard_allows_private_forwarded_chain_through_private_peer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.5"),
        );

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Allow
        );
    }

    #[test]
    fn authless_guard_allows_forwarded_proto_with_private_client_chain() {
        let mut headers = HeaderMap::new();
        headers.insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.5"),
        );

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Allow
        );
    }

    #[test]
    fn authless_guard_rejects_malformed_forwarded_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MalformedForwardedClient)
        );
    }

    #[test]
    fn authless_guard_rejects_malformed_ip_anywhere_in_forwarded_chain() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, not-an-ip"),
        );

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MalformedForwardedClient)
        );
    }

    fn authless_guard_test_app(
        snapshot: scryer_interface::context::AuthRuntimeStateSnapshot,
        policy: AuthlessAccessPolicy,
    ) -> Router {
        authless_guard_test_app_with_proof_state(
            snapshot,
            policy,
            AuthlessWebClientProofState::new(),
        )
    }

    fn authless_guard_test_app_with_proof_state(
        snapshot: scryer_interface::context::AuthRuntimeStateSnapshot,
        policy: AuthlessAccessPolicy,
        _web_client_proof: AuthlessWebClientProofState,
    ) -> Router {
        authless_guard_test_app_with_allowlist(snapshot, policy, AuthlessAccessAllowlist::default())
    }

    fn authless_guard_test_app_with_allowlist(
        snapshot: scryer_interface::context::AuthRuntimeStateSnapshot,
        policy: AuthlessAccessPolicy,
        allowlist: AuthlessAccessAllowlist,
    ) -> Router {
        let state = AuthlessAccessGuardState {
            auth_runtime: AuthRuntimeStateHandle::new(snapshot),
            policy,
            allowlist,
        };
        Router::new()
            .route("/graphql", get(|| async { "graphql ok" }))
            .route("/graphql/ws", get(|| async { "ws ok" }))
            .layer(axum::middleware::from_fn_with_state(
                state,
                enforce_authless_access_guard,
            ))
    }

    fn authless_web_client_test_app(
        snapshot: scryer_interface::context::AuthRuntimeStateSnapshot,
        policy: AuthlessAccessPolicy,
    ) -> Router {
        authless_web_client_test_app_with_allowlist(
            snapshot,
            policy,
            AuthlessAccessAllowlist::default(),
        )
    }

    fn authless_web_client_test_app_with_allowlist(
        snapshot: scryer_interface::context::AuthRuntimeStateSnapshot,
        policy: AuthlessAccessPolicy,
        allowlist: AuthlessAccessAllowlist,
    ) -> Router {
        let state = AuthlessWebClientProofRouteState {
            auth_runtime: AuthRuntimeStateHandle::new(snapshot),
            policy,
            proof: AuthlessWebClientProofState::new(),
            allowlist,
        };
        Router::new().route(
            "/authless-client",
            get(authless_web_client_proof_handler).with_state(state),
        )
    }

    fn request_with_peer(uri: &str, peer: SocketAddr) -> Request<Body> {
        let mut request = Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        request.extensions_mut().insert(ConnectInfo(peer));
        request
    }

    fn request_with_peer_and_authless_proof(
        uri: &str,
        peer: SocketAddr,
        proof_state: &AuthlessWebClientProofState,
    ) -> Request<Body> {
        let (nonce, proof, _) = proof_state.issue().expect("issue proof");
        let mut request = Request::builder()
            .uri(uri)
            .header(AUTHLESS_WEB_CLIENT_HEADER, proof)
            .header(
                header::COOKIE,
                format!("{AUTHLESS_WEB_CLIENT_COOKIE}={nonce}"),
            )
            .body(Body::empty())
            .expect("request");
        request.extensions_mut().insert(ConnectInfo(peer));
        request
    }

    fn authless_ws_test_actor() -> ResolvedActor {
        ResolvedActor {
            user: scryer_domain::User {
                id: "authless-ws-user".to_string(),
                username: "Anonymous".to_string(),
                password_hash: None,
                password_change_required: false,
                account_kind: Default::default(),
                authorization: Default::default(),
            },
            token_claims: AuthenticatedTokenClaims::default(),
            source: ResolvedActorSource::AuthlessDefault,
            api_key_id: None,
        }
    }

    fn authless_ws_proof_headers(proof_state: &AuthlessWebClientProofState) -> (HeaderMap, String) {
        let (nonce, proof, _) = proof_state.issue().expect("issue proof");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{AUTHLESS_WEB_CLIENT_COOKIE}={nonce}"))
                .expect("cookie header"),
        );
        (headers, proof)
    }

    async fn read_json_response(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        serde_json::from_slice(&body).expect("json response")
    }

    #[tokio::test]
    async fn authless_guard_middleware_allows_private_graphql_request() {
        let proof_state = AuthlessWebClientProofState::new();
        let app = authless_guard_test_app_with_proof_state(
            auth_disabled_snapshot(),
            protected_authless_policy(),
            proof_state.clone(),
        );

        let response = app
            .oneshot(request_with_peer_and_authless_proof(
                "/graphql",
                SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
                &proof_state,
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authless_guard_middleware_rejects_public_graphql_request() {
        let app = authless_guard_test_app(auth_disabled_snapshot(), protected_authless_policy());

        let response = app
            .oneshot(request_with_peer(
                "/graphql",
                SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn authless_guard_middleware_public_override_allowlist_allows_matching_graphql_request() {
        let proof_state = AuthlessWebClientProofState::new();
        let app = authless_guard_test_app_with_allowlist(
            auth_disabled_snapshot(),
            public_authless_policy(),
            AuthlessAccessAllowlist::parse("8.8.8.8"),
        );

        let response = app
            .oneshot(request_with_peer_and_authless_proof(
                "/graphql",
                SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
                &proof_state,
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authless_guard_middleware_allowlist_implies_public_access_for_matching_graphql_request()
     {
        let proof_state = AuthlessWebClientProofState::new();
        let app = authless_guard_test_app_with_allowlist(
            auth_disabled_snapshot(),
            protected_authless_policy(),
            AuthlessAccessAllowlist::parse("8.8.8.8"),
        );

        let response = app
            .oneshot(request_with_peer_and_authless_proof(
                "/graphql",
                SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
                &proof_state,
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authless_guard_middleware_rejects_public_websocket_route_before_handler() {
        let app = authless_guard_test_app(auth_disabled_snapshot(), protected_authless_policy());

        let response = app
            .oneshot(request_with_peer(
                "/graphql/ws",
                SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    fn local_bypass_snapshot() -> scryer_interface::context::AuthRuntimeStateSnapshot {
        scryer_interface::context::AuthRuntimeStateSnapshot {
            form_login_enabled: true,
            skip_login_for_local_ips: true,
            effective_form_login_enabled: true,
            webauthn_configured: false,
            passkey_enabled: false,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        }
    }

    #[test]
    fn local_ip_bypass_accepts_direct_private_and_loopback_clients() {
        let snapshot = local_bypass_snapshot();
        let headers = HeaderMap::new();

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 16, 5, 173), 3000))),
        ));
        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))),
        ));
        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv6Addr::LOCALHOST, 3000))),
        ));
    }

    #[test]
    fn default_session_persistence_requires_private_provenance() {
        let headers = HeaderMap::new();
        assert!(default_persist_session_for_request(
            &headers,
            Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))),
        ));
        assert!(default_persist_session_for_request(
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000))),
        ));
        assert!(!default_persist_session_for_request(
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
        ));
        assert!(!default_persist_session_for_request(&headers, None));
    }

    #[test]
    fn default_session_persistence_requires_an_entirely_private_forwarded_chain() {
        let peer = Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000)));
        let mut private_headers = HeaderMap::new();
        private_headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.5"),
        );
        assert!(default_persist_session_for_request(&private_headers, peer));
        assert!(!default_persist_session_for_request(
            &private_headers,
            Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
        ));

        let mut public_headers = HeaderMap::new();
        public_headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));
        assert!(!default_persist_session_for_request(&public_headers, peer));

        let mut malformed_headers = HeaderMap::new();
        malformed_headers.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        assert!(!default_persist_session_for_request(
            &malformed_headers,
            peer
        ));

        let mut missing_chain_headers = HeaderMap::new();
        missing_chain_headers.insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));
        assert!(!default_persist_session_for_request(
            &missing_chain_headers,
            peer
        ));
    }

    #[test]
    fn local_ip_bypass_claims_satisfy_step_up_checks() {
        let claims = mfa_bypass_token_claims();

        assert_eq!(claims.mfa_verified_until, Some(i64::MAX));
        assert_eq!(
            claims.session_scope,
            scryer_application::JwtSessionScope::Full
        );
    }

    #[test]
    fn forwarded_headers_from_trusted_proxy_are_used() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.2"),
        );

        let client_ip = request_client_ip(
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        );

        assert_eq!(client_ip, Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 25))));
    }

    #[test]
    fn forwarded_headers_from_untrusted_peer_are_ignored() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 8.8.8.8"),
        );

        let client_ip = request_client_ip(
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
        );

        assert_eq!(client_ip, Some(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn forwarded_ipv6_headers_from_trusted_proxy_are_used() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("[fc00::25]:8443"));

        let client_ip = request_client_ip(
            &headers,
            Some(SocketAddr::from((Ipv6Addr::LOCALHOST, 3000))),
        );

        assert_eq!(
            client_ip,
            Some(IpAddr::V6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0x25))),
        );
    }

    #[test]
    fn local_ip_bypass_accepts_local_forwarded_client_through_trusted_proxy() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.2"),
        );

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_accepts_private_forwarded_chain_through_trusted_proxy() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.5"),
        );

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_accepts_forwarded_proto_with_private_client_chain() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.5"),
        );

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_accepts_local_forwarded_ipv6_client_through_trusted_proxy() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("[fc00::25]:8443"));

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv6Addr::LOCALHOST, 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_forwarded_proto_without_client_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn spa_fallback_routes_do_not_consume_http_api_quota() {
        assert!(skip_http_rate_limit(&Method::GET, "/activity"));
        assert!(skip_http_rate_limit(&Method::GET, "/settings/profile"));
    }

    #[test]
    fn ticket_download_and_api_routes_consume_http_api_quota() {
        assert!(!skip_http_rate_limit(
            &Method::GET,
            "/backups/scryer.scryer-backup.enc/download"
        ));
        assert!(!skip_http_rate_limit(&Method::GET, "/api/system/jobs"));
    }

    #[test]
    fn oauth_token_and_decision_routes_use_oauth_quota() {
        assert!(!skip_http_rate_limit(&Method::POST, "/oauth/token"));
        assert!(!skip_http_rate_limit(
            &Method::POST,
            "/oauth/authorize/decision"
        ));
        assert_eq!(
            classify_http_rate_limit(&Method::POST, "/oauth/token"),
            Some(HttpRateLimitClass::OAuth)
        );
        assert_eq!(
            classify_http_rate_limit(&Method::POST, "/oauth/authorize/decision"),
            Some(HttpRateLimitClass::OAuth)
        );
        assert!(skip_http_rate_limit(&Method::GET, "/oauth/authorize"));
    }

    #[test]
    fn authless_web_client_route_uses_authless_client_quota() {
        assert!(!skip_http_rate_limit(&Method::GET, "/authless-client"));
        assert_eq!(
            classify_http_rate_limit(&Method::GET, "/authless-client"),
            Some(HttpRateLimitClass::AuthlessClient)
        );
    }

    #[tokio::test]
    async fn authless_web_client_proof_sets_hardened_cookie_and_cache_headers() {
        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), protected_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store, max-age=0"))
        );
        assert_eq!(
            response.headers().get(header::PRAGMA),
            Some(&HeaderValue::from_static("no-cache"))
        );
        assert_eq!(
            response.headers().get(header::EXPIRES),
            Some(&HeaderValue::from_static("0"))
        );
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("set-cookie");
        assert!(cookie.starts_with(&format!("{AUTHLESS_WEB_CLIENT_COOKIE}=")));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age=300"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(!cookie.contains("Secure"));

        let body = read_json_response(response).await;
        assert!(
            body["proof"]
                .as_str()
                .is_some_and(|proof| proof.matches('.').count() == 2)
        );
        assert!(body["expiresAt"].as_u64().is_some());
    }

    #[tokio::test]
    async fn authless_web_client_proof_public_override_allows_public_peer() {
        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), public_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = read_json_response(response).await;
        assert!(
            body["proof"]
                .as_str()
                .is_some_and(|proof| proof.matches('.').count() == 2)
        );
        assert!(body["expiresAt"].as_u64().is_some());
    }

    #[tokio::test]
    async fn authless_web_client_proof_public_override_allowlist_allows_matching_peer() {
        let response = authless_web_client_test_app_with_allowlist(
            auth_disabled_snapshot(),
            public_authless_policy(),
            AuthlessAccessAllowlist::parse("8.8.8.8"),
        )
        .oneshot(request_with_peer(
            "/authless-client",
            SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
        ))
        .await
        .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authless_web_client_proof_allowlist_implies_public_access_for_matching_peer() {
        let response = authless_web_client_test_app_with_allowlist(
            auth_disabled_snapshot(),
            protected_authless_policy(),
            AuthlessAccessAllowlist::parse("8.8.8.8"),
        )
        .oneshot(request_with_peer(
            "/authless-client",
            SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
        ))
        .await
        .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authless_web_client_proof_public_override_allowlist_rejects_unlisted_peer() {
        let response = authless_web_client_test_app_with_allowlist(
            auth_disabled_snapshot(),
            public_authless_policy(),
            AuthlessAccessAllowlist::parse("203.0.113.10"),
        )
        .oneshot(request_with_peer(
            "/authless-client",
            SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
        ))
        .await
        .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn authless_web_client_proof_reuses_existing_cookie_nonce() {
        let app =
            authless_web_client_test_app(auth_disabled_snapshot(), protected_authless_policy());
        let peer = SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000));
        let first_response = app
            .clone()
            .oneshot(request_with_peer("/authless-client", peer))
            .await
            .expect("first response");

        assert_eq!(first_response.status(), StatusCode::OK);
        let first_cookie = first_response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("first set-cookie")
            .to_string();
        let first_nonce = first_cookie
            .split_once('=')
            .and_then(|(_, rest)| rest.split_once(';').map(|(nonce, _)| nonce))
            .expect("first nonce")
            .to_string();
        let first_body = read_json_response(first_response).await;
        assert!(
            first_body["proof"]
                .as_str()
                .is_some_and(|proof| proof.starts_with(&format!("{first_nonce}.")))
        );

        let mut second_request = request_with_peer("/authless-client", peer);
        second_request.headers_mut().insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{AUTHLESS_WEB_CLIENT_COOKIE}={first_nonce}"))
                .expect("cookie header"),
        );
        let second_response = app.oneshot(second_request).await.expect("second response");

        assert_eq!(second_response.status(), StatusCode::OK);
        let second_cookie = second_response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("second set-cookie");
        assert!(second_cookie.starts_with(&format!("{AUTHLESS_WEB_CLIENT_COOKIE}={first_nonce};")));
        let second_body = read_json_response(second_response).await;
        assert!(
            second_body["proof"]
                .as_str()
                .is_some_and(|proof| proof.starts_with(&format!("{first_nonce}.")))
        );
    }

    #[tokio::test]
    async fn authless_web_client_proof_sets_secure_cookie_for_https_forwarded_request() {
        let mut request = request_with_peer(
            "/authless-client",
            SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000)),
        );
        request
            .headers_mut()
            .insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));
        request.headers_mut().insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.2"),
        );

        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), protected_authless_policy())
                .oneshot(request)
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("set-cookie");
        assert!(cookie.contains("Secure"));
    }

    #[tokio::test]
    async fn authless_web_client_proof_rejects_public_clients_when_protected() {
        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), protected_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store, max-age=0"))
        );
        let body = read_json_response(response).await;
        assert_eq!(
            body["error"],
            "Scryer web client proof is not available for this request"
        );
    }

    #[tokio::test]
    async fn authless_web_client_proof_rejects_cross_site_browser_requests() {
        let mut request = request_with_peer(
            "/authless-client",
            SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
        );
        request
            .headers_mut()
            .insert("sec-fetch-site", HeaderValue::from_static("cross-site"));

        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), protected_authless_policy())
                .oneshot(request)
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }

    #[tokio::test]
    async fn authless_web_client_proof_rejects_authenticated_credentials() {
        let mut request = request_with_peer(
            "/authless-client",
            SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
        );
        request.headers_mut().insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer sk_test_credential"),
        );

        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), protected_authless_policy())
                .oneshot(request)
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }

    #[tokio::test]
    async fn authless_web_client_proof_allows_explicit_public_authless_access() {
        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), public_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::SET_COOKIE).is_some());
    }

    #[tokio::test]
    async fn authless_web_client_proof_allows_local_ip_bypass_clients() {
        let response =
            authless_web_client_test_app(local_bypass_snapshot(), protected_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::SET_COOKIE).is_some());
    }

    #[tokio::test]
    async fn authless_web_client_proof_rejects_regular_login_mode() {
        let response =
            authless_web_client_test_app(auth_enabled_snapshot(), protected_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }

    #[test]
    fn authless_web_client_proof_requires_matching_cookie_nonce() {
        let proof_state = AuthlessWebClientProofState::new();
        let (nonce, proof, _) = proof_state.issue().expect("issue proof");
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHLESS_WEB_CLIENT_HEADER,
            HeaderValue::from_str(&proof).expect("proof header"),
        );
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{AUTHLESS_WEB_CLIENT_COOKIE}={nonce}"))
                .expect("cookie header"),
        );

        assert!(proof_state.validate_headers(&headers, None));

        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("scryer_authless_client=other"),
        );
        assert!(!proof_state.validate_headers(&headers, None));
    }

    #[test]
    fn title_images_and_static_assets_do_not_consume_http_api_quota() {
        assert!(skip_http_rate_limit(
            &Method::GET,
            "/images/titles/title-1/poster/original"
        ));
        assert!(skip_http_rate_limit(
            &Method::GET,
            "/images/titles/title-1/fanart/w1280"
        ));
        assert!(skip_http_rate_limit(
            &Method::GET,
            "/assets/index-B3b5rA.js"
        ));
        assert!(skip_http_rate_limit(&Method::GET, "/manifest.json"));
    }

    #[test]
    fn local_ip_bypass_rejects_public_ip_anywhere_in_forwarded_for_chain() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 8.8.8.8"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_public_ip_anywhere_in_forwarded_header_chain() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("for=192.168.1.25;proto=https, for=8.8.8.8"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_localhost_host_with_public_forwarded_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));
        headers.insert(header::HOST, HeaderValue::from_static("localhost:3000"));

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_private_host_with_public_forwarded_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("172.16.5.173:3000"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_public_host_with_public_forwarded_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("example.com:3000"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_private_forwarded_host_without_forwarded_client_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("172.16.5.173:3000"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_malformed_forwarded_ip_with_local_host() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        headers.insert(header::HOST, HeaderValue::from_static("localhost:3000"));

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_malformed_ip_anywhere_in_forwarded_chain() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, not-an-ip"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_public_peer_with_spoofed_local_forwarded_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("192.168.1.25"));

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
        ));
    }

    #[test]
    fn graphql_log_context_records_safe_operation_metadata_without_query_text() {
        let batch = async_graphql::BatchRequest::Single(
            async_graphql::Request::new("mutation Submit($password: String!) { submit }")
                .operation_name("Submit"),
        );

        let context = graphql_request_log_context(&batch, None, Ipv4Addr::new(127, 0, 0, 1).into());
        let encoded = serde_json::to_string(&context).expect("serialize context");

        assert_eq!(
            context
                .request
                .as_ref()
                .and_then(|request| request.operation_name.as_deref()),
            Some("Submit")
        );
        assert_eq!(
            context
                .request
                .as_ref()
                .and_then(|request| request.operation_type.as_deref()),
            Some("mutation")
        );
        assert!(encoded.contains("127.0.0.1"));
        assert!(!encoded.contains("password"));
        assert!(!encoded.contains("mutation Submit"));
    }

    #[test]
    fn graphql_operation_metadata_derives_a_named_operation_without_request_operation_name() {
        let request =
            GraphQLRequest::new("# accepted leading comment\nquery RefreshLibrary { health }");

        let (operation_name, operation_type) = graphql_request_operation_metadata(&request);

        assert_eq!(operation_name.as_deref(), Some("RefreshLibrary"));
        assert_eq!(operation_type.as_deref(), Some("query"));
    }
}
