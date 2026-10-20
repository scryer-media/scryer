//! Operation stage boundaries; never record arguments, results, or credentials.
//!
//! Every boundary is a `debug` event under target `scryer_auth_diagnostics`,
//! except a stage that ran for [`SLOW_STAGE_WARN_AFTER`] or more, which is
//! logged at `warn` when it returns or when its operation is dropped inside it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Identifies the last entered stage even when its future stops being polled.
pub struct DiagnosticTrace {
    id: u64,
    operation: &'static str,
    stage: &'static str,
    started: Instant,
    finished: bool,
}

impl DiagnosticTrace {
    pub fn new(operation: &'static str) -> Self {
        let trace = Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            operation,
            stage: "entered",
            started: Instant::now(),
            finished: false,
        };
        trace.emit("entered");
        trace
    }

    pub fn stage(&mut self, stage: &'static str) {
        self.emit("stage_returned");
        self.stage = stage;
        self.started = Instant::now();
        self.emit("stage_entered");
    }

    pub fn finish(&mut self) {
        self.finished = true;
        self.emit("returned");
    }

    fn emit(&self, event: &'static str) {
        let elapsed = self.started.elapsed();
        let elapsed_ms = elapsed.as_millis() as u64;
        // Levels are fixed per macro call, so a slow stage takes the second
        // call to reach the default `info` filter.
        if is_slow_stage_end(event, elapsed) {
            tracing::warn!(
                target: "scryer_auth_diagnostics",
                diagnostic_id = self.id,
                operation = self.operation,
                stage = self.stage,
                event,
                elapsed_ms,
                "slow operation stage"
            );
        } else {
            tracing::debug!(
                target: "scryer_auth_diagnostics",
                diagnostic_id = self.id,
                operation = self.operation,
                stage = self.stage,
                event,
                elapsed_ms,
                "scryer diagnostic boundary"
            );
        }
    }
}

/// A stage that took this long to return, or was still running when its
/// operation was dropped, is logged at `warn`. Everything else stays `debug`.
pub const SLOW_STAGE_WARN_AFTER: Duration = Duration::from_secs(5);

/// Only the events that close a stage carry a meaningful duration.
fn is_slow_stage_end(event: &str, elapsed: Duration) -> bool {
    matches!(event, "stage_returned" | "returned" | "scope_dropped")
        && elapsed >= SLOW_STAGE_WARN_AFTER
}

impl Drop for DiagnosticTrace {
    fn drop(&mut self) {
        if !self.finished {
            // Includes error returns and cancellation. Do not infer which.
            self.emit("scope_dropped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_slow_stage_end_is_escalated() {
        let slow = SLOW_STAGE_WARN_AFTER;
        let quick = SLOW_STAGE_WARN_AFTER - Duration::from_millis(1);
        for event in ["stage_returned", "returned", "scope_dropped"] {
            assert!(is_slow_stage_end(event, slow), "{event}");
            assert!(!is_slow_stage_end(event, quick), "{event}");
        }
        for event in ["entered", "stage_entered"] {
            assert!(!is_slow_stage_end(event, slow * 10), "{event}");
        }
    }
}
