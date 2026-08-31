use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;
use std::future::Future;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::num::NonZeroU32;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use governor::clock::Clock;
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use metrics::{counter, histogram};
use reqwest::header::{HeaderMap, LOCATION, RETRY_AFTER};
use reqwest::{
    Certificate, Client, RequestBuilder, Response, StatusCode, blocking::Client as BlockingClient,
};
use thiserror::Error;
use tokio::time::{Instant, sleep};
use tracing::debug;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RateLimitScopeKey(Arc<str>);

impl RateLimitScopeKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RateLimitScopeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for RateLimitScopeKey {
    fn from(value: &str) -> Self {
        Self(Arc::<str>::from(value))
    }
}

impl From<String> for RateLimitScopeKey {
    fn from(value: String) -> Self {
        Self(Arc::<str>::from(value))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryMode {
    SafeRead,
    ExplicitMutationRetry,
    NoRetry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectMode {
    NoFollow,
    TrustedFollow { max_hops: usize },
}

/// Matches the hop budget reqwest's default redirect policy allowed before
/// outbound requests moved to manual redirect handling.
pub const DEFAULT_TRUSTED_REDIRECT_HOPS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryAfterSource {
    HttpDate,
    Seconds,
    FallbackBackoff,
    ExistingCooldown,
}

impl RetryAfterSource {
    pub fn as_persistent_str(self) -> &'static str {
        match self {
            Self::HttpDate => "http_date",
            Self::Seconds => "seconds",
            Self::FallbackBackoff => "fallback_backoff",
            Self::ExistingCooldown => "existing_cooldown",
        }
    }

    pub fn from_persistent_str(value: &str) -> Option<Self> {
        match value {
            "http_date" => Some(Self::HttpDate),
            "seconds" => Some(Self::Seconds),
            "fallback_backoff" => Some(Self::FallbackBackoff),
            "existing_cooldown" => Some(Self::ExistingCooldown),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RateLimitRegistrySnapshot {
    pub host_rps: Vec<HostRpsSnapshotEntry>,
    pub destination_cooldowns: Vec<DestinationCooldownSnapshotEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostRpsSnapshotEntry {
    pub host_key: HostKey,
    pub lane: Arc<str>,
    pub available_in: Duration,
    pub profile: HostRpsProfile,
    pub profile_source: HostRpsProfileSource,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DestinationCooldownSnapshotEntry {
    pub destination_key: DestinationKey,
    pub available_in: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PersistedDestinationCooldown {
    pub destination_key: DestinationKey,
    pub cooldown_until: DateTime<Utc>,
    pub retry_after: Option<Duration>,
    pub source: RetryAfterSource,
    pub status_code: Option<u16>,
    pub message: Option<String>,
    pub observed_at: DateTime<Utc>,
}

fn destination_cooldown_is_newer_or_equal(
    candidate: &PersistedDestinationCooldown,
    existing: &PersistedDestinationCooldown,
) -> bool {
    candidate.observed_at > existing.observed_at
        || (candidate.observed_at == existing.observed_at
            && candidate.cooldown_until >= existing.cooldown_until)
}

#[derive(Clone, Debug)]
pub struct RequestPolicy {
    pub scope: RateLimitScopeKey,
    pub request_label: Cow<'static, str>,
    pub retry_mode: RetryMode,
    pub max_retries: u32,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    pub max_retry_after: Duration,
    pub redirect_mode: RedirectMode,
    pub host_rps_override: Option<HostRpsRequestOverride>,
    pub destination_cooldown_override: Option<DestinationKey>,
}

impl RequestPolicy {
    pub fn new(
        scope: impl Into<RateLimitScopeKey>,
        request_label: impl Into<Cow<'static, str>>,
        retry_mode: RetryMode,
    ) -> Self {
        let max_retries = match retry_mode {
            RetryMode::SafeRead => 2,
            RetryMode::ExplicitMutationRetry => 1,
            RetryMode::NoRetry => 0,
        };

        Self {
            scope: scope.into(),
            request_label: request_label.into(),
            retry_mode,
            max_retries,
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            max_retry_after: default_max_retry_after(),
            redirect_mode: RedirectMode::TrustedFollow {
                max_hops: DEFAULT_TRUSTED_REDIRECT_HOPS,
            },
            host_rps_override: None,
            destination_cooldown_override: None,
        }
    }

    pub fn safe_read(
        scope: impl Into<RateLimitScopeKey>,
        request_label: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(scope, request_label, RetryMode::SafeRead)
    }

    pub fn explicit_mutation_retry(
        scope: impl Into<RateLimitScopeKey>,
        request_label: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(scope, request_label, RetryMode::ExplicitMutationRetry)
    }

    pub fn no_retry(
        scope: impl Into<RateLimitScopeKey>,
        request_label: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(scope, request_label, RetryMode::NoRetry)
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_backoff(mut self, base_backoff: Duration, max_backoff: Duration) -> Self {
        self.base_backoff = base_backoff;
        self.max_backoff = max_backoff;
        self
    }

    pub fn with_max_retry_after(mut self, max_retry_after: Duration) -> Self {
        self.max_retry_after = max_retry_after;
        self
    }

    pub fn without_redirects(mut self) -> Self {
        self.redirect_mode = RedirectMode::NoFollow;
        self
    }

    pub fn with_trusted_redirects(mut self, max_hops: usize) -> Self {
        self.redirect_mode = RedirectMode::TrustedFollow { max_hops };
        self
    }

    pub fn with_host_rps_limit(
        mut self,
        lane: impl Into<Arc<str>>,
        profile: HostRpsProfile,
    ) -> Self {
        self.host_rps_override = Some(HostRpsRequestOverride {
            lane: lane.into(),
            profile,
        });
        self
    }

    pub fn with_destination_cooldown_key(mut self, destination: DestinationKey) -> Self {
        self.destination_cooldown_override = Some(destination);
        self
    }

    fn retry_allowed(&self, attempt: u32) -> bool {
        !matches!(self.retry_mode, RetryMode::NoRetry) && attempt <= self.max_retries
    }

    fn backoff_for_retry(&self, retry_index: u32) -> Duration {
        bounded_exponential_backoff(self.base_backoff, self.max_backoff, retry_index)
    }
}

pub const DEFAULT_HOST_RPS: f64 = 20.0;
pub const DEFAULT_HOST_RPS_BURST: u32 = 20;
pub const LOCAL_MANAGED_HOST_RPS: f64 = 10.0;
pub const LOCAL_MANAGED_HOST_RPS_BURST: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostRpsProfile {
    pub requests_per_second: f64,
    pub burst: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostRpsRequestOverride {
    pub lane: Arc<str>,
    pub profile: HostRpsProfile,
}

impl HostRpsProfile {
    pub const fn limited(requests_per_second: f64, burst: u32) -> Self {
        Self {
            requests_per_second,
            burst,
        }
    }

    pub const fn unthrottled() -> Self {
        Self {
            requests_per_second: f64::INFINITY,
            burst: u32::MAX,
        }
    }

    fn interval(self) -> Option<Duration> {
        (self.requests_per_second.is_finite() && self.requests_per_second > 0.0)
            .then(|| Duration::from_secs_f64(1.0 / self.requests_per_second))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostRpsProfileSource {
    UnknownPublicDefault,
    LocalOrManagedDefault,
    Loopback,
    ExplicitRegistration,
    RequestPolicyOverride,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostRpsProfileAssignment {
    pub profile: HostRpsProfile,
    pub source: HostRpsProfileSource,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HostKey(Arc<str>);

impl HostKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for HostKey {
    fn from(value: &str) -> Self {
        Self(Arc::<str>::from(normalize_host_key(value)))
    }
}

impl From<String> for HostKey {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DestinationKey(Arc<str>);

impl DestinationKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DestinationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for DestinationKey {
    fn from(value: &str) -> Self {
        Self(Arc::<str>::from(normalize_host_key(value)))
    }
}

impl From<String> for DestinationKey {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

#[derive(Default)]
struct HostRateLimitState {
    profiles: HashMap<HostKey, HostRpsProfileAssignment>,
    limiters: HashMap<HostRateLimiterKey, Arc<HostRateLimiterEntry>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct HostRateLimiterKey {
    host: HostKey,
    lane: Arc<str>,
}

struct HostRateLimiterEntry {
    assignment: HostRpsProfileAssignment,
    limiter: DefaultDirectRateLimiter,
    blocked_until: Mutex<Option<Instant>>,
}

impl HostRateLimiterEntry {
    fn new(assignment: HostRpsProfileAssignment, interval: Duration) -> Self {
        let interval = interval.max(Duration::from_nanos(1));
        let burst = NonZeroU32::new(assignment.profile.burst).unwrap_or(NonZeroU32::MIN);
        let quota = Quota::with_period(interval)
            .expect("host RPS interval must be non-zero")
            .allow_burst(burst);

        Self {
            assignment,
            limiter: RateLimiter::direct(quota),
            blocked_until: Mutex::new(None),
        }
    }

    fn observe_wait(&self, wait: Duration) {
        let deadline = Instant::now() + wait;
        let mut blocked_until = self
            .blocked_until
            .lock()
            .expect("host RPS observation lock poisoned");
        if blocked_until.is_none_or(|current| deadline > current) {
            *blocked_until = Some(deadline);
        }
    }
}

#[derive(Default)]
struct RateLimitRegistryState {
    host_rps: Mutex<HostRateLimitState>,
    destination_deadlines: Mutex<HashMap<DestinationKey, Instant>>,
    destination_cooldowns: Mutex<HashMap<DestinationKey, PersistedDestinationCooldown>>,
    dirty_destination_cooldowns: Mutex<HashMap<DestinationKey, PersistedDestinationCooldown>>,
}

#[derive(Clone)]
pub struct RateLimitRegistry {
    state: Arc<RateLimitRegistryState>,
}

impl RateLimitRegistry {
    pub fn new() -> Self {
        static SHARED: LazyLock<RateLimitRegistry> = LazyLock::new(RateLimitRegistry::isolated);
        SHARED.clone()
    }

    pub fn isolated() -> Self {
        Self {
            state: Arc::new(RateLimitRegistryState::default()),
        }
    }

    pub fn snapshot(&self) -> RateLimitRegistrySnapshot {
        let now = Instant::now();
        let mut host_rps = self
            .state
            .host_rps
            .lock()
            .expect("host RPS lock poisoned")
            .limiters
            .iter()
            .map(|(key, entry)| {
                let available_in = entry
                    .blocked_until
                    .lock()
                    .expect("host RPS observation lock poisoned")
                    .map(|deadline| deadline.saturating_duration_since(now))
                    .unwrap_or_default();
                HostRpsSnapshotEntry {
                    host_key: key.host.clone(),
                    lane: key.lane.clone(),
                    available_in,
                    profile: entry.assignment.profile,
                    profile_source: entry.assignment.source,
                }
            })
            .collect::<Vec<_>>();
        host_rps.sort_by(|left, right| {
            left.host_key
                .as_str()
                .cmp(right.host_key.as_str())
                .then_with(|| left.lane.cmp(&right.lane))
        });

        let mut destination_cooldowns = self
            .state
            .destination_deadlines
            .lock()
            .expect("destination deadline lock poisoned")
            .iter()
            .filter_map(|(destination_key, deadline)| {
                let available_in = deadline.saturating_duration_since(now);
                (!available_in.is_zero()).then(|| DestinationCooldownSnapshotEntry {
                    destination_key: destination_key.clone(),
                    available_in,
                })
            })
            .collect::<Vec<_>>();
        destination_cooldowns.sort_by(|left, right| {
            left.destination_key
                .as_str()
                .cmp(right.destination_key.as_str())
        });

        RateLimitRegistrySnapshot {
            host_rps,
            destination_cooldowns,
        }
    }

    pub async fn wait_for_destination_if_needed(
        &self,
        destination: &DestinationKey,
    ) -> Option<Duration> {
        let mut total_wait = Duration::ZERO;

        loop {
            let wait_duration = {
                let mut deadlines = self
                    .state
                    .destination_deadlines
                    .lock()
                    .expect("destination deadline lock poisoned");
                let Some(deadline) = deadlines.get(destination).copied() else {
                    break;
                };
                let now = Instant::now();
                let remaining = deadline.saturating_duration_since(now);
                if remaining.is_zero() {
                    deadlines.remove(destination);
                    break;
                } else {
                    remaining
                }
            };

            total_wait += wait_duration;
            sleep(wait_duration).await;
        }

        (!total_wait.is_zero()).then_some(total_wait)
    }

    pub fn active_destination_cooldown(&self, destination: &DestinationKey) -> Option<Duration> {
        let mut deadlines = self
            .state
            .destination_deadlines
            .lock()
            .expect("destination deadline lock poisoned");
        let deadline = deadlines.get(destination).copied()?;
        let now = Instant::now();
        let remaining = deadline.saturating_duration_since(now);
        if remaining.is_zero() {
            deadlines.remove(destination);
            None
        } else {
            Some(remaining)
        }
    }

    pub fn hydrate_destination_cooldowns<I>(&self, cooldowns: I)
    where
        I: IntoIterator<Item = PersistedDestinationCooldown>,
    {
        let now_wall = Utc::now();
        let now_instant = Instant::now();
        let mut deadlines = self
            .state
            .destination_deadlines
            .lock()
            .expect("destination deadline lock poisoned");
        let mut metadata = self
            .state
            .destination_cooldowns
            .lock()
            .expect("destination cooldown metadata lock poisoned");

        for cooldown in cooldowns {
            let Ok(delay) = (cooldown.cooldown_until - now_wall).to_std() else {
                continue;
            };
            if delay.is_zero() {
                continue;
            }
            deadlines.insert(cooldown.destination_key.clone(), now_instant + delay);
            metadata.insert(cooldown.destination_key.clone(), cooldown);
        }
    }

    pub fn drain_dirty_destination_cooldowns(&self) -> Vec<PersistedDestinationCooldown> {
        self.state
            .dirty_destination_cooldowns
            .lock()
            .expect("dirty destination cooldown lock poisoned")
            .drain()
            .map(|(_, cooldown)| cooldown)
            .collect()
    }

    pub fn requeue_dirty_destination_cooldowns<I>(&self, cooldowns: I)
    where
        I: IntoIterator<Item = PersistedDestinationCooldown>,
    {
        let mut dirty = self
            .state
            .dirty_destination_cooldowns
            .lock()
            .expect("dirty destination cooldown lock poisoned");
        for cooldown in cooldowns {
            match dirty.get(&cooldown.destination_key) {
                Some(existing) if !destination_cooldown_is_newer_or_equal(&cooldown, existing) => {}
                _ => {
                    dirty.insert(cooldown.destination_key.clone(), cooldown);
                }
            }
        }
    }

    pub fn wait_for_destination_if_needed_blocking(
        &self,
        destination: &DestinationKey,
    ) -> Option<Duration> {
        let mut total_wait = Duration::ZERO;

        loop {
            let wait_duration = {
                let mut deadlines = self
                    .state
                    .destination_deadlines
                    .lock()
                    .expect("destination deadline lock poisoned");
                let Some(deadline) = deadlines.get(destination).copied() else {
                    break;
                };
                let now = Instant::now();
                let remaining = deadline.saturating_duration_since(now);
                if remaining.is_zero() {
                    deadlines.remove(destination);
                    break;
                } else {
                    remaining
                }
            };

            total_wait += wait_duration;
            std::thread::sleep(wait_duration);
        }

        (!total_wait.is_zero()).then_some(total_wait)
    }

    pub async fn acquire_host_rps(&self, host: &HostKey) -> Option<Duration> {
        self.acquire_host_rps_for_request(host, None).await
    }

    async fn acquire_host_rps_for_request(
        &self,
        host: &HostKey,
        request_override: Option<&HostRpsRequestOverride>,
    ) -> Option<Duration> {
        let entry = self.host_limiter_for(host, request_override)?;
        let mut total_wait = Duration::ZERO;

        loop {
            match entry.limiter.check() {
                Ok(()) => return (!total_wait.is_zero()).then_some(total_wait),
                Err(not_until) => {
                    let wait = not_until.wait_time_from(entry.limiter.clock().now());
                    if wait.is_zero() {
                        continue;
                    }
                    entry.observe_wait(wait);
                    debug!(
                        host = host.as_str(),
                        requests_per_second = entry.assignment.profile.requests_per_second,
                        burst = entry.assignment.profile.burst,
                        profile_source = ?entry.assignment.source,
                        wait_ms = wait.as_millis(),
                        "outbound host quota waiting"
                    );
                    total_wait += wait;
                    sleep(wait).await;
                }
            }
        }
    }

    pub fn acquire_host_rps_blocking(&self, host: &HostKey) -> Option<Duration> {
        let entry = self.host_limiter_for(host, None)?;
        let mut total_wait = Duration::ZERO;

        loop {
            match entry.limiter.check() {
                Ok(()) => return (!total_wait.is_zero()).then_some(total_wait),
                Err(not_until) => {
                    let wait = not_until.wait_time_from(entry.limiter.clock().now());
                    if wait.is_zero() {
                        continue;
                    }
                    entry.observe_wait(wait);
                    debug!(
                        host = host.as_str(),
                        requests_per_second = entry.assignment.profile.requests_per_second,
                        burst = entry.assignment.profile.burst,
                        profile_source = ?entry.assignment.source,
                        wait_ms = wait.as_millis(),
                        "outbound host quota waiting"
                    );
                    total_wait += wait;
                    std::thread::sleep(wait);
                }
            }
        }
    }

    fn acquire_host_rps_blocking_until(
        &self,
        host: &HostKey,
        deadline: std::time::Instant,
    ) -> Result<Option<Duration>, ()> {
        let Some(entry) = self.host_limiter_for(host, None) else {
            return Ok(None);
        };
        let mut total_wait = Duration::ZERO;

        loop {
            match entry.limiter.check() {
                Ok(()) => return Ok((!total_wait.is_zero()).then_some(total_wait)),
                Err(not_until) => {
                    let wait = not_until.wait_time_from(entry.limiter.clock().now());
                    if wait.is_zero() {
                        continue;
                    }
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() || wait >= remaining {
                        return Err(());
                    }
                    entry.observe_wait(wait);
                    debug!(
                        host = host.as_str(),
                        requests_per_second = entry.assignment.profile.requests_per_second,
                        burst = entry.assignment.profile.burst,
                        profile_source = ?entry.assignment.source,
                        wait_ms = wait.as_millis(),
                        "outbound host quota waiting"
                    );
                    total_wait += wait;
                    std::thread::sleep(wait);
                }
            }
        }
    }

    pub fn register_host_profile(
        &self,
        host: HostKey,
        profile: HostRpsProfile,
        source: HostRpsProfileSource,
    ) {
        let mut host_rps = self.state.host_rps.lock().expect("host RPS lock poisoned");
        host_rps
            .profiles
            .insert(host.clone(), HostRpsProfileAssignment { profile, source });
        host_rps
            .limiters
            .retain(|key, _| key.host != host || key.lane.as_ref() != "default");
    }

    pub fn profile_for_host(&self, host: &HostKey) -> HostRpsProfileAssignment {
        self.state
            .host_rps
            .lock()
            .expect("host RPS lock poisoned")
            .profiles
            .get(host)
            .copied()
            .unwrap_or_else(|| classify_host_rps_profile(host.as_str()))
    }

    pub async fn record_destination_cooldown(
        &self,
        destination: &DestinationKey,
        delay: Duration,
        source: RetryAfterSource,
    ) -> (Duration, RetryAfterSource) {
        self.record_destination_cooldown_inner(destination, delay, source)
    }

    pub fn record_destination_cooldown_blocking(
        &self,
        destination: &DestinationKey,
        delay: Duration,
        source: RetryAfterSource,
    ) -> (Duration, RetryAfterSource) {
        self.record_destination_cooldown_inner(destination, delay, source)
    }

    fn record_destination_cooldown_inner(
        &self,
        destination: &DestinationKey,
        delay: Duration,
        source: RetryAfterSource,
    ) -> (Duration, RetryAfterSource) {
        let delay = delay.min(default_max_retry_after());
        if delay.is_zero() {
            return (Duration::ZERO, source);
        }

        let now_instant = Instant::now();
        let observed_at = Utc::now();
        let new_deadline = now_instant + delay;
        let mut deadlines = self
            .state
            .destination_deadlines
            .lock()
            .expect("destination deadline lock poisoned");

        let existing_deadline = deadlines
            .get(destination)
            .copied()
            .filter(|deadline| *deadline > now_instant);

        let effective_deadline = match existing_deadline {
            Some(existing) if existing > new_deadline => existing,
            _ => new_deadline,
        };

        deadlines.insert(destination.clone(), effective_deadline);

        let effective_delay = effective_deadline.saturating_duration_since(now_instant);
        let effective_source = match existing_deadline {
            Some(existing) if existing > new_deadline => RetryAfterSource::ExistingCooldown,
            _ => source,
        };

        if effective_source != RetryAfterSource::ExistingCooldown
            && let Ok(effective_chrono_delay) = chrono::Duration::from_std(effective_delay)
        {
            let cooldown = PersistedDestinationCooldown {
                destination_key: destination.clone(),
                cooldown_until: observed_at + effective_chrono_delay,
                retry_after: Some(delay),
                source,
                status_code: None,
                message: None,
                observed_at,
            };
            self.state
                .destination_cooldowns
                .lock()
                .expect("destination cooldown metadata lock poisoned")
                .insert(destination.clone(), cooldown.clone());
            self.state
                .dirty_destination_cooldowns
                .lock()
                .expect("dirty destination cooldown lock poisoned")
                .insert(destination.clone(), cooldown);
        }

        (effective_delay, effective_source)
    }

    fn host_limiter_for(
        &self,
        host: &HostKey,
        request_override: Option<&HostRpsRequestOverride>,
    ) -> Option<Arc<HostRateLimiterEntry>> {
        let mut host_rps = self.state.host_rps.lock().expect("host RPS lock poisoned");
        let (lane, assignment) = match request_override {
            Some(request_override) => (
                request_override.lane.clone(),
                HostRpsProfileAssignment {
                    profile: request_override.profile,
                    source: HostRpsProfileSource::RequestPolicyOverride,
                },
            ),
            None => (
                Arc::<str>::from("default"),
                host_rps
                    .profiles
                    .get(host)
                    .copied()
                    .unwrap_or_else(|| classify_host_rps_profile(host.as_str())),
            ),
        };
        let interval = assignment.profile.interval()?;
        let key = HostRateLimiterKey {
            host: host.clone(),
            lane,
        };

        if let Some(entry) = host_rps.limiters.get(&key)
            && entry.assignment == assignment
        {
            return Some(entry.clone());
        }

        let entry = Arc::new(HostRateLimiterEntry::new(assignment, interval));
        host_rps.limiters.insert(key, entry.clone());
        Some(entry)
    }
}

impl Default for RateLimitRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub const DEFAULT_USER_AGENT: &str = concat!("Scryer/", env!("CARGO_PKG_VERSION"));
pub const INDEXER_PROXY_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
/// Transport timeouts are per attempt; workflow-level guards own aggregate deadlines.
pub const STANDARD_HTTP_TIMEOUT: Duration = Duration::from_secs(60);
/// Per-attempt budget for native SABnzbd and NZBGet requests.
pub const DOWNLOAD_CLIENT_HTTP_TIMEOUT: Duration = Duration::from_secs(90);
/// Base wall-clock budget for indexer HTTP and plugin search operations.
pub const INDEXER_HTTP_TIMEOUT: Duration = Duration::from_secs(120);
/// Total invocation budget for download-client plugin operations.
pub const DOWNLOAD_CLIENT_PLUGIN_TIMEOUT: Duration = Duration::from_secs(240);
/// Default workflow deadline for reading download-client feedback.
pub const DEFAULT_DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT: Duration = Duration::from_secs(300);
/// Maximum operator-configurable request budget for an indexer proxy.
pub const MAX_INDEXER_PROXY_TIMEOUT_SECONDS: u32 = 120;
/// Scheduling/response grace added when an indexer may invoke a configured proxy.
pub const INDEXER_PROXY_TIMEOUT_GRACE: Duration = Duration::from_secs(5);
/// Budget for bounded operations that legitimately outlive an ordinary request.
pub const LONG_RUNNING_HTTP_OPERATION_TIMEOUT: Duration = Duration::from_secs(310);

/// Return the canonical wall-clock budget for one indexer operation.
///
/// Proxies do not extend this budget; all indexer paths share the same ceiling.
pub fn effective_indexer_timeout(_proxy_request_timeout_seconds: Option<u32>) -> Duration {
    INDEXER_HTTP_TIMEOUT
}

/// Return the bounded request budget for a solver or proxy-health request.
pub fn effective_indexer_proxy_request_timeout(request_timeout_seconds: u32) -> Duration {
    Duration::from_secs(u64::from(
        request_timeout_seconds.clamp(1, MAX_INDEXER_PROXY_TIMEOUT_SECONDS),
    ))
    .saturating_add(INDEXER_PROXY_TIMEOUT_GRACE)
    .min(INDEXER_HTTP_TIMEOUT)
}

#[derive(Debug, Error)]
pub enum OutboundDestinationError {
    #[error("{label} URL is invalid: {message}")]
    InvalidUrl {
        label: &'static str,
        message: String,
    },
    #[error("{label} URL must use http or https")]
    UnsupportedScheme { label: &'static str },
    #[error("{label} URL must include a host")]
    MissingHost { label: &'static str },
    #[error("{label} URL must not include embedded credentials")]
    EmbeddedCredentials { label: &'static str },
    #[error("{label} host failed to resolve: {host}: {source}")]
    ResolveFailed {
        label: &'static str,
        host: String,
        source: std::io::Error,
    },
    #[error("{label} host did not resolve: {host}")]
    NoResolvedAddresses { label: &'static str, host: String },
    #[error("{label} host resolves to a private or local address: {host}")]
    ForbiddenAddress { label: &'static str, host: String },
    #[error("failed to build pinned {label} client for {host}: {source}")]
    ClientBuild {
        label: &'static str,
        host: String,
        source: reqwest::Error,
    },
    #[error("{label} host resolves to a blocked link-local or cloud-metadata address: {host}")]
    BlockedLinkLocalOrMetadata { label: &'static str, host: String },
    #[error("failed to load {label} plugin trust bundle: {message}")]
    TrustBundle {
        label: &'static str,
        message: String,
    },
}

pub fn validate_operator_http_url(
    raw: &str,
    label: &'static str,
) -> Result<reqwest::Url, OutboundDestinationError> {
    let url = reqwest::Url::parse(raw).map_err(|source| OutboundDestinationError::InvalidUrl {
        label,
        message: source.to_string(),
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(OutboundDestinationError::UnsupportedScheme { label });
    }
    if url.host_str().is_none() {
        return Err(OutboundDestinationError::MissingHost { label });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(OutboundDestinationError::EmbeddedCredentials { label });
    }
    Ok(url)
}

pub fn validate_public_http_url(
    raw: &str,
    label: &'static str,
) -> Result<reqwest::Url, OutboundDestinationError> {
    validate_operator_http_url(raw, label)
}

pub fn validate_untrusted_public_http_url(
    raw: &str,
    label: &'static str,
) -> Result<reqwest::Url, OutboundDestinationError> {
    validate_operator_http_url(raw, label)
}

#[derive(Clone)]
pub struct PinnedPublicHttpTarget {
    url: reqwest::Url,
    host: String,
    resolved_addrs: Vec<SocketAddr>,
    client: Client,
}

impl PinnedPublicHttpTarget {
    pub fn url(&self) -> &reqwest::Url {
        &self.url
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn resolved_addrs(&self) -> &[SocketAddr] {
        &self.resolved_addrs
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
}

pub async fn prepare_untrusted_public_http_target(
    raw: &str,
    label: &'static str,
) -> Result<PinnedPublicHttpTarget, OutboundDestinationError> {
    let url = validate_untrusted_public_http_url(raw, label)?;
    prepare_untrusted_public_http_target_from_url(url, label).await
}

pub async fn prepare_untrusted_public_http_target_from_url(
    url: reqwest::Url,
    label: &'static str,
) -> Result<PinnedPublicHttpTarget, OutboundDestinationError> {
    let resolved_addrs = resolve_public_http_destination(&url, label).await?;
    let host = url
        .host_str()
        .ok_or(OutboundDestinationError::MissingHost { label })?
        .to_string();
    let client = reqwest_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &resolved_addrs)
        .build()
        .map_err(|source| OutboundDestinationError::ClientBuild {
            label,
            host: host.clone(),
            source,
        })?;

    Ok(PinnedPublicHttpTarget {
        url,
        host,
        resolved_addrs,
        client,
    })
}

pub async fn validate_public_http_destination(
    url: &reqwest::Url,
    label: &'static str,
) -> Result<(), OutboundDestinationError> {
    resolve_public_http_destination(url, label)
        .await
        .map(|_| ())
}

async fn resolve_public_http_destination(
    url: &reqwest::Url,
    label: &'static str,
) -> Result<Vec<SocketAddr>, OutboundDestinationError> {
    let host = url
        .host_str()
        .ok_or(OutboundDestinationError::MissingHost { label })?;
    let port = url
        .port_or_known_default()
        .ok_or(OutboundDestinationError::MissingHost { label })?;
    if let Some(ip) = parse_host_ip_literal(host) {
        validate_public_ip(ip, host, label)?;
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    let mut resolved = tokio::net::lookup_host((host, port))
        .await
        .map_err(|source| OutboundDestinationError::ResolveFailed {
            label,
            host: host.to_string(),
            source,
        })?;
    let mut resolved_addrs = Vec::new();
    for addr in &mut resolved {
        resolved_addrs.push(addr);
        validate_public_ip(addr.ip(), host, label)?;
    }
    if resolved_addrs.is_empty() {
        return Err(OutboundDestinationError::NoResolvedAddresses {
            label,
            host: host.to_string(),
        });
    }
    Ok(resolved_addrs)
}

fn validate_public_ip(
    ip: IpAddr,
    host: &str,
    label: &'static str,
) -> Result<(), OutboundDestinationError> {
    if public_http_ip_is_forbidden(ip) {
        return Err(OutboundDestinationError::ForbiddenAddress {
            label,
            host: host.to_string(),
        });
    }
    Ok(())
}

fn parse_host_ip_literal(host: &str) -> Option<IpAddr> {
    host.parse::<IpAddr>().ok().or_else(|| {
        host.strip_prefix('[')?
            .strip_suffix(']')?
            .parse::<IpAddr>()
            .ok()
    })
}

fn public_http_ip_is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin egress policy
// ---------------------------------------------------------------------------
//
// Plugins execute untrusted `.wasm` and can point the host at attacker-chosen
// destinations, so their egress needs a destination guard in addition to the
// per-plugin `allowed_hosts` allowlist (which stays the primary boundary).
//
// Scryer is self-hosted, so — unlike `public_http_ip_is_forbidden` — this
// policy deliberately ALLOWS RFC1918, loopback and IPv6 ULA: reaching a LAN or
// on-box companion service is a legitimate plugin use. It only HARD-BLOCKS the
// link-local range that fronts cloud instance-metadata services (IPv4
// 169.254.0.0/16 — AWS/Azure/GCP/DO/Alibaba IMDS at 169.254.169.254, IPv6
// fe80::/10) plus the `metadata.google.internal` hostname.
//
// Validated addresses are DNS-pinned into the returned client so a declared
// public host cannot DNS-rebind into the blocked range between validation and
// connection, and callers MUST re-validate every redirect hop by preparing a
// fresh target for the redirect location.

/// Hostnames that are always blocked for plugin egress regardless of the
/// per-plugin allowlist, because they front cloud instance-metadata services.
const BLOCKED_PLUGIN_EGRESS_HOSTS: &[&str] = &["metadata.google.internal"];

/// Returns true when `ip` is in the link-local range fronting cloud
/// instance-metadata endpoints and must never be reachable by plugin egress.
fn plugin_egress_ip_is_forbidden(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        // 169.254.0.0/16 covers AWS/Azure/GCP IMDS; 100.100.100.200 is
        // Alibaba Cloud IMDS.
        IpAddr::V4(ip) => ip.is_link_local() || ip.octets() == [100, 100, 100, 200],
        // fe80::/10 link-local.
        IpAddr::V6(ip) => ip.is_unicast_link_local(),
    }
}

/// Returns true when `host` is an always-blocked plugin-egress hostname.
fn plugin_egress_host_is_forbidden(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    BLOCKED_PLUGIN_EGRESS_HOSTS
        .iter()
        .any(|blocked| host.eq_ignore_ascii_case(blocked))
}

fn validate_plugin_egress_ip(
    ip: IpAddr,
    host: &str,
    label: &'static str,
) -> Result<(), OutboundDestinationError> {
    if plugin_egress_ip_is_forbidden(ip) {
        return Err(OutboundDestinationError::BlockedLinkLocalOrMetadata {
            label,
            host: host.to_string(),
        });
    }
    Ok(())
}

/// Validates the URL host against the always-blocked hostname list and returns
/// the host string for resolution.
fn plugin_egress_host_of<'a>(
    url: &'a reqwest::Url,
    label: &'static str,
) -> Result<&'a str, OutboundDestinationError> {
    let host = url
        .host_str()
        .ok_or(OutboundDestinationError::MissingHost { label })?;
    if plugin_egress_host_is_forbidden(host) {
        return Err(OutboundDestinationError::BlockedLinkLocalOrMetadata {
            label,
            host: host.to_string(),
        });
    }
    Ok(host)
}

async fn resolve_plugin_http_destination(
    url: &reqwest::Url,
    label: &'static str,
) -> Result<Vec<SocketAddr>, OutboundDestinationError> {
    let host = plugin_egress_host_of(url, label)?;
    let port = url
        .port_or_known_default()
        .ok_or(OutboundDestinationError::MissingHost { label })?;
    if let Some(ip) = parse_host_ip_literal(host) {
        validate_plugin_egress_ip(ip, host, label)?;
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    let mut resolved = tokio::net::lookup_host((host, port))
        .await
        .map_err(|source| OutboundDestinationError::ResolveFailed {
            label,
            host: host.to_string(),
            source,
        })?;
    let mut resolved_addrs = Vec::new();
    for addr in &mut resolved {
        resolved_addrs.push(addr);
        validate_plugin_egress_ip(addr.ip(), host, label)?;
    }
    if resolved_addrs.is_empty() {
        return Err(OutboundDestinationError::NoResolvedAddresses {
            label,
            host: host.to_string(),
        });
    }
    Ok(resolved_addrs)
}

fn resolve_plugin_http_destination_blocking(
    url: &reqwest::Url,
    label: &'static str,
) -> Result<Vec<SocketAddr>, OutboundDestinationError> {
    let host = plugin_egress_host_of(url, label)?;
    let port = url
        .port_or_known_default()
        .ok_or(OutboundDestinationError::MissingHost { label })?;
    if let Some(ip) = parse_host_ip_literal(host) {
        validate_plugin_egress_ip(ip, host, label)?;
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    let resolved = (host, port).to_socket_addrs().map_err(|source| {
        OutboundDestinationError::ResolveFailed {
            label,
            host: host.to_string(),
            source,
        }
    })?;
    let mut resolved_addrs = Vec::new();
    for addr in resolved {
        resolved_addrs.push(addr);
        validate_plugin_egress_ip(addr.ip(), host, label)?;
    }
    if resolved_addrs.is_empty() {
        return Err(OutboundDestinationError::NoResolvedAddresses {
            label,
            host: host.to_string(),
        });
    }
    Ok(resolved_addrs)
}

/// An async plugin-egress destination whose validated addresses are pinned into
/// the returned reqwest client. Redirects are disabled; the caller re-validates
/// each hop by preparing a fresh target for the redirect location.
#[derive(Clone)]
pub struct PinnedPluginHttpTarget {
    url: reqwest::Url,
    host: String,
    resolved_addrs: Vec<SocketAddr>,
    client: Client,
}

impl PinnedPluginHttpTarget {
    pub fn url(&self) -> &reqwest::Url {
        &self.url
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn resolved_addrs(&self) -> &[SocketAddr] {
        &self.resolved_addrs
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
}

/// Prepares a DNS-pinned, redirect-disabled async client for an untrusted
/// plugin-controlled URL under the plugin egress policy.
pub async fn prepare_plugin_http_target(
    raw: &str,
    label: &'static str,
) -> Result<PinnedPluginHttpTarget, OutboundDestinationError> {
    let url = validate_operator_http_url(raw, label)?;
    prepare_plugin_http_target_from_url(url, label).await
}

/// The trust-bundle-aware counterpart to [`prepare_plugin_http_target`].
/// Component plugins use this path so their async requests retain the same
/// operator-installed private roots as the legacy command host while keeping
/// DNS pinning and redirects disabled.
pub async fn prepare_plugin_http_target_with_extra_ca(
    raw: &str,
    extra_ca_bundle_pem: &str,
    label: &'static str,
) -> Result<PinnedPluginHttpTarget, OutboundDestinationError> {
    let url = validate_operator_http_url(raw, label)?;
    let resolved_addrs = resolve_plugin_http_destination(&url, label).await?;
    let host = url
        .host_str()
        .ok_or(OutboundDestinationError::MissingHost { label })?
        .to_string();
    let mut builder = reqwest_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &resolved_addrs);
    if !extra_ca_bundle_pem.trim().is_empty() {
        builder =
            builder.tls_certs_merge(uploaded_root_certificates(extra_ca_bundle_pem).map_err(
                |source| OutboundDestinationError::TrustBundle {
                    label,
                    message: source,
                },
            )?);
    }
    let client = builder
        .build()
        .map_err(|source| OutboundDestinationError::ClientBuild {
            label,
            host: host.clone(),
            source,
        })?;
    Ok(PinnedPluginHttpTarget {
        url,
        host,
        resolved_addrs,
        client,
    })
}

/// Same as [`prepare_plugin_http_target`] but for an already-parsed URL; use
/// this to re-validate a redirect location before following it.
pub async fn prepare_plugin_http_target_from_url(
    url: reqwest::Url,
    label: &'static str,
) -> Result<PinnedPluginHttpTarget, OutboundDestinationError> {
    let resolved_addrs = resolve_plugin_http_destination(&url, label).await?;
    let host = url
        .host_str()
        .ok_or(OutboundDestinationError::MissingHost { label })?
        .to_string();
    let client = reqwest_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &resolved_addrs)
        .build()
        .map_err(|source| OutboundDestinationError::ClientBuild {
            label,
            host: host.clone(),
            source,
        })?;

    Ok(PinnedPluginHttpTarget {
        url,
        host,
        resolved_addrs,
        client,
    })
}

/// The blocking counterpart to [`PinnedPluginHttpTarget`], built with the
/// plugin trust bundle. Used by the in-sandbox plugin HTTP host.
pub struct PinnedPluginBlockingHttpTarget {
    url: reqwest::Url,
    host: String,
    resolved_addrs: Vec<SocketAddr>,
    client: BlockingClient,
}

impl PinnedPluginBlockingHttpTarget {
    pub fn url(&self) -> &reqwest::Url {
        &self.url
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn resolved_addrs(&self) -> &[SocketAddr] {
        &self.resolved_addrs
    }

    pub fn client(&self) -> &BlockingClient {
        &self.client
    }

    pub fn into_client(self) -> BlockingClient {
        self.client
    }
}

/// Prepares a DNS-pinned, redirect-disabled blocking client for an untrusted
/// plugin-controlled URL under the plugin egress policy, merging the plugin
/// trust bundle when provided.
pub fn prepare_plugin_blocking_http_target(
    raw: &str,
    extra_ca_bundle_pem: &str,
    label: &'static str,
) -> Result<PinnedPluginBlockingHttpTarget, OutboundDestinationError> {
    let url = validate_operator_http_url(raw, label)?;
    let resolved_addrs = resolve_plugin_http_destination_blocking(&url, label)?;
    let host = url
        .host_str()
        .ok_or(OutboundDestinationError::MissingHost { label })?
        .to_string();
    let mut builder = blocking_reqwest_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &resolved_addrs);
    if !extra_ca_bundle_pem.trim().is_empty() {
        let certificates = uploaded_root_certificates(extra_ca_bundle_pem)
            .map_err(|message| OutboundDestinationError::TrustBundle { label, message })?;
        builder = builder.tls_certs_merge(certificates);
    }
    let client = builder
        .build()
        .map_err(|source| OutboundDestinationError::ClientBuild {
            label,
            host: host.clone(),
            source,
        })?;

    Ok(PinnedPluginBlockingHttpTarget {
        url,
        host,
        resolved_addrs,
        client,
    })
}

fn reqwest_client_builder() -> reqwest::ClientBuilder {
    install_default_rustls_provider();
    Client::builder()
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .timeout(STANDARD_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(DEFAULT_USER_AGENT)
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .zstd(true)
}

fn blocking_reqwest_client_builder() -> reqwest::blocking::ClientBuilder {
    install_default_rustls_provider();
    BlockingClient::builder()
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .timeout(STANDARD_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(DEFAULT_USER_AGENT)
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .zstd(true)
}

pub fn install_default_rustls_provider() {
    static INSTALL_RUSTLS_PROVIDER: OnceLock<()> = OnceLock::new();

    INSTALL_RUSTLS_PROVIDER.get_or_init(|| {
        // The provider is process-global; parallel workspace tests may install it first.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

pub fn generic_reqwest_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        reqwest_client_builder()
            .build()
            .expect("generic reqwest client should build")
    });
    CLIENT.clone()
}

/// Returns the no-redirect client for native indexer requests.
///
/// Request-level deadlines may shorten this budget, but native indexer paths
/// must not inherit the shorter generic HTTP ceiling while their coordinator
/// and plugin equivalents use `INDEXER_HTTP_TIMEOUT`.
pub fn indexer_reqwest_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        reqwest_client_builder()
            .timeout(INDEXER_HTTP_TIMEOUT)
            .build()
            .expect("indexer reqwest client should build")
    });
    CLIENT.clone()
}

/// Returns the operator-managed client used for normal indexer-proxy solve
/// requests. These requests deliberately present a stable browser-like user
/// agent while retaining the shared client's no-redirect policy.
pub fn indexer_proxy_reqwest_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        reqwest_client_builder()
            .user_agent(INDEXER_PROXY_USER_AGENT)
            .build()
            .expect("indexer proxy client should build")
    });
    CLIENT.clone()
}

/// Builds the indexer-proxy health client. The redirect limit preserves
/// reqwest's historical default for this path.
pub fn indexer_proxy_health_reqwest_client(timeout: Duration) -> Result<Client, reqwest::Error> {
    reqwest_client_builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(
            DEFAULT_TRUSTED_REDIRECT_HOPS,
        ))
        .user_agent(INDEXER_PROXY_USER_AGENT)
        .build()
}

pub fn external_arr_reqwest_client() -> Client {
    let mut builder = reqwest_client_builder().redirect(reqwest::redirect::Policy::none());
    if let Ok(proxy_url) = std::env::var("SCRYER_EXTERNAL_ARR_PROXY_URL")
        && !proxy_url.trim().is_empty()
        && let Ok(proxy) = reqwest::Proxy::all(proxy_url.trim())
    {
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .unwrap_or_else(|_| no_redirect_reqwest_client())
}

pub fn plugin_reqwest_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        reqwest_client_builder()
            .build()
            .expect("plugin reqwest client should build")
    });
    CLIENT.clone()
}

pub fn no_redirect_reqwest_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        reqwest_client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("no-redirect reqwest client should build")
    });
    CLIENT.clone()
}

pub fn smg_reqwest_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        reqwest_client_builder()
            .build()
            .expect("SMG reqwest client should build")
    });
    CLIENT.clone()
}

pub fn blocking_plugin_host_client(extra_ca_bundle_pem: &str) -> Result<BlockingClient, String> {
    let mut builder = blocking_reqwest_client_builder().redirect(reqwest::redirect::Policy::none());
    if !extra_ca_bundle_pem.trim().is_empty() {
        builder = builder.tls_certs_merge(uploaded_root_certificates(extra_ca_bundle_pem)?);
    }
    builder
        .build()
        .map_err(|error| format!("failed to build plugin HTTP client: {error}"))
}

/// Builds the blocking operator-managed client used for indexer-proxy solve
/// requests from the plugin host. Redirects remain disabled to preserve the
/// existing solve-call behavior.
pub fn blocking_indexer_proxy_reqwest_client(
    extra_ca_bundle_pem: &str,
) -> Result<BlockingClient, String> {
    let mut builder = blocking_reqwest_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(INDEXER_PROXY_USER_AGENT);
    if !extra_ca_bundle_pem.trim().is_empty() {
        builder = builder.tls_certs_merge(uploaded_root_certificates(extra_ca_bundle_pem)?);
    }
    builder
        .build()
        .map_err(|error| format!("failed to build indexer proxy HTTP client: {error}"))
}

/// Builds the async operator-managed client used by WASI Preview 2 indexer
/// components for challenge-solver requests. Redirects stay disabled and the
/// operator-installed private roots match the guarded target client.
pub fn indexer_proxy_reqwest_client_with_extra_ca(
    extra_ca_bundle_pem: &str,
) -> Result<Client, String> {
    let mut builder = reqwest_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(INDEXER_PROXY_USER_AGENT);
    if !extra_ca_bundle_pem.trim().is_empty() {
        builder = builder.tls_certs_merge(uploaded_root_certificates(extra_ca_bundle_pem)?);
    }
    builder
        .build()
        .map_err(|error| format!("failed to build indexer proxy HTTP client: {error}"))
}

pub fn blocking_reqwest_client() -> Result<BlockingClient, reqwest::Error> {
    blocking_reqwest_client_builder().build()
}

pub async fn send_reqwest_request(request: RequestBuilder) -> Result<Response, reqwest::Error> {
    let registry = RateLimitRegistry::new();
    let destination = request
        .try_clone()
        .and_then(|clone| clone.build().ok())
        .and_then(|request| destination_key_from_url(request.url()));
    let host = request
        .try_clone()
        .and_then(|clone| clone.build().ok())
        .and_then(|request| host_key_from_url(request.url()));

    if let Some(destination) = destination.as_ref() {
        let _ = registry.wait_for_destination_if_needed(destination).await;
    }
    if let Some(host) = host.as_ref() {
        let _ = registry.acquire_host_rps(host).await;
    }

    let response = request.send().await?;
    if response.status() == StatusCode::TOO_MANY_REQUESTS
        && let Some(destination) = destination_key_from_url(response.url()).or(destination)
    {
        let (delay, source) = retry_after_delay(response.headers(), Duration::from_secs(1));
        let _ = registry
            .record_destination_cooldown(&destination, delay, source)
            .await;
    }
    Ok(response)
}

/// Sends an async request while bounding time spent waiting for a persisted
/// destination cooldown. Artifact acquisition uses a zero-duration budget so
/// a cooling host is surfaced as a retryable application error instead of
/// consuming the entire fetch deadline.
pub async fn send_reqwest_request_with_cooldown_budget(
    request: RequestBuilder,
    max_cooldown_wait: Option<Duration>,
) -> Result<Response, AsyncOutboundHttpError> {
    let registry = RateLimitRegistry::new();
    let destination = request
        .try_clone()
        .and_then(|clone| clone.build().ok())
        .and_then(|request| destination_key_from_url(request.url()));
    let host = request
        .try_clone()
        .and_then(|clone| clone.build().ok())
        .and_then(|request| host_key_from_url(request.url()));

    if let Some(destination) = destination.as_ref()
        && let Some(remaining) = registry.active_destination_cooldown(destination)
    {
        if let Some(budget) = max_cooldown_wait
            && remaining > budget
        {
            return Err(AsyncOutboundHttpError::CooldownBudgetExceeded {
                destination: destination.clone(),
                remaining,
                budget,
            });
        }
        registry.wait_for_destination_if_needed(destination).await;
    }
    if let Some(host) = host.as_ref() {
        let _ = registry.acquire_host_rps(host).await;
    }
    let response = request.send().await?;
    if response.status() == StatusCode::TOO_MANY_REQUESTS
        && let Some(destination) = destination_key_from_url(response.url()).or(destination)
    {
        let (delay, source) = retry_after_delay(response.headers(), Duration::from_secs(1));
        let _ = registry
            .record_destination_cooldown(&destination, delay, source)
            .await;
    }
    Ok(response)
}

#[derive(Debug, Error)]
pub enum AsyncOutboundHttpError {
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error(
        "destination '{destination}' is rate limited for {remaining:?}, exceeding async wait budget {budget:?}"
    )]
    CooldownBudgetExceeded {
        destination: DestinationKey,
        remaining: Duration,
        budget: Duration,
    },
}

pub fn send_blocking_reqwest_request(
    request: reqwest::blocking::RequestBuilder,
) -> Result<reqwest::blocking::Response, reqwest::Error> {
    send_blocking_reqwest_request_with_cooldown_budget(request, None).map_err(|error| match error {
        BlockingOutboundHttpError::Request(error) => error,
        BlockingOutboundHttpError::CooldownBudgetExceeded { .. } => {
            unreachable!("unbounded blocking request cannot exhaust cooldown budget")
        }
        BlockingOutboundHttpError::DeadlineExceeded => {
            unreachable!("unbounded blocking request has no dispatch deadline")
        }
    })
}

pub fn send_blocking_reqwest_request_with_cooldown_budget(
    request: reqwest::blocking::RequestBuilder,
    max_cooldown_wait: Option<Duration>,
) -> Result<reqwest::blocking::Response, BlockingOutboundHttpError> {
    send_blocking_reqwest_request_with_cooldown_policy(request, max_cooldown_wait, None)
}

/// Sends a blocking request with the existing cooldown policy while ensuring
/// destination waiting and shared host pacing cannot outlive `deadline`.
pub fn send_blocking_reqwest_request_with_cooldown_budget_until(
    request: reqwest::blocking::RequestBuilder,
    max_cooldown_wait: Option<Duration>,
    deadline: std::time::Instant,
) -> Result<reqwest::blocking::Response, BlockingOutboundHttpError> {
    send_blocking_reqwest_request_with_cooldown_policy_inner(
        request,
        max_cooldown_wait,
        None,
        Some(deadline),
    )
}

pub fn send_blocking_reqwest_request_with_cooldown_policy(
    request: reqwest::blocking::RequestBuilder,
    max_cooldown_wait: Option<Duration>,
    destination_cooldown_override: Option<DestinationKey>,
) -> Result<reqwest::blocking::Response, BlockingOutboundHttpError> {
    send_blocking_reqwest_request_with_cooldown_policy_inner(
        request,
        max_cooldown_wait,
        destination_cooldown_override,
        None,
    )
}

fn send_blocking_reqwest_request_with_cooldown_policy_inner(
    mut request: reqwest::blocking::RequestBuilder,
    max_cooldown_wait: Option<Duration>,
    destination_cooldown_override: Option<DestinationKey>,
    deadline: Option<std::time::Instant>,
) -> Result<reqwest::blocking::Response, BlockingOutboundHttpError> {
    let registry = RateLimitRegistry::new();
    let has_destination_override = destination_cooldown_override.is_some();
    let destination = destination_cooldown_override.or_else(|| {
        request
            .try_clone()
            .and_then(|clone| clone.build().ok())
            .and_then(|request| destination_key_from_url(request.url()))
    });
    let host = request
        .try_clone()
        .and_then(|clone| clone.build().ok())
        .and_then(|request| host_key_from_url(request.url()));

    if let Some(destination) = destination.as_ref() {
        if let Some(remaining) = registry.active_destination_cooldown(destination) {
            if let Some(max_wait) = max_cooldown_wait
                && remaining > max_wait
            {
                return Err(BlockingOutboundHttpError::CooldownBudgetExceeded {
                    destination: destination.clone(),
                    remaining,
                    budget: max_wait,
                });
            }
            if deadline.is_some_and(|deadline| {
                remaining >= deadline.saturating_duration_since(std::time::Instant::now())
            }) {
                return Err(BlockingOutboundHttpError::DeadlineExceeded);
            }
        }
        let _ = registry.wait_for_destination_if_needed_blocking(destination);
    }
    if let Some(host) = host.as_ref() {
        match deadline {
            Some(deadline) => registry
                .acquire_host_rps_blocking_until(host, deadline)
                .map_err(|()| BlockingOutboundHttpError::DeadlineExceeded)?,
            None => registry.acquire_host_rps_blocking(host),
        };
    }
    if let Some(deadline) = deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(BlockingOutboundHttpError::DeadlineExceeded);
        }
        request = request.timeout(remaining);
    }

    let response = request.send()?;
    let response_destination = if has_destination_override {
        destination
    } else {
        destination_key_from_url(response.url()).or(destination)
    };
    if response.status() == StatusCode::TOO_MANY_REQUESTS
        && let Some(destination) = response_destination
    {
        let (delay, source) = retry_after_delay(response.headers(), Duration::from_secs(1));
        let _ = registry.record_destination_cooldown_blocking(&destination, delay, source);
    }
    Ok(response)
}

#[derive(Debug, Error)]
pub enum BlockingOutboundHttpError {
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error(
        "destination '{destination}' is rate limited for {remaining:?}, exceeding blocking wait budget {budget:?}"
    )]
    CooldownBudgetExceeded {
        destination: DestinationKey,
        remaining: Duration,
        budget: Duration,
    },
    #[error("outbound request deadline elapsed before dispatch")]
    DeadlineExceeded,
}

fn uploaded_root_certificates(bundle_pem: &str) -> Result<Vec<Certificate>, String> {
    if bundle_pem.trim().is_empty() {
        return Ok(Vec::new());
    }

    let certificates = Certificate::from_pem_bundle(bundle_pem.as_bytes())
        .map_err(|error| format!("failed to parse uploaded trusted certificate bundle: {error}"))?;
    if certificates.is_empty() {
        return Err(
            "uploaded trusted certificate bundle did not contain any X.509 certificates"
                .to_string(),
        );
    }
    Ok(certificates)
}

#[derive(Clone)]
pub struct OutboundHttpClient {
    client: Client,
    registry: RateLimitRegistry,
}

impl OutboundHttpClient {
    pub fn new(client: Client, registry: RateLimitRegistry) -> Self {
        Self { client, registry }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn registry(&self) -> &RateLimitRegistry {
        &self.registry
    }

    async fn send_builder_with_trusted_redirects(
        &self,
        builder: RequestBuilder,
        request_label: &str,
        host_rps_override: Option<&HostRpsRequestOverride>,
        max_hops: usize,
    ) -> Result<Response, reqwest::Error> {
        let mut response = builder.send().await?;
        for _ in 0..max_hops {
            let Some(next_url) = redirect_target_url(&response) else {
                return Ok(response);
            };
            if let Some(destination) = destination_key_from_url(&next_url) {
                let _ = self
                    .registry
                    .wait_for_destination_if_needed(&destination)
                    .await;
            }
            if let Some(host) = host_key_from_url(&next_url)
                && let Some(wait_duration) = self
                    .registry
                    .acquire_host_rps_for_request(&host, host_rps_override)
                    .await
            {
                debug!(
                    host = %host,
                    request_label,
                    wait_ms = wait_duration.as_millis(),
                    "outbound HTTP redirect host RPS wait"
                );
            }
            response = self.client.get(next_url).send().await?;
        }
        Ok(response)
    }

    pub async fn send<F>(
        &self,
        policy: RequestPolicy,
        build_request: F,
    ) -> Result<Response, OutboundHttpError>
    where
        F: Fn() -> RequestBuilder,
    {
        match self
            .send_async(policy, || async {
                Ok::<RequestBuilder, Infallible>(build_request())
            })
            .await
        {
            Ok(response) => Ok(response),
            Err(OutboundRequestError::Build(_)) => unreachable!("infallible builder"),
            Err(OutboundRequestError::Http(error)) => Err(error),
        }
    }

    pub async fn send_async<F, Fut, E>(
        &self,
        policy: RequestPolicy,
        build_request: F,
    ) -> Result<Response, OutboundRequestError<E>>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<RequestBuilder, E>>,
    {
        self.send_async_with_rate_limit_observer(policy, build_request, |_| async {})
            .await
    }

    /// Sends a request while giving callers ownership of every 429 response
    /// before cooldown and retry handling discards it.
    pub async fn send_with_rate_limit_observer<F, O, OFut>(
        &self,
        policy: RequestPolicy,
        build_request: F,
        observe_rate_limited_response: O,
    ) -> Result<Response, OutboundHttpError>
    where
        F: Fn() -> RequestBuilder,
        O: Fn(Response) -> OFut,
        OFut: Future<Output = ()>,
    {
        match self
            .send_async_with_rate_limit_observer(
                policy,
                || async { Ok::<RequestBuilder, Infallible>(build_request()) },
                observe_rate_limited_response,
            )
            .await
        {
            Ok(response) => Ok(response),
            Err(OutboundRequestError::Build(_)) => unreachable!("infallible builder"),
            Err(OutboundRequestError::Http(error)) => Err(error),
        }
    }

    pub async fn send_async_with_rate_limit_observer<F, Fut, E, O, OFut>(
        &self,
        policy: RequestPolicy,
        build_request: F,
        observe_rate_limited_response: O,
    ) -> Result<Response, OutboundRequestError<E>>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<RequestBuilder, E>>,
        O: Fn(Response) -> OFut,
        OFut: Future<Output = ()>,
    {
        let mut attempt = 0u32;

        loop {
            attempt += 1;
            let builder = build_request().await.map_err(OutboundRequestError::Build)?;
            let request_destination = policy.destination_cooldown_override.clone().or_else(|| {
                builder
                    .try_clone()
                    .and_then(|clone| clone.build().ok())
                    .and_then(|request| destination_key_from_url(request.url()))
            });

            if let Some(destination) = request_destination.as_ref()
                && let Some(wait_duration) = self
                    .registry
                    .wait_for_destination_if_needed(destination)
                    .await
            {
                counter!(
                    "scryer_outbound_http_destination_cooldown_wait_total",
                    "destination" => destination.to_string(),
                    "request_label" => policy.request_label.to_string()
                )
                .increment(1);
                histogram!(
                    "scryer_outbound_http_destination_cooldown_wait_seconds",
                    "destination" => destination.to_string(),
                    "request_label" => policy.request_label.to_string()
                )
                .record(wait_duration.as_secs_f64());
                debug!(
                    destination = %destination,
                    request_label = policy.request_label.as_ref(),
                    wait_ms = wait_duration.as_millis(),
                    "outbound HTTP destination cooldown wait"
                );
            }

            if let Some(host) = builder
                .try_clone()
                .and_then(|clone| clone.build().ok())
                .and_then(|request| host_key_from_url(request.url()))
                && let Some(wait_duration) = self
                    .registry
                    .acquire_host_rps_for_request(&host, policy.host_rps_override.as_ref())
                    .await
            {
                counter!(
                    "scryer_outbound_http_host_rps_wait_total",
                    "host" => host.to_string(),
                    "request_label" => policy.request_label.to_string()
                )
                .increment(1);
                histogram!(
                    "scryer_outbound_http_host_rps_wait_seconds",
                    "host" => host.to_string(),
                    "request_label" => policy.request_label.to_string()
                )
                .record(wait_duration.as_secs_f64());
                debug!(
                    host = %host,
                    request_label = policy.request_label.as_ref(),
                    wait_ms = wait_duration.as_millis(),
                    "outbound HTTP host RPS wait"
                );
            }

            let send_result = match policy.redirect_mode {
                RedirectMode::NoFollow => builder.send().await,
                RedirectMode::TrustedFollow { max_hops } => {
                    self.send_builder_with_trusted_redirects(
                        builder,
                        policy.request_label.as_ref(),
                        policy.host_rps_override.as_ref(),
                        max_hops,
                    )
                    .await
                }
            };

            match send_result {
                Ok(response) if response.status() != StatusCode::TOO_MANY_REQUESTS => {
                    return Ok(response);
                }
                Ok(response) => {
                    let retry_index = attempt.saturating_sub(1);
                    let fallback_backoff = policy.backoff_for_retry(retry_index);
                    let (candidate_delay, candidate_source) =
                        retry_after_delay(response.headers(), fallback_backoff);
                    let candidate_delay = candidate_delay.min(policy.max_retry_after);
                    let response_destination =
                        policy.destination_cooldown_override.clone().or_else(|| {
                            destination_key_from_url(response.url()).or(request_destination)
                        });
                    observe_rate_limited_response(response).await;
                    let (effective_delay, effective_source) =
                        if let Some(destination) = response_destination.as_ref() {
                            self.registry
                                .record_destination_cooldown(
                                    destination,
                                    candidate_delay,
                                    candidate_source,
                                )
                                .await
                        } else {
                            (candidate_delay, candidate_source)
                        };

                    counter!(
                        "scryer_outbound_http_429_total",
                        "scope" => policy.scope.to_string(),
                        "request_label" => policy.request_label.to_string(),
                        "source" => retry_after_source_label(effective_source).to_string()
                    )
                    .increment(1);

                    debug!(
                        scope = %policy.scope,
                        request_label = policy.request_label.as_ref(),
                        attempt,
                        retry_after_source = retry_after_source_label(effective_source),
                        retry_after_ms = effective_delay.as_millis(),
                        "outbound HTTP received 429"
                    );

                    if policy.retry_allowed(attempt) {
                        continue;
                    }

                    counter!(
                        "scryer_outbound_http_rate_limited_total",
                        "scope" => policy.scope.to_string(),
                        "request_label" => policy.request_label.to_string(),
                        "source" => retry_after_source_label(effective_source).to_string()
                    )
                    .increment(1);

                    return Err(OutboundRequestError::Http(OutboundHttpError::RateLimited(
                        RateLimitedError {
                            scope: policy.scope.clone(),
                            retry_after: Some(effective_delay),
                            attempts: attempt,
                            retry_after_source: effective_source,
                            request_label: policy.request_label.clone(),
                        },
                    )));
                }
                Err(source) => {
                    if is_retryable_transport_error(&source) && policy.retry_allowed(attempt) {
                        let backoff = policy.backoff_for_retry(attempt.saturating_sub(1));
                        counter!(
                            "scryer_outbound_http_transport_retry_total",
                            "scope" => policy.scope.to_string(),
                            "request_label" => policy.request_label.to_string()
                        )
                        .increment(1);
                        histogram!(
                            "scryer_outbound_http_transport_backoff_seconds",
                            "scope" => policy.scope.to_string(),
                            "request_label" => policy.request_label.to_string()
                        )
                        .record(backoff.as_secs_f64());
                        debug!(
                            scope = %policy.scope,
                            request_label = policy.request_label.as_ref(),
                            attempt,
                            backoff_ms = backoff.as_millis(),
                            error = %source,
                            "outbound HTTP transport retry"
                        );
                        sleep(backoff).await;
                        continue;
                    }

                    return Err(OutboundRequestError::Http(OutboundHttpError::Transport {
                        scope: policy.scope.clone(),
                        request_label: policy.request_label.clone(),
                        attempts: attempt,
                        source,
                    }));
                }
            }
        }
    }
}

#[derive(Debug, Error)]
#[error("request '{request_label}' was rate limited for scope '{scope}' after {attempts} attempts")]
pub struct RateLimitedError {
    pub scope: RateLimitScopeKey,
    pub retry_after: Option<Duration>,
    pub attempts: u32,
    pub retry_after_source: RetryAfterSource,
    pub request_label: Cow<'static, str>,
}

#[derive(Debug, Error)]
pub enum OutboundHttpError {
    #[error(transparent)]
    RateLimited(#[from] RateLimitedError),
    #[error(
        "request '{request_label}' transport failed for scope '{scope}' after {attempts} attempts: {source}"
    )]
    Transport {
        scope: RateLimitScopeKey,
        request_label: Cow<'static, str>,
        attempts: u32,
        #[source]
        source: reqwest::Error,
    },
}

#[derive(Debug)]
pub enum OutboundRequestError<E> {
    Build(E),
    Http(OutboundHttpError),
}

fn retry_after_delay(
    headers: &HeaderMap,
    fallback_delay: Duration,
) -> (Duration, RetryAfterSource) {
    let Some(raw_header) = headers.get(RETRY_AFTER) else {
        return (fallback_delay, RetryAfterSource::FallbackBackoff);
    };
    let Ok(raw_value) = raw_header.to_str() else {
        return (fallback_delay, RetryAfterSource::FallbackBackoff);
    };
    parse_retry_after(raw_value).unwrap_or((fallback_delay, RetryAfterSource::FallbackBackoff))
}

fn default_max_retry_after() -> Duration {
    const DEFAULT_MAX_RETRY_AFTER_SECS: u64 = 5 * 60;
    std::env::var("SCRYER_OUTBOUND_RETRY_AFTER_MAX_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_MAX_RETRY_AFTER_SECS))
}

pub fn parse_retry_after(raw_value: &str) -> Option<(Duration, RetryAfterSource)> {
    let trimmed = raw_value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(retry_at) = DateTime::parse_from_rfc2822(trimmed) {
        let retry_at = retry_at.with_timezone(&Utc);
        let now = Utc::now();
        if retry_at > now
            && let Ok(delay) = (retry_at - now).to_std()
            && !delay.is_zero()
        {
            return Some((delay, RetryAfterSource::HttpDate));
        }
    }

    if let Ok(seconds) = trimmed.parse::<u64>()
        && seconds > 0
    {
        return Some((Duration::from_secs(seconds), RetryAfterSource::Seconds));
    }

    None
}

pub fn host_key_from_url(url: &reqwest::Url) -> Option<HostKey> {
    url.host_str().map(HostKey::from)
}

pub fn destination_key_from_url(url: &reqwest::Url) -> Option<DestinationKey> {
    let host = url.host_str()?;
    let host_key = HostKey::from(host);
    if host_is_loopback(host_key.as_str()) {
        return url
            .port_or_known_default()
            .map(|port| DestinationKey::from(format!("{}:{port}", host_key.as_str())))
            .or_else(|| Some(DestinationKey::from(host_key.as_str())));
    }

    Some(DestinationKey::from(host_key.as_str()))
}

fn redirect_target_url(response: &Response) -> Option<reqwest::Url> {
    if !response.status().is_redirection() {
        return None;
    }
    let location = response.headers().get(LOCATION)?.to_str().ok()?;
    response.url().join(location).ok()
}

fn normalize_host_key(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn host_is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "::1")
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

fn classify_host_rps_profile(host: &str) -> HostRpsProfileAssignment {
    if host_is_loopback(host) {
        return HostRpsProfileAssignment {
            profile: HostRpsProfile::unthrottled(),
            source: HostRpsProfileSource::Loopback,
        };
    }

    if host_is_local_or_managed(host) {
        return HostRpsProfileAssignment {
            profile: HostRpsProfile::limited(LOCAL_MANAGED_HOST_RPS, LOCAL_MANAGED_HOST_RPS_BURST),
            source: HostRpsProfileSource::LocalOrManagedDefault,
        };
    }

    HostRpsProfileAssignment {
        profile: default_public_host_rps_profile(),
        source: HostRpsProfileSource::UnknownPublicDefault,
    }
}

fn host_is_local_or_managed(host: &str) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
            IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local(),
        };
    }

    !host.contains('.')
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".home.arpa")
}

fn default_public_host_rps_profile() -> HostRpsProfile {
    let rps = std::env::var("SCRYER_OUTBOUND_HOST_RPS")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(DEFAULT_HOST_RPS);
    HostRpsProfile::limited(rps, DEFAULT_HOST_RPS_BURST)
}

fn bounded_exponential_backoff(base: Duration, max: Duration, retry_index: u32) -> Duration {
    if base.is_zero() || max.is_zero() {
        return Duration::ZERO;
    }

    let shift = retry_index.min(31);
    let factor = 1u128 << shift;
    let base_millis = base.as_millis();
    let max_millis = max.as_millis();
    let scaled = base_millis.saturating_mul(factor).min(max_millis);
    Duration::from_millis(scaled.min(u64::MAX as u128) as u64)
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error.is_connect()
        || retryable_transport_error_text(&transport_error_chain_text(error))
}

fn transport_error_chain_text(error: &reqwest::Error) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = std::error::Error::source(error);
    while let Some(error) = source {
        messages.push(error.to_string());
        source = std::error::Error::source(error);
    }
    messages.join(": ")
}

fn retryable_transport_error_text(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("certificate") || normalized.contains("invalid url") {
        return false;
    }

    [
        "peer closed connection without sending tls close_notify",
        "connection closed before message completed",
        "connection reset",
        "unexpected eof",
        "end of file",
        "broken pipe",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn retry_after_source_label(source: RetryAfterSource) -> &'static str {
    match source {
        RetryAfterSource::HttpDate => "http_date",
        RetryAfterSource::Seconds => "seconds",
        RetryAfterSource::FallbackBackoff => "fallback_backoff",
        RetryAfterSource::ExistingCooldown => "existing_cooldown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::SystemTime;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn indexer_timeout_policy_is_fixed_for_direct_and_proxied_requests() {
        assert_eq!(effective_indexer_timeout(None), Duration::from_secs(120));
        assert_eq!(effective_indexer_timeout(Some(60)), INDEXER_HTTP_TIMEOUT);
        assert_eq!(
            effective_indexer_timeout(Some(u32::MAX)),
            INDEXER_HTTP_TIMEOUT
        );
        assert_eq!(
            effective_indexer_proxy_request_timeout(60),
            Duration::from_secs(65)
        );
        assert_eq!(
            effective_indexer_proxy_request_timeout(u32::MAX),
            INDEXER_HTTP_TIMEOUT
        );
        assert!(LONG_RUNNING_HTTP_OPERATION_TIMEOUT > INDEXER_HTTP_TIMEOUT);
    }

    #[test]
    fn parses_http_date_retry_after_first() {
        let retry_at = DateTime::<Utc>::from(SystemTime::now() + Duration::from_secs(60));
        let header = retry_at.to_rfc2822();
        let (delay, source) = parse_retry_after(&header).expect("expected parsed Retry-After");

        assert_eq!(source, RetryAfterSource::HttpDate);
        assert!(delay.as_secs() >= 59);
    }

    #[test]
    fn falls_back_to_seconds_when_date_parse_fails() {
        let (delay, source) = parse_retry_after("120").expect("expected parsed Retry-After");

        assert_eq!(source, RetryAfterSource::Seconds);
        assert_eq!(delay, Duration::from_secs(120));
    }

    #[test]
    fn falls_back_to_bounded_backoff_when_header_is_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("not-a-date"),
        );
        let (delay, source) = retry_after_delay(&headers, Duration::from_secs(7));

        assert_eq!(source, RetryAfterSource::FallbackBackoff);
        assert_eq!(delay, Duration::from_secs(7));
    }

    #[test]
    fn past_or_zero_retry_after_uses_fallback_backoff() {
        let past = DateTime::<Utc>::from(SystemTime::now() - Duration::from_secs(5)).to_rfc2822();
        let mut past_headers = HeaderMap::new();
        past_headers.insert(
            RETRY_AFTER,
            reqwest::header::HeaderValue::from_str(&past).expect("valid Retry-After header"),
        );
        let mut zero_headers = HeaderMap::new();
        zero_headers.insert(RETRY_AFTER, reqwest::header::HeaderValue::from_static("0"));
        let (past_delay, past_source) = retry_after_delay(&past_headers, Duration::from_secs(9));
        let (zero_delay, zero_source) = retry_after_delay(&zero_headers, Duration::from_secs(9));

        assert_eq!(past_source, RetryAfterSource::FallbackBackoff);
        assert_eq!(past_delay, Duration::from_secs(9));
        assert_eq!(zero_source, RetryAfterSource::FallbackBackoff);
        assert_eq!(zero_delay, Duration::from_secs(9));
    }

    #[test]
    fn operator_http_urls_allow_homelab_destinations() {
        for raw in [
            "http://localhost:9696",
            "http://127.0.0.1:8080",
            "http://192.168.1.50:9696",
            "http://10.42.0.12:8080",
            "http://prowlarr:9696",
        ] {
            validate_operator_http_url(raw, "operator integration")
                .unwrap_or_else(|error| panic!("{raw} should be operator-valid: {error}"));
        }
    }

    #[test]
    fn operator_http_urls_reject_bad_syntax_and_credentials() {
        assert!(matches!(
            validate_operator_http_url("ftp://example.test", "operator integration"),
            Err(OutboundDestinationError::UnsupportedScheme { .. })
        ));
        assert!(matches!(
            validate_operator_http_url("https://user:secret@example.test", "operator integration"),
            Err(OutboundDestinationError::EmbeddedCredentials { .. })
        ));
    }

    #[tokio::test]
    async fn untrusted_public_http_targets_reject_private_and_local_addresses() {
        for raw in [
            "http://127.0.0.1/",
            "http://10.0.0.1/",
            "http://172.16.0.1/",
            "http://192.168.1.1/",
            "http://169.254.1.1/",
            "http://0.0.0.0/",
            "http://255.255.255.255/",
            "http://224.0.0.1/",
            "http://[::1]/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            "http://[ff02::1]/",
        ] {
            assert!(
                matches!(
                    prepare_untrusted_public_http_target(raw, "untrusted fetch").await,
                    Err(OutboundDestinationError::ForbiddenAddress { .. })
                ),
                "{raw} should be blocked for untrusted fetches"
            );
        }
    }

    #[tokio::test]
    async fn untrusted_public_http_target_records_validated_socket_addresses() {
        let target = prepare_untrusted_public_http_target(
            "http://93.184.216.34:8080/artifact.wasm",
            "untrusted fetch",
        )
        .await
        .expect("literal public IP target should prepare");

        assert_eq!(target.host(), "93.184.216.34");
        assert_eq!(
            target.resolved_addrs(),
            &[SocketAddr::from(([93, 184, 216, 34], 8080))]
        );
    }

    const PLUGIN_EGRESS_BLOCKED_TARGETS: &[&str] = &[
        "http://169.254.169.254/latest/meta-data/",
        "http://169.254.1.1/",
        "http://100.100.100.200/latest/meta-data/",
        "http://[::ffff:169.254.169.254]/latest/meta-data/",
        "http://[fe80::1]/",
        "http://metadata.google.internal/computeMetadata/v1/",
    ];

    const PLUGIN_EGRESS_ALLOWED_TARGETS: &[&str] = &[
        "http://127.0.0.1/",
        "http://10.0.0.1/",
        "http://172.16.0.1/",
        "http://192.168.1.1/",
        "http://[::1]/",
        "http://[fc00::1]/",
    ];

    #[tokio::test]
    async fn plugin_http_targets_block_link_local_and_metadata() {
        for raw in PLUGIN_EGRESS_BLOCKED_TARGETS {
            assert!(
                matches!(
                    prepare_plugin_http_target(raw, "plugin egress").await,
                    Err(OutboundDestinationError::BlockedLinkLocalOrMetadata { .. })
                ),
                "{raw} must be blocked for plugin egress"
            );
        }
    }

    #[test]
    fn plugin_blocking_http_targets_block_link_local_and_metadata() {
        for raw in PLUGIN_EGRESS_BLOCKED_TARGETS {
            assert!(
                matches!(
                    prepare_plugin_blocking_http_target(raw, "", "plugin egress"),
                    Err(OutboundDestinationError::BlockedLinkLocalOrMetadata { .. })
                ),
                "{raw} must be blocked for blocking plugin egress"
            );
        }
    }

    #[tokio::test]
    async fn plugin_http_targets_allow_private_and_local_destinations() {
        for raw in PLUGIN_EGRESS_ALLOWED_TARGETS {
            prepare_plugin_http_target(raw, "plugin egress")
                .await
                .unwrap_or_else(|error| {
                    panic!("{raw} should be allowed for self-hosted plugins: {error}")
                });
        }
    }

    #[test]
    fn plugin_blocking_http_targets_allow_private_and_local_destinations() {
        for raw in PLUGIN_EGRESS_ALLOWED_TARGETS {
            prepare_plugin_blocking_http_target(raw, "", "plugin egress").unwrap_or_else(|error| {
                panic!("{raw} should be allowed for self-hosted plugins: {error}")
            });
        }
    }

    #[tokio::test]
    async fn plugin_http_redirect_to_metadata_is_rejected() {
        let (origin_url, _hits) = spawn_http_server(vec![http_response(
            302,
            &[("Location", "http://169.254.169.254/latest/meta-data/")],
            "",
        )])
        .await;

        // The loopback origin is allowed for self-hosted plugins...
        let target = prepare_plugin_http_target(&origin_url, "plugin egress")
            .await
            .expect("loopback origin should be allowed");
        let response = send_reqwest_request(target.client().get(target.url().clone()))
            .await
            .expect("origin request should return a response");
        assert_eq!(response.status(), StatusCode::FOUND);

        // ...but re-validating the redirect hop must reject the metadata target.
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("redirect should carry a Location header");
        let redirect_url = target
            .url()
            .join(location)
            .expect("redirect location should be a valid URL");
        assert!(
            matches!(
                prepare_plugin_http_target_from_url(redirect_url, "plugin egress redirect").await,
                Err(OutboundDestinationError::BlockedLinkLocalOrMetadata { .. })
            ),
            "redirect to cloud metadata must be rejected"
        );
    }

    #[tokio::test]
    async fn destination_override_isolates_children_while_sharing_the_host_limiter() {
        let (url, hits) = spawn_http_server(vec![
            http_response(429, &[("Retry-After", "60")], "rate limited"),
            http_response(200, &[], "ok"),
        ])
        .await;
        let registry = RateLimitRegistry::isolated();
        let client = OutboundHttpClient::new(generic_reqwest_client(), registry.clone());
        let child_a: DestinationKey = "parent:1".into();
        let child_b: DestinationKey = "parent:2".into();

        let first = client
            .send(
                RequestPolicy::no_retry("child-a", "child-a")
                    .with_host_rps_limit("prowlarr", HostRpsProfile::limited(1000.0, 20))
                    .with_destination_cooldown_key(child_a.clone()),
                || client.client().get(&url),
            )
            .await;
        assert!(matches!(first, Err(OutboundHttpError::RateLimited(_))));

        let second = client
            .send(
                RequestPolicy::no_retry("child-b", "child-b")
                    .with_host_rps_limit("prowlarr", HostRpsProfile::limited(1000.0, 20))
                    .with_destination_cooldown_key(child_b.clone()),
                || client.client().get(&url),
            )
            .await
            .expect("child B should not inherit child A's cooldown");

        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(hits.load(Ordering::Relaxed), 2);
        assert!(registry.active_destination_cooldown(&child_a).is_some());
        assert!(registry.active_destination_cooldown(&child_b).is_none());
        assert_eq!(registry.snapshot().host_rps.len(), 1);
    }

    #[tokio::test]
    async fn async_zero_cooldown_budget_returns_without_dispatching() {
        let (url, hits) = spawn_http_server(vec![
            http_response(429, &[("Retry-After", "60")], "rate limited"),
            http_response(200, &[], "must not be dispatched"),
        ])
        .await;
        let client = reqwest_client_builder().build().unwrap();

        let first = send_reqwest_request_with_cooldown_budget(client.get(&url), None)
            .await
            .expect("first request should return the 429 response");
        assert_eq!(first.status(), StatusCode::TOO_MANY_REQUESTS);
        drop(first);

        let error =
            send_reqwest_request_with_cooldown_budget(client.get(&url), Some(Duration::ZERO))
                .await
                .expect_err("active cooldown must exceed a zero wait budget");
        assert!(matches!(
            error,
            AsyncOutboundHttpError::CooldownBudgetExceeded { .. }
        ));
        assert_eq!(hits.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn blocking_destination_override_keeps_sibling_child_dispatchable() {
        let (url, hits) = spawn_http_server(vec![
            http_response(429, &[("Retry-After", "60")], "rate limited"),
            http_response(200, &[], "ok"),
        ])
        .await;

        let (first_status, blocked_a, second_status) = tokio::task::spawn_blocking(move || {
            let client = blocking_reqwest_client_builder().build().unwrap();
            let child_a: DestinationKey = "managed-indexer:blocking-parent:1".into();
            let child_b: DestinationKey = "managed-indexer:blocking-parent:2".into();

            let first = send_blocking_reqwest_request_with_cooldown_policy(
                client.get(&url),
                None,
                Some(child_a.clone()),
            )
            .unwrap();
            let first_status = first.status();
            drop(first);

            let blocked_a = send_blocking_reqwest_request_with_cooldown_policy(
                client.get(&url),
                Some(Duration::from_millis(1)),
                Some(child_a),
            )
            .unwrap_err();
            let second = send_blocking_reqwest_request_with_cooldown_policy(
                client.get(&url),
                Some(Duration::from_millis(1)),
                Some(child_b),
            )
            .unwrap();
            (first_status, blocked_a, second.status())
        })
        .await
        .unwrap();

        assert_eq!(first_status, StatusCode::TOO_MANY_REQUESTS);
        assert!(matches!(
            blocked_a,
            BlockingOutboundHttpError::CooldownBudgetExceeded { .. }
        ));
        assert_eq!(second_status, StatusCode::OK);
        assert_eq!(hits.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn destination_cooldown_records_dirty_persisted_state() {
        let registry = RateLimitRegistry::isolated();
        let destination: DestinationKey = "example.test".into();

        let _ = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_secs(30),
                RetryAfterSource::Seconds,
            )
            .await;

        let dirty = registry.drain_dirty_destination_cooldowns();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].destination_key, destination);
        assert_eq!(dirty[0].retry_after, Some(Duration::from_secs(30)));
        assert_eq!(dirty[0].source, RetryAfterSource::Seconds);
        assert!(dirty[0].cooldown_until > Utc::now());
        assert!(registry.drain_dirty_destination_cooldowns().is_empty());

        registry.requeue_dirty_destination_cooldowns(dirty.clone());
        assert_eq!(registry.drain_dirty_destination_cooldowns(), dirty);
    }

    #[tokio::test]
    async fn requeue_dirty_destination_cooldowns_keeps_newer_dirty_state() {
        let registry = RateLimitRegistry::isolated();
        let destination: DestinationKey = "example.test".into();

        let _ = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_secs(30),
                RetryAfterSource::Seconds,
            )
            .await;
        let older = registry.drain_dirty_destination_cooldowns();

        let _ = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_secs(120),
                RetryAfterSource::FallbackBackoff,
            )
            .await;
        registry.requeue_dirty_destination_cooldowns(older);

        let dirty = registry.drain_dirty_destination_cooldowns();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].destination_key, destination);
        assert_eq!(dirty[0].retry_after, Some(Duration::from_secs(120)));
        assert_eq!(dirty[0].source, RetryAfterSource::FallbackBackoff);
    }

    #[test]
    fn hydrate_destination_cooldowns_restores_active_deadline_without_dirty_state() {
        let registry = RateLimitRegistry::isolated();
        let destination: DestinationKey = "example.test".into();

        registry.hydrate_destination_cooldowns([PersistedDestinationCooldown {
            destination_key: destination.clone(),
            cooldown_until: Utc::now() + chrono::Duration::seconds(30),
            retry_after: Some(Duration::from_secs(30)),
            source: RetryAfterSource::ExistingCooldown,
            status_code: Some(429),
            message: Some("rate limited".to_string()),
            observed_at: Utc::now(),
        }]);

        assert!(registry.active_destination_cooldown(&destination).is_some());
        assert!(registry.drain_dirty_destination_cooldowns().is_empty());
    }

    #[tokio::test]
    async fn safe_read_retries_429_and_eventually_succeeds() {
        let (url, hits) = spawn_http_server(vec![
            http_response(429, &[("Retry-After", "bogus")], ""),
            http_response(
                200,
                &[("Content-Type", "application/json")],
                "{\"ok\":true}",
            ),
        ])
        .await;

        let client =
            OutboundHttpClient::new(generic_reqwest_client(), RateLimitRegistry::isolated());
        let policy = RequestPolicy::safe_read("test-server", "retry-test")
            .with_max_retries(1)
            .with_backoff(Duration::from_millis(5), Duration::from_millis(5));

        let response = client
            .send(policy, || client.client().get(&url))
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn retryable_transport_error_text_matches_transient_disconnects() {
        assert!(retryable_transport_error_text(
            "error sending request for url: client error (SendRequest): connection error: peer closed connection without sending TLS close_notify"
        ));
        assert!(retryable_transport_error_text(
            "connection closed before message completed"
        ));
        assert!(retryable_transport_error_text(
            "io error: unexpected EOF while reading response"
        ));
        assert!(retryable_transport_error_text("os error: broken pipe"));
    }

    #[test]
    fn retryable_transport_error_text_rejects_non_transient_failures() {
        assert!(!retryable_transport_error_text(
            "certificate verify failed: self signed certificate"
        ));
        assert!(!retryable_transport_error_text(
            "invalid url: relative URL without a base"
        ));
        assert!(!retryable_transport_error_text(
            "metadata gateway returned GraphQL validation error"
        ));
    }

    #[tokio::test]
    async fn safe_read_retries_dropped_transport_response() {
        let (url, hits) = spawn_http_server_with_dropped_first_response(http_response(
            200,
            &[("Content-Type", "application/json")],
            "{\"ok\":true}",
        ))
        .await;

        let client =
            OutboundHttpClient::new(generic_reqwest_client(), RateLimitRegistry::isolated());
        let policy = RequestPolicy::safe_read("test-server", "transport-retry-test")
            .with_max_retries(1)
            .with_backoff(Duration::from_millis(5), Duration::from_millis(5));

        let response = client
            .send(policy, || client.client().get(&url))
            .await
            .expect("dropped first response should be retried");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn no_retry_returns_rate_limited_error_immediately() {
        let (url, hits) =
            spawn_http_server(vec![http_response(429, &[("Retry-After", "bogus")], "")]).await;

        let client =
            OutboundHttpClient::new(generic_reqwest_client(), RateLimitRegistry::isolated());
        let policy = RequestPolicy::no_retry("test-server", "no-retry")
            .with_backoff(Duration::from_millis(5), Duration::from_millis(5));

        let error = client
            .send(policy, || client.client().get(&url))
            .await
            .expect_err("request should fail");

        match error {
            OutboundHttpError::RateLimited(rate_limited) => {
                assert_eq!(rate_limited.attempts, 1);
                assert_eq!(
                    rate_limited.retry_after_source,
                    RetryAfterSource::FallbackBackoff
                );
                assert_eq!(rate_limited.retry_after, Some(Duration::from_millis(5)));
            }
            other => panic!("expected rate limited error, got {other:?}"),
        }

        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn existing_cooldown_source_wins_when_longer() {
        let registry = RateLimitRegistry::isolated();
        let destination: DestinationKey = "indexer-1".into();

        let _ = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_millis(50),
                RetryAfterSource::Seconds,
            )
            .await;
        let (_, source) = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_millis(5),
                RetryAfterSource::FallbackBackoff,
            )
            .await;

        assert_eq!(source, RetryAfterSource::ExistingCooldown);
    }

    #[test]
    fn host_keys_are_normalized() {
        assert_eq!(HostKey::from("Example.COM.").as_str(), "example.com");
        assert_eq!(HostKey::from("[2001:db8::1]").as_str(), "2001:db8::1");
    }

    #[test]
    fn loopback_destination_keys_include_port() {
        let first = reqwest::Url::parse("http://127.0.0.1:3001/api").unwrap();
        let second = reqwest::Url::parse("http://127.0.0.1:3002/api").unwrap();
        let localhost = reqwest::Url::parse("http://localhost:3001/api").unwrap();

        assert_eq!(
            destination_key_from_url(&first).unwrap().as_str(),
            "127.0.0.1:3001"
        );
        assert_eq!(
            destination_key_from_url(&localhost).unwrap().as_str(),
            "localhost:3001"
        );
        assert_ne!(
            destination_key_from_url(&first),
            destination_key_from_url(&second)
        );
    }

    #[test]
    fn public_destination_keys_remain_host_scoped() {
        let first = reqwest::Url::parse("https://example.com:443/api").unwrap();
        let second = reqwest::Url::parse("https://example.com:8443/api").unwrap();

        assert_eq!(
            destination_key_from_url(&first).unwrap().as_str(),
            "example.com"
        );
        assert_eq!(
            destination_key_from_url(&first),
            destination_key_from_url(&second)
        );
    }

    #[test]
    fn registry_new_returns_shared_state() {
        let first = RateLimitRegistry::new();
        let second = RateLimitRegistry::new();

        assert!(Arc::ptr_eq(&first.state, &second.state));
    }

    #[test]
    fn blocking_send_refuses_destination_cooldown_beyond_budget() {
        let url = reqwest::Url::parse("http://bounded-cooldown-budget.invalid/test").unwrap();
        let destination = destination_key_from_url(&url).unwrap();
        let registry = RateLimitRegistry::new();
        let _ = registry.record_destination_cooldown_blocking(
            &destination,
            Duration::from_secs(5),
            RetryAfterSource::Seconds,
        );

        let client = blocking_reqwest_client_builder().build().unwrap();
        let error = send_blocking_reqwest_request_with_cooldown_budget(
            client.get(url),
            Some(Duration::from_millis(1)),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            BlockingOutboundHttpError::CooldownBudgetExceeded { .. }
        ));
    }

    #[test]
    fn host_rps_profiles_classify_public_local_and_loopback_hosts() {
        assert_eq!(DEFAULT_HOST_RPS, 20.0);
        let registry = RateLimitRegistry::isolated();

        let public = registry.profile_for_host(&HostKey::from("feed.animetosho.xyz"));
        assert_eq!(public.source, HostRpsProfileSource::UnknownPublicDefault);
        assert_eq!(
            public.profile,
            HostRpsProfile::limited(DEFAULT_HOST_RPS, DEFAULT_HOST_RPS_BURST)
        );

        let private_ip = registry.profile_for_host(&HostKey::from("192.168.1.20"));
        assert_eq!(
            private_ip.source,
            HostRpsProfileSource::LocalOrManagedDefault
        );
        assert_eq!(
            private_ip.profile,
            HostRpsProfile::limited(LOCAL_MANAGED_HOST_RPS, LOCAL_MANAGED_HOST_RPS_BURST)
        );

        let docker_service = registry.profile_for_host(&HostKey::from("prowlarr"));
        assert_eq!(
            docker_service.source,
            HostRpsProfileSource::LocalOrManagedDefault
        );

        let local_domain = registry.profile_for_host(&HostKey::from("indexer.home.arpa"));
        assert_eq!(
            local_domain.source,
            HostRpsProfileSource::LocalOrManagedDefault
        );

        let loopback = registry.profile_for_host(&HostKey::from("127.0.0.1"));
        assert_eq!(loopback.source, HostRpsProfileSource::Loopback);
        assert_eq!(loopback.profile, HostRpsProfile::unthrottled());
    }

    #[test]
    fn explicit_host_rps_profile_registration_overrides_classification() {
        let registry = RateLimitRegistry::isolated();
        let host = HostKey::from("proxy.example.com");
        let profile = HostRpsProfile::limited(LOCAL_MANAGED_HOST_RPS, LOCAL_MANAGED_HOST_RPS_BURST);

        registry.register_host_profile(
            host.clone(),
            profile,
            HostRpsProfileSource::ExplicitRegistration,
        );

        let assignment = registry.profile_for_host(&host);
        assert_eq!(assignment.profile, profile);
        assert_eq!(
            assignment.source,
            HostRpsProfileSource::ExplicitRegistration
        );
    }

    #[tokio::test]
    async fn host_rps_is_shared_per_host() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "rps.example.test".into();

        for _ in 0..DEFAULT_HOST_RPS_BURST {
            assert_eq!(registry.acquire_host_rps(&host).await, None);
        }
        assert!(registry.acquire_host_rps(&host).await.is_some());
    }

    #[tokio::test]
    async fn request_policy_lane_is_independent_from_default_host_capacity() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "import.example.test".into();
        let request_override = HostRpsRequestOverride {
            lane: Arc::from("external_import"),
            profile: HostRpsProfile::limited(200.0, 200),
        };

        assert_eq!(
            registry
                .acquire_host_rps_for_request(&host, Some(&request_override))
                .await,
            None
        );

        for _ in 0..DEFAULT_HOST_RPS_BURST {
            assert_eq!(registry.acquire_host_rps(&host).await, None);
        }

        let snapshot = registry.snapshot();
        assert!(snapshot.host_rps.iter().any(|entry| {
            entry.host_key == host
                && entry.lane.as_ref() == "external_import"
                && entry.profile == HostRpsProfile::limited(200.0, 200)
                && entry.profile_source == HostRpsProfileSource::RequestPolicyOverride
        }));
    }

    #[tokio::test]
    async fn different_hosts_have_independent_governor_buckets() {
        let registry = RateLimitRegistry::isolated();
        let first_host: HostKey = "first.example.test".into();
        let second_host: HostKey = "second.example.test".into();

        for _ in 0..DEFAULT_HOST_RPS_BURST {
            assert_eq!(registry.acquire_host_rps(&first_host).await, None);
        }

        assert_eq!(registry.acquire_host_rps(&second_host).await, None);
    }

    #[tokio::test]
    async fn blocking_and_async_callers_share_governor_capacity() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "shared-blocking.example.test".into();

        for _ in 0..DEFAULT_HOST_RPS_BURST {
            assert_eq!(registry.acquire_host_rps(&host).await, None);
        }

        let blocking_registry = registry.clone();
        let blocking_host = host.clone();
        let blocking = tokio::task::spawn_blocking(move || {
            blocking_registry.acquire_host_rps_blocking(&blocking_host)
        });
        assert!(blocking.await.unwrap().is_some());
    }

    #[test]
    fn blocking_host_pacing_refuses_to_outlive_deadline() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "deadline-blocking.example.test".into();
        registry.register_host_profile(
            host.clone(),
            HostRpsProfile::limited(1.0, 1),
            HostRpsProfileSource::ExplicitRegistration,
        );

        assert_eq!(registry.acquire_host_rps_blocking(&host), None);
        let started_at = std::time::Instant::now();
        assert!(
            registry
                .acquire_host_rps_blocking_until(&host, started_at + Duration::from_millis(25),)
                .is_err()
        );
        assert!(started_at.elapsed() < Duration::from_millis(200));
    }

    #[tokio::test]
    async fn async_host_pacing_is_cancellable_by_caller_deadline() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "deadline-async.example.test".into();
        registry.register_host_profile(
            host.clone(),
            HostRpsProfile::limited(1.0, 1),
            HostRpsProfileSource::ExplicitRegistration,
        );

        assert_eq!(registry.acquire_host_rps(&host).await, None);
        let started_at = Instant::now();
        assert!(
            tokio::time::timeout(Duration::from_millis(25), registry.acquire_host_rps(&host),)
                .await
                .is_err()
        );
        assert!(started_at.elapsed() < Duration::from_millis(200));
    }

    #[tokio::test]
    async fn loopback_hosts_bypass_default_rps() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "127.0.0.1".into();

        for _ in 0..8 {
            assert_eq!(registry.acquire_host_rps(&host).await, None);
        }
    }

    #[tokio::test]
    async fn snapshot_reports_host_rps_and_destination_cooldowns() {
        let registry = RateLimitRegistry::isolated();
        let host: HostKey = "snapshot.example.test".into();
        let destination: DestinationKey = "snapshot.example.test".into();

        for _ in 0..DEFAULT_HOST_RPS_BURST {
            assert_eq!(registry.acquire_host_rps(&host).await, None);
        }
        let waiting_registry = registry.clone();
        let waiting_host = host.clone();
        let waiting =
            tokio::spawn(async move { waiting_registry.acquire_host_rps(&waiting_host).await });
        sleep(Duration::from_millis(5)).await;
        let _ = registry
            .record_destination_cooldown(
                &destination,
                Duration::from_secs(1),
                RetryAfterSource::Seconds,
            )
            .await;

        let snapshot = registry.snapshot();

        assert!(snapshot.host_rps.iter().any(|entry| {
            entry.host_key == host
                && entry.lane.as_ref() == "default"
                && !entry.available_in.is_zero()
                && entry.profile_source == HostRpsProfileSource::UnknownPublicDefault
        }));
        assert!(
            snapshot
                .destination_cooldowns
                .iter()
                .any(|entry| entry.destination_key == destination && !entry.available_in.is_zero())
        );
        assert!(waiting.await.unwrap().is_some());
    }

    #[tokio::test]
    async fn outbound_client_paces_redirect_target_host() {
        let (target_bound_url, target_hits) = spawn_http_server(vec![http_response(
            200,
            &[("Content-Type", "text/plain")],
            "ok",
        )])
        .await;
        let target_addr = bound_url_socket_addr(&target_bound_url);
        let (origin_bound_url, origin_hits) = spawn_http_server(vec![http_response(
            302,
            &[("Location", "http://target.test/test")],
            "",
        )])
        .await;
        let origin_addr = bound_url_socket_addr(&origin_bound_url);
        let registry = RateLimitRegistry::isolated();
        let client = reqwest_client_builder()
            .resolve_to_addrs("origin.test", &[origin_addr])
            .resolve_to_addrs("target.test", &[target_addr])
            .build()
            .expect("client should build");
        let outbound = OutboundHttpClient::new(client.clone(), registry.clone());

        let response = outbound
            .send(
                RequestPolicy::safe_read("redirect-test", "redirect-test")
                    .with_trusted_redirects(DEFAULT_TRUSTED_REDIRECT_HOPS),
                || client.get("http://origin.test/test"),
            )
            .await
            .expect("redirected request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(origin_hits.load(Ordering::SeqCst), 1);
        assert_eq!(target_hits.load(Ordering::SeqCst), 1);
        let snapshot = registry.snapshot();
        assert!(
            snapshot
                .host_rps
                .iter()
                .any(|entry| entry.host_key == HostKey::from("origin.test"))
        );
        assert!(
            snapshot
                .host_rps
                .iter()
                .any(|entry| entry.host_key == HostKey::from("target.test"))
        );
    }

    #[tokio::test]
    async fn outbound_client_follows_redirects_by_default() {
        let (target_bound_url, target_hits) = spawn_http_server(vec![http_response(
            200,
            &[("Content-Type", "text/plain")],
            "ok",
        )])
        .await;
        let (origin_bound_url, origin_hits) = spawn_http_server(vec![http_response(
            302,
            &[("Location", target_bound_url.as_str())],
            "",
        )])
        .await;
        let registry = RateLimitRegistry::isolated();
        let client = reqwest_client_builder()
            .build()
            .expect("client should build");
        let outbound = OutboundHttpClient::new(client.clone(), registry);

        let response = outbound
            .send(
                RequestPolicy::safe_read("redirect-test", "redirect-test"),
                || client.get(origin_bound_url.clone()),
            )
            .await
            .expect("redirected request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(origin_hits.load(Ordering::SeqCst), 1);
        assert_eq!(target_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn outbound_client_without_redirects_does_not_follow() {
        let (target_bound_url, target_hits) = spawn_http_server(vec![http_response(
            200,
            &[("Content-Type", "text/plain")],
            "ok",
        )])
        .await;
        let (origin_bound_url, origin_hits) = spawn_http_server(vec![http_response(
            302,
            &[("Location", target_bound_url.as_str())],
            "",
        )])
        .await;
        let registry = RateLimitRegistry::isolated();
        let client = reqwest_client_builder()
            .build()
            .expect("client should build");
        let outbound = OutboundHttpClient::new(client.clone(), registry);

        let response = outbound
            .send(
                RequestPolicy::safe_read("redirect-test", "redirect-test").without_redirects(),
                || client.get(origin_bound_url.clone()),
            )
            .await
            .expect("redirect response should be returned");

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(origin_hits.load(Ordering::SeqCst), 1);
        assert_eq!(target_hits.load(Ordering::SeqCst), 0);
    }

    fn http_response(status: u16, headers: &[(&str, &str)], body: &str) -> String {
        let mut response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (name, value) in headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        response.push_str(body);
        response
    }

    async fn spawn_http_server(responses: Vec<String>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener.local_addr().expect("listener should have addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_task = hits.clone();

        tokio::spawn(async move {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                hits_for_task.fetch_add(1, Ordering::SeqCst);
                if read_request(&mut stream).await.is_err() {
                    break;
                }
                if stream.write_all(response.as_bytes()).await.is_err() {
                    break;
                }
                let _ = stream.shutdown().await;
            }
        });

        (format!("http://{address}/test"), hits)
    }

    async fn spawn_http_server_with_dropped_first_response(
        success_response: String,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener.local_addr().expect("listener should have addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_task = hits.clone();

        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            hits_for_task.fetch_add(1, Ordering::SeqCst);
            let _ = read_request(&mut stream).await;
            let _ = stream.shutdown().await;

            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            hits_for_task.fetch_add(1, Ordering::SeqCst);
            if read_request(&mut stream).await.is_err() {
                return;
            }
            let _ = stream.write_all(success_response.as_bytes()).await;
            let _ = stream.shutdown().await;
        });

        (format!("http://{address}/test"), hits)
    }

    fn bound_url_socket_addr(url: &str) -> SocketAddr {
        let url = reqwest::Url::parse(url).expect("bound URL should parse");
        SocketAddr::new(
            url.host_str()
                .expect("bound URL should include host")
                .parse()
                .expect("bound URL host should parse"),
            url.port().expect("bound URL should include port"),
        )
    }

    async fn read_request(stream: &mut tokio::net::TcpStream) -> io::Result<()> {
        let mut buffer = vec![0u8; 4096];
        let mut received = Vec::new();
        loop {
            let read = stream.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            received.extend_from_slice(&buffer[..read]);
            if received.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        Ok(())
    }
}
