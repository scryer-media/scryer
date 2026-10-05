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
    /// Publish time of the oldest release the poll returned, across every page.
    pub rss_oldest_release_published_at: Option<DateTime<Utc>>,
    /// The indexer stopped paging at its page ceiling before it finished.
    pub rss_stopped_at_page_ceiling: bool,
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

/// Freshness risk at or above this bar escalates an RSS poll past its cadence.
///
/// The scheduler raises the risk when a feed shows signs of having moved on
/// without us (a gap between the last release we saw and the oldest one the
/// feed still carries), which is exactly when waiting out the interval loses
/// releases.
pub const RSS_FRESHNESS_ESCALATION_THRESHOLD: f64 = 0.85;

/// The one rule for "is this RSS destination due?".
///
/// Lives here because two callers must agree on it: the scheduler, which
/// defers a candidate that is not due, and the RSS worker, which skips a whole
/// cycle when nothing is. A destination the scheduler holds no freshness for
/// has never been polled, so it is due.
pub fn rss_poll_is_due(
    latest_safe_poll_at: Option<DateTime<Utc>>,
    freshness_risk: Option<f64>,
    now: DateTime<Utc>,
) -> bool {
    let Some(latest_safe_poll_at) = latest_safe_poll_at else {
        return true;
    };
    now >= latest_safe_poll_at
        || freshness_risk.is_some_and(|risk| risk >= RSS_FRESHNESS_ESCALATION_THRESHOLD)
}

/// Whether an RSS poll read back far enough to reach the previous poll's
/// newest release, either by returning that release again or by returning a
/// release published no later than it.
///
/// A poll with no previous marker, or one that returned nothing, has no window
/// to judge and counts as covered. Shared by the scheduler, which records the
/// uncovered window, and the search client, which logs it.
///
/// A poll the indexer cut off at its page ceiling counts as covered only when
/// it returned the marker release itself. Usenet publish times are post times
/// while feeds are ordered by listing time, so one late-listed old post would
/// otherwise make an unfinished read look complete.
pub fn rss_poll_reached_marker(
    previous_identity: Option<&str>,
    previous_published_at: Option<DateTime<Utc>>,
    seen_identities: &[String],
    oldest_published_at: Option<DateTime<Utc>>,
    stopped_at_page_ceiling: bool,
) -> bool {
    let Some(previous_identity) = previous_identity else {
        return true;
    };
    if seen_identities.is_empty()
        || seen_identities
            .iter()
            .any(|identity| identity == previous_identity)
    {
        return true;
    }
    if stopped_at_page_ceiling {
        return false;
    }
    matches!(
        (oldest_published_at, previous_published_at),
        (Some(oldest), Some(previous)) if oldest <= previous
    )
}

/// Which RSS-capable indexers are due for a poll right now.
///
/// Both lists empty means "not known" — an implementation that cannot answer
/// says so by staying silent, and the caller polls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RssDueIndexers {
    /// Indexer ids whose cadence is due (or that have never been polled).
    pub due: Vec<String>,
    /// Indexer ids the cadence is deliberately holding back this tick.
    pub deferred: Vec<String>,
}

impl RssDueIndexers {
    /// No answer available; the caller must poll.
    pub fn unknown() -> Self {
        Self::default()
    }

    /// Whether a sync cycle has any indexer work to do.
    ///
    /// Deliberately true when nothing is known: silence must never suppress a
    /// poll, only a positive "every indexer is holding" may.
    pub fn poll_is_warranted(&self) -> bool {
        !self.due.is_empty() || self.deferred.is_empty()
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
    use super::{
        RSS_FRESHNESS_ESCALATION_THRESHOLD, RssDueIndexers, rss_poll_is_due,
        rss_poll_reached_marker,
    };
    use chrono::{Duration, Utc};

    #[test]
    fn an_rss_poll_reaches_its_marker_by_identity_or_by_date() {
        let marker_at = Utc::now() - Duration::hours(2);
        let marker = Some("marker-guid");
        let seen = vec!["newer-guid".to_string(), "marker-guid".to_string()];
        let unseen = vec!["newer-guid".to_string(), "other-guid".to_string()];

        assert!(rss_poll_reached_marker(None, None, &unseen, None, false));
        assert!(rss_poll_reached_marker(
            marker,
            Some(marker_at),
            &[],
            None,
            false
        ));
        assert!(rss_poll_reached_marker(
            marker,
            Some(marker_at),
            &seen,
            Some(marker_at + Duration::hours(1)),
            false,
        ));
        assert!(rss_poll_reached_marker(
            marker,
            Some(marker_at),
            &unseen,
            Some(marker_at),
            false,
        ));
        assert!(!rss_poll_reached_marker(
            marker,
            Some(marker_at),
            &unseen,
            Some(marker_at + Duration::seconds(1)),
            false,
        ));
        assert!(!rss_poll_reached_marker(
            marker,
            None,
            &unseen,
            Some(marker_at),
            false
        ));
    }

    #[test]
    fn a_poll_cut_off_at_its_page_ceiling_reaches_the_marker_only_by_identity() {
        let marker_at = Utc::now() - Duration::hours(2);
        let marker = Some("marker-guid");
        let seen = vec!["newer-guid".to_string(), "marker-guid".to_string()];
        let unseen = vec!["newer-guid".to_string(), "late-listed-guid".to_string()];

        // A late-listed old post predates the marker, but the read stopped
        // at the ceiling, so its date proves nothing.
        assert!(!rss_poll_reached_marker(
            marker,
            Some(marker_at),
            &unseen,
            Some(marker_at - Duration::days(30)),
            true,
        ));
        assert!(rss_poll_reached_marker(
            marker,
            Some(marker_at),
            &seen,
            Some(marker_at + Duration::hours(1)),
            true,
        ));
    }

    #[test]
    fn a_destination_with_no_freshness_has_never_been_polled_and_is_due() {
        assert!(rss_poll_is_due(None, None, Utc::now()));
        assert!(rss_poll_is_due(None, Some(0.0), Utc::now()));
    }

    #[test]
    fn a_destination_inside_its_cadence_is_not_due() {
        let now = Utc::now();
        assert!(!rss_poll_is_due(
            Some(now + Duration::seconds(1)),
            Some(0.0),
            now
        ));
        assert!(rss_poll_is_due(Some(now), Some(0.0), now));
        assert!(rss_poll_is_due(
            Some(now - Duration::seconds(1)),
            Some(0.0),
            now
        ));
    }

    #[test]
    fn a_feed_at_risk_of_having_moved_on_polls_before_its_cadence() {
        let now = Utc::now();
        let later = Some(now + Duration::minutes(10));
        assert!(!rss_poll_is_due(
            later,
            Some(RSS_FRESHNESS_ESCALATION_THRESHOLD - 0.01),
            now
        ));
        assert!(rss_poll_is_due(
            later,
            Some(RSS_FRESHNESS_ESCALATION_THRESHOLD),
            now
        ));
    }

    #[test]
    fn only_a_positive_all_deferred_answer_suppresses_a_cycle() {
        assert!(
            RssDueIndexers::unknown().poll_is_warranted(),
            "silence must never suppress a poll"
        );
        assert!(
            RssDueIndexers {
                due: vec!["indexer-a".to_string()],
                deferred: vec!["indexer-b".to_string()],
            }
            .poll_is_warranted()
        );
        assert!(
            !RssDueIndexers {
                due: Vec::new(),
                deferred: vec!["indexer-b".to_string()],
            }
            .poll_is_warranted()
        );
    }

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
