//! Tells a starved async runtime apart from an exhausted connection pool.
//!
//! Both look the same from inside a request: sqlx reports a slow connection
//! acquire and a slow statement, because its timers run on the same runtime
//! and include the wait for a connection. This monitor samples the two causes
//! directly, once a second, without taking a connection.
//!
//! On sqlite it also names whoever keeps the single writer gate unavailable,
//! so a write stall shows up at the default log level without any filter
//! change.

use std::time::Duration;

use metrics::gauge;
use scryer_infrastructure_sql::runtime::{PoolUsage, StoreDatastore};
use scryer_infrastructure_sql::writer_gate::{
    WriterGateSnapshot, WriterGateStall, describe_writer_gate_stall,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
/// A one second tick that arrives this late means no worker thread was free to
/// poll it: blocking work is running on the async runtime.
const RUNTIME_STALL_WARN_LAG: Duration = Duration::from_secs(1);
/// A pool that is momentarily full is normal under a burst of reads.
const POOL_SATURATION_WARN_SAMPLES: u32 = 3;
const WARN_REPEAT_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
enum HealthWarning {
    RuntimeStalled { lag: Duration },
    PoolSaturated { samples: u32 },
    WriterGateStalled { stall: WriterGateStall },
}

#[derive(Default)]
struct HealthWatch {
    saturated_samples: u32,
    since_runtime_warning: Option<Duration>,
    since_pool_warning: Option<Duration>,
    since_writer_gate_warning: Option<Duration>,
}

impl HealthWatch {
    /// `elapsed` is the time since the previous sample; `writer_gate` is the
    /// sqlite writer-gate snapshot taken with it, read against `now`.
    fn observe(
        &mut self,
        elapsed: Duration,
        usage: PoolUsage,
        writer_gate: Option<&WriterGateSnapshot>,
        now: Instant,
    ) -> Vec<HealthWarning> {
        let mut warnings = Vec::new();
        for since in [
            &mut self.since_runtime_warning,
            &mut self.since_pool_warning,
            &mut self.since_writer_gate_warning,
        ] {
            if let Some(waited) = since {
                *waited += elapsed;
                if *waited >= WARN_REPEAT_INTERVAL {
                    *since = None;
                }
            }
        }

        let lag = elapsed.saturating_sub(SAMPLE_INTERVAL);
        if lag >= RUNTIME_STALL_WARN_LAG && self.since_runtime_warning.is_none() {
            self.since_runtime_warning = Some(Duration::ZERO);
            warnings.push(HealthWarning::RuntimeStalled { lag });
        }

        if usage.is_saturated() {
            self.saturated_samples += 1;
        } else {
            self.saturated_samples = 0;
        }
        if self.saturated_samples >= POOL_SATURATION_WARN_SAMPLES
            && self.since_pool_warning.is_none()
        {
            self.since_pool_warning = Some(Duration::ZERO);
            warnings.push(HealthWarning::PoolSaturated {
                samples: self.saturated_samples,
            });
        }

        if self.since_writer_gate_warning.is_none()
            && let Some(stall) =
                writer_gate.and_then(|snapshot| describe_writer_gate_stall(snapshot, now))
        {
            self.since_writer_gate_warning = Some(Duration::ZERO);
            warnings.push(HealthWarning::WriterGateStalled { stall });
        }
        warnings
    }
}

/// `name@secs` with whole seconds, the form the stall warning uses for every
/// transaction it names.
fn name_at_secs(transaction: &str, duration: Duration) -> String {
    format!("{transaction}@{}s", duration.as_secs())
}

fn warn_writer_gate_stalled(stall: &WriterGateStall) {
    let holder = stall.holder.as_ref().map_or_else(
        || "none".to_owned(),
        |holder| name_at_secs(holder.transaction, holder.held_for),
    );
    let last_released = stall.last_released.as_ref().map_or_else(
        || "none".to_owned(),
        |release| name_at_secs(release.transaction, release.ago),
    );
    let permit_handed_to = stall.permit_handed_to.as_ref().map_or_else(
        || "none".to_owned(),
        |waiter| name_at_secs(waiter.transaction, waiter.waited),
    );
    let oldest_waiters = stall
        .oldest_waiters
        .iter()
        .map(|waiter| name_at_secs(waiter.transaction, waiter.waited))
        .collect::<Vec<_>>()
        .join(",");
    tracing::warn!(
        shape = %stall.shape.as_str(),
        unavailable_secs = stall.unavailable_for.as_secs(),
        holder = %holder,
        last_released_ago = %last_released,
        permit_handed_to = %permit_handed_to,
        waiters = stall.waiter_count,
        oldest_waiters = %oldest_waiters,
        "sqlite writer gate stalled"
    );
}

pub(crate) fn describe_runtime_health_metrics() {
    metrics::describe_gauge!(
        "scryer_runtime_tick_lag_seconds",
        metrics::Unit::Seconds,
        "How late the last one second health tick was polled. Sustained lag means blocking work is starving the async runtime, which inflates every other latency the process reports."
    );
    metrics::describe_gauge!(
        "scryer_datastore_pool_open_connections",
        "Datastore connections currently open, idle or checked out."
    );
    metrics::describe_gauge!(
        "scryer_datastore_pool_idle_connections",
        "Open datastore connections nobody is using. Zero while open connections sit at the maximum means the next query waits."
    );
    metrics::describe_gauge!(
        "scryer_datastore_writer_gate_held",
        "1 while a write holds the sqlite single-writer gate, otherwise 0."
    );
}

pub(crate) async fn start_runtime_health_monitor(
    datastore: StoreDatastore,
    token: CancellationToken,
) {
    let mut ticks = tokio::time::interval(SAMPLE_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticks.tick().await;
    let mut last_sample = tokio::time::Instant::now();
    let mut watch = HealthWatch::default();

    loop {
        tokio::select! {
            _ = token.cancelled() => return,
            _ = ticks.tick() => {}
        }
        let now = tokio::time::Instant::now();
        let elapsed = now.duration_since(last_sample);
        last_sample = now;
        let usage = datastore.pool_usage();
        let writer_gate = datastore.writer_gate_snapshot();

        gauge!("scryer_runtime_tick_lag_seconds")
            .set(elapsed.saturating_sub(SAMPLE_INTERVAL).as_secs_f64());
        gauge!("scryer_datastore_pool_open_connections").set(f64::from(usage.open_connections));
        gauge!("scryer_datastore_pool_idle_connections").set(usage.idle_connections as f64);
        gauge!("scryer_datastore_writer_gate_held").set(if usage.writer_gate_held {
            1.0
        } else {
            0.0
        });

        for warning in watch.observe(elapsed, usage, writer_gate.as_ref(), now) {
            match warning {
                HealthWarning::RuntimeStalled { lag } => tracing::warn!(
                    lag_ms = lag.as_millis() as u64,
                    pool_open = usage.open_connections,
                    pool_idle = usage.idle_connections,
                    pool_max = usage.max_connections,
                    writer_gate_held = usage.writer_gate_held,
                    "async runtime stalled: a one second health tick ran late, so slow-query and slow-acquire timings around it are inflated"
                ),
                HealthWarning::PoolSaturated { samples } => tracing::warn!(
                    saturated_seconds = samples,
                    pool_max = usage.max_connections,
                    writer_gate_held = usage.writer_gate_held,
                    "datastore connection pool saturated: every connection is checked out and queries are waiting for one"
                ),
                HealthWarning::WriterGateStalled { stall } => warn_writer_gate_stalled(&stall),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_infrastructure_sql::writer_gate::{
        WriterGateHolder, WriterGateRelease, WriterGateWaiter,
    };

    fn usage(open: u32, idle: usize) -> PoolUsage {
        PoolUsage {
            max_connections: 16,
            open_connections: open,
            idle_connections: idle,
            writer_gate_held: false,
        }
    }

    /// A sample without a writer gate, as postgres takes them.
    fn observe_pool(
        watch: &mut HealthWatch,
        elapsed: Duration,
        usage: PoolUsage,
    ) -> Vec<HealthWarning> {
        watch.observe(elapsed, usage, None, Instant::now())
    }

    fn held_gate_usage() -> PoolUsage {
        PoolUsage {
            writer_gate_held: true,
            ..usage(4, 4)
        }
    }

    fn waiter(id: u64, transaction: &'static str, waiting_since: Instant) -> WriterGateWaiter {
        WriterGateWaiter {
            id,
            transaction,
            waiting_since,
        }
    }

    fn stalled_shapes(warnings: &[HealthWarning]) -> Vec<&'static str> {
        warnings
            .iter()
            .filter_map(|warning| match warning {
                HealthWarning::WriterGateStalled { stall } => Some(stall.shape.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_long_registered_hold_reports_a_held_gate() {
        let base = Instant::now();
        let snapshot = WriterGateSnapshot {
            available: false,
            holder: Some(WriterGateHolder {
                transaction: "fixture_background_write",
                acquired_at: base,
            }),
            last_released: None,
            waiters: vec![waiter(7, "fixture_login_cleanup", base)],
        };
        let mut watch = HealthWatch::default();
        let warnings = watch.observe(
            SAMPLE_INTERVAL,
            held_gate_usage(),
            Some(&snapshot),
            base + Duration::from_secs(15),
        );
        let [HealthWarning::WriterGateStalled { stall }] = warnings.as_slice() else {
            panic!("expected one writer gate warning, got {warnings:?}");
        };
        assert_eq!(stall.shape.as_str(), "held");
        assert_eq!(
            stall.holder.as_ref().map(|holder| holder.transaction),
            Some("fixture_background_write")
        );
        assert_eq!(stall.unavailable_for, Duration::from_secs(15));
        assert_eq!(stall.waiter_count, 1);
    }

    #[test]
    fn an_old_waiter_with_no_holder_reports_an_orphaned_permit() {
        let base = Instant::now();
        let snapshot = WriterGateSnapshot {
            available: false,
            holder: None,
            last_released: Some(WriterGateRelease {
                transaction: "fixture_background_write",
                released_at: base + Duration::from_secs(1),
                held_for: Duration::from_millis(40),
            }),
            waiters: vec![
                waiter(3, "fixture_login_cleanup", base),
                waiter(9, "fixture_passkey_cleanup", base + Duration::from_secs(20)),
            ],
        };
        let mut watch = HealthWatch::default();
        let warnings = watch.observe(
            SAMPLE_INTERVAL,
            held_gate_usage(),
            Some(&snapshot),
            base + Duration::from_secs(30),
        );
        let [HealthWarning::WriterGateStalled { stall }] = warnings.as_slice() else {
            panic!("expected one writer gate warning, got {warnings:?}");
        };
        assert_eq!(stall.shape.as_str(), "orphaned_permit");
        assert_eq!(stall.holder, None);
        assert_eq!(stall.unavailable_for, Duration::from_secs(29));
        assert_eq!(stall.waiter_count, 2);
        assert_eq!(
            stall
                .permit_handed_to
                .as_ref()
                .map(|waiter| (waiter.transaction, waiter.waited)),
            Some(("fixture_login_cleanup", Duration::from_secs(30)))
        );
        assert_eq!(
            stall
                .oldest_waiters
                .iter()
                .map(|waiter| name_at_secs(waiter.transaction, waiter.waited))
                .collect::<Vec<_>>(),
            vec!["fixture_login_cleanup@30s", "fixture_passkey_cleanup@10s"]
        );
    }

    #[test]
    fn a_brief_wait_on_the_writer_gate_is_quiet() {
        let base = Instant::now();
        let mut watch = HealthWatch::default();
        let busy = WriterGateSnapshot {
            available: false,
            holder: Some(WriterGateHolder {
                transaction: "fixture_background_write",
                acquired_at: base,
            }),
            last_released: None,
            waiters: vec![waiter(1, "fixture_login_cleanup", base)],
        };
        assert!(
            watch
                .observe(
                    SAMPLE_INTERVAL,
                    held_gate_usage(),
                    Some(&busy),
                    base + Duration::from_secs(3),
                )
                .is_empty()
        );
        // A release a moment ago with the next waiter not yet polled is the
        // ordinary hand-off, not an orphaned permit.
        let handing_off = WriterGateSnapshot {
            holder: None,
            last_released: Some(WriterGateRelease {
                transaction: "fixture_background_write",
                released_at: base + Duration::from_secs(29),
                held_for: Duration::from_secs(29),
            }),
            ..busy
        };
        assert!(
            watch
                .observe(
                    SAMPLE_INTERVAL,
                    held_gate_usage(),
                    Some(&handing_off),
                    base + Duration::from_secs(30),
                )
                .is_empty()
        );
    }

    #[test]
    fn a_stalled_writer_gate_is_reported_once_per_repeat_interval() {
        let base = Instant::now();
        let snapshot = WriterGateSnapshot {
            available: false,
            holder: Some(WriterGateHolder {
                transaction: "fixture_background_write",
                acquired_at: base,
            }),
            last_released: None,
            waiters: Vec::new(),
        };
        let mut watch = HealthWatch::default();
        let mut now = base + Duration::from_secs(10);
        let mut sample = |watch: &mut HealthWatch, elapsed: Duration| {
            now += elapsed;
            watch.observe(elapsed, held_gate_usage(), Some(&snapshot), now)
        };
        assert_eq!(
            stalled_shapes(&sample(&mut watch, Duration::ZERO)),
            ["held"]
        );
        for _ in 0..29 {
            assert!(sample(&mut watch, SAMPLE_INTERVAL).is_empty());
        }
        assert_eq!(
            stalled_shapes(&sample(&mut watch, SAMPLE_INTERVAL)),
            ["held"]
        );
        assert!(sample(&mut watch, SAMPLE_INTERVAL).is_empty());
    }

    #[test]
    fn an_on_time_tick_with_a_free_connection_is_quiet() {
        let mut watch = HealthWatch::default();
        assert!(observe_pool(&mut watch, SAMPLE_INTERVAL, usage(16, 1)).is_empty());
        assert!(observe_pool(&mut watch, Duration::from_millis(1_900), usage(4, 4)).is_empty());
    }

    #[test]
    fn a_late_tick_reports_the_runtime_once_per_repeat_interval() {
        let mut watch = HealthWatch::default();
        assert_eq!(
            observe_pool(&mut watch, Duration::from_secs(15), usage(4, 4)),
            vec![HealthWarning::RuntimeStalled {
                lag: Duration::from_secs(14)
            }]
        );
        assert!(observe_pool(&mut watch, Duration::from_secs(5), usage(4, 4)).is_empty());
        assert_eq!(
            observe_pool(&mut watch, Duration::from_secs(40), usage(4, 4)),
            vec![HealthWarning::RuntimeStalled {
                lag: Duration::from_secs(39)
            }]
        );
    }

    #[test]
    fn only_a_sustained_full_pool_is_reported() {
        let mut watch = HealthWatch::default();
        assert!(observe_pool(&mut watch, SAMPLE_INTERVAL, usage(16, 0)).is_empty());
        assert!(observe_pool(&mut watch, SAMPLE_INTERVAL, usage(16, 0)).is_empty());
        assert!(observe_pool(&mut watch, SAMPLE_INTERVAL, usage(16, 2)).is_empty());
        assert!(observe_pool(&mut watch, SAMPLE_INTERVAL, usage(16, 0)).is_empty());
        assert!(observe_pool(&mut watch, SAMPLE_INTERVAL, usage(16, 0)).is_empty());
        assert_eq!(
            observe_pool(&mut watch, SAMPLE_INTERVAL, usage(16, 0)),
            vec![HealthWarning::PoolSaturated { samples: 3 }]
        );
        assert!(observe_pool(&mut watch, SAMPLE_INTERVAL, usage(16, 0)).is_empty());
    }

    #[test]
    fn a_pool_below_its_maximum_is_never_saturated() {
        let mut watch = HealthWatch::default();
        for _ in 0..5 {
            assert!(observe_pool(&mut watch, SAMPLE_INTERVAL, usage(8, 0)).is_empty());
        }
    }
}
