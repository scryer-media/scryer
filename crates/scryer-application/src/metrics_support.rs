//! Gauge families that answer "when did this last happen?" and "is this
//! healthy right now?".
//!
//! Counters tell an operator how often something ran; they cannot tell them
//! that the RSS sync silently stopped six hours ago, because a counter that
//! stops moving looks exactly like a counter nobody scraped. The freshness
//! gauges here carry the wall-clock timestamp of the last run, the last
//! success and the next scheduled run, so `time() - metric` is an alert.
//!
//! Label discipline matches the rest of the crate: every label value is either
//! a `&'static str` from a bounded enum (`task`, `job_key`, `source`,
//! `status`) or a configured root-folder path, of which an operator has a
//! handful. Nothing free-form (titles, file paths, ids, error text) ever
//! reaches a label.

use chrono::{DateTime, Utc};
use metrics::{Unit, describe_gauge};

use crate::jobs::definitions::JobRunStatus;

/// Unix timestamp of the last completed run of a scheduled task.
pub(crate) const TASK_LAST_RUN_TIMESTAMP_SECONDS: &str = "scryer_task_last_run_timestamp_seconds";
/// Unix timestamp of the last run of a scheduled task that finished without panicking.
pub(crate) const TASK_LAST_SUCCESS_TIMESTAMP_SECONDS: &str =
    "scryer_task_last_success_timestamp_seconds";
/// Unix timestamp of the last terminal job run, whatever its outcome.
pub(crate) const JOB_LAST_RUN_TIMESTAMP_SECONDS: &str = "scryer_job_last_run_timestamp_seconds";
/// Unix timestamp of the last job run that ended `Completed` or `Warning`.
pub(crate) const JOB_LAST_SUCCESS_TIMESTAMP_SECONDS: &str =
    "scryer_job_last_success_timestamp_seconds";
/// Unix timestamp the scheduler currently intends to run a job next; `0` when unscheduled.
pub(crate) const JOB_NEXT_RUN_TIMESTAMP_SECONDS: &str = "scryer_job_next_run_timestamp_seconds";
/// One-hot indicator of the current status of each health-check source.
pub(crate) const HEALTH_CHECK_STATUS: &str = "scryer_health_check_status";
/// Bytes an unprivileged writer can still use on the filesystem backing a root folder.
pub(crate) const ROOT_FOLDER_FREE_BYTES: &str = "scryer_root_folder_free_bytes";
/// Total size of the filesystem backing a root folder.
pub(crate) const ROOT_FOLDER_TOTAL_BYTES: &str = "scryer_root_folder_total_bytes";

/// Seconds since the unix epoch, as Prometheus timestamp gauges express them.
///
/// Goes through milliseconds rather than `timestamp()` plus a nanosecond
/// fraction so a sub-second instant is preserved without the leap-second edge
/// case in `timestamp_subsec_nanos`, whose value can exceed one second.
pub(crate) fn unix_seconds(at: DateTime<Utc>) -> f64 {
    at.timestamp_millis() as f64 / 1_000.0
}

/// The freshness gauges a terminal job run should set, as `(metric name, value)`.
///
/// Factored out of the two call sites in the job runner so the "which statuses
/// count as a success" rule lives in one testable place: `Warning` is a
/// success because the job finished and produced its output, it just had
/// something to say about it.
pub(crate) fn job_completion_gauge_values(
    status: JobRunStatus,
    completed_at: DateTime<Utc>,
) -> Vec<(&'static str, f64)> {
    let value = unix_seconds(completed_at);
    let mut values = vec![(JOB_LAST_RUN_TIMESTAMP_SECONDS, value)];
    if matches!(status, JobRunStatus::Completed | JobRunStatus::Warning) {
        values.push((JOB_LAST_SUCCESS_TIMESTAMP_SECONDS, value));
    }
    values
}

/// Registers HELP/UNIT metadata for every gauge family this work package adds.
///
/// Called once by the binary's metrics setup at startup, before anything has
/// been recorded, so the scrape surface is self-describing even while a family
/// is still empty.
pub fn describe_freshness_and_health_metrics() {
    describe_gauge!(
        TASK_LAST_RUN_TIMESTAMP_SECONDS,
        Unit::Seconds,
        "Unix timestamp at which each scheduled task last finished a run, by task."
    );
    describe_gauge!(
        TASK_LAST_SUCCESS_TIMESTAMP_SECONDS,
        Unit::Seconds,
        "Unix timestamp at which each scheduled task last finished a run without panicking, by task."
    );
    describe_gauge!(
        JOB_LAST_RUN_TIMESTAMP_SECONDS,
        Unit::Seconds,
        "Unix timestamp at which each job last reached a terminal run, by job key."
    );
    describe_gauge!(
        JOB_LAST_SUCCESS_TIMESTAMP_SECONDS,
        Unit::Seconds,
        "Unix timestamp at which each job last completed successfully (completed or warning), by job key."
    );
    describe_gauge!(
        JOB_NEXT_RUN_TIMESTAMP_SECONDS,
        Unit::Seconds,
        "Unix timestamp at which the scheduler intends to run each job next, by job key; 0 when the job is not scheduled."
    );
    describe_gauge!(
        HEALTH_CHECK_STATUS,
        "Current health-check state as a one-hot indicator: 1 for the status a source is in, 0 for every other status, by source and status."
    );
    describe_gauge!(
        ROOT_FOLDER_FREE_BYTES,
        Unit::Bytes,
        "Bytes still available to an unprivileged writer on the filesystem backing each configured root folder."
    );
    describe_gauge!(
        ROOT_FOLDER_TOTAL_BYTES,
        Unit::Bytes,
        "Total size of the filesystem backing each configured root folder."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_seconds_round_trips_a_known_timestamp() {
        let at = DateTime::parse_from_rfc3339("2026-09-03T12:34:56.250Z")
            .expect("fixture timestamp parses")
            .with_timezone(&Utc);

        assert_eq!(unix_seconds(at), 1_788_438_896.25);
        assert_eq!(
            DateTime::from_timestamp_millis((unix_seconds(at) * 1_000.0) as i64),
            Some(at)
        );
    }

    #[test]
    fn job_completion_gauges_treat_warning_as_a_success() {
        let at = DateTime::from_timestamp(1_700_000_000, 0).expect("fixture timestamp is in range");

        for status in [JobRunStatus::Completed, JobRunStatus::Warning] {
            assert_eq!(
                job_completion_gauge_values(status, at),
                vec![
                    (JOB_LAST_RUN_TIMESTAMP_SECONDS, 1_700_000_000.0),
                    (JOB_LAST_SUCCESS_TIMESTAMP_SECONDS, 1_700_000_000.0),
                ],
                "{status:?} should refresh both freshness gauges"
            );
        }
    }

    #[test]
    fn job_completion_gauges_omit_success_for_a_failed_run() {
        let at = DateTime::from_timestamp(1_700_000_000, 0).expect("fixture timestamp is in range");

        assert_eq!(
            job_completion_gauge_values(JobRunStatus::Failed, at),
            vec![(JOB_LAST_RUN_TIMESTAMP_SECONDS, 1_700_000_000.0)]
        );
    }
}
