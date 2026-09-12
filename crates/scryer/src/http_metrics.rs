//! HTTP serving metrics for the axum application.
//!
//! This module owns the outermost middleware layer on the application router. It answers the two
//! questions an operator asks first — "is Scryer slow right now?" and "what is erroring?" — for
//! every request the process serves, including the ones that never reach a handler (rate limited,
//! CORS rejected, unauthorised, UI fallback).
//!
//! Two label-cardinality rules keep the series count bounded and predictable:
//!
//! * **`route` is the axum route *template*, never the raw path.** [`MatchedPath`] is inserted into
//!   the request extensions by the router before layers added with `Router::layer` run, so the
//!   middleware sees `/images/titles/{title_id}/{kind}/{variant}` rather than one series per title
//!   id. Requests that matched no route (the UI fallback) are labelled [`FALLBACK_ROUTE`].
//! * **`method` is drawn from a fixed set.** `http::Method` accepts any RFC 7230 token, so an
//!   unrecognised method would otherwise let a client mint series at will; anything outside the
//!   standard methods is folded into `OTHER`.
//!
//! The in-flight gauge is maintained by a drop guard rather than by a matching `decrement` after
//! `next.run`, so a panicking handler or a client that disconnects mid-response (which drops the
//! middleware future) still leaves the gauge balanced.

use std::time::Instant;

use axum::body::Body;
use axum::extract::MatchedPath;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use metrics::{counter, gauge, histogram};

/// `route` label for requests that matched no route template (the UI fallback).
const FALLBACK_ROUTE: &str = "fallback";

/// `method` label for any method outside the standard set.
const OTHER_METHOD: &str = "OTHER";

/// Balances `scryer_http_requests_in_flight` across every exit path.
///
/// Held for the lifetime of the wrapped `next.run` future, so cancellation (client disconnect)
/// and panics decrement exactly once, just like a normal return.
struct InFlightGuard;

impl InFlightGuard {
    fn enter() -> Self {
        gauge!("scryer_http_requests_in_flight").increment(1.0);
        Self
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        gauge!("scryer_http_requests_in_flight").decrement(1.0);
    }
}

/// Balances `scryer_ws_connections` for one accepted GraphQL WebSocket connection.
///
/// Created inside the `on_upgrade` closure so it covers exactly the window in which a connection
/// is live, and dropped by every exit path — normal close, transport error, task cancellation at
/// shutdown — so the gauge cannot drift upwards over a long uptime.
pub(crate) struct WsConnectionGuard;

impl WsConnectionGuard {
    /// Counts one established connection and takes the live-connection gauge.
    pub(crate) fn accept() -> Self {
        counter!("scryer_ws_connections_total").increment(1);
        gauge!("scryer_ws_connections").increment(1.0);
        Self
    }
}

impl Drop for WsConnectionGuard {
    fn drop(&mut self) {
        gauge!("scryer_ws_connections").decrement(1.0);
    }
}

/// Folds a request method into the bounded `method` label set.
fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::CONNECT => "CONNECT",
        Method::OPTIONS => "OPTIONS",
        Method::TRACE => "TRACE",
        Method::PATCH => "PATCH",
        _ => OTHER_METHOD,
    }
}

/// Buckets a response status into its Prometheus-conventional class.
fn status_class(status: StatusCode) -> &'static str {
    match status.as_u16() / 100 {
        1 => "1xx",
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        5 => "5xx",
        // `StatusCode` cannot be constructed outside 100..=999, so this is only reachable for
        // non-standard 6xx-9xx codes produced by a proxy-shaped handler.
        _ => "other",
    }
}

/// Returns the route template the request matched, or [`FALLBACK_ROUTE`].
fn route_label(request: &Request<Body>) -> String {
    request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_owned())
        .unwrap_or_else(|| FALLBACK_ROUTE.to_owned())
}

/// Records serving metrics for one HTTP request.
///
/// Installed as the outermost layer on the application router so that it observes the final
/// status of every response, including responses produced by the rate-limit, authless-guard and
/// CORS layers rather than by a route handler.
pub(crate) async fn record_http_metrics(request: Request<Body>, next: Next) -> Response {
    let method = method_label(request.method());
    let route = route_label(&request);

    let in_flight = InFlightGuard::enter();
    let started = Instant::now();
    let response = next.run(request).await;
    let elapsed = started.elapsed().as_secs_f64();
    drop(in_flight);

    histogram!(
        "scryer_http_request_duration_seconds",
        "method" => method,
        "route" => route.clone(),
    )
    .record(elapsed);
    counter!(
        "scryer_http_requests_total",
        "method" => method,
        "route" => route,
        "status_class" => status_class(response.status()),
    )
    .increment(1);

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::Router;
    use axum::routing::get;
    use metrics::{Key, with_local_recorder};
    use metrics_util::MetricKind;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshot};
    use tower::ServiceExt;

    /// Finds one metric by kind, name and exact label set.
    fn find(
        snapshot: &[(
            metrics_util::CompositeKey,
            Option<metrics::Unit>,
            Option<metrics::SharedString>,
            DebugValue,
        )],
        kind: MetricKind,
        name: &str,
        labels: &[(&str, &str)],
    ) -> Option<DebugValue> {
        snapshot.iter().find_map(|(composite, _, _, value)| {
            if composite.kind() != kind {
                return None;
            }
            let key: &Key = composite.key();
            if key.name() != name {
                return None;
            }
            let actual: Vec<(String, String)> = key
                .labels()
                .map(|label| (label.key().to_owned(), label.value().to_owned()))
                .collect();
            let expected: Vec<(String, String)> = labels
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect();
            if actual != expected {
                return None;
            }
            Some(match value {
                DebugValue::Counter(count) => DebugValue::Counter(*count),
                DebugValue::Gauge(gauge) => DebugValue::Gauge(*gauge),
                DebugValue::Histogram(samples) => DebugValue::Histogram(samples.clone()),
            })
        })
    }

    /// Drives one request through a router carrying the middleware and returns the snapshot.
    ///
    /// A current-thread runtime keeps every emission on the thread that installed the local
    /// recorder; `oneshot` never spawns, so nothing escapes to another worker.
    fn snapshot_for_request(uri: &str) -> Snapshot {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime builds");

        with_local_recorder(&recorder, || {
            runtime.block_on(async {
                let app = Router::new()
                    .route("/items/{id}", get(|| async { "ok" }))
                    .fallback(get(|| async { "fallback" }))
                    .layer(axum::middleware::from_fn(record_http_metrics));

                let response = app
                    .oneshot(
                        Request::builder()
                            .uri(uri)
                            .body(Body::empty())
                            .expect("request builds"),
                    )
                    .await
                    .expect("router is infallible");
                assert_eq!(response.status(), StatusCode::OK);
            });
        });

        snapshotter.snapshot()
    }

    #[test]
    fn counts_matched_route_by_template() {
        let snapshot = snapshot_for_request("/items/abc123").into_vec();

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_http_requests_total",
                &[
                    ("method", "GET"),
                    ("route", "/items/{id}"),
                    ("status_class", "2xx"),
                ],
            ),
            Some(DebugValue::Counter(1)),
            "expected one 2xx count on the route template, got:\n{snapshot:?}"
        );

        let duration = find(
            &snapshot,
            MetricKind::Histogram,
            "scryer_http_request_duration_seconds",
            &[("method", "GET"), ("route", "/items/{id}")],
        )
        .unwrap_or_else(|| panic!("expected a duration sample, got:\n{snapshot:?}"));
        match duration {
            DebugValue::Histogram(samples) => assert_eq!(samples.len(), 1),
            other => panic!("expected a histogram, got {other:?}"),
        }
    }

    #[test]
    fn unmatched_requests_are_labelled_fallback() {
        let snapshot = snapshot_for_request("/no/such/route").into_vec();

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_http_requests_total",
                &[
                    ("method", "GET"),
                    ("route", "fallback"),
                    ("status_class", "2xx"),
                ],
            ),
            Some(DebugValue::Counter(1)),
            "expected the fallback route label, got:\n{snapshot:?}"
        );
    }

    #[test]
    fn in_flight_gauge_returns_to_zero() {
        let snapshot = snapshot_for_request("/items/abc123").into_vec();

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Gauge,
                "scryer_http_requests_in_flight",
                &[],
            ),
            Some(DebugValue::Gauge(0.0.into())),
            "in-flight gauge must return to zero, got:\n{snapshot:?}"
        );
    }

    // `Snapshotter::snapshot` resets every value it reads, so each of these takes exactly one
    // snapshot: the increment and the decrement have to be observed by separate recorders.

    #[test]
    fn in_flight_guard_increments_exactly_once_while_held() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        let held = with_local_recorder(&recorder, || {
            let guard = InFlightGuard::enter();
            let held = snapshotter.snapshot().into_vec();
            drop(guard);
            held
        });

        assert_eq!(
            find(
                &held,
                MetricKind::Gauge,
                "scryer_http_requests_in_flight",
                &[]
            ),
            Some(DebugValue::Gauge(1.0.into())),
            "guard must increment exactly once, got:\n{held:?}"
        );
    }

    #[test]
    fn in_flight_guard_decrements_exactly_once_on_drop() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        let after = with_local_recorder(&recorder, || {
            drop(InFlightGuard::enter());
            snapshotter.snapshot().into_vec()
        });

        assert_eq!(
            find(
                &after,
                MetricKind::Gauge,
                "scryer_http_requests_in_flight",
                &[],
            ),
            Some(DebugValue::Gauge(0.0.into())),
            "guard must decrement exactly once on drop, got:\n{after:?}"
        );
    }

    #[test]
    fn ws_connection_guard_balances_the_gauge() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        let held = with_local_recorder(&recorder, || {
            let guard = WsConnectionGuard::accept();
            let held = snapshotter.snapshot().into_vec();
            drop(guard);
            held
        });

        assert_eq!(
            find(&held, MetricKind::Gauge, "scryer_ws_connections", &[]),
            Some(DebugValue::Gauge(1.0.into())),
            "accepting a connection must increment exactly once, got:\n{held:?}"
        );
        assert_eq!(
            find(
                &held,
                MetricKind::Counter,
                "scryer_ws_connections_total",
                &[]
            ),
            Some(DebugValue::Counter(1)),
            "accepting a connection must be counted once, got:\n{held:?}"
        );

        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let after = with_local_recorder(&recorder, || {
            drop(WsConnectionGuard::accept());
            snapshotter.snapshot().into_vec()
        });

        assert_eq!(
            find(&after, MetricKind::Gauge, "scryer_ws_connections", &[]),
            Some(DebugValue::Gauge(0.0.into())),
            "closing a connection must decrement exactly once, got:\n{after:?}"
        );
    }

    #[test]
    fn status_classes_cover_every_range() {
        assert_eq!(status_class(StatusCode::CONTINUE), "1xx");
        assert_eq!(status_class(StatusCode::OK), "2xx");
        assert_eq!(status_class(StatusCode::FOUND), "3xx");
        assert_eq!(status_class(StatusCode::TOO_MANY_REQUESTS), "4xx");
        assert_eq!(status_class(StatusCode::INTERNAL_SERVER_ERROR), "5xx");
    }

    #[test]
    fn method_labels_are_bounded() {
        assert_eq!(method_label(&Method::GET), "GET");
        assert_eq!(method_label(&Method::PATCH), "PATCH");
        assert_eq!(
            method_label(&Method::from_bytes(b"WEIRD").expect("valid token")),
            "OTHER"
        );
    }
}
