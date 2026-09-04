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

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use metrics::{Unit, describe_counter, describe_gauge, describe_histogram, gauge};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use metrics_util::MetricKindMask;
use scryer_application::AppError;
use scryer_domain::AppPermissionMask;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::middleware::{AuthState, map_app_error, resolve_actor};

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
    record_build_info();
    Some(handle)
}

/// State for the `/metrics` route: the auth machinery that identifies the caller and the
/// exporter handle that renders the payload.
#[derive(Clone)]
pub(crate) struct MetricsRouteState {
    pub(crate) auth: AuthState,
    pub(crate) handle: PrometheusHandle,
}

/// Serves the Prometheus exposition to an authorised scraper.
///
/// The payload names every configured indexer, download client, and library root, so it is
/// not public. The only accepted credential is an API key presented as a bearer token whose
/// owner holds `MANAGE_SYSTEM_SETTINGS`. A browser session, the authless default actor, and
/// the local-IP bypass are all refused on purpose: a scraper is a machine, and a machine gets
/// a revocable key, not a login. Anything short of a usable key answers 401, a valid key
/// without the permission answers 403, and neither carries a body.
pub(crate) async fn metrics_endpoint(
    State(state): State<MetricsRouteState>,
    headers: HeaderMap,
) -> Response {
    let actor = match resolve_actor(&state.auth, &headers, None).await {
        Ok(Some(actor)) if actor.is_api_key() => actor,
        Ok(_) | Err(AppError::Unauthorized(_)) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(error) => return map_app_error(error),
    };
    if !actor
        .user
        .authorization
        .app
        .contains(AppPermissionMask::MANAGE_SYSTEM_SETTINGS)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    state.handle.render().into_response()
}

/// Publishes the identity of this process: which build is running, and since when.
///
/// `scryer_build_info` follows the usual info-metric convention — the value is always 1 and the
/// interesting content is in the labels, so dashboards join on it (`… * on(instance)
/// scryer_build_info`) to attribute a series to a version. `scryer_process_start_time_seconds` is
/// the standard way to compute uptime (`time() - scryer_process_start_time_seconds`) and, more
/// usefully, to detect restarts as a step change rather than inferring them from counter resets.
///
/// Called once from [`install_prometheus_recorder`], so "start time" is the moment metrics came
/// up during boot rather than the kernel's process start; the difference is milliseconds.
///
/// Deliberately no CPU/RSS collector here: that needs a process-collector dependency and is out
/// of scope for this change.
pub fn record_build_info() {
    gauge!(
        "scryer_build_info",
        "version" => crate::VERSION,
        "target_os" => std::env::consts::OS,
        "target_arch" => std::env::consts::ARCH,
    )
    .set(1.0);

    let start_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since_epoch| since_epoch.as_secs_f64());
    gauge!("scryer_process_start_time_seconds").set(start_time);
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
    scryer_application::describe_acquisition_metrics();
    scryer_application::describe_domain_event_metrics();
    scryer_application::describe_freshness_and_health_metrics();
    scryer_application::describe_download_queue_metrics();
    scryer_infrastructure_acquisition::describe_indexer_metrics();
    scryer_infrastructure_acquisition::describe_download_client_router_metrics();
    scryer_interface::describe_graphql_metrics();
    scryer_outbound_http::describe_outbound_http_metrics();

    // --- Process identity -----------------------------------------------------------------
    describe_gauge!(
        "scryer_build_info",
        "Always 1; carries the running build's version, target OS and target architecture as labels so dashboards can join a series to a release."
    );
    describe_gauge!(
        "scryer_process_start_time_seconds",
        Unit::Seconds,
        "Unix timestamp at which this process installed its metrics recorder during boot. Subtract from time() for uptime; a step change marks a restart."
    );

    // --- HTTP serving ---------------------------------------------------------------------
    describe_gauge!(
        "scryer_http_requests_in_flight",
        "HTTP requests currently being served, across every route."
    );
    describe_counter!(
        "scryer_http_requests_total",
        "HTTP requests served, labelled by method, matched route template (or `fallback`) and response status class. Counts responses produced by middleware, such as rate-limit rejections, as well as by handlers."
    );
    describe_histogram!(
        "scryer_http_request_duration_seconds",
        Unit::Seconds,
        "Wall-clock time to produce an HTTP response, labelled by method and matched route template. Measured at the outermost layer, so it includes rate limiting, the authless guard, CORS and compression."
    );

    // --- GraphQL WebSocket transport -------------------------------------------------------
    describe_gauge!(
        "scryer_ws_connections",
        "GraphQL WebSocket connections currently established."
    );
    describe_counter!(
        "scryer_ws_connections_total",
        "GraphQL WebSocket connections established since process start."
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

    // --- Indexer strategy selection (query families live in describe_indexer_metrics) -----
    describe_histogram!(
        "scryer_indexer_auto_strategies",
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

    // --- External subtitle probe ------------------------------------------------------------
    describe_counter!(
        "scryer_subtitle_external_probe_cache_hit_total",
        "External subtitle probes served from the probe cache."
    );
    describe_counter!(
        "scryer_subtitle_external_probe_cache_miss_total",
        "External subtitle probes that had to read the subtitle file."
    );
    describe_counter!(
        "scryer_subtitle_external_probe_decode_failed_total",
        "External subtitle files that could not be decoded as text."
    );
    describe_counter!(
        "scryer_subtitle_external_probe_skipped_non_text_total",
        "External subtitle files skipped because their contents are not text."
    );
    describe_counter!(
        "scryer_subtitle_external_probe_skipped_size_total",
        "External subtitle files skipped because they exceed the probe size limit."
    );
    describe_counter!(
        "scryer_subtitle_external_probe_unresolved_language_total",
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

    #[test]
    fn build_info_carries_version_and_target_labels() {
        let recorder = prometheus_builder().build_recorder();
        let rendered = with_local_recorder(&recorder, || {
            record_build_info();
            recorder.handle().render()
        });

        let line = rendered
            .lines()
            .find(|line| line.starts_with("scryer_build_info{"))
            .unwrap_or_else(|| panic!("expected a scryer_build_info series, got:\n{rendered}"));
        for label in [
            format!("version=\"{}\"", crate::VERSION),
            format!("target_os=\"{}\"", std::env::consts::OS),
            format!("target_arch=\"{}\"", std::env::consts::ARCH),
        ] {
            assert!(
                line.contains(&label),
                "expected {label} in {line:?}, got:\n{rendered}"
            );
        }
        let value: f64 = line
            .rsplit(' ')
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("expected a numeric value in {line:?}"));
        assert!(
            (value - 1.0).abs() < f64::EPSILON,
            "build info must always be 1, got {value}"
        );
    }

    #[test]
    fn process_start_time_is_a_unix_timestamp() {
        let recorder = prometheus_builder().build_recorder();
        let rendered = with_local_recorder(&recorder, || {
            record_build_info();
            recorder.handle().render()
        });

        let line = rendered
            .lines()
            .find(|line| line.starts_with("scryer_process_start_time_seconds "))
            .unwrap_or_else(|| {
                panic!("expected a scryer_process_start_time_seconds series, got:\n{rendered}")
            });
        let value: f64 = line
            .rsplit(' ')
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("expected a numeric value in {line:?}"));
        // Sanity floor: any plausible wall clock is well past 2020-01-01.
        assert!(
            value > 1_577_836_800.0,
            "expected a unix timestamp, got {value}"
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

    #[tokio::test]
    async fn metrics_endpoint_requires_an_api_key_with_the_system_settings_permission() {
        use crate::middleware::integration_test_common as common;
        use crate::middleware::{AuthlessWebClientProofState, WebSocketOriginPolicy};
        use crate::rate_limit::ScryerRateLimiter;
        use axum::Router;
        use axum::body::{Body, to_bytes};
        use axum::http::{HeaderValue, header};
        use axum::routing::get;
        use scryer_application::{ApiKeyExpiryPreset, CreateApiKey};
        use tower::ServiceExt as _;

        let context = common::TestContext::new().await;
        let admin = context
            .app
            .find_or_create_default_user()
            .await
            .expect("default administrator");
        let ordinary = context
            .app
            .create_user(
                &admin,
                "metrics-ordinary".into(),
                "ordinary-password".into(),
                AppPermissionMask::NONE,
                Vec::new(),
            )
            .await
            .expect("create ordinary actor");
        let ordinary = context
            .app
            .attach_user_authorization(ordinary)
            .await
            .expect("ordinary authorization");
        let api_key = |actor: &scryer_domain::User| {
            let app = context.app.clone();
            let actor = actor.clone();
            async move {
                app.create_api_key(
                    &actor,
                    CreateApiKey {
                        label: "prometheus".into(),
                        expiry: ApiKeyExpiryPreset::Never,
                    },
                )
                .await
                .expect("create API key")
                .raw_key
            }
        };
        let admin_key = api_key(&admin).await;
        let ordinary_key = api_key(&ordinary).await;
        let admin_session = context
            .app
            .issue_access_token(&admin)
            .await
            .expect("issue administrator session token");

        let recorder = prometheus_builder().build_recorder();
        with_local_recorder(&recorder, || {
            gauge!("scryer_metrics_route_probe").set(1.0);
        });
        let state = MetricsRouteState {
            auth: AuthState {
                app: context.app.clone(),
                schema: context.schema.clone(),
                // The shared fixture leaves authless local access on, which resolves a
                // default administrator for an anonymous request everywhere else. The
                // metrics route must still refuse it.
                auth_runtime: context.auth_runtime.clone(),
                rate_limiter: ScryerRateLimiter::from_env(),
                ws_origin_policy: WebSocketOriginPolicy::default(),
                authless_web_client_proof: AuthlessWebClientProofState::new(),
            },
            handle: recorder.handle(),
        };
        let router = Router::new()
            .route("/metrics", get(metrics_endpoint))
            .with_state(state);
        let respond = |token: Option<String>| {
            let router = router.clone();
            async move {
                let mut request = axum::http::Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("metrics request");
                if let Some(token) = token {
                    request.headers_mut().insert(
                        header::AUTHORIZATION,
                        HeaderValue::from_str(&format!("Bearer {token}")).expect("authorization"),
                    );
                }
                router.oneshot(request).await.expect("metrics response")
            }
        };

        assert_eq!(respond(None).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            respond(Some("scryer_not_a_real_key".into())).await.status(),
            StatusCode::UNAUTHORIZED
        );
        // A logged-in administrator is not a scraper: sessions are refused even with the
        // permission.
        assert_eq!(
            respond(Some(admin_session)).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            respond(Some(ordinary_key)).await.status(),
            StatusCode::FORBIDDEN
        );

        let response = respond(Some(admin_key)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("metrics body");
        let body = String::from_utf8(body.to_vec()).expect("utf-8 exposition");
        assert!(
            body.contains("scryer_metrics_route_probe 1"),
            "exposition should carry the recorded gauge: {body}"
        );
    }
}
