//! Canonical FlareSolverr-compatible challenge-solver protocol support.
//!
//! This module is the single owner of the solver wire format, challenge
//! detection, solved-session reuse, solver-vs-target error classification, and
//! runtime solver-health bookkeeping. Transport stays with the callers: the
//! plugin HTTP host solves over a blocking client, the download router over an
//! async client, and the connection probe over its own short-lived client.

use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::IndexerProxyConfigRepository;
use scryer_domain::IndexerProxyHealthStatus;

/// Path of the FlareSolverr-compatible solve endpoint under the proxy base URL.
pub const SOLVER_SOLVE_PATH: &str = "/v1";

/// Bytes of a response body inspected for challenge and rate-limit markers.
pub const CHALLENGE_BODY_PREVIEW_BYTES: usize = 256 * 1024;

/// How long a solved clearance session (cookies + user agent) stays reusable.
const SOLVED_SESSION_TTL: Duration = Duration::from_secs(30 * 60);

/// Canonical solver-side failure messages. Classification, operational-backoff
/// exemption, and proxy-health recording all key off these exact strings, so
/// every caller must build solver-side errors from them.
pub const BYPARR_UNREACHABLE_MESSAGE: &str = "Byparr service could not be reached.";
pub const BYPARR_TIMEOUT_MESSAGE: &str = "Byparr timed out while resolving the indexer request.";
pub const BYPARR_UNAVAILABLE_MESSAGE: &str = "Byparr service is temporarily unavailable.";
pub const BYPARR_MALFORMED_MESSAGE: &str = "Byparr returned malformed solver output.";
pub const BYPARR_UNREADABLE_MESSAGE: &str = "Byparr returned an unreadable response.";
/// The solver answered but produced no usable solution. This is deliberately
/// not classified as a solver-service failure: the origin is still challenging
/// and the challenge may simply be unsolvable.
pub const BYPARR_NO_SOLUTION_MESSAGE: &str = "Byparr did not return a solved response.";

pub const TRAWL_UNREACHABLE_MESSAGE: &str = "Trawl service could not be reached.";
pub const TRAWL_TIMEOUT_MESSAGE: &str = "Trawl timed out while resolving the indexer request.";
pub const TRAWL_UNAVAILABLE_MESSAGE: &str = "Trawl service is temporarily unavailable.";
pub const TRAWL_MALFORMED_MESSAGE: &str = "Trawl returned malformed solver output.";
pub const TRAWL_UNREADABLE_MESSAGE: &str = "Trawl returned an unreadable response.";
pub const TRAWL_NO_SOLUTION_MESSAGE: &str = "Trawl did not return a solved response.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolverErrorKind {
    Unreachable,
    Timeout,
    Unavailable,
    Malformed,
    Unreadable,
    MissingSolution,
}

/// Reported when a transport proxy (plain HTTP or SOCKS5) reaches a
/// challenge-solver code path. Transport proxies carry bytes and solve
/// nothing, so this message names a routing bug rather than a service fault.
pub const TRANSPORT_PROXY_NOT_A_SOLVER_MESSAGE: &str =
    "transport proxies do not solve browser challenges";

pub fn solver_provider_name(provider: scryer_domain::IndexerProxyProviderType) -> &'static str {
    match provider {
        scryer_domain::IndexerProxyProviderType::Byparr => "Byparr",
        scryer_domain::IndexerProxyProviderType::Trawl => "Trawl",
        scryer_domain::IndexerProxyProviderType::Http => "HTTP proxy",
        scryer_domain::IndexerProxyProviderType::Socks5 => "SOCKS5 proxy",
    }
}

pub fn solver_error_message(
    provider: scryer_domain::IndexerProxyProviderType,
    kind: SolverErrorKind,
) -> &'static str {
    use scryer_domain::IndexerProxyProviderType::{Byparr, Http, Socks5, Trawl};

    match (provider, kind) {
        (Http | Socks5, _) => TRANSPORT_PROXY_NOT_A_SOLVER_MESSAGE,
        (Byparr, SolverErrorKind::Unreachable) => BYPARR_UNREACHABLE_MESSAGE,
        (Byparr, SolverErrorKind::Timeout) => BYPARR_TIMEOUT_MESSAGE,
        (Byparr, SolverErrorKind::Unavailable) => BYPARR_UNAVAILABLE_MESSAGE,
        (Byparr, SolverErrorKind::Malformed) => BYPARR_MALFORMED_MESSAGE,
        (Byparr, SolverErrorKind::Unreadable) => BYPARR_UNREADABLE_MESSAGE,
        (Byparr, SolverErrorKind::MissingSolution) => BYPARR_NO_SOLUTION_MESSAGE,
        (Trawl, SolverErrorKind::Unreachable) => TRAWL_UNREACHABLE_MESSAGE,
        (Trawl, SolverErrorKind::Timeout) => TRAWL_TIMEOUT_MESSAGE,
        (Trawl, SolverErrorKind::Unavailable) => TRAWL_UNAVAILABLE_MESSAGE,
        (Trawl, SolverErrorKind::Malformed) => TRAWL_MALFORMED_MESSAGE,
        (Trawl, SolverErrorKind::Unreadable) => TRAWL_UNREADABLE_MESSAGE,
        (Trawl, SolverErrorKind::MissingSolution) => TRAWL_NO_SOLUTION_MESSAGE,
    }
}

/// True when an error message names a solver-service failure (Byparr itself
/// unreachable, timing out, rate limited, or speaking garbage) rather than a
/// failure of the target indexer. Matches by substring because callers wrap
/// these messages in transport- and layer-specific envelopes.
pub fn is_solver_service_error_message(message: &str) -> bool {
    [
        BYPARR_UNREACHABLE_MESSAGE,
        BYPARR_TIMEOUT_MESSAGE,
        BYPARR_UNAVAILABLE_MESSAGE,
        BYPARR_MALFORMED_MESSAGE,
        BYPARR_UNREADABLE_MESSAGE,
        TRAWL_UNREACHABLE_MESSAGE,
        TRAWL_TIMEOUT_MESSAGE,
        TRAWL_UNAVAILABLE_MESSAGE,
        TRAWL_MALFORMED_MESSAGE,
        TRAWL_UNREADABLE_MESSAGE,
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

pub fn solver_solve_endpoint(base_url: &str) -> String {
    format!("{}{SOLVER_SOLVE_PATH}", base_url.trim_end_matches('/'))
}

/// FlareSolverr-compatible solve request. Byparr interprets `maxTimeout` in
/// seconds, while Trawl follows the FlareSolverr contract and expects
/// milliseconds.
#[derive(Serialize)]
pub struct ChallengeSolverRequest<'a> {
    cmd: &'static str,
    url: &'a str,
    #[serde(rename = "maxTimeout")]
    max_timeout: u32,
}

pub fn solver_solve_request(
    provider: scryer_domain::IndexerProxyProviderType,
    url: &str,
    request_timeout_seconds: u32,
) -> ChallengeSolverRequest<'_> {
    let max_timeout = match provider {
        // Transport proxies never reach a solve endpoint; seconds is the
        // harmless reading if a caller ever gets here by mistake.
        scryer_domain::IndexerProxyProviderType::Byparr
        | scryer_domain::IndexerProxyProviderType::Http
        | scryer_domain::IndexerProxyProviderType::Socks5 => request_timeout_seconds,
        scryer_domain::IndexerProxyProviderType::Trawl => {
            request_timeout_seconds.saturating_mul(1_000)
        }
    };
    ChallengeSolverRequest {
        cmd: "request.get",
        url,
        max_timeout,
    }
}

#[derive(Deserialize)]
struct ChallengeSolverResponse {
    status: Option<String>,
    solution: Option<ChallengeSolverSolution>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChallengeSolverSolution {
    pub url: Option<String>,
    pub status: Option<u16>,
    pub cookies: Option<Vec<serde_json::Value>>,
    #[serde(default, alias = "userAgent", alias = "user_agent")]
    pub user_agent: Option<String>,
    pub headers: Option<serde_json::Value>,
    pub response: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChallengeSolverParseError {
    Malformed,
    ServiceError,
    MissingSolution,
}

impl ChallengeSolverParseError {
    pub fn message(self, provider: scryer_domain::IndexerProxyProviderType) -> &'static str {
        match self {
            Self::Malformed => solver_error_message(provider, SolverErrorKind::Malformed),
            Self::ServiceError => solver_error_message(provider, SolverErrorKind::Unavailable),
            Self::MissingSolution => {
                solver_error_message(provider, SolverErrorKind::MissingSolution)
            }
        }
    }
}

/// Parse a raw solve-endpoint response body into its solution.
pub fn parse_solver_solution(
    body: &[u8],
) -> Result<ChallengeSolverSolution, ChallengeSolverParseError> {
    let parsed: ChallengeSolverResponse =
        serde_json::from_slice(body).map_err(|_| ChallengeSolverParseError::Malformed)?;
    if parsed
        .status
        .as_deref()
        .is_some_and(|status| status.trim().eq_ignore_ascii_case("error"))
    {
        return Err(ChallengeSolverParseError::ServiceError);
    }
    parsed
        .solution
        .ok_or(ChallengeSolverParseError::MissingSolution)
}

/// Statuses that can carry a browser challenge instead of real content.
pub fn challenge_candidate_status(status: u16) -> bool {
    matches!(status, 200 | 403 | 503)
}

/// True when a direct origin response looks like a solvable browser challenge.
/// A 503 that carries `Retry-After` without challenge markers is provider
/// backpressure, not a challenge.
pub fn looks_like_challenge_response(
    status: u16,
    headers: &BTreeMap<String, String>,
    body: &[u8],
) -> bool {
    if status == 429 || !challenge_candidate_status(status) {
        return false;
    }
    if body.is_empty() || !is_text_like_response(headers, body) {
        return false;
    }
    let has_marker = challenge_marker_present(body);
    if status == 503 && header_value(headers, "retry-after").is_some() && !has_marker {
        return false;
    }
    if status == 200 && !successful_response_challenge_marker_present(body) {
        return false;
    }
    has_marker
}

fn successful_response_challenge_marker_present(body: &[u8]) -> bool {
    let preview = &body[..body.len().min(CHALLENGE_BODY_PREVIEW_BYTES)];
    let preview = String::from_utf8_lossy(preview).to_ascii_lowercase();
    [
        "cf-chl",
        "challenge-platform",
        "<title>just a moment",
        "<title>checking your browser",
        "<title>attention required! | cloudflare",
        "<title>ddos-guard",
    ]
    .iter()
    .any(|marker| preview.contains(marker))
}

pub fn challenge_marker_present(body: &[u8]) -> bool {
    let preview = &body[..body.len().min(CHALLENGE_BODY_PREVIEW_BYTES)];
    let preview = String::from_utf8_lossy(preview).to_ascii_lowercase();
    [
        "cf-chl",
        "challenge-platform",
        "just a moment",
        "checking your browser",
        "ddos-guard",
        "captcha",
        "turnstile",
    ]
    .iter()
    .any(|marker| preview.contains(marker))
}

fn is_text_like_response(headers: &BTreeMap<String, String>, body: &[u8]) -> bool {
    if let Some(content_type) = header_value(headers, "content-type") {
        let content_type = content_type.to_ascii_lowercase();
        return content_type.contains("text/html")
            || content_type.contains("text/plain")
            || content_type.contains("application/xhtml+xml");
    }

    let preview = &body[..body.len().min(CHALLENGE_BODY_PREVIEW_BYTES)];
    !preview.contains(&0) && std::str::from_utf8(preview).is_ok()
}

pub fn header_value<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// Read a header from a solution's JSON header object, case-insensitively.
pub fn solution_header_string(headers: Option<&serde_json::Value>, name: &str) -> Option<String> {
    headers
        .and_then(|value| value.as_object())
        .and_then(|object| {
            object
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .and_then(|(_, value)| value.as_str())
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn retry_after_from_solution_headers(headers: Option<&serde_json::Value>) -> Option<Duration> {
    solution_header_string(headers, "retry-after")
        .and_then(|value| scryer_outbound_http::parse_retry_after(&value).map(|(delay, _)| delay))
}

pub fn retry_after_from_solution(solution: &ChallengeSolverSolution) -> Option<Duration> {
    retry_after_from_solution_headers(solution.headers.as_ref())
}

/// Response headers safe to surface from a solved page to consumers.
pub fn safe_solution_response_headers(
    value: Option<&serde_json::Value>,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    let Some(object) = value.and_then(|value| value.as_object()) else {
        return headers;
    };
    for (name, value) in object {
        let normalized = name.to_ascii_lowercase();
        if !matches!(
            normalized.as_str(),
            "content-type" | "content-disposition" | "cache-control" | "etag" | "last-modified"
        ) {
            continue;
        }
        let Some(value) = value.as_str() else {
            continue;
        };
        if reqwest::header::HeaderName::from_bytes(normalized.as_bytes()).is_err()
            || reqwest::header::HeaderValue::from_str(value).is_err()
        {
            continue;
        }
        headers.insert(normalized, value.to_string());
    }
    headers
}

/// Headers (user agent + clearance cookies) that let the original request be
/// replayed directly against the origin after a solve.
pub fn solution_retry_headers(solution: &ChallengeSolverSolution) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if let Some(user_agent) = solution.user_agent.as_deref()
        && !user_agent.trim().is_empty()
        && reqwest::header::HeaderValue::from_str(user_agent).is_ok()
    {
        headers.push(("user-agent".to_string(), user_agent.to_string()));
    }
    if let Some(cookie_header) = solution_cookie_header(solution.cookies.as_deref()) {
        headers.push(("cookie".to_string(), cookie_header));
    }
    headers
}

pub fn solution_cookie_header(cookies: Option<&[serde_json::Value]>) -> Option<String> {
    let mut pairs = Vec::new();
    for cookie in cookies.unwrap_or_default() {
        if let Some(text) = cookie.as_str() {
            if safe_cookie_pair(text) {
                pairs.push(text.to_string());
            }
            continue;
        }
        let Some(object) = cookie.as_object() else {
            continue;
        };
        let name = object
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim();
        let value = object
            .get("value")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim();
        let pair = format!("{name}={value}");
        if safe_cookie_pair(&pair) {
            pairs.push(pair);
        }
    }
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

pub fn safe_cookie_pair(pair: &str) -> bool {
    let Some((name, value)) = pair.split_once('=') else {
        return false;
    };
    !name.trim().is_empty() && !name.contains([';', '\r', '\n']) && !value.contains(['\r', '\n'])
}

/// True when a solved page body reads as a rate-limit notice rather than
/// content. Requires both markers so bare "429" substrings cannot match.
pub fn solved_body_looks_rate_limited(body: &[u8]) -> bool {
    let preview = &body[..body.len().min(CHALLENGE_BODY_PREVIEW_BYTES)];
    let preview = String::from_utf8_lossy(preview).to_ascii_lowercase();
    preview.contains("429") && preview.contains("too many requests")
}

/// Canonical target-rate-limit message, shaped so `RateLimitSignal` parses the
/// status and retry-after back out at the feedback layer.
pub fn rate_limit_message_with_retry_after(retry_after: Option<Duration>) -> String {
    match retry_after {
        Some(delay) => format!(
            "HTTP 429: too many requests; retry_after_seconds={}",
            delay.as_secs()
        ),
        None => "HTTP 429: too many requests".to_string(),
    }
}

pub fn target_rate_limit_message(solution: &ChallengeSolverSolution) -> String {
    rate_limit_message_with_retry_after(retry_after_from_solution(solution))
}

pub fn sanitized_url_for_log(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut url) => {
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => "<invalid-url>".to_string(),
    }
}

/// Redact credential-bearing query values from solver error text before it is
/// logged or persisted as proxy health detail.
pub fn sanitize_indexer_proxy_error(message: &str) -> String {
    let mut sanitized = message.to_string();
    for marker in [
        "apikey=", "api_key=", "token=", "passkey=", "auth=", "rsskey=", "jwt=",
    ] {
        let mut search_start = 0;
        while let Some(relative_start) = sanitized[search_start..].to_ascii_lowercase().find(marker)
        {
            let start = search_start + relative_start;
            let value_start = start + marker.len();
            let value_end = sanitized[value_start..]
                .find(['&', ' ', '\'', '"'])
                .map(|offset| value_start + offset)
                .unwrap_or_else(|| sanitized.len());
            if sanitized[value_start..value_end].eq("REDACTED") {
                search_start = value_end;
                continue;
            }
            sanitized.replace_range(value_start..value_end, "REDACTED");
            search_start = value_start + "REDACTED".len();
        }
    }
    sanitized
}

#[derive(Clone)]
struct SolvedSession {
    headers: Vec<(String, String)>,
    solved_at: Instant,
}

/// Process-shared cache of solved clearance sessions keyed by proxy config and
/// origin host, so one solve serves subsequent requests until it expires or a
/// challenge shows it is stale.
pub struct SolvedSessionCache {
    sessions: Mutex<HashMap<(String, String), SolvedSession>>,
    ttl: Duration,
}

static SHARED_SOLVED_SESSIONS: LazyLock<SolvedSessionCache> =
    LazyLock::new(|| SolvedSessionCache::with_ttl(SOLVED_SESSION_TTL));

impl SolvedSessionCache {
    pub fn shared() -> &'static SolvedSessionCache {
        &SHARED_SOLVED_SESSIONS
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    fn key(proxy_config_id: &str, url: &str) -> Option<(String, String)> {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))?;
        Some((proxy_config_id.to_string(), host))
    }

    /// Headers to inject for a request, if a fresh solved session exists for
    /// this proxy + origin host. Expired entries are dropped on read.
    pub fn session_headers(&self, proxy_config_id: &str, url: &str) -> Vec<(String, String)> {
        let Some(key) = Self::key(proxy_config_id, url) else {
            return Vec::new();
        };
        let mut sessions = self
            .sessions
            .lock()
            .expect("solved session cache lock poisoned");
        let Some(session) = sessions.get(&key) else {
            return Vec::new();
        };
        if session.solved_at.elapsed() >= self.ttl {
            sessions.remove(&key);
            return Vec::new();
        }
        session.headers.clone()
    }

    /// Store a reusable clearance session. A user agent alone is not proof of
    /// clearance: an unchallenged endpoint can replay successfully with it and
    /// otherwise poison later requests to the same origin. Keep the user agent
    /// alongside the session, but only admit solutions with a solver cookie.
    pub fn store_solution(
        &self,
        proxy_config_id: &str,
        url: &str,
        solution: &ChallengeSolverSolution,
    ) {
        let headers = solution_retry_headers(solution);
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        {
            return;
        }
        let Some(key) = Self::key(proxy_config_id, url) else {
            return;
        };
        self.sessions
            .lock()
            .expect("solved session cache lock poisoned")
            .insert(
                key,
                SolvedSession {
                    headers,
                    solved_at: Instant::now(),
                },
            );
    }

    /// Drop a session that failed to bypass a challenge.
    pub fn invalidate(&self, proxy_config_id: &str, url: &str) {
        let Some(key) = Self::key(proxy_config_id, url) else {
            return;
        };
        self.sessions
            .lock()
            .expect("solved session cache lock poisoned")
            .remove(&key);
    }
}

#[derive(Clone, Debug)]
pub struct SolverHealthEvent {
    pub proxy_config_id: String,
    pub healthy: bool,
    pub message: Option<String>,
    pub observed_at: chrono::DateTime<Utc>,
}

/// Process-shared ledger of solver-service outcomes observed at solve sites
/// that cannot reach the repository themselves (the blocking plugin HTTP
/// host). Async flows drain it into persisted proxy health after their pass.
pub struct SolverHealthLedger {
    events: Mutex<HashMap<String, SolverHealthEvent>>,
}

static SHARED_SOLVER_HEALTH: LazyLock<SolverHealthLedger> = LazyLock::new(|| SolverHealthLedger {
    events: Mutex::new(HashMap::new()),
});

impl SolverHealthLedger {
    pub fn shared() -> &'static SolverHealthLedger {
        &SHARED_SOLVER_HEALTH
    }

    pub fn record_success(&self, proxy_config_id: &str) {
        self.record(SolverHealthEvent {
            proxy_config_id: proxy_config_id.to_string(),
            healthy: true,
            message: None,
            observed_at: Utc::now(),
        });
    }

    pub fn record_failure(&self, proxy_config_id: &str, message: &str) {
        self.record(SolverHealthEvent {
            proxy_config_id: proxy_config_id.to_string(),
            healthy: false,
            message: Some(sanitize_indexer_proxy_error(message)),
            observed_at: Utc::now(),
        });
    }

    fn record(&self, event: SolverHealthEvent) {
        self.events
            .lock()
            .expect("solver health ledger lock poisoned")
            .insert(event.proxy_config_id.clone(), event);
    }

    pub fn drain(&self) -> Vec<SolverHealthEvent> {
        self.events
            .lock()
            .expect("solver health ledger lock poisoned")
            .drain()
            .map(|(_, event)| event)
            .collect()
    }
}

/// Persist any pending solver-health observations. Health writes go through
/// the dedicated repository method so they never bump `updated_at`, which
/// doubles as the plugin client cache revision.
pub async fn flush_solver_health(repo: &dyn IndexerProxyConfigRepository) {
    for event in SolverHealthLedger::shared().drain() {
        let status = if event.healthy {
            IndexerProxyHealthStatus::Healthy
        } else {
            IndexerProxyHealthStatus::Unhealthy
        };
        let existing = match repo.get_by_id(&event.proxy_config_id).await {
            Ok(Some(existing)) => existing,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    proxy_config_id = event.proxy_config_id.as_str(),
                    error = %error,
                    "failed to load indexer proxy config for health update"
                );
                continue;
            }
        };
        let unchanged = existing.last_health_status == Some(status)
            && existing.last_error_message == event.message;
        if unchanged {
            continue;
        }
        let error_at = (!event.healthy).then_some(event.observed_at);
        if let Err(error) = repo
            .record_health(
                &event.proxy_config_id,
                status,
                event.message.clone(),
                error_at,
            )
            .await
        {
            tracing::warn!(
                proxy_config_id = event.proxy_config_id.as_str(),
                error = %error,
                "failed to record indexer proxy runtime health"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solution_from_json(value: serde_json::Value) -> ChallengeSolverSolution {
        serde_json::from_value(value).expect("solution should deserialize")
    }

    #[test]
    fn solve_requests_serialize_provider_timeout_units() {
        let byparr = serde_json::to_value(solver_solve_request(
            scryer_domain::IndexerProxyProviderType::Byparr,
            "https://example.com/",
            60,
        ))
        .expect("Byparr payload should serialize");
        let trawl = serde_json::to_value(solver_solve_request(
            scryer_domain::IndexerProxyProviderType::Trawl,
            "https://example.com/",
            60,
        ))
        .expect("Trawl payload should serialize");

        assert_eq!(byparr["cmd"], "request.get");
        assert_eq!(byparr["url"], "https://example.com/");
        assert_eq!(byparr["maxTimeout"], 60);
        assert_eq!(trawl["cmd"], "request.get");
        assert_eq!(trawl["url"], "https://example.com/");
        assert_eq!(trawl["maxTimeout"], 60_000);
    }

    #[test]
    fn parse_solver_solution_classifies_malformed_errors_and_missing_solutions() {
        assert_eq!(
            parse_solver_solution(b"not json").unwrap_err(),
            ChallengeSolverParseError::Malformed
        );
        assert_eq!(
            parse_solver_solution(
                br#"{"status":"error","message":"Browser pool initializing","solution":{"url":"https://example.com/","status":0,"headers":{},"response":"","cookies":[],"userAgent":""}}"#,
            )
            .unwrap_err(),
            ChallengeSolverParseError::ServiceError
        );
        assert_eq!(
            parse_solver_solution(br#"{"status":"ok","message":""}"#).unwrap_err(),
            ChallengeSolverParseError::MissingSolution
        );
        let solution = parse_solver_solution(
            br#"{"status":"ok","message":"","version":"2.0.0","solution":{"url":"https://example.com/","status":200,"headers":{"content-type":"text/html"},"cookies":[{"name":"cf_clearance","value":"abc"}],"userAgent":"UA","response":"<html>Trawl</html>"}}"#,
        )
        .expect("solution should parse");
        assert_eq!(solution.status, Some(200));
        assert_eq!(solution.user_agent.as_deref(), Some("UA"));
        assert_eq!(solution.response.as_deref(), Some("<html>Trawl</html>"));
        assert_eq!(solution.cookies.as_deref().map(<[_]>::len), Some(1));
        assert_eq!(
            solution_header_string(solution.headers.as_ref(), "content-type").as_deref(),
            Some("text/html")
        );
    }

    #[test]
    fn challenge_detection_requires_candidate_status_and_marker() {
        let html_headers = BTreeMap::from([(
            "content-type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )]);
        let challenge_body = b"<html><title>Just a moment</title>cf-chl</html>";

        assert!(looks_like_challenge_response(
            403,
            &html_headers,
            challenge_body
        ));
        assert!(looks_like_challenge_response(
            200,
            &html_headers,
            challenge_body
        ));
        assert!(!looks_like_challenge_response(
            200,
            &html_headers,
            b"<html><p>This form may use captcha or turnstile verification.</p></html>"
        ));
        assert!(looks_like_challenge_response(
            403,
            &html_headers,
            b"<html><p>Complete the captcha or turnstile challenge.</p></html>"
        ));
        assert!(looks_like_challenge_response(
            503,
            &html_headers,
            challenge_body
        ));
        assert!(!looks_like_challenge_response(
            429,
            &html_headers,
            challenge_body
        ));
        assert!(!looks_like_challenge_response(
            500,
            &html_headers,
            challenge_body
        ));
        assert!(!looks_like_challenge_response(
            403,
            &html_headers,
            b"<html>plain error page</html>"
        ));
        assert!(!looks_like_challenge_response(403, &html_headers, b""));
    }

    #[test]
    fn service_unavailable_with_retry_after_and_no_marker_is_not_a_challenge() {
        let headers = BTreeMap::from([
            ("content-type".to_string(), "text/html".to_string()),
            ("retry-after".to_string(), "120".to_string()),
        ]);

        assert!(!looks_like_challenge_response(
            503,
            &headers,
            b"<html>server busy</html>"
        ));
        assert!(looks_like_challenge_response(
            503,
            &headers,
            b"<html>checking your browser</html>"
        ));
    }

    #[test]
    fn binary_bodies_are_not_challenges() {
        let headers = BTreeMap::new();
        let mut body = b"captcha".to_vec();
        body.push(0);

        assert!(!looks_like_challenge_response(200, &headers, &body));
    }

    #[test]
    fn solved_body_rate_limit_sniff_requires_both_markers() {
        assert!(solved_body_looks_rate_limited(
            b"<html>429 too many requests</html>"
        ));
        assert!(!solved_body_looks_rate_limited(b"release id 429001"));
        assert!(!solved_body_looks_rate_limited(b"too many requests"));
    }

    #[test]
    fn rate_limit_messages_round_trip_through_rate_limit_signal() {
        let with_retry = rate_limit_message_with_retry_after(Some(Duration::from_secs(90)));
        let signal = crate::RateLimitSignal::from_text(&with_retry)
            .expect("canonical message should classify");
        assert_eq!(signal.retry_after, Some(Duration::from_secs(90)));

        let without_retry = rate_limit_message_with_retry_after(None);
        let signal = crate::RateLimitSignal::from_text(&without_retry)
            .expect("canonical message should classify");
        assert_eq!(signal.retry_after, None);
    }

    #[test]
    fn solver_service_messages_do_not_classify_as_rate_limits() {
        for message in [
            BYPARR_UNREACHABLE_MESSAGE,
            BYPARR_TIMEOUT_MESSAGE,
            BYPARR_UNAVAILABLE_MESSAGE,
            BYPARR_MALFORMED_MESSAGE,
            BYPARR_UNREADABLE_MESSAGE,
            BYPARR_NO_SOLUTION_MESSAGE,
            TRAWL_UNREACHABLE_MESSAGE,
            TRAWL_TIMEOUT_MESSAGE,
            TRAWL_UNAVAILABLE_MESSAGE,
            TRAWL_MALFORMED_MESSAGE,
            TRAWL_UNREADABLE_MESSAGE,
            TRAWL_NO_SOLUTION_MESSAGE,
        ] {
            assert!(
                crate::RateLimitSignal::from_text(message).is_none(),
                "{message} must not classify as a rate limit"
            );
        }
    }

    #[test]
    fn solver_service_error_classification_matches_wrapped_messages() {
        assert!(is_solver_service_error_message(&format!(
            "repository: {BYPARR_TIMEOUT_MESSAGE}"
        )));
        assert!(is_solver_service_error_message(BYPARR_UNAVAILABLE_MESSAGE));
        assert!(is_solver_service_error_message(&format!(
            "repository: {TRAWL_TIMEOUT_MESSAGE}"
        )));
        assert!(is_solver_service_error_message(TRAWL_UNAVAILABLE_MESSAGE));
        assert!(!is_solver_service_error_message(BYPARR_NO_SOLUTION_MESSAGE));
        assert!(!is_solver_service_error_message(TRAWL_NO_SOLUTION_MESSAGE));
        assert!(!is_solver_service_error_message("indexer search timed out"));
    }

    #[test]
    fn solution_cookie_header_skips_unsafe_pairs() {
        let solution = solution_from_json(serde_json::json!({
            "cookies": [
                {"name": "cf_clearance", "value": "abc"},
                {"name": "bad;name", "value": "x"},
                {"name": "crlf", "value": "x\r\ny"},
                "plain=ok",
                "noequals",
            ],
            "userAgent": "Mozilla/5.0",
        }));

        let headers = solution_retry_headers(&solution);
        assert_eq!(
            headers,
            vec![
                ("user-agent".to_string(), "Mozilla/5.0".to_string()),
                (
                    "cookie".to_string(),
                    "cf_clearance=abc; plain=ok".to_string()
                ),
            ]
        );
    }

    #[test]
    fn safe_solution_response_headers_allowlists() {
        let headers = safe_solution_response_headers(Some(&serde_json::json!({
            "Content-Type": "application/x-nzb",
            "Set-Cookie": "secret=1",
            "X-Random": "nope",
            "ETag": "\"abc\"",
        })));

        assert_eq!(
            headers.get("content-type").map(String::as_str),
            Some("application/x-nzb")
        );
        assert_eq!(headers.get("etag").map(String::as_str), Some("\"abc\""));
        assert!(!headers.contains_key("set-cookie"));
        assert!(!headers.contains_key("x-random"));
    }

    #[test]
    fn retry_after_from_solution_headers_parses_seconds() {
        let headers = serde_json::json!({"Retry-After": "300"});
        assert_eq!(
            retry_after_from_solution_headers(Some(&headers)),
            Some(Duration::from_secs(300))
        );
        assert_eq!(retry_after_from_solution_headers(None), None);
    }

    #[test]
    fn solved_session_cache_stores_reuses_and_invalidates() {
        let cache = SolvedSessionCache::with_ttl(Duration::from_secs(60));
        let solution = solution_from_json(serde_json::json!({
            "cookies": [{"name": "cf_clearance", "value": "abc"}],
            "userAgent": "Mozilla/5.0",
        }));

        cache.store_solution("proxy-1", "https://indexer.example/api?t=search", &solution);

        let headers = cache.session_headers("proxy-1", "https://indexer.example/rss");
        assert_eq!(headers.len(), 2, "same host should reuse the session");
        assert!(
            cache
                .session_headers("proxy-1", "https://other.example/api")
                .is_empty()
        );
        assert!(
            cache
                .session_headers("proxy-2", "https://indexer.example/api")
                .is_empty()
        );

        cache.invalidate("proxy-1", "https://indexer.example/download/123");
        assert!(
            cache
                .session_headers("proxy-1", "https://indexer.example/api")
                .is_empty()
        );
    }

    #[test]
    fn solved_session_cache_expires_by_ttl() {
        let cache = SolvedSessionCache::with_ttl(Duration::ZERO);
        let solution = solution_from_json(serde_json::json!({
            "cookies": [{"name": "cf_clearance", "value": "abc"}],
        }));

        cache.store_solution("proxy-1", "https://indexer.example/api", &solution);

        assert!(
            cache
                .session_headers("proxy-1", "https://indexer.example/api")
                .is_empty()
        );
    }

    #[test]
    fn solved_session_cache_skips_empty_solutions() {
        let cache = SolvedSessionCache::with_ttl(Duration::from_secs(60));
        let solution = solution_from_json(serde_json::json!({"status": 200}));

        cache.store_solution("proxy-1", "https://indexer.example/api", &solution);

        assert!(
            cache
                .session_headers("proxy-1", "https://indexer.example/api")
                .is_empty()
        );
    }

    #[test]
    fn solved_session_cache_skips_user_agent_only_solutions() {
        let cache = SolvedSessionCache::with_ttl(Duration::from_secs(60));
        let solution = solution_from_json(serde_json::json!({
            "status": 200,
            "userAgent": "Mozilla/5.0",
        }));

        cache.store_solution("proxy-1", "https://indexer.example/api", &solution);

        assert!(
            cache
                .session_headers("proxy-1", "https://indexer.example/api")
                .is_empty()
        );
    }

    #[test]
    fn sanitize_indexer_proxy_error_redacts_sensitive_query_values_once() {
        let message =
            "Byparr failed for https://example.invalid/api?t=search&apikey=abc123&token=def456";

        let sanitized = sanitize_indexer_proxy_error(message);

        assert_eq!(
            sanitized,
            "Byparr failed for https://example.invalid/api?t=search&apikey=REDACTED&token=REDACTED",
        );
    }

    #[test]
    fn sanitize_indexer_proxy_error_handles_all_sensitive_markers() {
        let message = "api_key=a passkey=b auth=c rsskey=d jwt=e apikey=f";

        let sanitized = sanitize_indexer_proxy_error(message);

        assert_eq!(
            sanitized,
            "api_key=REDACTED passkey=REDACTED auth=REDACTED rsskey=REDACTED jwt=REDACTED apikey=REDACTED",
        );
    }

    #[test]
    fn sanitize_indexer_proxy_error_does_not_loop_on_already_redacted_value() {
        let message = "request failed: apikey=REDACTED&token=still-secret";

        let sanitized = sanitize_indexer_proxy_error(message);

        assert_eq!(sanitized, "request failed: apikey=REDACTED&token=REDACTED");
    }

    #[test]
    fn health_ledger_keeps_latest_event_per_proxy() {
        let ledger = SolverHealthLedger {
            events: Mutex::new(HashMap::new()),
        };
        ledger.record_failure("proxy-1", BYPARR_TIMEOUT_MESSAGE);
        ledger.record_success("proxy-1");
        ledger.record_failure("proxy-2", "Byparr failed apikey=secret");

        let mut events = ledger.drain();
        events.sort_by(|left, right| left.proxy_config_id.cmp(&right.proxy_config_id));

        assert_eq!(events.len(), 2);
        assert!(events[0].healthy);
        assert!(!events[1].healthy);
        assert_eq!(
            events[1].message.as_deref(),
            Some("Byparr failed apikey=REDACTED")
        );
        assert!(ledger.drain().is_empty());
    }
}
