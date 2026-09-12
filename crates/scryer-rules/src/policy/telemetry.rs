//! Optional host-owned timing sink. The rules runtime has no recorder dependency.
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct PolicyObservation<'a> {
    pub family: &'static str,
    pub stage: &'static str,
    pub phase: &'static str,
    pub rule_id: &'a str,
    pub temperature: &'static str,
    pub outcome: &'static str,
    pub elapsed: Duration,
}

pub trait PolicyObserver: Send + Sync {
    fn observe(&self, observation: PolicyObservation<'_>);
}

pub(crate) struct PolicyTimer<'a> {
    observer: Option<Arc<dyn PolicyObserver>>,
    started: Option<Instant>,
    observation: PolicyObservation<'a>,
}

impl<'a> PolicyTimer<'a> {
    pub(crate) fn new(
        observer: &Option<Arc<dyn PolicyObserver>>,
        family: &'static str,
        stage: &'static str,
        phase: &'static str,
        rule_id: &'a str,
        temperature: &'static str,
    ) -> Self {
        Self {
            observer: observer.clone(),
            started: observer.as_ref().map(|_| Instant::now()),
            observation: PolicyObservation {
                family,
                stage,
                phase,
                rule_id,
                temperature,
                outcome: "error",
                elapsed: Duration::ZERO,
            },
        }
    }

    pub(crate) fn finish(mut self, outcome: &'static str) {
        self.observation.outcome = outcome;
    }
}

impl Drop for PolicyTimer<'_> {
    fn drop(&mut self) {
        if let (Some(observer), Some(started)) = (&self.observer, self.started) {
            observer.observe(PolicyObservation {
                elapsed: started.elapsed(),
                ..self.observation
            });
        }
    }
}
