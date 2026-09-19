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

use std::time::Instant;

use tokio::sync::{Mutex, MutexGuard};

pub const WRITER_GATE_WAIT_SECONDS: &str = "scryer_datastore_writer_gate_wait_seconds";
pub const WRITER_GATE_HOLD_SECONDS: &str = "scryer_datastore_writer_gate_hold_seconds";
pub const WRITER_TRANSACTIONS_TOTAL: &str = "scryer_datastore_writer_transactions_total";

const OUTCOME_COMMITTED: &str = "committed";
const OUTCOME_ROLLED_BACK: &str = "rolled_back";

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
        let guard = gate.lock().await;
        let acquired_at = Instant::now();
        metrics::histogram!(WRITER_GATE_WAIT_SECONDS, "transaction" => transaction).record(
            acquired_at
                .saturating_duration_since(requested_at)
                .as_secs_f64(),
        );
        WriterGateHold {
            _guard: guard,
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
        metrics::histogram!(WRITER_GATE_HOLD_SECONDS, "transaction" => self.transaction)
            .record(self.acquired_at.elapsed().as_secs_f64());
        metrics::counter!(
            WRITER_TRANSACTIONS_TOTAL,
            "transaction" => self.transaction,
            "outcome" => self.outcome,
        )
        .increment(1);
    }
}
