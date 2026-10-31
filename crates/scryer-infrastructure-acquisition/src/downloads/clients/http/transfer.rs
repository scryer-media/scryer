//! One file's transfer: the request, its redirects, the response checks and
//! the streaming write into the attempt's `.part` file.
//!
//! Every response is judged before a byte is written. A ranged request is
//! only continued on a `206` whose `Content-Range` starts where the partial
//! file ends, covers the representation the partial was written against,
//! and carries the same validators. Anything else starts a new attempt into
//! a fresh `.part` file; partial bytes from different representations are
//! never spliced together, and no earlier partial is truncated or removed.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use scryer_application::{AppResult, http_release_refusal};
use scryer_plugin_sdk::{PluginHttpResource, PluginHttpVerify};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::state::{HttpJobFile, PublicationCheckpoint, RecordedValidator};

/// Redirect hops followed for one request before giving up.
const MAX_REDIRECT_HOPS: usize = scryer_outbound_http::DEFAULT_TRUSTED_REDIRECT_HOPS;
/// How long the server may take to answer with headers.
const RESPONSE_HEADERS_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a body may stall between chunks.
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Bytes buffered in memory before they are handed to the operating system.
/// The buffer is flushed and synced only at a checkpoint, never per chunk.
const WRITE_BUFFER_BYTES: usize = 1024 * 1024;
/// The whole-request ceiling. A large file on a slow link takes a long time;
/// stalls are caught by the idle timeout instead.
const WHOLE_REQUEST_TIMEOUT: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Bytes written between durable checkpoints.
pub(crate) const CHECKPOINT_BYTES: u64 = 64 * 1024 * 1024;
/// The longest a transfer goes without a durable checkpoint, however slowly
/// its bytes arrive.
pub(crate) const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(30);

/// When a transfer makes its written bytes durable and records them: after
/// `bytes` more bytes or `interval` of wall time, whichever comes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CheckpointCadence {
    pub bytes: u64,
    pub interval: Duration,
}
/// Transport failures (timeouts, resets, 5xx, 429) tolerated per file.
const MAX_TRANSIENT_FAILURES: u32 = 5;
/// Fresh attempts started because a resume could not be trusted.
const MAX_RESTARTS: u32 = 3;
/// Further requests for a resource after its first `403 Forbidden`, in case
/// the refusal was spurious. A geo block answers 403 every time, so the
/// resource is given up on after this many more.
pub(crate) const MAX_FORBIDDEN_RETRIES: u32 = 2;
/// The longest `Retry-After` honored; a longer one fails the file instead of
/// parking a worker for hours.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(300);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Partial file names a fresh attempt tries before giving up, when files
/// already sit under the names it would use.
const MAX_PART_NAME_PROBES: u32 = 64;
const EGRESS_LABEL: &str = "HTTP download";

/// Headers this client owns on every request. A resource may not set them.
const CONTROLLED_HEADERS: [HeaderName; 7] = [
    header::HOST,
    header::RANGE,
    header::IF_RANGE,
    header::ACCEPT_ENCODING,
    header::CONTENT_LENGTH,
    header::TRANSFER_ENCODING,
    header::CONNECTION,
];

/// How requests leave the process.
#[derive(Clone)]
pub(crate) enum Egress {
    /// Each hop is checked under the plugin egress policy (cloud metadata
    /// and link-local destinations are refused) and dialled through a
    /// client pinned to the addresses that were checked, so the name cannot
    /// rebind between the check and the connection.
    Direct,
    /// Through the operator's assigned proxy, which resolves names itself.
    Proxied(reqwest::Client),
}

impl Egress {
    async fn client_for(&self, url: &reqwest::Url) -> Result<reqwest::Client, TransferError> {
        match self {
            Self::Direct => scryer_outbound_http::prepare_plugin_http_target_from_url(
                url.clone(),
                EGRESS_LABEL,
                // Bytes are stored and counted exactly as sent: a body the
                // server encoded despite `Accept-Encoding: identity` must not
                // be decoded into something other than the announced length.
                &scryer_outbound_http::PluginEgressPolicy::default().with_raw_bodies(),
            )
            .await
            .map(|target| target.client().clone())
            .map_err(|error| TransferError::Permanent(error.to_string())),
            Self::Proxied(client) => {
                scryer_outbound_http::validate_operator_http_url(url.as_str(), EGRESS_LABEL)
                    .map_err(|error| TransferError::Permanent(error.to_string()))?;
                Ok(client.clone())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TransferError {
    /// Worth another try: a timeout, a reset, a 5xx.
    Transient(String),
    /// The server asked to wait this long first.
    RetryAfter(Duration, String),
    /// Retrying cannot help.
    Permanent(String),
    /// An actionable refusal, persisted separately from its explanation.
    Refused(&'static str, String),
    /// `403 Forbidden`: often a geo block, sometimes a one-off refusal.
    Forbidden(String),
    /// Any other 4xx: this resource is not available to this client, though
    /// another of the release's resources may be.
    Rejected(String),
    Cancelled,
}

impl TransferError {
    pub(crate) fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Refused(reason, _) => Some(reason),
            _ => None,
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Transient(message)
            | Self::RetryAfter(_, message)
            | Self::Permanent(message)
            | Self::Refused(_, message)
            | Self::Forbidden(message)
            | Self::Rejected(message) => message.clone(),
            Self::Cancelled => String::new(),
        }
    }
}

/// What was asked of the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RangeRequest {
    Whole,
    From {
        offset: u64,
        total: u64,
        validator: RecordedValidator,
    },
}

/// What a response means for the partial file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResponseVerdict {
    /// The whole representation, from byte zero.
    Fresh {
        total: Option<u64>,
        validator: Option<RecordedValidator>,
    },
    /// The requested continuation of the partial file.
    Continue,
    /// The partial cannot be continued; start a new attempt.
    Restart(String),
    Failed(TransferError),
}

/// Judge a response before any of its body is written.
pub(crate) fn judge_response(
    status: StatusCode,
    headers: &HeaderMap,
    request: &RangeRequest,
) -> ResponseVerdict {
    if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE {
        let message = format!("the server answered {status}");
        return match headers
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| parse_retry_after(value, chrono::Utc::now()))
        {
            Some(wait) if wait <= MAX_RETRY_AFTER => {
                ResponseVerdict::Failed(TransferError::RetryAfter(wait, message))
            }
            Some(wait) => ResponseVerdict::Failed(TransferError::Permanent(format!(
                "{message} and asked to wait {}s, longer than the client waits",
                wait.as_secs()
            ))),
            None => ResponseVerdict::Failed(TransferError::Transient(message)),
        };
    }
    if status == StatusCode::RANGE_NOT_SATISFIABLE {
        return match request {
            RangeRequest::From { .. } => {
                ResponseVerdict::Restart("the server refused the resume range (416)".into())
            }
            RangeRequest::Whole => ResponseVerdict::Failed(TransferError::Permanent(
                "the server answered 416 to a whole-file request".into(),
            )),
        };
    }
    if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT {
        return ResponseVerdict::Failed(TransferError::Transient(format!(
            "the server answered {status}"
        )));
    }
    if status == StatusCode::FORBIDDEN {
        return ResponseVerdict::Failed(TransferError::Forbidden(format!(
            "the server answered {status}"
        )));
    }
    if status.is_client_error() {
        return ResponseVerdict::Failed(TransferError::Rejected(format!(
            "the server answered {status}"
        )));
    }
    if !status.is_success() {
        return ResponseVerdict::Failed(TransferError::Permanent(format!(
            "the server answered {status}"
        )));
    }

    let content_length = header_u64(headers, header::CONTENT_LENGTH);
    let response_validator = response_validator(headers);
    match (status, request) {
        (StatusCode::OK, RangeRequest::Whole) => ResponseVerdict::Fresh {
            total: content_length,
            validator: response_validator,
        },
        // The server ignored the range or the representation changed under
        // `If-Range`: it is sending the whole thing again.
        (StatusCode::OK, RangeRequest::From { .. }) => {
            ResponseVerdict::Restart("the server answered the resume with the whole file".into())
        }
        (
            StatusCode::PARTIAL_CONTENT,
            RangeRequest::From {
                offset,
                total,
                validator,
            },
        ) => {
            let Some(range) = headers
                .get(header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range)
            else {
                return ResponseVerdict::Restart(
                    "the resume answer has no usable Content-Range".into(),
                );
            };
            if range.start != *offset
                || range.complete_length != Some(*total)
                || range.end.checked_add(1) != Some(*total)
            {
                return ResponseVerdict::Restart(format!(
                    "the resume answer covers bytes {}-{}/{} instead of {offset}-{}/{total}",
                    range.start,
                    range.end,
                    range
                        .complete_length
                        .map_or_else(|| "*".to_string(), |length| length.to_string()),
                    total.saturating_sub(1)
                ));
            }
            if let Some(length) = content_length
                && Some(length) != (range.end + 1).checked_sub(range.start)
            {
                return ResponseVerdict::Restart(
                    "the resume answer's Content-Length disagrees with its Content-Range".into(),
                );
            }
            if validator_changed(validator, response_validator.as_ref()) {
                return ResponseVerdict::Restart(
                    "the file changed on the server since the partial was written".into(),
                );
            }
            ResponseVerdict::Continue
        }
        (StatusCode::PARTIAL_CONTENT, RangeRequest::Whole) => ResponseVerdict::Failed(
            TransferError::Transient("the server answered a whole-file request with 206".into()),
        ),
        _ => ResponseVerdict::Failed(TransferError::Permanent(format!(
            "the server answered {status}"
        ))),
    }
}

/// A validator change is only provable when both sides state the same kind.
fn validator_changed(recorded: &RecordedValidator, response: Option<&RecordedValidator>) -> bool {
    let Some(response) = response else {
        return false;
    };
    if let (Some(recorded), Some(current)) = (&recorded.etag, &response.etag) {
        return recorded != current;
    }
    if let (Some(recorded), Some(current)) = (&recorded.last_modified, &response.last_modified) {
        return recorded != current;
    }
    false
}

/// The strong validators a response carries; weak entity tags are dropped.
fn response_validator(headers: &HeaderMap) -> Option<RecordedValidator> {
    let etag = headers
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.starts_with("W/"))
        .map(str::to_string);
    let last_modified = headers
        .get(header::LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    (etag.is_some() || last_modified.is_some()).then_some(RecordedValidator {
        etag,
        last_modified,
    })
}

fn header_u64(headers: &HeaderMap, name: HeaderName) -> Option<u64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentRange {
    pub start: u64,
    pub end: u64,
    pub complete_length: Option<u64>,
}

/// `bytes <start>-<end>/<length or *>`.
pub(crate) fn parse_content_range(raw: &str) -> Option<ContentRange> {
    let rest = raw.trim().strip_prefix("bytes")?.trim_start();
    let (range, length) = rest.split_once('/')?;
    let (start, end) = range.trim().split_once('-')?;
    let start = start.trim().parse::<u64>().ok()?;
    let end = end.trim().parse::<u64>().ok()?;
    if end < start {
        return None;
    }
    let complete_length = match length.trim() {
        "*" => None,
        value => Some(value.parse::<u64>().ok()?),
    };
    if complete_length.is_some_and(|length| end >= length) {
        return None;
    }
    Some(ContentRange {
        start,
        end,
        complete_length,
    })
}

/// Delay-seconds or an HTTP date, relative to `now`.
pub(crate) fn parse_retry_after(raw: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
    let raw = raw.trim();
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = chrono::DateTime::parse_from_rfc2822(raw).ok()?;
    let wait = at.with_timezone(&chrono::Utc) - now;
    Some(wait.to_std().unwrap_or(Duration::ZERO))
}

/// The resource's own headers, but only for a request to the resource's own
/// origin. A redirect to another origin carries none of them.
pub(crate) fn scoped_headers(
    resource: &PluginHttpResource,
    resource_url: &reqwest::Url,
    request_url: &reqwest::Url,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if resource_url.origin() != request_url.origin() {
        return headers;
    }
    for entry in &resource.headers {
        let Ok(name) = HeaderName::from_bytes(entry.name.trim().as_bytes()) else {
            warn!(
                header = entry.name.as_str(),
                "skipping an invalid resource header name"
            );
            continue;
        };
        if CONTROLLED_HEADERS.contains(&name) {
            continue;
        }
        let Ok(value) = HeaderValue::from_str(&entry.value) else {
            warn!(
                header = entry.name.as_str(),
                "skipping a resource header with an invalid value"
            );
            continue;
        };
        headers.append(name, value);
    }
    headers
}

/// Receives each durable change to a file while it transfers.
#[async_trait]
pub(crate) trait Checkpoint: Send + Sync {
    async fn record(&self, file: &HttpJobFile) -> AppResult<()>;

    /// Record a `403 Forbidden` answer: the file carries its own count, and
    /// the job counts them across every resource it tries.
    async fn record_forbidden(&self, file: &HttpJobFile) -> AppResult<()> {
        self.record(file).await
    }
}

/// Shared pacing for every transfer of one client.
pub(crate) struct BandwidthLimiter {
    bytes_per_second: AtomicU64,
    next_free: std::sync::Mutex<tokio::time::Instant>,
    /// Wakes every pending wait when the rate changes, so it is paced by the
    /// new rate instead of sleeping out a delay the old one set.
    rate_changed: tokio::sync::Notify,
}

/// A paced wait was cut short because the transfer was cancelled.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PacingCancelled;

impl BandwidthLimiter {
    pub(crate) fn new(bytes_per_second: u64) -> Self {
        Self {
            bytes_per_second: AtomicU64::new(bytes_per_second),
            next_free: std::sync::Mutex::new(tokio::time::Instant::now()),
            rate_changed: tokio::sync::Notify::new(),
        }
    }

    pub(crate) fn set_rate(&self, bytes_per_second: u64) {
        let previous = self
            .bytes_per_second
            .swap(bytes_per_second, Ordering::Relaxed);
        if previous == bytes_per_second {
            return;
        }
        // Reservations made at the old rate no longer hold; each pending
        // wait reserves again from now at the new one.
        *self.lock_next_free() = tokio::time::Instant::now();
        self.rate_changed.notify_waiters();
    }

    fn lock_next_free(&self) -> std::sync::MutexGuard<'_, tokio::time::Instant> {
        self.next_free
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Wait until `bytes` more may be sent, or until `cancel` fires.
    pub(crate) async fn consume(
        &self,
        bytes: u64,
        cancel: &CancellationToken,
    ) -> Result<(), PacingCancelled> {
        loop {
            let rate_changed = self.rate_changed.notified();
            tokio::pin!(rate_changed);
            // Registered before the rate is read, so a change made after
            // this point always wakes the wait below.
            rate_changed.as_mut().enable();
            let rate = self.bytes_per_second.load(Ordering::Relaxed);
            if rate == 0 || bytes == 0 {
                return Ok(());
            }
            let (start, end, wait) = {
                let mut next_free = self.lock_next_free();
                let now = tokio::time::Instant::now();
                let start = (*next_free).max(now);
                let end = start + Duration::from_secs_f64(bytes as f64 / rate as f64);
                *next_free = end;
                (start, end, start.saturating_duration_since(now))
            };
            if wait.is_zero() {
                return Ok(());
            }
            tokio::select! {
                () = cancel.cancelled() => {
                    // Hand back the slot when nothing was reserved after it.
                    let mut next_free = self.lock_next_free();
                    if *next_free == end {
                        *next_free = start;
                    }
                    return Err(PacingCancelled);
                }
                () = tokio::time::sleep(wait) => return Ok(()),
                () = &mut rate_changed => {}
            }
        }
    }
}

/// A failed request, described without its URL. reqwest names the request
/// URL in its errors, and a CDN URL carries signed query tokens that must not
/// reach the queue document, the logs or the UI.
pub(crate) fn describe_request_error(prefix: &str, error: reqwest::Error) -> String {
    format!("{prefix}: {}", error.without_url())
}

/// What the probe could tell about a finished media file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MediaProbeOutcome {
    /// The container was read. `None` means that fact stayed unknown.
    Probed {
        duration_seconds: Option<u64>,
        has_video: Option<bool>,
        /// The container format the analyzer recognized, e.g. `matroska`.
        container: Option<String>,
    },
    /// The container could not be read at all (an unsupported format, for
    /// one); nothing about it can be checked.
    Unverifiable(String),
}

/// Reads the facts a finished media file is checked against.
#[async_trait]
pub(crate) trait MediaProbe: Send + Sync {
    async fn probe(&self, path: &Path) -> MediaProbeOutcome;
}

/// The host's pure-Rust media analyzer, the one imports and library scans
/// use.
pub(crate) struct NativeMediaProbe;

#[async_trait]
impl MediaProbe for NativeMediaProbe {
    async fn probe(&self, path: &Path) -> MediaProbeOutcome {
        use scryer_application::{MediaAnalysisOutcome, MediaAnalyzer};
        let positive = |seconds: Option<i32>| {
            seconds
                .and_then(|seconds| u64::try_from(seconds).ok())
                .filter(|seconds| *seconds > 0)
        };
        match scryer_application::NativeMediaAnalyzer
            .analyze_file(path.to_path_buf())
            .await
        {
            Ok(MediaAnalysisOutcome::Valid(analysis)) => MediaProbeOutcome::Probed {
                duration_seconds: positive(analysis.duration_seconds),
                has_video: Some(true),
                container: analysis.container_format.clone(),
            },
            Ok(MediaAnalysisOutcome::Inconclusive(analysis)) => MediaProbeOutcome::Probed {
                duration_seconds: positive(analysis.duration_seconds),
                // An incomplete read that found no video proves nothing.
                has_video: (analysis.video_codec.is_some()
                    || analysis.details.selected_video_id.is_some())
                .then_some(true),
                container: analysis.container_format.clone(),
            },
            Ok(MediaAnalysisOutcome::NoVideo) => MediaProbeOutcome::Probed {
                duration_seconds: None,
                has_video: Some(false),
                container: None,
            },
            Ok(MediaAnalysisOutcome::Invalid(reason)) => MediaProbeOutcome::Unverifiable(reason),
            Err(error) => MediaProbeOutcome::Unverifiable(error.to_string()),
        }
    }
}

/// How the media file of a job is checked once its bytes are all written.
pub(crate) struct MediaCheck<'a> {
    pub probe: &'a dyn MediaProbe,
    pub expected_duration_seconds: Option<u64>,
    pub verify: PluginHttpVerify,
}

/// Judge a probed media file against the job's expectations. Facts the probe
/// could not establish are not held against the file.
pub(crate) fn judge_media(
    outcome: &MediaProbeOutcome,
    expected_duration_seconds: Option<u64>,
    verify: &PluginHttpVerify,
) -> Result<(), TransferError> {
    let MediaProbeOutcome::Probed {
        duration_seconds,
        has_video,
        ..
    } = outcome
    else {
        return Ok(());
    };
    if verify.require_video && *has_video == Some(false) {
        return Err(TransferError::Permanent(
            "the finished download has no video track".into(),
        ));
    }
    let ratio = f64::from(verify.min_duration_ratio);
    if let (Some(expected), Some(actual)) = (expected_duration_seconds, *duration_seconds)
        && expected > 0
        && ratio.is_finite()
        && ratio > 0.0
    {
        let minimum = ratio.min(1.0) * expected as f64;
        if (actual as f64) < minimum {
            return Err(TransferError::Permanent(format!(
                "the finished download runs {actual}s but {expected}s were expected \
                 (at least {}s required)",
                minimum.ceil() as u64
            )));
        }
    }
    Ok(())
}

async fn verify_media(
    check: &MediaCheck<'_>,
    path: &Path,
) -> Result<MediaProbeOutcome, TransferError> {
    let outcome = check.probe.probe(path).await;
    match &outcome {
        MediaProbeOutcome::Unverifiable(reason) => debug!(
            path = %path.display(),
            reason = reason.as_str(),
            "HTTP download media could not be probed; leaving it unverified"
        ),
        MediaProbeOutcome::Probed {
            duration_seconds: None,
            ..
        } if check.expected_duration_seconds.is_some() => debug!(
            path = %path.display(),
            "HTTP download media has no readable duration; its running time is unverified"
        ),
        MediaProbeOutcome::Probed { .. } => {}
    }
    judge_media(&outcome, check.expected_duration_seconds, &check.verify)?;
    Ok(outcome)
}

/// The video extension for a container the analyzer names.
fn container_extension(container: &str) -> Option<&'static str> {
    match container.to_ascii_lowercase().as_str() {
        "matroska" | "mkv" => Some("mkv"),
        "webm" => Some("webm"),
        "mp4" => Some("mp4"),
        "mov" => Some("mov"),
        "mpegts" | "ts" => Some("ts"),
        "m2ts" => Some("m2ts"),
        "avi" => Some("avi"),
        "flv" => Some("flv"),
        "ogg" => Some("ogv"),
        "asf" | "wmv" => Some("wmv"),
        _ => None,
    }
}

/// The final name for a media file whose name does not say it is video
/// (no container was stated and the URL had no usable extension), from the
/// container the probe read. A name that already carries a video extension
/// is kept.
pub(crate) fn media_path_for_probed_container(
    final_path: &Path,
    outcome: &MediaProbeOutcome,
) -> Option<std::path::PathBuf> {
    if scryer_domain::canonical_video_extension(final_path).is_some() {
        return None;
    }
    let MediaProbeOutcome::Probed {
        container: Some(container),
        ..
    } = outcome
    else {
        return None;
    };
    container_extension(container).map(|extension| final_path.with_extension(extension))
}

/// The current time, injectable so tests decide when a URL has expired.
pub(crate) type Clock = Arc<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync>;

pub(crate) fn system_clock() -> Clock {
    Arc::new(chrono::Utc::now)
}

/// Why a file whose signed URL ran out fails: asking again cannot help, and
/// another variant of the same release was signed the same way.
const EXPIRED_URL: &str = "the resource URL has expired; grab the release again";

/// The resource states an expiry and it has passed.
fn has_expired(resource: &PluginHttpResource, now: chrono::DateTime<chrono::Utc>) -> bool {
    resource
        .expires_at
        .as_deref()
        .and_then(|expires_at| chrono::DateTime::parse_from_rfc3339(expires_at).ok())
        .is_some_and(|expires_at| expires_at.with_timezone(&chrono::Utc) <= now)
}

/// Permits for actual request hosts, shared by every file in an engine.
pub(crate) struct HostLimiter {
    state: std::sync::Mutex<(usize, std::collections::HashMap<String, usize>)>,
    changed: tokio::sync::Notify,
    #[cfg(test)]
    pub(crate) blocked: tokio::sync::Notify,
}

impl HostLimiter {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            state: std::sync::Mutex::new((limit.max(1), Default::default())),
            changed: tokio::sync::Notify::new(),
            #[cfg(test)]
            blocked: tokio::sync::Notify::new(),
        }
    }

    pub(crate) fn set_limit(&self, limit: usize) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).0 = limit.max(1);
        self.changed.notify_waiters();
    }

    pub(crate) async fn acquire(
        &self,
        url: &reqwest::Url,
        cancel: &CancellationToken,
    ) -> Result<HostPermit<'_>, TransferError> {
        let host = url
            .host_str()
            .unwrap_or_default()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if cancel.is_cancelled() {
                return Err(TransferError::Cancelled);
            }
            {
                let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                let limit = state.0;
                if state.1.get(&host).copied().unwrap_or(0) < limit {
                    *state.1.entry(host.clone()).or_default() += 1;
                    return Ok(HostPermit {
                        limiter: self,
                        host,
                    });
                }
            }
            #[cfg(test)]
            self.blocked.notify_one();
            tokio::select! {
                () = cancel.cancelled() => return Err(TransferError::Cancelled),
                () = &mut changed => {}
            }
        }
    }
}

pub(crate) struct HostPermit<'a> {
    limiter: &'a HostLimiter,
    host: String,
}

impl Drop for HostPermit<'_> {
    fn drop(&mut self) {
        let mut state = self.limiter.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(count) = state.1.get_mut(&self.host) {
            *count -= 1;
            if *count == 0 {
                state.1.remove(&self.host);
            }
        }
        drop(state);
        self.limiter.changed.notify_waiters();
    }
}

/// Everything one file transfer needs from its job and engine.
pub(crate) struct TransferContext<'a> {
    pub egress: Egress,
    pub checkpoint: &'a dyn Checkpoint,
    pub cancel: &'a CancellationToken,
    pub live_bytes: &'a Arc<AtomicU64>,
    pub limiter: Option<&'a BandwidthLimiter>,
    pub host_limiter: Option<&'a HostLimiter>,
    /// The first retry's wait; each further retry doubles it, up to a cap.
    pub backoff_unit: Duration,
    pub cadence: CheckpointCadence,
    pub clock: Clock,
    /// Set for the job's media file only; sidecars are not probed.
    pub media_check: Option<MediaCheck<'a>>,
}

/// The wait before retry number `failures` (1-based).
fn backoff(unit: Duration, failures: u32) -> Duration {
    unit.saturating_mul(1u32 << failures.min(6))
        .min(MAX_BACKOFF)
}

/// Fetch one file to completion, resuming its partial when that is safe.
///
/// On success the bytes sit at `final_path` and `file.complete` is set. A
/// cancellation checkpoints what was written and returns `Cancelled`.
pub(crate) async fn transfer_file(
    context: &TransferContext<'_>,
    file: &mut HttpJobFile,
) -> Result<(), TransferError> {
    if file.complete {
        return Ok(());
    }
    recover_legacy_partial(context, file).await?;
    if already_in_place(context, file).await? {
        // Moved into place before an interruption kept the move from being
        // recorded: only the final checks are left, and nothing is fetched.
        if let Some(check) = &context.media_check {
            verify_media(check, &file.final_path).await?;
        }
        file.complete = true;
        info!(
            path = %file.final_path.display(),
            "HTTP download found its finished file already in place"
        );
        return record(context, file).await;
    }
    // Recorded before an interruption: the resource already used up its
    // retries, so it is not asked again.
    if file.forbidden_answers > MAX_FORBIDDEN_RETRIES {
        return Err(TransferError::Forbidden(format!(
            "the server answered {} to all {} requests",
            StatusCode::FORBIDDEN,
            file.forbidden_answers
        )));
    }
    if has_expired(&file.resource, (context.clock)()) {
        return Err(TransferError::Permanent(EXPIRED_URL.into()));
    }
    let resource_url = reqwest::Url::parse(&file.resource.url)
        .map_err(|error| TransferError::Permanent(format!("invalid resource URL: {error}")))?;

    let mut transient_failures = 0u32;
    let mut restarts = 0u32;
    loop {
        if context.cancel.is_cancelled() {
            return Err(TransferError::Cancelled);
        }
        let part_len = existing_len(&file.part_path()).await?;

        // A partial whose every byte is already recorded only needs its
        // final checks.
        if let Some(total) = file.total_bytes
            && file.bytes_done == total
            && part_len == total
            && total > 0
        {
            return finish_file(context, file).await;
        }

        let range = resume_range(file, part_len);
        if range == RangeRequest::Whole && (file.bytes_done > 0 || part_len > 0) {
            start_new_attempt(file);
            record(context, file).await?;
        }

        let (response, permit) =
            match open_resource(context, &file.resource, &resource_url, &range).await {
                Ok(response) => response,
                Err(TransferError::Cancelled) => return Err(TransferError::Cancelled),
                Err(TransferError::Permanent(message)) => {
                    return Err(TransferError::Permanent(message));
                }
                Err(error) => {
                    transient_failures += 1;
                    wait_before_retry(context, &error, transient_failures).await?;
                    continue;
                }
            };

        let offset = match judge_response(response.status(), response.headers(), &range) {
            ResponseVerdict::Fresh { total, validator } => {
                file.total_bytes = total;
                file.validator = validator;
                file.bytes_done = 0;
                record(context, file).await?;
                0
            }
            ResponseVerdict::Continue => file.bytes_done,
            ResponseVerdict::Restart(reason) => {
                restarts += 1;
                info!(
                    url_host = resource_url.host_str().unwrap_or_default(),
                    attempt = file.attempt,
                    reason = reason.as_str(),
                    "HTTP download starting a new attempt"
                );
                if restarts > MAX_RESTARTS {
                    return Err(TransferError::Permanent(format!(
                        "gave up after {MAX_RESTARTS} fresh attempts: {reason}"
                    )));
                }
                start_new_attempt(file);
                record(context, file).await?;
                continue;
            }
            ResponseVerdict::Failed(TransferError::Permanent(message)) => {
                return Err(TransferError::Permanent(message));
            }
            // A refusal after the URL expired is the expiry, not the
            // resource: it is neither retried nor counted against the
            // variant, and no other variant is tried.
            ResponseVerdict::Failed(TransferError::Rejected(_) | TransferError::Forbidden(_))
                if has_expired(&file.resource, (context.clock)()) =>
            {
                return Err(TransferError::Permanent(EXPIRED_URL.into()));
            }
            ResponseVerdict::Failed(TransferError::Rejected(message)) => {
                return Err(TransferError::Rejected(message));
            }
            ResponseVerdict::Failed(TransferError::Forbidden(message)) => {
                drop(response);
                drop(permit);
                file.forbidden_answers = file.forbidden_answers.saturating_add(1);
                context
                    .checkpoint
                    .record_forbidden(file)
                    .await
                    .map_err(|error| TransferError::Permanent(error.to_string()))?;
                wait_after_forbidden(context, file, message).await?;
                continue;
            }
            ResponseVerdict::Failed(error) => {
                drop(response);
                drop(permit);
                transient_failures += 1;
                wait_before_retry(context, &error, transient_failures).await?;
                continue;
            }
        };

        let streamed = stream_body(context, file, response, offset).await;
        drop(permit);
        match streamed {
            Ok(()) => return finish_file(context, file).await,
            Err(StreamError::Cancelled) => return Err(TransferError::Cancelled),
            Err(StreamError::Permanent(message)) => return Err(TransferError::Permanent(message)),
            Err(StreamError::Restart(reason)) => {
                restarts += 1;
                if restarts > MAX_RESTARTS {
                    return Err(TransferError::Permanent(format!(
                        "gave up after {MAX_RESTARTS} fresh attempts: {reason}"
                    )));
                }
                start_new_attempt(file);
                record(context, file).await?;
            }
            Err(StreamError::Transient(message)) => {
                transient_failures += 1;
                wait_before_retry(
                    context,
                    &TransferError::Transient(message),
                    transient_failures,
                )
                .await?;
            }
        }
    }
}

/// Older records derived the partial from the final name, even after a probe
/// changed its extension. Recover only one unambiguous recorded attempt.
async fn recover_legacy_partial(
    context: &TransferContext<'_>,
    file: &mut HttpJobFile,
) -> Result<(), TransferError> {
    if file.partial_path.is_some()
        || file.publication.is_some()
        || file.bytes_done == 0
        || file.role != super::state::HttpFileRole::Media
    {
        return Ok(());
    }
    let extension =
        super::selection::extension_for(&file.resource, None).unwrap_or_else(|| "bin".into());
    let original_final = file.final_path.with_extension(extension);
    if original_final == file.final_path {
        return Ok(());
    }
    let original_part = super::state::part_path_for(&original_final, file.attempt);
    let metadata = match tokio::fs::symlink_metadata(&original_part).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(TransferError::Refused(
                http_release_refusal::RECOVERY_UNVERIFIED,
                error.to_string(),
            ));
        }
    };
    if !metadata.is_file()
        || metadata.len() < file.bytes_done
        || tokio::fs::try_exists(file.part_path())
            .await
            .unwrap_or(true)
    {
        return Err(TransferError::Refused(
            http_release_refusal::RECOVERY_UNVERIFIED,
            "the legacy partial path is ambiguous; all files were preserved".into(),
        ));
    }
    file.partial_path = Some(original_part);
    record(context, file).await
}

/// Recover only the exact output whose identity was durably recorded before
/// publication. Legacy final-only files are preserved for operator attention.
async fn already_in_place(
    context: &TransferContext<'_>,
    file: &HttpJobFile,
) -> Result<bool, TransferError> {
    if tokio::fs::try_exists(file.part_path())
        .await
        .unwrap_or(true)
    {
        return Ok(false);
    }
    match tokio::fs::symlink_metadata(&file.final_path).await {
        Ok(metadata) => {
            if let Some(proof) = &file.publication
                && proof.attempt == file.attempt
                && proof.partial_path == file.part_path()
                && proof.final_path == file.final_path
                && file.total_bytes == Some(proof.size)
                && file.bytes_done == proof.size
                && metadata.is_file()
                && metadata.len() == proof.size
                && hash_file(context, &file.final_path, ChecksumAlgorithm::Sha256).await?
                    == proof.sha256
            {
                return Ok(true);
            }
            Err(TransferError::Refused(
                http_release_refusal::RECOVERY_UNVERIFIED,
                "the existing output could not be verified against this download; it was preserved"
                    .into(),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(TransferError::Permanent(format!(
            "cannot inspect {}: {error}",
            file.final_path.display()
        ))),
    }
}

/// The range to ask for: a continuation only when the partial is known to
/// hold the recorded bytes of a representation with a known length and a
/// strong validator, and the indexer has not said the server ignores ranges.
pub(crate) fn resume_range(file: &HttpJobFile, part_len: u64) -> RangeRequest {
    if file.bytes_done == 0
        || file.resource.range_support == scryer_plugin_sdk::HttpRangeSupport::KnownNo
    {
        return RangeRequest::Whole;
    }
    match (file.total_bytes, file.validator.as_ref()) {
        (Some(total), Some(validator))
            if validator.if_range_value().is_some()
                && file.bytes_done < total
                && part_len >= file.bytes_done
                && part_len <= total =>
        {
            RangeRequest::From {
                offset: file.bytes_done,
                total,
                validator: validator.clone(),
            }
        }
        _ => RangeRequest::Whole,
    }
}

/// Leave the current partial where it is and write the next attempt to a
/// new `.part` file.
pub(crate) fn start_new_attempt(file: &mut HttpJobFile) {
    file.attempt = file.attempt.saturating_add(1);
    file.partial_path = None;
    file.publication = None;
    file.bytes_done = 0;
    file.total_bytes = None;
    file.validator = None;
}

async fn record(context: &TransferContext<'_>, file: &HttpJobFile) -> Result<(), TransferError> {
    context
        .checkpoint
        .record(file)
        .await
        .map_err(|error| TransferError::Permanent(error.to_string()))
}

async fn existing_len(path: &Path) -> Result<u64, TransferError> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(TransferError::Permanent(format!(
            "cannot inspect {}: {error}",
            path.display()
        ))),
    }
}

async fn wait_before_retry(
    context: &TransferContext<'_>,
    error: &TransferError,
    failures: u32,
) -> Result<(), TransferError> {
    let (wait, message) = match error {
        TransferError::RetryAfter(wait, message) => (*wait, message.clone()),
        TransferError::Transient(message) => {
            (backoff(context.backoff_unit, failures), message.clone())
        }
        other => return Err(other.clone()),
    };
    if failures > MAX_TRANSIENT_FAILURES {
        return Err(TransferError::Permanent(format!(
            "gave up after {MAX_TRANSIENT_FAILURES} retries: {message}"
        )));
    }
    warn!(
        retry_in_secs = wait.as_secs(),
        failures,
        error = message.as_str(),
        "HTTP download will retry"
    );
    sleep_unless_cancelled(context, wait).await
}

/// After a `403 Forbidden`: give the resource up once its retries are spent,
/// otherwise wait the usual backoff before asking again.
async fn wait_after_forbidden(
    context: &TransferContext<'_>,
    file: &HttpJobFile,
    message: String,
) -> Result<(), TransferError> {
    let answers = file.forbidden_answers;
    if answers > MAX_FORBIDDEN_RETRIES {
        return Err(TransferError::Forbidden(format!(
            "{message} to all {answers} requests"
        )));
    }
    let wait = backoff(context.backoff_unit, answers);
    warn!(
        url_host = reqwest::Url::parse(&file.resource.url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_default(),
        retry_in_secs = wait.as_secs(),
        forbidden_answers = answers,
        "HTTP download was refused with 403 Forbidden; asking again"
    );
    sleep_unless_cancelled(context, wait).await
}

async fn sleep_unless_cancelled(
    context: &TransferContext<'_>,
    wait: Duration,
) -> Result<(), TransferError> {
    if wait.is_zero() {
        return if context.cancel.is_cancelled() {
            Err(TransferError::Cancelled)
        } else {
            Ok(())
        };
    }
    tokio::select! {
        () = context.cancel.cancelled() => Err(TransferError::Cancelled),
        () = tokio::time::sleep(wait) => Ok(()),
    }
}

/// Send the request, following redirects by hand so every hop is checked
/// and the resource's headers stay on its own origin.
async fn open_resource<'a>(
    context: &'a TransferContext<'_>,
    resource: &PluginHttpResource,
    resource_url: &reqwest::Url,
    range: &RangeRequest,
) -> Result<(reqwest::Response, Option<HostPermit<'a>>), TransferError> {
    let mut url = resource_url.clone();
    for _ in 0..=MAX_REDIRECT_HOPS {
        let permit = match context.host_limiter {
            Some(limiter) => Some(limiter.acquire(&url, context.cancel).await?),
            None => None,
        };
        let client = tokio::select! {
            () = context.cancel.cancelled() => return Err(TransferError::Cancelled),
            client = context.egress.client_for(&url) => client?,
        };
        let mut request = client
            .get(url.clone())
            .headers(scoped_headers(resource, resource_url, &url))
            .header(header::ACCEPT_ENCODING, "identity")
            .timeout(WHOLE_REQUEST_TIMEOUT);
        if let RangeRequest::From {
            offset, validator, ..
        } = range
        {
            request = request.header(header::RANGE, format!("bytes={offset}-"));
            if let Some(value) = validator.if_range_value() {
                request = request.header(header::IF_RANGE, value);
            }
        }
        let response = tokio::select! {
            () = context.cancel.cancelled() => return Err(TransferError::Cancelled),
            result = tokio::time::timeout(RESPONSE_HEADERS_TIMEOUT, request.send()) => match result {
                Err(_) => return Err(TransferError::Transient("the server did not answer in time".into())),
                Ok(Err(error)) => return Err(TransferError::Transient(describe_request_error("request failed", error))),
                Ok(Ok(response)) => response,
            },
        };
        if !response.status().is_redirection() || response.status() == StatusCode::NOT_MODIFIED {
            return Ok((response, permit));
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                TransferError::Permanent(format!(
                    "the server answered {} without a location",
                    response.status()
                ))
            })?;
        let next = url.join(location).map_err(|error| {
            TransferError::Permanent(format!("the redirect location is invalid: {error}"))
        })?;
        if url.scheme() == "https" && next.scheme() != "https" {
            return Err(TransferError::Permanent(
                "refusing a redirect from https to an insecure URL".into(),
            ));
        }
        url = next;
    }
    Err(TransferError::Permanent(format!(
        "more than {MAX_REDIRECT_HOPS} redirects"
    )))
}

enum StreamError {
    Transient(String),
    Restart(String),
    Permanent(String),
    Cancelled,
}

fn disk_error(path: &Path, error: &std::io::Error) -> StreamError {
    if error.kind() == std::io::ErrorKind::StorageFull {
        StreamError::Permanent(format!("the disk is full writing {}", path.display()))
    } else {
        StreamError::Permanent(format!("writing {} failed: {error}", path.display()))
    }
}

/// Write the body into the attempt's `.part` file from `offset`, recording
/// progress only after the bytes are flushed and synced to disk.
///
/// Writes go through a 1 MiB buffer. `live_bytes` counts what reached the
/// buffer, for the queue's live view; `bytes_done` advances only after a
/// checkpoint has flushed and synced it, so a crash resumes from bytes that
/// are known to be on disk.
async fn stream_body(
    context: &TransferContext<'_>,
    file: &mut HttpJobFile,
    mut response: reqwest::Response,
    offset: u64,
) -> Result<(), StreamError> {
    let part_path = file.part_path();
    if let Some(parent) = part_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| disk_error(parent, &error))?;
    }
    let mut opened = if offset == 0 {
        open_fresh_part(context, file).await?
    } else {
        // A continuation writes on from the recorded offset of the partial
        // it was judged against; it never creates or truncates one.
        tokio::fs::OpenOptions::new()
            .write(true)
            .open(&part_path)
            .await
            .map_err(|error| disk_error(&part_path, &error))?
    };
    let part_path = file.part_path();
    opened
        .seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|error| disk_error(&part_path, &error))?;
    let mut output = BufWriter::with_capacity(WRITE_BUFFER_BYTES, opened);

    let mut written = offset;
    let mut unrecorded = 0u64;
    let mut last_checkpoint = tokio::time::Instant::now();
    context.live_bytes.store(written, Ordering::Relaxed);
    loop {
        let chunk = tokio::select! {
            () = context.cancel.cancelled() => {
                checkpoint_written(context, file, &mut output, &part_path, written).await?;
                return Err(StreamError::Cancelled);
            }
            next = tokio::time::timeout(BODY_IDLE_TIMEOUT, response.chunk()) => match next {
                Err(_) => {
                    checkpoint_written(context, file, &mut output, &part_path, written).await?;
                    return Err(StreamError::Transient("the transfer stalled".into()));
                }
                Ok(Err(error)) => {
                    checkpoint_written(context, file, &mut output, &part_path, written).await?;
                    return Err(StreamError::Transient(describe_request_error("the transfer broke off", error)));
                }
                Ok(Ok(chunk)) => chunk,
            },
        };
        let Some(chunk) = chunk else {
            break;
        };
        let length = chunk.len() as u64;
        if let Some(total) = file.total_bytes
            && written + length > total
        {
            return Err(StreamError::Restart(format!(
                "the server sent more than the {total} bytes it announced"
            )));
        }
        if let Some(limiter) = context.limiter
            && limiter.consume(length, context.cancel).await.is_err()
        {
            checkpoint_written(context, file, &mut output, &part_path, written).await?;
            return Err(StreamError::Cancelled);
        }
        output
            .write_all(&chunk)
            .await
            .map_err(|error| disk_error(&part_path, &error))?;
        written += length;
        unrecorded += length;
        context.live_bytes.store(written, Ordering::Relaxed);
        if unrecorded >= context.cadence.bytes
            || last_checkpoint.elapsed() >= context.cadence.interval
        {
            checkpoint_written(context, file, &mut output, &part_path, written).await?;
            unrecorded = 0;
            last_checkpoint = tokio::time::Instant::now();
        }
    }

    checkpoint_written(context, file, &mut output, &part_path, written).await?;
    match file.total_bytes {
        Some(total) if written < total => Err(StreamError::Transient(format!(
            "the transfer ended after {written} of {total} bytes"
        ))),
        Some(_) => Ok(()),
        None => {
            // No stated length: what arrived is the representation. It cannot
            // be resumed, so a broken body would have restarted instead.
            file.total_bytes = Some(written);
            Ok(())
        }
    }
}

/// Create the partial a fresh attempt writes. Only a name no file holds is
/// used: an existing file under the attempt's name (an earlier attempt's
/// partial, or anything else) is left as it is and the next name is tried.
async fn open_fresh_part(
    context: &TransferContext<'_>,
    file: &mut HttpJobFile,
) -> Result<tokio::fs::File, StreamError> {
    for _ in 0..MAX_PART_NAME_PROBES {
        let part_path = file.part_path();
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part_path)
            .await
        {
            Ok(opened) => {
                file.partial_path = Some(part_path);
                context
                    .checkpoint
                    .record(file)
                    .await
                    .map_err(|error| StreamError::Permanent(error.to_string()))?;
                return Ok(opened);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                file.attempt = file.attempt.saturating_add(1);
                file.partial_path = None;
                file.publication = None;
                context
                    .checkpoint
                    .record(file)
                    .await
                    .map_err(|error| StreamError::Permanent(error.to_string()))?;
            }
            Err(error) => return Err(disk_error(&part_path, &error)),
        }
    }
    Err(StreamError::Permanent(format!(
        "no free partial file name next to {} after {MAX_PART_NAME_PROBES} tries",
        file.final_path.display()
    )))
}

async fn checkpoint_written(
    context: &TransferContext<'_>,
    file: &mut HttpJobFile,
    output: &mut BufWriter<tokio::fs::File>,
    part_path: &Path,
    written: u64,
) -> Result<(), StreamError> {
    output
        .flush()
        .await
        .map_err(|error| disk_error(part_path, &error))?;
    output
        .get_ref()
        .sync_data()
        .await
        .map_err(|error| disk_error(part_path, &error))?;
    if file.bytes_done == written {
        return Ok(());
    }
    file.bytes_done = written;
    context
        .checkpoint
        .record(file)
        .await
        .map_err(|error| StreamError::Permanent(error.to_string()))
}

/// Check the finished partial and move it to its final name.
async fn finish_file(
    context: &TransferContext<'_>,
    file: &mut HttpJobFile,
) -> Result<(), TransferError> {
    let part_path = file.part_path();
    let on_disk = existing_len(&part_path).await?;
    let total = file.total_bytes.unwrap_or(file.bytes_done);
    check_final_size(on_disk, total, &file.resource)?;
    if let Some(expected) = file
        .resource
        .validators
        .checksum
        .as_deref()
        .and_then(|raw| {
            let parsed = ExpectedChecksum::parse(raw);
            if parsed.is_none() {
                warn!(
                    checksum = raw,
                    "HTTP download checksum is in a form this client does not read; it was not checked"
                );
            }
            parsed
        })
    {
        let actual = hash_file(context, &part_path, expected.algorithm).await?;
        if actual != expected.digest {
            // The partial stays where it is, under its partial name.
            return Err(TransferError::Permanent(format!(
                "the download does not match the indexer's {} checksum",
                expected.algorithm.label()
            )));
        }
    }
    // Checked under the partial's name, so a file that fails never appears
    // under its final one.
    if let Some(check) = &context.media_check {
        let outcome = verify_media(check, &part_path).await?;
        file.partial_path = Some(part_path.clone());
        // Named for what the probe read, so the import recognizes it.
        if let Some(named) = media_path_for_probed_container(&file.final_path, &outcome) {
            file.final_path = named;
        }
        if scryer_domain::canonical_video_extension(&file.final_path).is_none() {
            return Err(TransferError::Refused(http_release_refusal::UNKNOWN_CONTAINER,
                "the payload could not be identified as a supported video container; the partial was preserved".into()));
        }
    }
    // Recorded before the file is published, so an interruption between the
    // publish and the record below finds every byte accounted for and
    // recognizes the published file instead of fetching it again.
    // Always recorded: a length learned only when the body ended was never
    // part of a checkpoint.
    file.total_bytes = Some(total);
    file.bytes_done = total;
    file.partial_path = Some(part_path.clone());
    let publication = PublicationCheckpoint {
        attempt: file.attempt,
        partial_path: part_path.clone(),
        final_path: file.final_path.clone(),
        size: total,
        sha256: hash_file(context, &part_path, ChecksumAlgorithm::Sha256).await?,
    };
    if file
        .publication
        .as_ref()
        .is_some_and(|recorded| recorded != &publication)
    {
        return Err(TransferError::Refused(
            http_release_refusal::RECOVERY_UNVERIFIED,
            "the partial no longer matches its publication checkpoint; it was preserved".into(),
        ));
    }
    file.publication = Some(publication);
    record(context, file).await?;
    publish_part(&part_path, &file.final_path).await?;
    file.complete = true;
    record(context, file).await
}

fn destination_exists(final_path: &Path) -> TransferError {
    TransferError::Refused(
        http_release_refusal::DESTINATION_EXISTS,
        format!(
            "{} already exists; it was left untouched",
            final_path.display()
        ),
    )
}

/// Give the checked partial its final name without ever replacing a file
/// already there.
///
/// A hard link is created only when the name is free, atomically, so a file
/// another writer puts under the final name in the meantime is kept. The
/// partial name is then dropped; only the job's own `.part` is removed, and
/// only once its bytes are reachable under the final name. A filesystem that
/// cannot hard-link refuses publication instead of risking replacement.
pub(crate) async fn publish_part(part_path: &Path, final_path: &Path) -> Result<(), TransferError> {
    match tokio::fs::hard_link(part_path, final_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // An interruption after the link and before the partial name was
            // dropped leaves the same bytes under both names. Anything else
            // under the final name is not this job's file.
            if !same_contents(part_path, final_path).await? {
                return Err(destination_exists(final_path));
            }
        }
        Err(link_error) => {
            return Err(TransferError::Refused(
                http_release_refusal::PUBLICATION_FAILED,
                format!(
                    "could not publish without replacing a destination: {link_error}; the partial was preserved"
                ),
            ));
        }
    }
    tokio::fs::remove_file(part_path).await.map_err(|error| {
        TransferError::Permanent(format!(
            "{} is in place but its partial {} could not be removed: {error}",
            final_path.display(),
            part_path.display()
        ))
    })
}

/// Both paths hold the same bytes.
async fn same_contents(left: &Path, right: &Path) -> Result<bool, TransferError> {
    use tokio::io::AsyncReadExt;
    let inspect = |path: &Path, error: std::io::Error| {
        TransferError::Permanent(format!("cannot inspect {}: {error}", path.display()))
    };
    let (left_meta, right_meta) = (
        tokio::fs::symlink_metadata(left)
            .await
            .map_err(|error| inspect(left, error))?,
        tokio::fs::symlink_metadata(right)
            .await
            .map_err(|error| inspect(right, error))?,
    );
    if !left_meta.is_file() || !right_meta.is_file() || left_meta.len() != right_meta.len() {
        return Ok(false);
    }
    let mut left_file = tokio::fs::File::open(left)
        .await
        .map_err(|error| inspect(left, error))?;
    let mut right_file = tokio::fs::File::open(right)
        .await
        .map_err(|error| inspect(right, error))?;
    let mut left_buf = vec![0u8; 64 * 1024];
    let mut right_buf = vec![0u8; 64 * 1024];
    loop {
        let read = left_file
            .read(&mut left_buf)
            .await
            .map_err(|error| inspect(left, error))?;
        if read == 0 {
            return Ok(true);
        }
        right_file
            .read_exact(&mut right_buf[..read])
            .await
            .map_err(|error| inspect(right, error))?;
        if left_buf[..read] != right_buf[..read] {
            return Ok(false);
        }
    }
}

/// A digest algorithm an indexer may state a checksum in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChecksumAlgorithm {
    Sha256,
    Sha1,
    Md5,
}

impl ChecksumAlgorithm {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Sha256 => "SHA-256",
            Self::Sha1 => "SHA-1",
            Self::Md5 => "MD5",
        }
    }

    fn digest_len(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha1 => 20,
            Self::Md5 => 16,
        }
    }
}

/// The checksum an indexer stated for a resource.
///
/// The SDK leaves the format open, so this reads the common spellings:
/// `sha256:<hex>`, `sha1:<hex>`, `md5:<hex>` (prefix case-insensitive, with or
/// without a hyphen such as `sha-256`), and a bare 64-digit hex string as
/// SHA-256. Anything else is not checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedChecksum {
    pub algorithm: ChecksumAlgorithm,
    pub digest: Vec<u8>,
}

impl ExpectedChecksum {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let (algorithm, hex) = match raw.split_once(':') {
            Some((prefix, hex)) => {
                let algorithm = match prefix.trim().to_ascii_lowercase().replace('-', "").as_str() {
                    "sha256" => ChecksumAlgorithm::Sha256,
                    "sha1" => ChecksumAlgorithm::Sha1,
                    "md5" => ChecksumAlgorithm::Md5,
                    _ => return None,
                };
                (algorithm, hex.trim())
            }
            None if raw.len() == 64 => (ChecksumAlgorithm::Sha256, raw),
            None => return None,
        };
        let digest = decode_hex(hex)?;
        (digest.len() == algorithm.digest_len()).then_some(Self { algorithm, digest })
    }
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |byte: u8| (byte as char).to_digit(16);
            Some((digit(pair[0])? * 16 + digit(pair[1])?) as u8)
        })
        .collect()
}

/// Hash a file without holding it in memory.
async fn hash_file(
    context: &TransferContext<'_>,
    path: &Path,
    algorithm: ChecksumAlgorithm,
) -> Result<Vec<u8>, TransferError> {
    use md5::Digest as _;
    use tokio::io::AsyncReadExt;

    enum Hasher {
        Lc(aws_lc_rs::digest::Context),
        Md5(md5::Md5),
    }
    let mut hasher = match algorithm {
        ChecksumAlgorithm::Sha256 => {
            Hasher::Lc(aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256))
        }
        ChecksumAlgorithm::Sha1 => Hasher::Lc(aws_lc_rs::digest::Context::new(
            &aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY,
        )),
        ChecksumAlgorithm::Md5 => Hasher::Md5(md5::Md5::new()),
    };
    let read_error = |error: std::io::Error| {
        TransferError::Permanent(format!("could not read {}: {error}", path.display()))
    };
    let mut input = tokio::fs::File::open(path).await.map_err(read_error)?;
    let mut buffer = vec![0u8; WRITE_BUFFER_BYTES];
    loop {
        if context.cancel.is_cancelled() {
            return Err(TransferError::Cancelled);
        }
        let read = input.read(&mut buffer).await.map_err(read_error)?;
        if read == 0 {
            break;
        }
        match &mut hasher {
            Hasher::Lc(context) => context.update(&buffer[..read]),
            Hasher::Md5(md5) => md5.update(&buffer[..read]),
        }
    }
    Ok(match hasher {
        Hasher::Lc(context) => context.finish().as_ref().to_vec(),
        Hasher::Md5(md5) => md5.finalize().to_vec(),
    })
}

/// The finished size must be what the transfer announced and, when the
/// indexer stated an exact size, that size. An estimate is only logged.
pub(crate) fn check_final_size(
    on_disk: u64,
    announced: u64,
    resource: &PluginHttpResource,
) -> Result<(), TransferError> {
    if on_disk != announced {
        return Err(TransferError::Permanent(format!(
            "the partial file holds {on_disk} bytes but the transfer announced {announced}"
        )));
    }
    if let Some(exact) = resource
        .size_bytes
        .and_then(|size| u64::try_from(size).ok())
        && exact != on_disk
    {
        return Err(TransferError::Permanent(format!(
            "the download is {on_disk} bytes but the indexer stated exactly {exact}"
        )));
    }
    if let Some(estimate) = resource
        .size_estimate
        .and_then(|size| u64::try_from(size).ok())
        && estimate != on_disk
    {
        info!(
            size_bytes = on_disk,
            size_estimate = estimate,
            "HTTP download size differs from the indexer's estimate"
        );
    }
    Ok(())
}
