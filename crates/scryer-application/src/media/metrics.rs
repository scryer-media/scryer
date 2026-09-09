//! Probe telemetry uses bounded operation/status labels, never media names or payloads.
use metrics::{
    Unit, counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram,
};
use scryer_media_types::{ProbeReport, ProbeStatus};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(crate) enum ProbeOperation {
    Catalog,
    Import,
    Discovery,
    Diagnostics,
}
impl ProbeOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::Import => "import",
            Self::Discovery => "discovery",
            Self::Diagnostics => "diagnostics",
        }
    }
}

pub(crate) fn record_probe(
    operation: ProbeOperation,
    elapsed: Duration,
    report: Option<&ProbeReport>,
) {
    let operation = operation.label();
    let status = match report.map(|report| report.status) {
        None => "error",
        Some(ProbeStatus::Unknown) => "unknown",
        Some(ProbeStatus::Complete) => "complete",
        Some(ProbeStatus::Incomplete) => "incomplete",
        Some(ProbeStatus::Unsupported) => "unsupported",
        Some(ProbeStatus::Encrypted) => "encrypted",
        Some(ProbeStatus::Malformed) => "malformed",
    };
    counter!("scryer_media_probes_total", "operation" => operation, "status" => status)
        .increment(1);
    histogram!("scryer_media_probe_duration_seconds", "operation" => operation)
        .record(elapsed.as_secs_f64());
    if matches!(status, "error" | "malformed") {
        counter!("scryer_media_probe_failures_total", "operation" => operation).increment(1);
    }
    if let Some(report) = report {
        counter!("scryer_media_probe_bytes_read_total", "operation" => operation)
            .increment(report.bytes_read);
        if report.status != ProbeStatus::Complete {
            counter!("scryer_media_probe_incomplete_total", "operation" => operation).increment(1);
        }
        if report.budget_exhausted {
            counter!("scryer_media_probe_budget_exhausted_total", "operation" => operation)
                .increment(1);
        }
    }
}

pub(crate) struct RefreshFlight;
impl RefreshFlight {
    pub(crate) fn begin() -> Self {
        gauge!("scryer_media_refresh_in_flight").increment(1.0);
        Self
    }
}
impl Drop for RefreshFlight {
    fn drop(&mut self) {
        gauge!("scryer_media_refresh_in_flight").decrement(1.0);
    }
}

#[derive(Default)]
pub(crate) struct RefreshProgress {
    pub(crate) updated: u32,
    pub(crate) failed: u32,
}
impl Drop for RefreshProgress {
    fn drop(&mut self) {
        counter!("scryer_media_refresh_files_total", "outcome" => "published")
            .increment(u64::from(self.updated));
        counter!("scryer_media_refresh_files_total", "outcome" => "failed")
            .increment(u64::from(self.failed));
    }
}

pub(crate) fn describe() {
    describe_counter!(
        "scryer_media_probes_total",
        "Completed native media inspections by operation and result status."
    );
    describe_histogram!(
        "scryer_media_probe_duration_seconds",
        Unit::Seconds,
        "Native media inspection wall time."
    );
    describe_counter!(
        "scryer_media_probe_bytes_read_total",
        Unit::Bytes,
        "Bytes reported read by completed media inspections; excludes errors without a report."
    );
    describe_counter!(
        "scryer_media_probe_failures_total",
        "Media inspection errors and malformed results."
    );
    describe_counter!(
        "scryer_media_probe_incomplete_total",
        "Media inspections without complete results, including unsupported and encrypted media."
    );
    describe_counter!(
        "scryer_media_probe_budget_exhausted_total",
        "Media inspections that exhausted a read or metadata budget."
    );
    describe_gauge!(
        "scryer_media_refresh_in_flight",
        "Stale metadata inspections currently in progress."
    );
    describe_counter!(
        "scryer_media_refresh_files_total",
        "Stale metadata attempts persisted, by outcome; source races are excluded."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    #[test]
    fn probe_metrics_distinguish_incomplete_results_and_errors_without_file_labels() {
        let recorder = DebuggingRecorder::new();
        let snapshot = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            record_probe(
                ProbeOperation::Import,
                Duration::from_millis(25),
                Some(&ProbeReport {
                    status: ProbeStatus::Incomplete,
                    bytes_read: 8192,
                    budget_exhausted: true,
                    ..Default::default()
                }),
            );
            record_probe(ProbeOperation::Import, Duration::from_millis(10), None);
            {
                let _flight = RefreshFlight::begin();
            }
            {
                let _progress = RefreshProgress {
                    updated: 2,
                    failed: 1,
                };
            }
        });
        let values = snapshot.snapshot().into_vec();
        for (key, _, _, _) in &values {
            assert!(
                key.key()
                    .labels()
                    .all(|label| matches!(label.key(), "operation" | "status" | "outcome"))
            );
        }
        let counter_value = |name| {
            values
                .iter()
                .filter(|(key, _, _, _)| key.key().name() == name)
                .map(|(_, _, _, value)| match value {
                    DebugValue::Counter(value) => *value,
                    _ => panic!("expected counter"),
                })
                .sum::<u64>()
        };
        assert_eq!(counter_value("scryer_media_probes_total"), 2);
        assert_eq!(counter_value("scryer_media_probe_bytes_read_total"), 8192);
        assert_eq!(counter_value("scryer_media_probe_incomplete_total"), 1);
        assert_eq!(counter_value("scryer_media_probe_failures_total"), 1);
        assert_eq!(
            counter_value("scryer_media_probe_budget_exhausted_total"),
            1
        );
        assert_eq!(counter_value("scryer_media_refresh_files_total"), 3);
        assert!(values.iter().any(|(key, _, _, value)| key.key().name()
            == "scryer_media_refresh_in_flight"
            && matches!(value, DebugValue::Gauge(value) if value.into_inner() == 0.0)));
    }
}
