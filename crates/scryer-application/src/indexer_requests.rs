//! Per-request accounting for outbound indexer HTTP calls.
//!
//! Two numbers come out of one place here, and they mean different things:
//!
//! * the **dashboard** number ([`IndexerStatsTracker::record_api_request_sent`])
//!   counts requests Scryer *sent*. It is incremented before the response
//!   exists, so a timeout, a retry, a solver replay and a caps refresh all
//!   count exactly like a successful search page. It is the number an operator
//!   compares against the indexer's own quota page, so it must never depend on
//!   parsing what came back.
//! * the **metric** `scryer_indexer_api_requests_total` counts the same
//!   requests once each, labelled by what was asked (`call`) and what came back
//!   (`result`), for the breakdown a dashboard cannot carry.
//!
//! Because both come from [`IndexerRequestTally`], the invariant
//! `dashboard total == sum over labels of the metric` holds by construction:
//! a send registers a pending outcome, resolving it emits exactly one metric
//! sample, and anything still pending when the tally drops is emitted as
//! `unknown` rather than lost.

use std::sync::{Arc, Mutex};

use metrics::counter;

use crate::indexer_errors::classify_indexer_http_response;
use crate::ports::IndexerStatsTracker;
use crate::types::{
    CapturedIndexerHttpResponse, IndexerErrorClassification, IndexerErrorOperation,
};

/// Metric name for the per-request breakdown. Deliberately distinct from
/// `scryer_indexer_queries_total`, which counts *strategies* rather than HTTP
/// requests and stays as it is.
pub const INDEXER_API_REQUESTS_METRIC: &str = "scryer_indexer_api_requests_total";

/// `call` label for a request that carries no newznab `t=` function, such as a
/// Prowlarr JSON API call or a torrent-site scrape.
pub const CALL_OTHER: &str = "other";
/// `call` label for a POST to a challenge solver on an indexer's behalf. The
/// request does not reach the indexer, but it is spent on that indexer's
/// account, so it is counted and labelled rather than hidden.
pub const CALL_SOLVER: &str = "solver";

/// What came back for one sent request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexerRequestResult {
    /// 2xx with no newznab error document in the body.
    Success,
    /// A newznab `<error code=...>` document, or a non-2xx status; carries the
    /// snake_case classification so `result` reads the same way the indexer
    /// error history does.
    Classified(&'static str),
    /// A status the shared classifier does not name, e.g. `http_451`. The
    /// statuses it does name (401, 403, 404, 408, 429, 5xx…) arrive as
    /// [`Self::Classified`] so `result` reads the same way the indexer error
    /// history does.
    HttpStatus(u16),
    /// The request was sent but no response arrived: timeout, DNS failure,
    /// connection reset, response too large to accept.
    NoResponse,
    /// The operation was cancelled before the outcome was known.
    Cancelled,
    /// The tally was dropped with the outcome still pending. A non-zero count
    /// here means a code path returns without resolving its send.
    Unknown,
}

impl IndexerRequestResult {
    pub fn label(&self) -> String {
        match self {
            Self::Success => "success".to_string(),
            Self::Classified(label) => (*label).to_string(),
            Self::HttpStatus(status) => format!("http_{status}"),
            Self::NoResponse => "no_response".to_string(),
            Self::Cancelled => "cancelled".to_string(),
            Self::Unknown => "unknown".to_string(),
        }
    }

    /// Classify a response the way the indexer error history does, so a
    /// 200-with-`<error code=500>` reads as `newznab_request_limit_reached`
    /// rather than `success`.
    pub fn from_response(response: &CapturedIndexerHttpResponse) -> Self {
        match classify_indexer_http_response(response) {
            // The classifier answers `Unknown` for any status outside its
            // table. Carrying the status instead keeps that case readable and
            // leaves `unknown` to mean only "no outcome was ever recorded".
            Some(classified)
                if classified.classification == IndexerErrorClassification::Unknown
                    && !(200..300).contains(&response.status) =>
            {
                Self::HttpStatus(response.status)
            }
            Some(classified) => Self::Classified(classified.classification.as_str()),
            None if (200..300).contains(&response.status) => Self::Success,
            None => Self::HttpStatus(response.status),
        }
    }
}

/// The `call` label for a request URL: its newznab `t=` function when it has
/// one, else [`CALL_OTHER`]. Kept to a short allowlist so a malformed or
/// hostile `t=` cannot inflate metric cardinality.
pub fn newznab_call_label(url: &str) -> &'static str {
    let Some(query) = url.split_once('?').map(|(_, query)| query) else {
        return CALL_OTHER;
    };
    for pair in query.split('&') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("t") {
            continue;
        }
        return match value.to_ascii_lowercase().as_str() {
            "caps" => "caps",
            "search" => "search",
            "tvsearch" => "tvsearch",
            "movie" => "movie",
            "music" => "music",
            "book" => "book",
            "get" => "get",
            "details" => "details",
            "getnfo" => "getnfo",
            "cart" => "cart",
            "register" => "register",
            _ => CALL_OTHER,
        };
    }
    CALL_OTHER
}

/// Counts outbound requests for one indexer operation.
///
/// Create one per logical transport call, then pair every send with its
/// outcome:
///
/// ```ignore
/// let tally = IndexerRequestTally::new(id, name, operation, stats);
/// tally.note_sent(&url);              // dashboard += 1, outcome pending
/// let response = client.send().await; // ...
/// tally.note_response(&captured);     // metric += 1, pending resolved
/// ```
///
/// Sends may outnumber resolutions when a transport retries internally; a
/// single [`IndexerRequestTally::note_result`] resolves every pending send with
/// the same outcome, which is what a retried timeout actually is.
pub struct IndexerRequestTally {
    indexer_id: String,
    indexer_name: String,
    operation: IndexerErrorOperation,
    stats: Arc<dyn IndexerStatsTracker>,
    pending: Mutex<Vec<&'static str>>,
}

impl IndexerRequestTally {
    pub fn new(
        indexer_id: String,
        indexer_name: String,
        operation: IndexerErrorOperation,
        stats: Arc<dyn IndexerStatsTracker>,
    ) -> Self {
        Self {
            indexer_id,
            indexer_name,
            operation,
            stats,
            pending: Mutex::new(Vec::new()),
        }
    }

    /// One request is about to leave for `url`. Counts it on the dashboard
    /// immediately: whatever happens next, the indexer has been asked.
    pub fn note_sent(&self, url: &str) {
        self.note_sent_call(newznab_call_label(url));
    }

    /// As [`Self::note_sent`], for a request whose `call` label is not derived
    /// from a newznab URL (a solver POST, say).
    pub fn note_sent_call(&self, call: &'static str) {
        self.stats
            .record_api_request_sent(&self.indexer_id, &self.indexer_name);
        if let Ok(mut pending) = self.pending.lock() {
            pending.push(call);
        }
    }

    /// Resolve every pending send with `result`, emitting one metric sample
    /// each.
    pub fn note_result(&self, result: IndexerRequestResult) {
        let pending = match self.pending.lock() {
            Ok(mut pending) => std::mem::take(&mut *pending),
            Err(_) => return,
        };
        let label = result.label();
        for call in pending {
            counter!(
                INDEXER_API_REQUESTS_METRIC,
                "indexer" => self.indexer_name.clone(),
                "operation" => self.operation.as_str(),
                "call" => call,
                "result" => label.clone(),
            )
            .increment(1);
        }
    }

    /// Resolve every pending send with the outcome carried by `response`.
    pub fn note_response(&self, response: &CapturedIndexerHttpResponse) {
        self.note_result(IndexerRequestResult::from_response(response));
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.pending
            .lock()
            .map(|pending| pending.len())
            .unwrap_or(0)
    }
}

impl Drop for IndexerRequestTally {
    fn drop(&mut self) {
        // A send that returned through `?` without an explicit outcome is still
        // a request the indexer served. Emitting it as `unknown` keeps the
        // metric total equal to the dashboard total and makes the gap findable.
        self.note_result(IndexerRequestResult::Unknown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::IndexerQueryStats;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[derive(Default)]
    struct CountingTracker {
        sent: AtomicU32,
    }

    impl IndexerStatsTracker for CountingTracker {
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
            self.sent.fetch_add(1, Ordering::Relaxed);
        }
        fn all_stats(&self) -> Vec<IndexerQueryStats> {
            Vec::new()
        }
    }

    fn tally(stats: Arc<CountingTracker>) -> IndexerRequestTally {
        IndexerRequestTally::new(
            "idx-1".to_string(),
            "Indexer One".to_string(),
            IndexerErrorOperation::AutomaticSearch,
            stats,
        )
    }

    fn response(status: u16, body: &str) -> CapturedIndexerHttpResponse {
        CapturedIndexerHttpResponse {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn every_send_counts_on_the_dashboard_even_without_a_response() {
        let stats = Arc::new(CountingTracker::default());
        let tally = tally(Arc::clone(&stats));
        tally.note_sent("https://indexer.test/api?t=tvsearch&q=x");
        tally.note_sent("https://indexer.test/api?t=tvsearch&q=x");
        assert_eq!(stats.sent.load(Ordering::Relaxed), 2);
        assert_eq!(tally.pending_len(), 2);

        // A retried timeout resolves both attempts at once.
        tally.note_result(IndexerRequestResult::NoResponse);
        assert_eq!(tally.pending_len(), 0);
    }

    #[test]
    fn a_newznab_error_document_on_http_200_is_not_a_success() {
        let limit_reached = response(
            200,
            r#"<?xml version="1.0"?><error code="500" description="Request limit reached"/>"#,
        );
        assert_eq!(
            IndexerRequestResult::from_response(&limit_reached),
            IndexerRequestResult::Classified("newznab_request_limit_reached")
        );
        assert_eq!(
            IndexerRequestResult::from_response(&limit_reached).label(),
            "newznab_request_limit_reached"
        );
    }

    #[test]
    fn plain_statuses_and_transport_failures_get_their_own_labels() {
        assert_eq!(
            IndexerRequestResult::from_response(&response(200, "<rss/>")),
            IndexerRequestResult::Success
        );
        // Statuses the shared classifier names keep that name, so a Grafana
        // panel and the indexer error history agree on what happened.
        assert_eq!(
            IndexerRequestResult::from_response(&response(503, "")).label(),
            "http_server_error"
        );
        assert_eq!(
            IndexerRequestResult::from_response(&response(429, "")).label(),
            "http_rate_limited"
        );
        // Anything it does not name falls back to the bare status.
        assert_eq!(
            IndexerRequestResult::from_response(&response(451, "")).label(),
            "http_451"
        );
        assert_eq!(IndexerRequestResult::NoResponse.label(), "no_response");
    }

    #[test]
    fn call_labels_come_from_the_newznab_function_and_stay_bounded() {
        assert_eq!(
            newznab_call_label("https://indexer.test/api?t=caps&apikey=x"),
            "caps"
        );
        assert_eq!(
            newznab_call_label("https://indexer.test/api?apikey=x&t=TVSEARCH&q=y"),
            "tvsearch"
        );
        assert_eq!(newznab_call_label("https://indexer.test/rss"), CALL_OTHER);
        assert_eq!(
            newznab_call_label("https://indexer.test/api?t=../../etc/passwd"),
            CALL_OTHER,
            "an unrecognised t= must not become a metric label"
        );
    }

    #[test]
    fn dropping_with_an_unresolved_send_still_accounts_for_it() {
        let stats = Arc::new(CountingTracker::default());
        {
            let tally = tally(Arc::clone(&stats));
            tally.note_sent_call(CALL_SOLVER);
            // Dropped without an outcome, as an early `?` return would.
        }
        assert_eq!(stats.sent.load(Ordering::Relaxed), 1);
    }
}
