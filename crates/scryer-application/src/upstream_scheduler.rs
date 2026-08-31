use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_outbound_http::{DestinationKey, HostKey};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::AppResult;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SchedulerCandidateId(Arc<str>);

impl SchedulerCandidateId {
    pub fn new() -> Self {
        Self(Arc::<str>::from(Uuid::new_v4().to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SchedulerCandidateId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SchedulerCandidateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for SchedulerCandidateId {
    fn from(value: String) -> Self {
        Self(Arc::<str>::from(value))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccountQuotaKey(Arc<str>);

impl AccountQuotaKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AccountQuotaKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for AccountQuotaKey {
    fn from(value: &str) -> Self {
        Self(Arc::<str>::from(value.trim().to_ascii_lowercase()))
    }
}

impl From<String> for AccountQuotaKey {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SchedulerPluginKind {
    Indexer,
    Subtitle,
    DownloadClient,
    Metadata,
    Maintenance,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SchedulerIntent {
    InteractiveSearch,
    BackgroundAcquisition,
    BackgroundRss,
    SubtitleSearch,
    SubtitleDownload,
    Maintenance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SchedulerOperation {
    Search,
    Rss,
    Grab,
    Download,
    CapsRefresh,
    ConnectionCheck,
    MetadataRefresh,
    ProwlarrSync,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EstimatedCost {
    pub api_calls: f64,
    pub grab_calls: f64,
}

impl EstimatedCost {
    pub const ONE_API_CALL: Self = Self {
        api_calls: 1.0,
        grab_calls: 0.0,
    };
}

impl Default for EstimatedCost {
    fn default() -> Self {
        Self::ONE_API_CALL
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExpectedValueHint {
    pub score: f64,
}

impl ExpectedValueHint {
    pub const NEUTRAL: Self = Self { score: 1.0 };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RateLimitCooldownAction {
    #[default]
    None,
    AlreadyRecorded,
    RecordFallback,
}

impl Default for ExpectedValueHint {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchLearningContext {
    pub indexer_id: String,
    pub facet: String,
    pub strategy: String,
    pub suppressed: bool,
    pub historically_useful: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RssFreshnessContext {
    pub last_successful_poll_at: Option<DateTime<Utc>>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub target_interval: Duration,
    pub latest_safe_poll_at: DateTime<Utc>,
    pub estimated_feed_depth: Option<u32>,
    pub freshness_risk: f64,
    pub destination_recent_activity_at: Option<DateTime<Utc>>,
    pub account_quota_budget: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct SchedulerCandidate {
    pub candidate_id: SchedulerCandidateId,
    pub plugin_config_id: Option<String>,
    pub plugin_kind: SchedulerPluginKind,
    pub operation: SchedulerOperation,
    pub intent: SchedulerIntent,
    pub host_key: HostKey,
    pub destination_key: DestinationKey,
    pub account_quota_key: Option<AccountQuotaKey>,
    pub rss_request_key: Option<String>,
    pub estimated_cost: EstimatedCost,
    pub expected_value: ExpectedValueHint,
    pub learning_context: Option<SearchLearningContext>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub freshness: Option<RssFreshnessContext>,
    pub cancel_token: CancellationToken,
}

#[derive(Clone, Debug)]
pub struct SchedulerBatchRequest {
    pub batch_id: String,
    pub now: DateTime<Utc>,
    pub candidates: Vec<SchedulerCandidate>,
}

#[derive(Clone, Debug)]
pub struct SchedulerBatchDecision {
    pub batch_id: String,
    /// Decisions are ordered from highest to lowest dispatch priority.
    /// Candidates with equal priority retain their input order.
    pub decisions: Vec<SchedulerAdmission>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchedulerLease {
    pub lease_id: String,
    pub candidate_id: SchedulerCandidateId,
    pub host_key: HostKey,
    pub destination_key: DestinationKey,
    pub account_quota_key: Option<AccountQuotaKey>,
    pub rss_request_key: Option<String>,
    pub operation: SchedulerOperation,
    pub intent: SchedulerIntent,
    pub issued_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionReason {
    InteractiveDeadline,
    HighCapacity,
    BackgroundValue,
    RssFreshness,
    SubtitleAllowed,
    MaintenanceAllowed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeferralReason {
    LowValueBackground,
    DestinationRecentlyUsed,
    RssCadence,
    SubtitleYieldedToAcquisition,
    MaintenanceLowPriority,
    AccountQuotaProbePending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    Cancelled,
    DeadlineExpired,
    LearningSuppressed,
    AccountQuotaExhausted,
    DestinationCooldown,
    HostUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchedulerAdmission {
    Admit {
        candidate_id: SchedulerCandidateId,
        lease: SchedulerLease,
        reason: AdmissionReason,
    },
    Defer {
        candidate_id: SchedulerCandidateId,
        retry_after: Option<Duration>,
        reason: DeferralReason,
    },
    Skip {
        candidate_id: SchedulerCandidateId,
        reason: SkipReason,
        retry_after: Option<Duration>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SchedulerFeedback {
    pub lease: Option<SchedulerLease>,
    pub host_key: HostKey,
    pub destination_key: DestinationKey,
    pub account_quota_key: Option<AccountQuotaKey>,
    pub outcome: SchedulerFeedbackOutcome,
    pub observed_api_current: Option<u64>,
    pub observed_api_max: Option<u64>,
    pub observed_grab_current: Option<u64>,
    pub observed_grab_max: Option<u64>,
    pub retry_after: Option<Duration>,
    pub cooldown_action: RateLimitCooldownAction,
    pub rss_last_seen_release_identity: Option<String>,
    pub rss_last_seen_release_published_at: Option<DateTime<Utc>>,
    pub rss_feed_result_count: Option<u32>,
    pub rss_seen_release_identities: Vec<String>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerFeedbackOutcome {
    Success,
    EmptySuccess,
    RateLimited,
    TransportFailure,
    ProviderFailure,
    Cancelled,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SchedulerSnapshotFilter {
    pub host_key: Option<HostKey>,
    pub destination_key: Option<DestinationKey>,
    pub account_quota_key: Option<AccountQuotaKey>,
}

impl SchedulerSnapshotFilter {
    pub fn from_raw_keys(
        host_key: Option<String>,
        destination_key: Option<String>,
        account_quota_key: Option<String>,
    ) -> Self {
        Self {
            host_key: host_key.map(HostKey::from),
            destination_key: destination_key.map(DestinationKey::from),
            account_quota_key: account_quota_key.map(AccountQuotaKey::from),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SchedulerSnapshot {
    pub entries: Vec<SchedulerSnapshotEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SchedulerSnapshotEntry {
    pub host_key: HostKey,
    pub destination_key: DestinationKey,
    pub account_quota_key: Option<AccountQuotaKey>,
    pub rss_request_key: Option<String>,
    pub last_decision: Option<String>,
    pub last_feedback_at: Option<DateTime<Utc>>,
    pub last_successful_at: Option<DateTime<Utc>>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub api_remaining_fraction: Option<f64>,
    pub quota_observed_at: Option<DateTime<Utc>>,
    pub quota_probe_after: Option<DateTime<Utc>>,
    pub quota_reset_at: Option<DateTime<Utc>>,
    pub quota_source: Option<String>,
    pub quota_stale: bool,
    pub rss_last_successful_poll_at: Option<DateTime<Utc>>,
    pub rss_last_attempt_at: Option<DateTime<Utc>>,
    pub rss_target_interval: Option<Duration>,
    pub rss_latest_safe_poll_at: Option<DateTime<Utc>>,
    pub rss_estimated_feed_depth: Option<u32>,
    pub rss_freshness_risk: Option<f64>,
    pub rss_destination_recent_activity_at: Option<DateTime<Utc>>,
    pub rss_last_seen_release_identity: Option<String>,
    pub rss_last_seen_release_published_at: Option<DateTime<Utc>>,
    pub rss_last_feed_gap_start_at: Option<DateTime<Utc>>,
    pub rss_last_feed_gap_end_at: Option<DateTime<Utc>>,
    pub admitted_count: u64,
    pub deferred_count: u64,
    pub skipped_count: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutboundRateLimitSnapshot {
    pub host_rps: Vec<OutboundHostRpsSnapshotEntry>,
    pub destination_cooldowns: Vec<OutboundDestinationCooldownSnapshotEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutboundHostRpsSnapshotEntry {
    pub host_key: String,
    pub lane: String,
    pub available_in: Duration,
    pub requests_per_second: f64,
    pub burst: u32,
    pub profile_source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutboundDestinationCooldownSnapshotEntry {
    pub destination_key: String,
    pub available_in: Duration,
}

impl From<scryer_outbound_http::RateLimitRegistrySnapshot> for OutboundRateLimitSnapshot {
    fn from(snapshot: scryer_outbound_http::RateLimitRegistrySnapshot) -> Self {
        Self {
            host_rps: snapshot
                .host_rps
                .into_iter()
                .map(|entry| OutboundHostRpsSnapshotEntry {
                    host_key: entry.host_key.to_string(),
                    lane: entry.lane.to_string(),
                    available_in: entry.available_in,
                    requests_per_second: entry.profile.requests_per_second,
                    burst: entry.profile.burst,
                    profile_source: format!("{:?}", entry.profile_source),
                })
                .collect(),
            destination_cooldowns: snapshot
                .destination_cooldowns
                .into_iter()
                .map(|entry| OutboundDestinationCooldownSnapshotEntry {
                    destination_key: entry.destination_key.to_string(),
                    available_in: entry.available_in,
                })
                .collect(),
        }
    }
}

#[async_trait]
pub trait UpstreamScheduler: Send + Sync {
    async fn admit_batch(
        &self,
        request: SchedulerBatchRequest,
    ) -> AppResult<SchedulerBatchDecision>;

    async fn record_feedback(&self, feedback: SchedulerFeedback) -> AppResult<()>;

    async fn snapshot(&self, filter: SchedulerSnapshotFilter) -> AppResult<SchedulerSnapshot>;

    async fn flush_pending(&self) -> AppResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SchedulerSnapshotFilter;

    #[test]
    fn snapshot_filter_from_raw_keys_normalizes_values() {
        let filter = SchedulerSnapshotFilter::from_raw_keys(
            Some("Feed.AnimeTosho.XYZ".to_string()),
            Some("Indexer:Feed.AnimeTosho.XYZ".to_string()),
            Some("Account:AnimeTosho".to_string()),
        );

        assert_eq!(
            filter.host_key.as_ref().map(|key| key.to_string()),
            Some("feed.animetosho.xyz".to_string())
        );
        assert_eq!(
            filter.destination_key.as_ref().map(|key| key.to_string()),
            Some("indexer:feed.animetosho.xyz".to_string())
        );
        assert_eq!(
            filter.account_quota_key.as_ref().map(|key| key.to_string()),
            Some("account:animetosho".to_string())
        );
    }
}
