//! Instrumentation for the single sqlite writer gate.
//!
//! On sqlite every write serializes behind one `tokio::sync::Mutex` (the
//! writer gate), so throughput is bounded by how long each holder keeps it and
//! how long the queue behind it waits. The only pre-existing signal was the
//! instantaneous `scryer_datastore_writer_gate_held` gauge, which cannot tell
//! a gate that is briefly busy from a gate that is saturated.
//!
//! [`WriterGateHold`] wraps the lock itself rather than any one call site, so
//! every entry point that serializes on the gate is covered by construction.
//! The cost is two `Instant::now()` per acquisition.
//!
//! Labels come from the `&'static str` operation names the runtime already
//! threads through (`"replace_title_more_like_this_items"` and friends), never
//! from SQL text or data, so the label set is bounded by the code.
//!
//! Postgres does not take the gate and is not instrumented here; its
//! concurrency is the connection pool, which `scryer_datastore_pool_*` covers.

use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, MutexGuard};
use tokio::time::Instant;

pub const WRITER_GATE_WAIT_SECONDS: &str = "scryer_datastore_writer_gate_wait_seconds";
pub const WRITER_GATE_HOLD_SECONDS: &str = "scryer_datastore_writer_gate_hold_seconds";
pub const WRITER_TRANSACTIONS_TOTAL: &str = "scryer_datastore_writer_transactions_total";

const OUTCOME_COMMITTED: &str = "committed";
const OUTCOME_ROLLED_BACK: &str = "rolled_back";

/// How long the gate may stay with one holder, or stay unavailable with no
/// holder, before it counts as stalled. Ordinary writes hold it for
/// milliseconds and the busy-retry ladder gives up well inside two minutes, so
/// ten seconds is far outside healthy behaviour while still surfacing a stall
/// in the first log lines after boot. A hold that reaches it is also logged at
/// `warn` when it ends.
pub const WRITER_GATE_STALL_THRESHOLD: Duration = Duration::from_secs(10);

/// A stall report names at most this many waiters, oldest first, and carries
/// the total separately.
pub const WRITER_GATE_STALL_REPORTED_WAITERS: usize = 8;

/// Who holds, last released and waits on each writer gate, keyed by the
/// gate's address. Production has one gate for the life of the process, so
/// the map stays one entry long. The lock is never held across an await and
/// only for a few map operations.
static REGISTRY: StdMutex<BTreeMap<usize, GateRecord>> = StdMutex::new(BTreeMap::new());
static NEXT_WAITER_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
struct GateRecord {
    holder: Option<WriterGateHolder>,
    last_released: Option<WriterGateRelease>,
    waiters: BTreeMap<u64, WriterGateWaiter>,
}

fn with_gate_record<R>(gate_id: usize, update: impl FnOnce(&mut GateRecord) -> R) -> R {
    let mut registry = REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    update(registry.entry(gate_id).or_default())
}

fn registry_key(gate: &Mutex<()>) -> usize {
    std::ptr::from_ref(gate) as usize
}

/// The transaction holding a writer gate, as registered when it acquired it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterGateHolder {
    pub transaction: &'static str,
    pub acquired_at: Instant,
}

/// The most recent hold that ended on a writer gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterGateRelease {
    pub transaction: &'static str,
    pub released_at: Instant,
    pub held_for: Duration,
}

/// A transaction that asked for a writer gate and has not yet been polled to
/// the point of holding it. The id grows in registration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterGateWaiter {
    pub id: u64,
    pub transaction: &'static str,
    pub waiting_since: Instant,
}

/// A point-in-time view of one writer gate. Taking it never waits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterGateSnapshot {
    /// The gate could be locked at the moment of the snapshot.
    pub available: bool,
    pub holder: Option<WriterGateHolder>,
    pub last_released: Option<WriterGateRelease>,
    /// Oldest first.
    pub waiters: Vec<WriterGateWaiter>,
}

/// Reads the registry for `gate`.
///
/// `available` comes from `try_lock`, which takes and immediately returns the
/// permit when it is free, exactly as the pool-usage sample already does.
pub fn writer_gate_snapshot(gate: &Mutex<()>) -> WriterGateSnapshot {
    let available = gate.try_lock().is_ok();
    let (holder, last_released, mut waiters) = with_gate_record(registry_key(gate), |record| {
        (
            record.holder.clone(),
            record.last_released.clone(),
            record.waiters.values().cloned().collect::<Vec<_>>(),
        )
    });
    waiters.sort_by_key(|waiter| (waiter.waiting_since, waiter.id));
    WriterGateSnapshot {
        available,
        holder,
        last_released,
        waiters,
    }
}

/// How a stalled gate is stuck.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriterGateStallShape {
    /// A registered holder has kept the gate past the threshold: its
    /// transaction is parked or slow while holding it.
    Held,
    /// Nobody is registered as holding the gate, yet it is unavailable and a
    /// waiter has queued past the threshold. Tokio's mutex is fair: a release
    /// hands the permit to the front of the queue even when that waiter's
    /// future is never polled again, so the gate stays taken by a waiter that
    /// never finishes acquiring it.
    OrphanedPermit,
}

impl WriterGateStallShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::OrphanedPermit => "orphaned_permit",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StalledHolder {
    pub transaction: &'static str,
    pub held_for: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleasedBeforeStall {
    pub transaction: &'static str,
    pub ago: Duration,
    pub held_for: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StalledWaiter {
    pub id: u64,
    pub transaction: &'static str,
    pub waited: Duration,
}

/// Why a writer gate counts as stalled, in terms an operator can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterGateStall {
    pub shape: WriterGateStallShape,
    /// How long the gate has been unavailable: since the holder acquired it,
    /// or since the last release when nobody holds it.
    pub unavailable_for: Duration,
    pub holder: Option<StalledHolder>,
    pub last_released: Option<ReleasedBeforeStall>,
    pub waiter_count: usize,
    /// At most [`WRITER_GATE_STALL_REPORTED_WAITERS`], oldest first.
    pub oldest_waiters: Vec<StalledWaiter>,
    /// Orphaned permit only: the oldest waiter registered before the last
    /// release, so the release handed the permit to it.
    pub permit_handed_to: Option<StalledWaiter>,
}

/// Decides whether `snapshot` shows a stalled gate at `now`.
///
/// A gate is stalled when it is unavailable and either its registered holder
/// has held it for [`WRITER_GATE_STALL_THRESHOLD`] or more, or nobody holds it
/// and both the oldest waiter and the time since the last release have reached
/// the threshold. Requiring the release to be old as well keeps the ordinary
/// hand-off (released a moment ago, next waiter about to be polled) quiet.
pub fn describe_writer_gate_stall(
    snapshot: &WriterGateSnapshot,
    now: Instant,
) -> Option<WriterGateStall> {
    if snapshot.available {
        return None;
    }
    let waited = |waiter: &WriterGateWaiter| StalledWaiter {
        id: waiter.id,
        transaction: waiter.transaction,
        waited: now.saturating_duration_since(waiter.waiting_since),
    };
    let since_release = snapshot
        .last_released
        .as_ref()
        .map(|release| now.saturating_duration_since(release.released_at));
    let oldest = snapshot.waiters.first();

    let (shape, unavailable_for, holder, permit_handed_to) = match &snapshot.holder {
        Some(holder) => {
            let held_for = now.saturating_duration_since(holder.acquired_at);
            if held_for < WRITER_GATE_STALL_THRESHOLD {
                return None;
            }
            (
                WriterGateStallShape::Held,
                held_for,
                Some(StalledHolder {
                    transaction: holder.transaction,
                    held_for,
                }),
                None,
            )
        }
        None => {
            let oldest = oldest?;
            let oldest_wait = now.saturating_duration_since(oldest.waiting_since);
            let unavailable_for = since_release.unwrap_or(oldest_wait);
            if oldest_wait < WRITER_GATE_STALL_THRESHOLD
                || unavailable_for < WRITER_GATE_STALL_THRESHOLD
            {
                return None;
            }
            let handed_to = snapshot
                .last_released
                .as_ref()
                .filter(|release| oldest.waiting_since <= release.released_at)
                .map(|_| waited(oldest));
            (
                WriterGateStallShape::OrphanedPermit,
                unavailable_for,
                None,
                handed_to,
            )
        }
    };

    Some(WriterGateStall {
        shape,
        unavailable_for,
        holder,
        last_released: snapshot
            .last_released
            .as_ref()
            .zip(since_release)
            .map(|(release, ago)| ReleasedBeforeStall {
                transaction: release.transaction,
                ago,
                held_for: release.held_for,
            }),
        waiter_count: snapshot.waiters.len(),
        oldest_waiters: snapshot
            .waiters
            .iter()
            .take(WRITER_GATE_STALL_REPORTED_WAITERS)
            .map(waited)
            .collect(),
        permit_handed_to,
    })
}

/// Keeps a waiter in the registry for exactly as long as its acquire future
/// is waiting: removed when it acquires, or when the future is dropped first.
struct WaiterRegistration {
    gate_id: usize,
    waiter_id: u64,
    acquired: bool,
}

impl WaiterRegistration {
    fn register(gate_id: usize, transaction: &'static str, waiting_since: Instant) -> Self {
        let waiter_id = NEXT_WAITER_ID.fetch_add(1, Ordering::Relaxed);
        with_gate_record(gate_id, |record| {
            record.waiters.insert(
                waiter_id,
                WriterGateWaiter {
                    id: waiter_id,
                    transaction,
                    waiting_since,
                },
            );
        });
        Self {
            gate_id,
            waiter_id,
            acquired: false,
        }
    }

    fn acquired(mut self, transaction: &'static str, acquired_at: Instant) {
        with_gate_record(self.gate_id, |record| {
            record.waiters.remove(&self.waiter_id);
            record.holder = Some(WriterGateHolder {
                transaction,
                acquired_at,
            });
        });
        self.acquired = true;
    }
}

impl Drop for WaiterRegistration {
    fn drop(&mut self) {
        if !self.acquired {
            with_gate_record(self.gate_id, |record| {
                record.waiters.remove(&self.waiter_id);
            });
        }
    }
}

/// Registers HELP/UNIT metadata for the writer-gate families.
///
/// Public because the binary's metrics setup calls it once at startup, after
/// the recorder is installed, so `/metrics` is self-describing even before the
/// first write and on deployments that run postgres (where the families stay
/// empty).
pub fn describe_writer_gate_metrics() {
    metrics::describe_histogram!(
        WRITER_GATE_WAIT_SECONDS,
        metrics::Unit::Seconds,
        "Time a sqlite write waited for the single writer gate, from requesting it to holding it, \
         by transaction name. Rising wait with flat hold means the queue behind the gate is the \
         bottleneck."
    );
    metrics::describe_histogram!(
        WRITER_GATE_HOLD_SECONDS,
        metrics::Unit::Seconds,
        "Time one sqlite write held the writer gate, from acquiring it to releasing it, including \
         the commit and any busy retries, by transaction name. This is the service time every \
         other writer queues behind."
    );
    metrics::describe_counter!(
        WRITER_TRANSACTIONS_TOTAL,
        "Sqlite writes that took the writer gate, by transaction name and outcome (`committed` \
         when the operation returned success, `rolled_back` otherwise)."
    );
}

/// A held writer gate that reports its wait, its hold and its outcome.
///
/// The outcome starts at `rolled_back` and is promoted by [`Self::committed`]
/// only on a successful return, so an early `?`, a panic or a dropped future
/// is still counted — as a rollback, which is what it is.
pub struct WriterGateHold<'gate> {
    _guard: MutexGuard<'gate, ()>,
    gate_id: usize,
    transaction: &'static str,
    acquired_at: Instant,
    outcome: &'static str,
}

impl<'gate> WriterGateHold<'gate> {
    /// Waits for the gate, recording the wait, and returns the held guard.
    pub async fn acquire(
        gate: &'gate Mutex<()>,
        transaction: &'static str,
    ) -> WriterGateHold<'gate> {
        let requested_at = Instant::now();
        let gate_id = registry_key(gate);
        tracing::debug!(target: "scryer_auth_diagnostics", gate_id, transaction, event = "writer_wait", "scryer writer boundary");
        let registration = WaiterRegistration::register(gate_id, transaction, requested_at);
        let guard = gate.lock().await;
        tracing::debug!(target: "scryer_auth_diagnostics", gate_id, transaction, event = "writer_acquired", "scryer writer boundary");
        let acquired_at = Instant::now();
        registration.acquired(transaction, acquired_at);
        metrics::histogram!(WRITER_GATE_WAIT_SECONDS, "transaction" => transaction).record(
            acquired_at
                .saturating_duration_since(requested_at)
                .as_secs_f64(),
        );
        WriterGateHold {
            _guard: guard,
            gate_id,
            transaction,
            acquired_at,
            outcome: OUTCOME_ROLLED_BACK,
        }
    }

    /// Marks this hold as having committed; call it once the operation has
    /// returned success.
    pub fn committed(&mut self) {
        self.outcome = OUTCOME_COMMITTED;
    }
}

impl Drop for WriterGateHold<'_> {
    fn drop(&mut self) {
        tracing::debug!(target: "scryer_auth_diagnostics", gate_id = self.gate_id, transaction = self.transaction, event = "writer_releasing", "scryer writer boundary");
        let released_at = Instant::now();
        let held_for = released_at.saturating_duration_since(self.acquired_at);
        if held_for >= WRITER_GATE_STALL_THRESHOLD {
            tracing::warn!(
                transaction = %self.transaction,
                held_secs = held_for.as_secs(),
                outcome = %self.outcome,
                "sqlite writer gate held for a long time"
            );
        }
        metrics::histogram!(WRITER_GATE_HOLD_SECONDS, "transaction" => self.transaction)
            .record(held_for.as_secs_f64());
        metrics::counter!(
            WRITER_TRANSACTIONS_TOTAL,
            "transaction" => self.transaction,
            "outcome" => self.outcome,
        )
        .increment(1);
        // Last, with nothing that can block left between it and the `_guard`
        // drop that follows this body: the registry says "released" only
        // once the permit is about to move on, and never after the next
        // holder could have registered itself.
        with_gate_record(self.gate_id, |record| {
            record.holder = None;
            record.last_released = Some(WriterGateRelease {
                transaction: self.transaction,
                released_at,
                held_for,
            });
        });
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};
    use tracing::{
        Event, Metadata, Subscriber,
        field::{Field, Visit},
        span::{Attributes, Id, Record},
    };

    #[derive(Clone, Default)]
    struct Capture(Arc<StdMutex<Vec<Vec<(String, String)>>>>);

    struct Fields(Vec<(String, String)>);
    impl Visit for Fields {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0.push((field.name().into(), format!("{value:?}")));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.push((field.name().into(), value.into()));
        }
    }
    impl Subscriber for Capture {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
        fn event(&self, event: &Event<'_>) {
            let metadata = event.metadata();
            let mut fields = Fields(vec![
                ("level".into(), metadata.level().to_string()),
                ("target".into(), metadata.target().into()),
            ]);
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
    }

    #[tokio::test]
    async fn diagnostic_trace_identifies_owner_while_authentication_waits() {
        use std::{
            future::{Future, poll_fn},
            task::Poll,
        };
        let capture = Capture::default();
        let _subscriber = tracing::subscriber::set_default(capture.clone());
        let gate = Mutex::new(());
        let owner = WriterGateHold::acquire(&gate, "fixture_background_write").await;
        let mut waiting = Box::pin(WriterGateHold::acquire(
            &gate,
            "delete_expired_webauthn_challenges",
        ));
        poll_fn(|cx| {
            assert!(waiting.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let events = capture.0.lock().unwrap().clone();
        let has = |event: &str, operation: &str| {
            events.iter().any(|fields| {
                fields.contains(&("event".into(), event.into()))
                    && fields.contains(&("transaction".into(), operation.into()))
            })
        };
        assert!(has("writer_acquired", "fixture_background_write"));
        assert!(has("writer_wait", "delete_expired_webauthn_challenges"));
        assert!(!has(
            "writer_acquired",
            "delete_expired_webauthn_challenges"
        ));
        assert!(!has("writer_releasing", "fixture_background_write"));
        drop(owner);
        let authentication = tokio::time::timeout(std::time::Duration::from_secs(30), waiting)
            .await
            .unwrap();
        drop(authentication);
        assert!(gate.try_lock().is_ok());
    }

    /// Polls `future` exactly once and asserts it is still waiting.
    async fn poll_once_pending<F: std::future::Future + Unpin>(future: &mut F) {
        std::future::poll_fn(|cx| {
            assert!(std::pin::Pin::new(&mut *future).poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }

    const FAILURE_BOUND: Duration = Duration::from_secs(30);

    #[tokio::test(start_paused = true)]
    async fn a_released_permit_handed_to_an_unpolled_waiter_reports_an_orphaned_permit() {
        let gate = Mutex::new(());
        let holder = WriterGateHold::acquire(&gate, "fixture_background_write").await;
        let mut orphan = Box::pin(WriterGateHold::acquire(&gate, "fixture_login_cleanup"));
        poll_once_pending(&mut orphan).await;
        let registered_at = Instant::now();

        let while_held = writer_gate_snapshot(&gate);
        assert_eq!(
            while_held.holder.as_ref().map(|holder| holder.transaction),
            Some("fixture_background_write")
        );
        assert_eq!(while_held.waiters.len(), 1);

        tokio::time::advance(Duration::from_secs(2)).await;
        // The fair mutex assigns the permit to the queued waiter here, but the
        // waiter is never polled, so it never registers as the holder.
        drop(holder);
        let released_at = Instant::now();

        let orphaned = writer_gate_snapshot(&gate);
        assert!(!orphaned.available);
        assert_eq!(orphaned.holder, None);
        assert_eq!(orphaned.waiters.len(), 1);
        let waiter = orphaned.waiters[0].clone();
        assert_eq!(waiter.transaction, "fixture_login_cleanup");
        assert_eq!(waiter.waiting_since, registered_at);
        assert_eq!(
            orphaned.last_released,
            Some(WriterGateRelease {
                transaction: "fixture_background_write",
                released_at,
                held_for: Duration::from_secs(2),
            })
        );
        assert_eq!(describe_writer_gate_stall(&orphaned, Instant::now()), None);

        tokio::time::advance(WRITER_GATE_STALL_THRESHOLD + Duration::from_secs(1)).await;
        let stalled = writer_gate_snapshot(&gate);
        assert_eq!(stalled.waiters, vec![waiter.clone()]);
        let stall = describe_writer_gate_stall(&stalled, Instant::now()).expect("stalled");
        let expected_waiter = StalledWaiter {
            id: waiter.id,
            transaction: "fixture_login_cleanup",
            waited: Duration::from_secs(13),
        };
        assert_eq!(
            stall,
            WriterGateStall {
                shape: WriterGateStallShape::OrphanedPermit,
                unavailable_for: Duration::from_secs(11),
                holder: None,
                last_released: Some(ReleasedBeforeStall {
                    transaction: "fixture_background_write",
                    ago: Duration::from_secs(11),
                    held_for: Duration::from_secs(2),
                }),
                waiter_count: 1,
                oldest_waiters: vec![expected_waiter.clone()],
                permit_handed_to: Some(expected_waiter),
            }
        );
        assert_eq!(stall.shape.as_str(), "orphaned_permit");

        let late = tokio::time::timeout(FAILURE_BOUND, orphan)
            .await
            .expect("the assigned permit completes the acquire on its next poll");
        let acquired = writer_gate_snapshot(&gate);
        assert_eq!(
            acquired.holder.as_ref().map(|holder| holder.transaction),
            Some("fixture_login_cleanup")
        );
        assert!(acquired.waiters.is_empty());
        drop(late);

        let released = writer_gate_snapshot(&gate);
        assert!(released.available);
        assert_eq!(released.holder, None);
        assert!(released.waiters.is_empty());
        assert_eq!(
            released
                .last_released
                .as_ref()
                .map(|release| release.transaction),
            Some("fixture_login_cleanup")
        );
        assert_eq!(describe_writer_gate_stall(&released, Instant::now()), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_long_registered_hold_reports_the_holder() {
        let gate = Mutex::new(());
        let holder = WriterGateHold::acquire(&gate, "fixture_background_write").await;
        let mut waiter = Box::pin(WriterGateHold::acquire(&gate, "fixture_login_cleanup"));
        poll_once_pending(&mut waiter).await;

        tokio::time::advance(WRITER_GATE_STALL_THRESHOLD - Duration::from_secs(1)).await;
        assert_eq!(
            describe_writer_gate_stall(&writer_gate_snapshot(&gate), Instant::now()),
            None
        );

        tokio::time::advance(Duration::from_secs(1)).await;
        let stall = describe_writer_gate_stall(&writer_gate_snapshot(&gate), Instant::now())
            .expect("stalled");
        assert_eq!(stall.shape, WriterGateStallShape::Held);
        assert_eq!(
            stall.holder,
            Some(StalledHolder {
                transaction: "fixture_background_write",
                held_for: WRITER_GATE_STALL_THRESHOLD,
            })
        );
        assert_eq!(stall.unavailable_for, WRITER_GATE_STALL_THRESHOLD);
        assert_eq!(stall.waiter_count, 1);
        assert_eq!(stall.permit_handed_to, None);

        drop(holder);
        drop(
            tokio::time::timeout(FAILURE_BOUND, waiter)
                .await
                .expect("the waiter acquires after the release"),
        );
        assert!(gate.try_lock().is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_acquire_leaves_the_waiter_list() {
        let gate = Mutex::new(());
        let holder = WriterGateHold::acquire(&gate, "fixture_background_write").await;
        let mut waiter = Box::pin(WriterGateHold::acquire(&gate, "fixture_login_cleanup"));
        poll_once_pending(&mut waiter).await;
        assert_eq!(writer_gate_snapshot(&gate).waiters.len(), 1);

        drop(waiter);
        assert!(writer_gate_snapshot(&gate).waiters.is_empty());

        drop(holder);
        let after = writer_gate_snapshot(&gate);
        assert!(after.available);
        assert_eq!(after.holder, None);
    }

    #[test]
    fn a_stall_report_names_only_the_oldest_waiters() {
        let base = Instant::now();
        let waiters = (0..12u64)
            .map(|id| WriterGateWaiter {
                id,
                transaction: "fixture_queued_write",
                waiting_since: base + Duration::from_secs(id),
            })
            .collect::<Vec<_>>();
        let snapshot = WriterGateSnapshot {
            available: false,
            holder: Some(WriterGateHolder {
                transaction: "fixture_background_write",
                acquired_at: base,
            }),
            last_released: None,
            waiters,
        };
        let stall =
            describe_writer_gate_stall(&snapshot, base + Duration::from_secs(60)).expect("stalled");
        assert_eq!(stall.waiter_count, 12);
        assert_eq!(
            stall
                .oldest_waiters
                .iter()
                .map(|waiter| waiter.id)
                .collect::<Vec<_>>(),
            (0..WRITER_GATE_STALL_REPORTED_WAITERS as u64).collect::<Vec<_>>()
        );
        assert_eq!(stall.last_released, None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hold_past_the_threshold_warns_when_it_ends() {
        let capture = Capture::default();
        let _subscriber = tracing::subscriber::set_default(capture.clone());
        let gate = Mutex::new(());

        let brief = WriterGateHold::acquire(&gate, "fixture_quick_write").await;
        tokio::time::advance(WRITER_GATE_STALL_THRESHOLD - Duration::from_secs(1)).await;
        drop(brief);

        let mut slow = WriterGateHold::acquire(&gate, "fixture_slow_write").await;
        tokio::time::advance(WRITER_GATE_STALL_THRESHOLD + Duration::from_secs(2)).await;
        slow.committed();
        drop(slow);

        let warnings = capture
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|fields| fields.contains(&("level".into(), "WARN".into())))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let warning = &warnings[0];
        assert!(warning.contains(&("transaction".into(), "fixture_slow_write".into())));
        assert!(warning.contains(&("held_secs".into(), "12".into())));
        assert!(warning.contains(&("outcome".into(), "committed".into())));
    }
}
