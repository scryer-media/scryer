//! Prometheus recorder configuration for the Scryer binary.
//!
//! This module is the single place where the `metrics_exporter_prometheus` recorder is built,
//! installed, described, and maintained. Keeping it in one place means the bucket ladders, the
//! idle timeout, and the HELP text for every metric family stay together and testable without
//! installing a global recorder.
//!
//! Three things matter for correctness of the rendered `/metrics` payload:
//!
//! * **Buckets.** Without bucket overrides the exporter renders every `histogram!` as a rolling
//!   summary (`quantile=` series over a 60 s window), which makes `histogram_quantile()`
//!   unusable and silently drops infrequent metrics between scrapes. Every distribution we emit
//!   is matched by one of the ladders below, so all of them render as real
//!   `_bucket`/`_sum`/`_count` histograms.
//! * **Upkeep.** The exporter only drains raw histogram samples during upkeep or a render. An
//!   instance with metrics enabled and no scraper would otherwise grow without bound, so
//!   [`spawn_upkeep`] runs [`PrometheusHandle::run_upkeep`] on a timer.
//! * **Descriptions.** `describe_*!` calls are routed to whichever recorder is installed at the
//!   time, so [`describe_metrics`] must run *after* the recorder is installed; describing against
//!   the no-op recorder silently loses the HELP text.

use std::time::Duration;

use metrics::{Unit, describe_counter, describe_gauge, describe_histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use metrics_util::MetricKindMask;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

/// Environment variable that gates the Prometheus recorder and the `/metrics` route.
const METRICS_ENV: &str = "SCRYER_METRICS";

/// Bucket ladder applied to every metric whose name ends in `_seconds`.
///
/// One ladder has to serve both sub-second HTTP/indexer latencies and multi-minute scheduled
/// tasks, hence the wide tail out to 30 minutes.
const LATENCY_SECONDS_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0,
    1800.0,
];

/// Bucket ladder for import-lane permit occupancy (a small count, not a duration).
const IMPORT_LANE_PERMIT_BUCKETS: &[f64] = &[0.0, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 32.0];

/// Bucket ladder for the number of automatic indexer search strategies selected per query.
const INDEXER_STRATEGY_BUCKETS: &[f64] = &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0];

/// Prefix matcher for the automatic-strategy distribution.
///
/// Deliberately a prefix rather than a full match: the metric is being renamed from
/// `scryer_indexer_auto_strategy_count` to `scryer_indexer_auto_strategies`, and both spellings
/// must land on this ladder.
const INDEXER_STRATEGY_PREFIX: &str = "scryer_indexer_auto_strateg";

/// How long a label set may go without an update before the exporter evicts it.
///
/// Generous on purpose: daily scheduled tasks must not be evicted between runs, and the point of
/// the timeout is to retire label sets for deleted indexers, download clients, and hosts rather
/// than to prune idle-but-live series.
const IDLE_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

/// How often the upkeep task drains raw histogram samples.
const UPKEEP_INTERVAL: Duration = Duration::from_secs(15);

/// Returns whether the Prometheus recorder should be installed, based on `SCRYER_METRICS`.
pub fn metrics_enabled_from_env() -> bool {
    std::env::var(METRICS_ENV)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Builds the configured Prometheus exporter builder.
///
/// This is the single source of truth for bucket ladders and the idle timeout, and is used both
/// by [`install_prometheus_recorder`] and by the tests (via `build_recorder`).
///
/// Bucket matcher precedence inside the exporter is `Full` > `Prefix` > `Suffix`, so the two
/// count-valued ladders win over the `_seconds` suffix ladder even if a name were to match both.
/// Unit suffixes and global labels are deliberately left off: metric names are already explicit
/// about their unit, and global labels would break existing dashboards.
pub fn prometheus_builder() -> PrometheusBuilder {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Suffix("_seconds".to_string()),
            LATENCY_SECONDS_BUCKETS,
        )
        .expect("latency bucket ladder is non-empty")
        .set_buckets_for_metric(
            Matcher::Full("scryer_import_lane_active_permits".to_string()),
            IMPORT_LANE_PERMIT_BUCKETS,
        )
        .expect("import lane permit bucket ladder is non-empty")
        .set_buckets_for_metric(
            Matcher::Prefix(INDEXER_STRATEGY_PREFIX.to_string()),
            INDEXER_STRATEGY_BUCKETS,
        )
        .expect("indexer strategy bucket ladder is non-empty")
        .idle_timeout(MetricKindMask::ALL, Some(IDLE_TIMEOUT))
}

/// Installs the global Prometheus recorder when `SCRYER_METRICS` enables it.
///
/// Returns `None` when metrics are disabled, in which case every `metrics::*!` call in the
/// process stays a no-op and no `/metrics` route is mounted.
///
/// # Panics
///
/// Panics if the recorder cannot be installed (for example if another global recorder is already
/// registered); this matches the previous inline behaviour in `main`.
pub fn install_prometheus_recorder() -> Option<PrometheusHandle> {
    if !metrics_enabled_from_env() {
        return None;
    }

    let handle = prometheus_builder()
        .install_recorder()
        .expect("failed to install prometheus metrics recorder");
    tracing::info!("prometheus metrics enabled at /metrics");
    // Descriptions must be registered against the installed recorder, not the no-op one.
    describe_metrics();
    Some(handle)
}

/// Spawns the background task that runs exporter upkeep until `shutdown` is cancelled.
///
/// Upkeep drains raw histogram samples into their aggregated form. Without it, a process with
/// metrics enabled and no scraper accumulates samples indefinitely.
///
/// The returned `JoinHandle` may be dropped; the task exits on its own when the token fires.
pub fn spawn_upkeep(
    handle: PrometheusHandle,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(UPKEEP_INTERVAL);
        // Never burst-catch-up after a suspend or a long stall.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                _ = ticker.tick() => handle.run_upkeep(),
            }
        }
    })
}

/// Registers HELP text (and units) for every metric family emitted anywhere in the workspace.
///
/// Must be called with the target recorder installed (globally or as a local recorder).
pub fn describe_metrics() {
    // Families owned by other crates describe themselves; call them here so every family
    // is registered against the installed recorder.
    scryer_application::describe_domain_event_metrics();

    // --- Acquisition: grabs and RSS ------------------------------------------------------
    describe_counter!(
        "scryer_grabs_total",
        "Releases sent to a download client, labelled by the indexer that supplied the release and the media facet it was grabbed for."
    );
    describe_counter!(
        "scryer_rss_sync_total",
        "RSS sync cycles started, including cycles that exited early because no indexer was due."
    );
    describe_histogram!(
        "scryer_rss_sync_duration_seconds",
        Unit::Seconds,
        "Wall-clock duration of one RSS sync cycle, including early exits."
    );
    describe_counter!(
        "scryer_rss_releases_fetched_total",
        "Releases returned by indexer RSS feeds across all completed RSS sync cycles."
    );
    describe_counter!(
        "scryer_rss_releases_matched_total",
        "Fetched RSS releases that matched a monitored title or episode."
    );
    describe_counter!(
        "scryer_rss_releases_grabbed_total",
        "Matched RSS releases that were actually grabbed."
    );

    // --- Acquisition: background workers -------------------------------------------------
    describe_counter!(
        "scryer_background_acquisition_title_work_total",
        "Background title-level acquisition units of work, labelled by outcome (completed or failed)."
    );
    describe_counter!(
        "scryer_background_acquisition_target_work_total",
        "Background target-level acquisition units of work, labelled by outcome (completed or failed)."
    );
    describe_counter!(
        "scryer_background_acquisition_scan_owned_yields_total",
        "Times background acquisition yielded because a library scan owned the facet it wanted to work on."
    );

    // --- Wanted projection ---------------------------------------------------------------
    describe_counter!(
        "scryer_wanted_projection_cache_total",
        "Wanted-projection cache lookups, labelled by result (hit or miss)."
    );
    describe_histogram!(
        "scryer_wanted_projection_rebuild_duration_seconds",
        Unit::Seconds,
        "Time taken to rebuild a wanted projection, labelled by the projection kind."
    );
    describe_gauge!(
        "scryer_wanted_projection_items",
        "Number of rows in the most recently rebuilt wanted projection, labelled by the projection kind."
    );

    // --- Scheduled tasks and polling workers ---------------------------------------------
    describe_counter!(
        "scryer_task_runs_total",
        "Scheduled task executions that completed, labelled by task name."
    );
    describe_counter!(
        "scryer_task_errors_total",
        "Scheduled task executions that returned an error, labelled by task name."
    );
    describe_counter!(
        "scryer_task_panics_total",
        "Scheduled task executions that panicked, labelled by task name."
    );
    describe_histogram!(
        "scryer_task_duration_seconds",
        Unit::Seconds,
        "Wall-clock duration of one scheduled task execution, labelled by task name."
    );
    describe_counter!(
        "scryer_background_worker_errors_total",
        "Errors raised by a polling background worker, labelled by worker name and the call-site context that failed."
    );
    describe_counter!(
        "scryer_background_worker_stale_recoveries_total",
        "Stale work items recovered by a polling background worker, labelled by worker name and context."
    );

    // --- Title metadata hydration ---------------------------------------------------------
    describe_counter!(
        "scryer_title_metadata_hydration_attempts_total",
        "Titles dispatched to metadata hydration."
    );
    describe_counter!(
        "scryer_title_metadata_hydration_success_total",
        "Titles whose metadata hydration succeeded."
    );
    describe_counter!(
        "scryer_title_metadata_hydration_failure_total",
        "Title metadata hydration failures that are still eligible for retry."
    );
    describe_counter!(
        "scryer_title_metadata_hydration_terminal_failures_total",
        "Titles abandoned by metadata hydration after exhausting their retry budget."
    );
    describe_counter!(
        "scryer_title_metadata_hydration_scan_owned_yields_total",
        "Times the hydration loop yielded because a library scan owned an active facet."
    );
    describe_counter!(
        "scryer_title_metadata_hydration_scan_owned_rechecks_total",
        "Times the hydration loop re-checked facet ownership before dispatching work."
    );
    describe_gauge!(
        "scryer_title_metadata_hydration_pending",
        "Titles currently due for metadata hydration at the start of a hydration pass."
    );

    // --- Movie SMG identity backfill -------------------------------------------------------
    describe_counter!(
        "scryer_movie_smg_identity_backfill_linked_total",
        "Movie titles linked to an SMG identity by the backfill job."
    );
    describe_counter!(
        "scryer_movie_smg_identity_backfill_unresolved_total",
        "Movie titles the SMG identity backfill job could not resolve."
    );
    describe_counter!(
        "scryer_movie_smg_identity_backfill_errors_total",
        "Errors raised while running the movie SMG identity backfill job."
    );

    // --- Download queue ---------------------------------------------------------------------
    describe_gauge!(
        "scryer_download_queue_items",
        "Items currently tracked in the download queue, labelled by queue state."
    );
    describe_gauge!(
        "scryer_download_queue_snapshot_items",
        "Items in the most recently committed download-queue snapshot."
    );
    describe_gauge!(
        "scryer_download_queue_snapshot_age_seconds",
        Unit::Seconds,
        "Age of the download-queue snapshot that served the most recent queue page request."
    );
    describe_gauge!(
        "scryer_download_queue_legacy_subscriptions",
        "Currently open legacy (non-snapshot) download-queue subscriptions."
    );
    describe_counter!(
        "scryer_download_queue_snapshot_refresh_total",
        "Download-queue snapshot refresh attempts, labelled by result (success or error)."
    );
    describe_counter!(
        "scryer_download_queue_read_model_total",
        "Download-queue read-model builds, labelled by result (hit reuses a cached model, miss rebuilds it)."
    );
    describe_counter!(
        "scryer_download_queue_cache_total",
        "Download-queue page requests, labelled by whether a ready snapshot served the request (hit) or not (miss)."
    );
    describe_counter!(
        "scryer_download_queue_revision_notifications_total",
        "Download-queue revision notifications published to subscribers."
    );
    describe_histogram!(
        "scryer_download_queue_refresh_duration_seconds",
        Unit::Seconds,
        "Duration of one full download-queue refresh cycle across all download clients."
    );
    describe_histogram!(
        "scryer_download_queue_page_duration_seconds",
        Unit::Seconds,
        "Duration of serving one download-queue page request."
    );
    describe_histogram!(
        "scryer_download_client_refresh_duration_seconds",
        Unit::Seconds,
        "Duration of refreshing queue state from a single download client."
    );

    // --- Indexer queries ---------------------------------------------------------------------
    describe_counter!(
        "scryer_indexer_queries_total",
        "Indexer queries issued, labelled by indexer name, result status, and search mode."
    );
    describe_counter!(
        "scryer_indexer_query_results_total",
        "Releases returned by indexer queries, labelled by indexer name and search mode."
    );
    describe_histogram!(
        "scryer_indexer_query_duration_seconds",
        Unit::Seconds,
        "Duration of one indexer query, labelled by indexer name and search mode."
    );
    describe_histogram!(
        "scryer_indexer_auto_strategy_count",
        "Number of automatic search strategies (primary plus fallback) selected for a query, labelled by indexer name and the source of its capabilities."
    );

    // --- Import lanes ---------------------------------------------------------------------
    describe_counter!(
        "scryer_import_lane_acquisitions_total",
        "Import-lane permit acquisitions, labelled by lane and by whether the lane was saturated when the permit was requested."
    );
    describe_histogram!(
        "scryer_import_lane_wait_seconds",
        Unit::Seconds,
        "Time spent waiting for an import-lane permit, labelled by lane."
    );
    describe_histogram!(
        "scryer_import_lane_active_permits",
        "Import-lane permits in use at the moment a permit was acquired, labelled by lane."
    );

    // --- Outbound HTTP ---------------------------------------------------------------------
    describe_counter!(
        "scryer_outbound_http_429_total",
        "Outbound HTTP 429 responses received, labelled by rate-limit scope, request label, and the source of the retry-after hint."
    );
    describe_counter!(
        "scryer_outbound_http_rate_limited_total",
        "Outbound HTTP requests abandoned because rate limiting exhausted the retry budget, labelled by scope, request label, and retry-after source."
    );
    describe_counter!(
        "scryer_outbound_http_transport_retry_total",
        "Outbound HTTP requests retried after a transport error, labelled by scope and request label."
    );
    describe_histogram!(
        "scryer_outbound_http_transport_backoff_seconds",
        Unit::Seconds,
        "Backoff applied before an outbound HTTP transport retry, labelled by scope and request label."
    );
    describe_counter!(
        "scryer_outbound_http_destination_cooldown_wait_total",
        "Outbound HTTP requests delayed by a destination cooldown, labelled by destination and request label."
    );
    describe_histogram!(
        "scryer_outbound_http_destination_cooldown_wait_seconds",
        Unit::Seconds,
        "Time an outbound HTTP request waited on a destination cooldown, labelled by destination and request label."
    );
    describe_counter!(
        "scryer_outbound_http_host_rps_wait_total",
        "Outbound HTTP requests delayed by the per-host request-rate limiter, labelled by host and request label."
    );
    describe_histogram!(
        "scryer_outbound_http_host_rps_wait_seconds",
        Unit::Seconds,
        "Time an outbound HTTP request waited on the per-host request-rate limiter, labelled by host and request label."
    );

    // --- External subtitle probe (names are currently unprefixed) --------------------------
    describe_counter!(
        "subtitle_external_probe_cache_hit_total",
        "External subtitle probes served from the probe cache."
    );
    describe_counter!(
        "subtitle_external_probe_cache_miss_total",
        "External subtitle probes that had to read the subtitle file."
    );
    describe_counter!(
        "subtitle_external_probe_decode_failed_total",
        "External subtitle files that could not be decoded as text."
    );
    describe_counter!(
        "subtitle_external_probe_skipped_non_text_total",
        "External subtitle files skipped because their contents are not text."
    );
    describe_counter!(
        "subtitle_external_probe_skipped_size_total",
        "External subtitle files skipped because they exceed the probe size limit."
    );
    describe_counter!(
        "subtitle_external_probe_unresolved_language_total",
        "External subtitle files whose language could not be resolved."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use metrics::{histogram, with_local_recorder};

    #[test]
    fn seconds_metrics_render_as_histogram_buckets() {
        let recorder = prometheus_builder().build_recorder();
        let rendered = with_local_recorder(&recorder, || {
            histogram!("scryer_task_duration_seconds", "task" => "x").record(0.5);
            recorder.handle().render()
        });

        assert!(
            rendered.contains("scryer_task_duration_seconds_bucket{"),
            "expected bucket series, got:\n{rendered}"
        );
        assert!(
            rendered.contains("le=\"0.5\""),
            "expected an le=\"0.5\" bucket, got:\n{rendered}"
        );
        assert!(
            rendered.contains("le=\"+Inf\""),
            "expected an le=\"+Inf\" bucket, got:\n{rendered}"
        );
        assert!(
            rendered.contains("scryer_task_duration_seconds_sum"),
            "expected a _sum series, got:\n{rendered}"
        );
        assert!(
            rendered.contains("scryer_task_duration_seconds_count"),
            "expected a _count series, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("quantile="),
            "histograms must not render as summaries, got:\n{rendered}"
        );
    }

    #[test]
    fn count_valued_metrics_get_linear_ladder() {
        let recorder = prometheus_builder().build_recorder();
        let rendered = with_local_recorder(&recorder, || {
            histogram!("scryer_import_lane_active_permits", "lane" => "default").record(3.0);
            recorder.handle().render()
        });

        assert!(
            rendered.contains("le=\"1\""),
            "expected an le=\"1\" bucket, got:\n{rendered}"
        );
        assert!(
            rendered.contains("le=\"32\""),
            "expected an le=\"32\" bucket, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("le=\"0.005\""),
            "count-valued metric must not use the latency ladder, got:\n{rendered}"
        );
    }

    #[test]
    fn strategy_prefix_matches_both_spellings() {
        let recorder = prometheus_builder().build_recorder();
        let rendered = with_local_recorder(&recorder, || {
            histogram!("scryer_indexer_auto_strategy_count", "indexer" => "a").record(2.0);
            histogram!("scryer_indexer_auto_strategies", "indexer" => "a").record(2.0);
            recorder.handle().render()
        });

        let count_line = rendered.lines().find(|line| {
            line.starts_with("scryer_indexer_auto_strategy_count_bucket{")
                && line.contains("le=\"10\"")
        });
        assert!(
            count_line.is_some(),
            "expected the legacy spelling on the strategy ladder, got:\n{rendered}"
        );

        let renamed_line = rendered.lines().find(|line| {
            line.starts_with("scryer_indexer_auto_strategies_bucket{") && line.contains("le=\"10\"")
        });
        assert!(
            renamed_line.is_some(),
            "expected the renamed spelling on the strategy ladder, got:\n{rendered}"
        );
    }

    #[test]
    fn describe_metrics_emits_help_text() {
        let recorder = prometheus_builder().build_recorder();
        let rendered = with_local_recorder(&recorder, || {
            describe_metrics();
            histogram!("scryer_task_duration_seconds", "task" => "x").record(0.1);
            recorder.handle().render()
        });

        let help_line = rendered
            .lines()
            .find(|line| line.starts_with("# HELP scryer_task_duration_seconds "))
            .unwrap_or_else(|| panic!("expected a HELP line, got:\n{rendered}"));
        let help_text = help_line
            .trim_start_matches("# HELP scryer_task_duration_seconds ")
            .trim();
        assert!(
            !help_text.is_empty(),
            "HELP text must not be empty, got: {help_line:?}"
        );
    }

    #[tokio::test]
    async fn upkeep_task_stops_on_cancel() {
        let recorder = prometheus_builder().build_recorder();
        let shutdown = CancellationToken::new();
        let task = spawn_upkeep(recorder.handle(), shutdown.clone());

        shutdown.cancel();

        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("upkeep task did not stop within 2s after cancellation")
            .expect("upkeep task panicked");
    }
}
