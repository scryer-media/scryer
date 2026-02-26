use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use scryer_application::{AppError, AppResult};
use reqwest::header::HeaderMap;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tracing::{debug, info, warn};

use crate::download_clients::parse_retry_after;

// ---------------------------------------------------------------------------
// NewznabApiLimits — parsed from XML body or HTTP headers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct NewznabApiLimits {
    pub api_current: Option<u32>,
    pub api_max: Option<u32>,
    pub grab_current: Option<u32>,
    pub grab_max: Option<u32>,
}

/// Parse `<newznab:apilimits apiCurrent="42" apiMax="500" ...>` from an XML body.
/// Uses simple string scanning — no XML parser dependency needed.
pub fn parse_newznab_apilimits_xml(body: &str) -> Option<NewznabApiLimits> {
    // Look for "apilimits" in the body (handles both <newznab:apilimits and <apilimits)
    let lower = body.to_ascii_lowercase();
    let idx = lower.find("apilimits")?;
    // Find the enclosing tag: scan backward for '<' and forward for '>' or '/>'
    let tag_start = body[..idx].rfind('<')?;
    let tag_end = body[idx..].find('>')? + idx + 1;
    let tag = &body[tag_start..tag_end];

    let api_current = extract_xml_attr(tag, "apiCurrent")
        .or_else(|| extract_xml_attr(tag, "apicurrent"));
    let api_max = extract_xml_attr(tag, "apiMax")
        .or_else(|| extract_xml_attr(tag, "apimax"));
    let grab_current = extract_xml_attr(tag, "grabCurrent")
        .or_else(|| extract_xml_attr(tag, "grabcurrent"));
    let grab_max = extract_xml_attr(tag, "grabMax")
        .or_else(|| extract_xml_attr(tag, "grabmax"));

    if api_current.is_none() && api_max.is_none() && grab_current.is_none() && grab_max.is_none()
    {
        return None;
    }

    Some(NewznabApiLimits {
        api_current,
        api_max,
        grab_current,
        grab_max,
    })
}

fn extract_xml_attr(tag: &str, attr_name: &str) -> Option<u32> {
    // Case-insensitive search for the attribute name
    let lower_tag = tag.to_ascii_lowercase();
    let lower_attr = attr_name.to_ascii_lowercase();
    let attr_pattern = format!("{}=\"", lower_attr);
    let pos = lower_tag.find(&attr_pattern)?;
    let value_start = pos + attr_pattern.len();
    let rest = &tag[value_start..];
    let value_end = rest.find('"')?;
    rest[..value_end].trim().parse::<u32>().ok()
}

/// Parse API limit hints from `X-RateLimit-Remaining` / `X-RateLimit-Limit` headers.
/// Returns `None` when headers are absent (the common case for NZBGeek).
pub fn parse_apilimits_from_headers(headers: &HeaderMap) -> Option<NewznabApiLimits> {
    let limit = headers
        .get("X-RateLimit-Limit")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u32>().ok());
    let remaining = headers
        .get("X-RateLimit-Remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u32>().ok());

    let (api_max, api_current) = match (limit, remaining) {
        (Some(lim), Some(rem)) => (Some(lim), Some(lim.saturating_sub(rem))),
        _ => return None,
    };

    Some(NewznabApiLimits {
        api_current,
        api_max,
        grab_current: None,
        grab_max: None,
    })
}

// ---------------------------------------------------------------------------
// NewznabRateLimiter — bounded concurrency with per-slot cooldown
// ---------------------------------------------------------------------------

pub struct NewznabRateLimiterConfig {
    pub label: String,
    pub cooldown_ms: u64,
    pub max_concurrent_requests: u32,
    pub base_backoff_seconds: u64,
    pub max_backoff_seconds: u64,
    pub jitter_max_ms: u64,
}

impl Default for NewznabRateLimiterConfig {
    fn default() -> Self {
        Self {
            label: "indexer".to_string(),
            cooldown_ms: 1100,
            max_concurrent_requests: 4,
            base_backoff_seconds: 10,
            max_backoff_seconds: 900,
            jitter_max_ms: 250,
        }
    }
}

#[derive(Clone)]
pub struct NewznabRateLimiter {
    inner: Arc<RateLimiterInner>,
}

struct RateLimiterInner {
    label: String,
    concurrency: Arc<Semaphore>,
    state: Mutex<RateLimiterState>,
    cooldown_interval: Duration,
    base_backoff_seconds: u64,
    max_backoff_seconds: u64,
    #[allow(dead_code)]
    jitter_max_ms: u64,
}

#[derive(Default)]
struct RateLimiterState {
    disabled_until: Option<Instant>,
    consecutive_failures: u32,
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
    conservative_mode: bool,
}

impl NewznabRateLimiter {
    pub fn new(config: NewznabRateLimiterConfig) -> Self {
        let concurrency = config.max_concurrent_requests.max(1);
        Self {
            inner: Arc::new(RateLimiterInner {
                label: config.label,
                concurrency: Arc::new(Semaphore::new(concurrency as usize)),
                state: Mutex::new(RateLimiterState::default()),
                cooldown_interval: Duration::from_millis(config.cooldown_ms.max(100)),
                base_backoff_seconds: config.base_backoff_seconds.max(1),
                max_backoff_seconds: config
                    .max_backoff_seconds
                    .max(config.base_backoff_seconds.max(1)),
                jitter_max_ms: config.jitter_max_ms,
            }),
        }
    }

    /// Build from an `IndexerConfig` — uses the DB `rate_limit_seconds` and
    /// `rate_limit_burst` columns that were previously stored but never read.
    pub fn from_indexer_config(
        label: &str,
        rate_limit_seconds: Option<i64>,
        rate_limit_burst: Option<i64>,
    ) -> Self {
        let cooldown_ms = rate_limit_seconds
            .filter(|&s| s > 0)
            .map(|s| (s as u64) * 1000)
            .unwrap_or(1100);
        let max_concurrent = rate_limit_burst
            .filter(|&b| b > 0)
            .map(|b| b as u32)
            .unwrap_or(4);
        Self::new(NewznabRateLimiterConfig {
            label: label.to_string(),
            cooldown_ms,
            max_concurrent_requests: max_concurrent,
            ..Default::default()
        })
    }

    /// Acquire a rate-limit permit. Blocks if all concurrent slots are cooling
    /// down. Returns an error if the indexer is in backoff or quota is exhausted.
    pub async fn acquire(&self) -> AppResult<RateLimitGuard> {
        // Acquire semaphore permit — blocks if all N slots are in cooldown
        let permit = self
            .inner
            .concurrency
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| {
                AppError::Repository(format!(
                    "{}: rate limiter semaphore closed",
                    self.inner.label
                ))
            })?;

        // Brief mutex hold to check gates
        let cooldown = {
            let state = self.inner.state.lock().await;

            // Check backoff
            if let Some(disabled_until) = state.disabled_until {
                let now = Instant::now();
                if disabled_until > now {
                    let remaining = disabled_until.duration_since(now);
                    return Err(AppError::Repository(format!(
                        "{}: rate limited for {} more seconds",
                        self.inner.label,
                        remaining.as_secs()
                    )));
                }
            }

            // Check daily API quota exhaustion
            if let (Some(current), Some(max)) = (state.api_current, state.api_max) {
                if current >= max {
                    return Err(AppError::Repository(format!(
                        "{}: daily API quota exhausted ({}/{})",
                        self.inner.label, current, max
                    )));
                }
            }

            // Triple cooldown when in conservative mode (< 10% quota remaining)
            if state.conservative_mode {
                self.inner.cooldown_interval * 3
            } else {
                self.inner.cooldown_interval
            }
        };

        Ok(RateLimitGuard {
            permit: Some(permit),
            cooldown,
            label: self.inner.label.clone(),
        })
    }

    /// Record the outcome of a request. Updates quota counters, resets or
    /// escalates backoff.
    pub async fn record_response(
        &self,
        status_code: Option<reqwest::StatusCode>,
        error_code: Option<&str>,
        headers: &HeaderMap,
        api_limits: Option<&NewznabApiLimits>,
    ) -> AppResult<()> {
        let mut state = self.inner.state.lock().await;

        // Update API quota from response data
        if let Some(limits) = api_limits {
            if limits.api_current.is_some() || limits.api_max.is_some() {
                state.api_current = limits.api_current;
                state.api_max = limits.api_max;
            }
            if limits.grab_current.is_some() || limits.grab_max.is_some() {
                state.grab_current = limits.grab_current;
                state.grab_max = limits.grab_max;
            }

            // Check if we should enter conservative mode (< 10% remaining)
            state.conservative_mode = match (state.api_current, state.api_max) {
                (Some(current), Some(max)) if max > 0 => {
                    let remaining = max.saturating_sub(current);
                    let threshold = max / 10; // 10%
                    if remaining <= threshold {
                        info!(
                            label = %self.inner.label,
                            current = current,
                            max = max,
                            remaining = remaining,
                            "entering conservative mode — API quota running low"
                        );
                        true
                    } else {
                        false
                    }
                }
                _ => state.conservative_mode, // preserve if no data
            };
        }

        // Success path
        if status_code == Some(reqwest::StatusCode::OK) && error_code.is_none() {
            state.consecutive_failures = 0;
            state.disabled_until = None;
            return Ok(());
        }

        // Non-blocking error codes (e.g. error 109 = invalid user-agent)
        if let Some("109") = error_code {
            warn!(
                label = %self.inner.label,
                error_code = error_code.unwrap_or("unknown"),
                "indexer returned a non-blocking error"
            );
            state.consecutive_failures = 0;
            return Ok(());
        }

        // Failure path — exponential backoff
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        let status_backoff = retry_delay_for(status_code, error_code, headers);
        let failure_step = state.consecutive_failures.min(6);
        let exponential_seconds = (self
            .inner
            .base_backoff_seconds
            .saturating_mul(2u64.pow(failure_step.saturating_sub(1))))
        .min(self.inner.max_backoff_seconds);

        let actual = std::cmp::max(
            status_backoff.unwrap_or_default(),
            Duration::from_secs(exponential_seconds),
        );
        state.disabled_until = Some(Instant::now() + actual);

        if status_backoff.is_none() {
            debug!(
                label = %self.inner.label,
                consecutive_failures = state.consecutive_failures,
                delay_seconds = actual.as_secs(),
                error_code = error_code.unwrap_or("none"),
                "indexer error — applying exponential backoff"
            );
            return Ok(());
        }

        warn!(
            label = %self.inner.label,
            status = status_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "none".to_string()),
            error_code = error_code.unwrap_or("none"),
            delay_seconds = actual.as_secs(),
            consecutive_failures = state.consecutive_failures,
            "indexer blocked temporarily due to recent failures"
        );
        Ok(())
    }
}

/// Maps HTTP status codes and error codes to recommended retry delays.
pub fn retry_delay_for(
    status_code: Option<reqwest::StatusCode>,
    error_code: Option<&str>,
    headers: &HeaderMap,
) -> Option<Duration> {
    let status_based = match status_code {
        Some(code) if code == reqwest::StatusCode::UNAUTHORIZED => Some(Duration::from_secs(120)),
        Some(code) if code == reqwest::StatusCode::TOO_MANY_REQUESTS => {
            Some(Duration::from_secs(60))
        }
        Some(code) if code == reqwest::StatusCode::FORBIDDEN => Some(Duration::from_secs(120)),
        Some(code) if code == reqwest::StatusCode::SERVICE_UNAVAILABLE => {
            Some(Duration::from_secs(45))
        }
        Some(code)
            if code == reqwest::StatusCode::BAD_GATEWAY
                || code == reqwest::StatusCode::GATEWAY_TIMEOUT
                || code == reqwest::StatusCode::BAD_REQUEST =>
        {
            Some(Duration::from_secs(20))
        }
        _ => None,
    };

    if let Some(delay) = status_based {
        return Some(delay);
    }

    let header_retry = parse_retry_after(headers);
    if header_retry.is_some() {
        return header_retry;
    }

    if let Some(code) = error_code {
        if code == "109" {
            return None;
        }
    }

    None
}

/// Add jitter to a duration to prevent thundering herd.
#[allow(dead_code)]
pub fn add_jitter(delay: Duration, jitter_max_ms: u64) -> Duration {
    if jitter_max_ms == 0 {
        return delay;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_nanos())
        .unwrap_or_default();
    let jitter_ms = (nanos % u128::from(jitter_max_ms)) as u64;
    delay.saturating_add(Duration::from_millis(jitter_ms))
}

// ---------------------------------------------------------------------------
// RateLimitGuard — RAII guard with cooldown-on-drop
// ---------------------------------------------------------------------------

/// Holds a semaphore permit for the duration of a request. On drop, spawns a
/// background task that keeps the permit for `cooldown` before releasing it,
/// preventing the next request from firing immediately.
pub struct RateLimitGuard {
    permit: Option<OwnedSemaphorePermit>,
    cooldown: Duration,
    label: String,
}

impl std::fmt::Debug for RateLimitGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimitGuard")
            .field("label", &self.label)
            .field("cooldown", &self.cooldown)
            .field("has_permit", &self.permit.is_some())
            .finish()
    }
}

impl Drop for RateLimitGuard {
    fn drop(&mut self) {
        if let Some(permit) = self.permit.take() {
            let cooldown = self.cooldown;
            let label = self.label.clone();
            tokio::spawn(async move {
                tokio::time::sleep(cooldown).await;
                drop(permit);
                debug!(label = %label, cooldown_ms = cooldown.as_millis(), "rate limit slot released");
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderMap;

    #[test]
    fn test_xml_apilimits_parsing() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:newznab="http://www.newznab.com/DTD/2010/feeds/attributes/">
<channel>
<newznab:apilimits apiCurrent="42" apiMax="500" grabCurrent="10" grabMax="100"/>
<item><title>Test</title></item>
</channel>
</rss>"#;
        let limits = parse_newznab_apilimits_xml(xml).unwrap();
        assert_eq!(limits.api_current, Some(42));
        assert_eq!(limits.api_max, Some(500));
        assert_eq!(limits.grab_current, Some(10));
        assert_eq!(limits.grab_max, Some(100));
    }

    #[test]
    fn test_xml_apilimits_case_insensitive() {
        let xml = r#"<apilimits apicurrent="5" apimax="200"/>"#;
        let limits = parse_newznab_apilimits_xml(xml).unwrap();
        assert_eq!(limits.api_current, Some(5));
        assert_eq!(limits.api_max, Some(200));
        assert_eq!(limits.grab_current, None);
        assert_eq!(limits.grab_max, None);
    }

    #[test]
    fn test_xml_apilimits_missing() {
        let xml = r#"<rss><channel><item>stuff</item></channel></rss>"#;
        assert!(parse_newznab_apilimits_xml(xml).is_none());
    }

    #[test]
    fn test_header_apilimits_parsing() {
        let mut headers = HeaderMap::new();
        headers.insert("X-RateLimit-Limit", "500".parse().unwrap());
        headers.insert("X-RateLimit-Remaining", "458".parse().unwrap());

        let limits = parse_apilimits_from_headers(&headers).unwrap();
        assert_eq!(limits.api_current, Some(42)); // 500 - 458
        assert_eq!(limits.api_max, Some(500));
        assert_eq!(limits.grab_current, None);
        assert_eq!(limits.grab_max, None);
    }

    #[test]
    fn test_header_apilimits_missing() {
        let headers = HeaderMap::new();
        assert!(parse_apilimits_from_headers(&headers).is_none());
    }

    #[tokio::test]
    async fn test_quota_exhausted() {
        let limiter = NewznabRateLimiter::new(NewznabRateLimiterConfig {
            label: "test".to_string(),
            cooldown_ms: 100,
            max_concurrent_requests: 2,
            ..Default::default()
        });

        // Manually set quota to exhausted
        {
            let mut state = limiter.inner.state.lock().await;
            state.api_current = Some(500);
            state.api_max = Some(500);
        }

        let result = limiter.acquire().await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("quota exhausted"), "got: {err_msg}");
    }

    #[tokio::test]
    async fn test_conservative_mode_threshold() {
        let limiter = NewznabRateLimiter::new(NewznabRateLimiterConfig {
            label: "test".to_string(),
            cooldown_ms: 100,
            max_concurrent_requests: 2,
            ..Default::default()
        });

        // Report a response with 9% remaining → should trigger conservative mode
        let limits = NewznabApiLimits {
            api_current: Some(460),
            api_max: Some(500),
            ..Default::default()
        };
        limiter
            .record_response(
                Some(reqwest::StatusCode::OK),
                None,
                &HeaderMap::new(),
                Some(&limits),
            )
            .await
            .unwrap();

        let state = limiter.inner.state.lock().await;
        assert!(state.conservative_mode);
    }

    #[tokio::test]
    async fn test_concurrent_permits() {
        let limiter = NewznabRateLimiter::new(NewznabRateLimiterConfig {
            label: "test".to_string(),
            cooldown_ms: 5000, // long cooldown to keep permits held
            max_concurrent_requests: 2,
            ..Default::default()
        });

        // Acquire 2 permits — should succeed immediately
        let _g1 = limiter.acquire().await.unwrap();
        let _g2 = limiter.acquire().await.unwrap();

        // 3rd acquire should not succeed immediately (semaphore exhausted)
        let try_acquire = tokio::time::timeout(
            Duration::from_millis(50),
            limiter.acquire(),
        )
        .await;
        assert!(try_acquire.is_err(), "3rd acquire should have timed out");
    }

    #[tokio::test]
    async fn test_backoff_escalation() {
        let limiter = NewznabRateLimiter::new(NewznabRateLimiterConfig {
            label: "test".to_string(),
            cooldown_ms: 100,
            max_concurrent_requests: 4,
            base_backoff_seconds: 10,
            max_backoff_seconds: 900,
            jitter_max_ms: 0,
        });

        // Record 3 consecutive failures
        for _ in 0..3 {
            limiter
                .record_response(
                    Some(reqwest::StatusCode::INTERNAL_SERVER_ERROR),
                    None,
                    &HeaderMap::new(),
                    None,
                )
                .await
                .unwrap();
        }

        let state = limiter.inner.state.lock().await;
        assert_eq!(state.consecutive_failures, 3);
        assert!(state.disabled_until.is_some());
        // After 3 failures with base=10: 10 * 2^2 = 40 seconds
        let disabled = state.disabled_until.unwrap();
        let remaining = disabled.duration_since(Instant::now());
        assert!(remaining.as_secs() >= 35, "expected ~40s backoff, got {}s", remaining.as_secs());
    }

    #[test]
    fn test_add_jitter() {
        let base = Duration::from_secs(10);
        let jittered = add_jitter(base, 250);
        assert!(jittered >= base);
        assert!(jittered <= base + Duration::from_millis(250));
    }

    #[test]
    fn test_add_jitter_zero() {
        let base = Duration::from_secs(10);
        let jittered = add_jitter(base, 0);
        assert_eq!(jittered, base);
    }

    #[test]
    fn test_from_indexer_config_defaults() {
        let limiter = NewznabRateLimiter::from_indexer_config("test", None, None);
        assert_eq!(limiter.inner.cooldown_interval, Duration::from_millis(1100));
        assert_eq!(limiter.inner.concurrency.available_permits(), 4);
    }

    #[test]
    fn test_from_indexer_config_custom() {
        let limiter = NewznabRateLimiter::from_indexer_config("test", Some(2), Some(6));
        assert_eq!(limiter.inner.cooldown_interval, Duration::from_millis(2000));
        assert_eq!(limiter.inner.concurrency.available_permits(), 6);
    }
}
