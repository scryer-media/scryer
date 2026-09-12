//! Live scoring cost, independent of score history. Never label by release,
//! title, source text, error text, or content hash. Preview IDs are coalesced
//! because each unsaved draft can otherwise create a new time series.
use metrics::{Unit, counter, describe_counter, describe_histogram, histogram};
use scryer_rules::policy::telemetry::{PolicyObservation, PolicyObserver};
use std::time::Instant;

#[derive(Clone, Copy)]
pub(crate) enum Purpose {
    Live,
    Preview,
}

impl Purpose {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Preview => "preview",
        }
    }
}

pub(crate) struct RulesObserver(pub(crate) Purpose);

impl PolicyObserver for RulesObserver {
    fn observe(&self, event: PolicyObservation<'_>) {
        let purpose = self.0.label();
        let id = if matches!(self.0, Purpose::Preview) {
            "preview"
        } else {
            event.rule_id
        };
        match event.stage {
            "rule_evaluate" => {
                counter!("scryer_rule_evaluations_total", "purpose" => purpose, "rule_set_id" => id.to_owned(), "phase" => event.phase, "outcome" => event.outcome).increment(1);
                if !matches!(event.outcome, "skipped" | "held") {
                    histogram!("scryer_rule_evaluation_seconds", "purpose" => purpose, "rule_set_id" => id.to_owned(), "phase" => event.phase, "temperature" => event.temperature).record(event.elapsed.as_secs_f64());
                }
            }
            "policy_load" => histogram!("scryer_rule_load_seconds", "purpose" => purpose, "rule_set_id" => id.to_owned(), "outcome" => event.outcome).record(event.elapsed.as_secs_f64()),
            stage => histogram!("scryer_rules_stage_seconds", "purpose" => purpose, "stage" => stage, "phase" => event.phase, "outcome" => event.outcome).record(event.elapsed.as_secs_f64()),
        }
    }
}

pub(crate) struct StageTimer {
    start: Instant,
    purpose: Purpose,
    stage: &'static str,
    outcome: &'static str,
}

impl StageTimer {
    pub(crate) fn new(stage: &'static str, purpose: Purpose) -> Self {
        Self {
            start: Instant::now(),
            stage,
            purpose,
            outcome: "ok",
        }
    }

    pub(crate) fn outcome(&mut self, outcome: &'static str) {
        self.outcome = outcome;
    }
}

impl Drop for StageTimer {
    fn drop(&mut self) {
        histogram!("scryer_rules_stage_seconds", "purpose" => self.purpose.label(), "stage" => self.stage, "phase" => "none", "outcome" => self.outcome).record(self.start.elapsed().as_secs_f64());
    }
}

pub fn describe_rule_metrics() {
    describe_histogram!(
        "scryer_rule_load_seconds",
        Unit::Seconds,
        "Per-ruleset source validation, parsing, wrapper loading and reference analysis; shared lazy compilation can occur during first evaluation."
    );
    describe_histogram!(
        "scryer_rule_evaluation_seconds",
        Unit::Seconds,
        "Per-ruleset evaluation and decoding time including no-match and failed evaluations. First means first actual invocation in this evaluator."
    );
    describe_counter!(
        "scryer_rule_evaluations_total",
        "Per-ruleset invocations, errors, no-matches, facet skips and observation holds; non-selected phases are not counted."
    );
    describe_histogram!(
        "scryer_rules_stage_seconds",
        Unit::Seconds,
        "Rule engine build, acquisition, input preparation, rule application, canonical scoring and scan-batch wall time. Stages nest; do not sum them."
    );
    describe_counter!(
        "scryer_scoring_batch_candidates_total",
        "Candidates entering scored search batches, including candidates filtered before canonical scoring."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics::with_local_recorder;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use std::time::Duration;

    #[test]
    fn rule_metrics_keep_no_matches_and_errors_but_exclude_skips_from_latency() {
        let recorder = DebuggingRecorder::new();
        with_local_recorder(&recorder, || {
            let observer = RulesObserver(Purpose::Live);
            for outcome in ["ok", "no_match", "error", "skipped", "held"] {
                observer.observe(PolicyObservation {
                    family: "release",
                    stage: "rule_evaluate",
                    phase: "baseline",
                    rule_id: "stable-id",
                    temperature: "warm",
                    outcome,
                    elapsed: Duration::from_micros(8),
                });
            }
        });
        let metrics = recorder.snapshotter().snapshot().into_vec();
        assert_eq!(metrics.len(), 6);
        assert_eq!(
            metrics
                .iter()
                .filter(|(_, _, _, value)| matches!(value, DebugValue::Counter(1)))
                .count(),
            5
        );
        let histogram = metrics
            .iter()
            .find(|(key, _, _, _)| key.key().name() == "scryer_rule_evaluation_seconds")
            .unwrap();
        match &histogram.3 {
            DebugValue::Histogram(values) => assert_eq!(values.len(), 3),
            other => panic!("expected latency samples, got {other:?}"),
        }
    }

    #[test]
    fn preview_metrics_coalesce_ephemeral_identities_and_stay_separate_from_live() {
        let recorder = DebuggingRecorder::new();
        with_local_recorder(&recorder, || {
            for purpose in [Purpose::Preview, Purpose::Live] {
                for id in ["draft-one", "draft-two"] {
                    RulesObserver(purpose).observe(PolicyObservation {
                        family: "release",
                        stage: "policy_load",
                        phase: "none",
                        rule_id: id,
                        temperature: "none",
                        outcome: "ok",
                        elapsed: Duration::from_micros(8),
                    });
                }
            }
        });
        let series = recorder.snapshotter().snapshot().into_vec();
        assert_eq!(series.len(), 3);
        let preview: Vec<_> = series
            .iter()
            .filter(|(key, _, _, _)| {
                key.key()
                    .labels()
                    .any(|label| label.key() == "purpose" && label.value() == "preview")
            })
            .collect();
        assert_eq!(preview.len(), 1);
        assert!(
            preview[0]
                .0
                .key()
                .labels()
                .any(|label| label.key() == "rule_set_id" && label.value() == "preview")
        );
        assert!(matches!(&preview[0].3, DebugValue::Histogram(values) if values.len() == 2));
    }

    #[test]
    fn engine_build_failure_is_timed_in_the_requested_purpose() {
        let recorder = DebuggingRecorder::new();
        with_local_recorder(&recorder, || {
            let policy = scryer_rules::UserPolicy {
                id: "broken".into(),
                name: "Broken".into(),
                rego_source: "invalid rego!".into(),
                origin: scryer_rules::PolicyOrigin::System,
                applied_facets: vec![],
            };
            assert!(
                crate::AppUseCase::build_user_rules_engine_for_purpose(
                    vec![],
                    vec![policy],
                    Purpose::Preview
                )
                .is_err()
            );
        });
        let series = recorder.snapshotter().snapshot().into_vec();
        assert!(series.iter().any(|(key, _, _, value)| {
            key.key().name() == "scryer_rules_stage_seconds"
                && [
                    ("purpose", "preview"),
                    ("stage", "engine_build"),
                    ("outcome", "error"),
                ]
                .iter()
                .all(|(name, expected)| {
                    key.key()
                        .labels()
                        .any(|label| label.key() == *name && label.value() == *expected)
                })
                && matches!(value, DebugValue::Histogram(values) if values.len() == 1)
        }));
    }
}
