//! In-memory schedule for user-defined scheduled scripts ("custom jobs").
//!
//! The schedule set is loaded from the script store only when the worker
//! starts and when a scheduled script is created, updated, toggled or
//! deleted. Everything else reads the in-memory set:
//!
//! - the worker sleeps on one timer set to the earliest deadline, or parks on
//!   the reload notification alone when nothing is scheduled, and wakes only
//!   for that deadline, a reload, or shutdown;
//! - on a deadline it takes the due scripts from memory, hands each to the
//!   dispatcher, advances only those scripts' next fire, and re-arms;
//! - the jobs query reads `next_run_at` from the same set.
//!
//! An interval schedule anchors on its previous fire, or on the moment the
//! script was first scheduled (worker start, creation, or a schedule edit).
//! Daily, weekly and cron schedules evaluate their next occurrence after the
//! same anchor. A manual schedule never contributes a deadline.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_domain::{ExecutionMode, PostProcessingScript, ScriptSchedule, ScriptTrigger};
use tokio::sync::Notify;
use tracing::{info, warn};

use crate::AppResult;
use crate::jobs::JobTriggerSource;

use super::schedule::{describe_schedule, next_fire_after};

/// How long after the worker starts a run-on-startup script fires. Matches the
/// delay of the startup health checks, so a restart does not stack script
/// runs on top of service initialization.
pub const CUSTOM_JOB_STARTUP_DELAY: chrono::Duration = chrono::Duration::seconds(30);

/// Wall-clock source the schedule evaluates against.
#[derive(Clone, Copy, Debug)]
pub enum SchedulerClock {
    /// The host wall clock. Deadlines are converted to a sleep at arm time,
    /// so a wall-clock jump is noticed at the next wake.
    System,
    /// Wall time derived from the tokio clock, so a paused test clock drives
    /// the schedule.
    Tokio {
        base_utc: DateTime<Utc>,
        base_instant: tokio::time::Instant,
    },
}

impl SchedulerClock {
    pub fn tokio_at(base_utc: DateTime<Utc>) -> Self {
        Self::Tokio {
            base_utc,
            base_instant: tokio::time::Instant::now(),
        }
    }

    pub fn now(&self) -> DateTime<Utc> {
        match self {
            Self::System => Utc::now(),
            Self::Tokio {
                base_utc,
                base_instant,
            } => {
                let elapsed = tokio::time::Instant::now().saturating_duration_since(*base_instant);
                *base_utc
                    + chrono::Duration::from_std(elapsed)
                        .unwrap_or_else(|_| chrono::Duration::zero())
            }
        }
    }

    fn instant_for(&self, deadline: DateTime<Utc>) -> tokio::time::Instant {
        let now = self.now();
        let delay = (deadline - now).to_std().unwrap_or_default();
        tokio::time::Instant::now() + delay
    }
}

/// One enabled scheduled script as the scheduler holds it.
#[derive(Clone, Debug)]
pub struct CustomJobEntry {
    pub script_id: String,
    pub name: String,
    pub description: String,
    pub schedule: ScriptSchedule,
    /// Human-readable schedule, computed when the schedule is loaded.
    pub schedule_description: String,
    pub execution_mode: ExecutionMode,
    pub run_on_startup: bool,
    /// Previous fire, or when the script was first scheduled.
    anchor: DateTime<Utc>,
    /// Next schedule-driven fire; `None` for a manual schedule.
    next_fire: Option<DateTime<Utc>>,
    /// Pending run-on-startup fire, cleared once it fires.
    startup_fire: Option<DateTime<Utc>>,
}

impl CustomJobEntry {
    /// The earliest pending fire, schedule-driven or startup.
    pub fn next_run_at(&self) -> Option<DateTime<Utc>> {
        match (self.next_fire, self.startup_fire) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// A script that came due, as handed to the dispatcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DueCustomJob {
    pub script_id: String,
    pub trigger_source: JobTriggerSource,
    pub execution_mode: ExecutionMode,
}

#[derive(Default)]
struct SchedulerState {
    entries: HashMap<String, CustomJobEntry>,
    /// The one deadline the worker's timer is armed for, if any.
    armed: Option<DateTime<Utc>>,
}

/// Shared schedule state: written by the worker and the configuration-change
/// path, read by the jobs query.
#[derive(Clone)]
pub struct CustomJobScheduler {
    state: Arc<Mutex<SchedulerState>>,
    reload: Arc<Notify>,
    clock: SchedulerClock,
}

impl Default for CustomJobScheduler {
    fn default() -> Self {
        Self::new(SchedulerClock::System)
    }
}

fn schedule_trigger_source(schedule: &ScriptSchedule) -> JobTriggerSource {
    match schedule {
        ScriptSchedule::Daily { .. } | ScriptSchedule::Weekly { .. } => {
            JobTriggerSource::ScheduledDaily
        }
        ScriptSchedule::Manual | ScriptSchedule::Interval { .. } | ScriptSchedule::Cron { .. } => {
            JobTriggerSource::ScheduledInterval
        }
    }
}

/// The first fire after `anchor`, never earlier than `not_before`: a fire
/// missed while the host was busy or asleep runs once, not once per missed
/// occurrence.
fn next_fire(
    schedule: &ScriptSchedule,
    anchor: DateTime<Utc>,
    not_before: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    let next = match next_fire_after(schedule, anchor) {
        Ok(next) => next?,
        Err(error) => {
            warn!(error = %error, "scheduled script has an invalid schedule; not scheduling it");
            return None;
        }
    };
    match not_before {
        Some(now) if next <= now => next_fire_after(schedule, now).ok().flatten(),
        _ => Some(next),
    }
}

impl CustomJobScheduler {
    pub fn new(clock: SchedulerClock) -> Self {
        Self {
            state: Arc::new(Mutex::new(SchedulerState::default())),
            reload: Arc::new(Notify::new()),
            clock,
        }
    }

    pub fn clock(&self) -> SchedulerClock {
        self.clock
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SchedulerState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Replace the schedule set with `scripts` (enabled scheduled scripts).
    /// A script whose schedule is unchanged keeps its anchor and pending
    /// fires; a new or re-scheduled script anchors now. Wakes the worker so
    /// it re-arms its timer.
    pub fn apply(&self, scripts: Vec<PostProcessingScript>) {
        let now = self.clock.now();
        {
            let mut state = self.lock();
            let mut previous = std::mem::take(&mut state.entries);
            for script in scripts {
                if !script.enabled || script.trigger != ScriptTrigger::Schedule {
                    continue;
                }
                let Some(schedule) = script.schedule.clone() else {
                    continue;
                };
                let entry = match previous.remove(&script.id) {
                    Some(existing) if existing.schedule == schedule => CustomJobEntry {
                        name: script.name.clone(),
                        description: script.description.clone(),
                        execution_mode: script.execution_mode,
                        run_on_startup: script.run_on_startup,
                        startup_fire: existing.startup_fire.filter(|_| script.run_on_startup),
                        ..existing
                    },
                    existing => CustomJobEntry {
                        script_id: script.id.clone(),
                        name: script.name.clone(),
                        description: script.description.clone(),
                        next_fire: next_fire(&schedule, now, None),
                        schedule_description: describe_schedule(&schedule),
                        schedule,
                        execution_mode: script.execution_mode,
                        run_on_startup: script.run_on_startup,
                        anchor: now,
                        startup_fire: existing
                            .and_then(|entry| entry.startup_fire)
                            .filter(|_| script.run_on_startup),
                    },
                };
                state.entries.insert(script.id, entry);
            }
        }
        self.reload.notify_one();
    }

    /// Queue the one run-on-startup fire for every script that asks for it.
    pub fn arm_startup_fires(&self, at: DateTime<Utc>) {
        let mut state = self.lock();
        for entry in state.entries.values_mut() {
            if entry.run_on_startup {
                entry.startup_fire = Some(at);
            }
        }
    }

    /// Every scheduled script, sorted by name then id.
    pub fn entries(&self) -> Vec<CustomJobEntry> {
        let mut entries = self.lock().entries.values().cloned().collect::<Vec<_>>();
        entries.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.script_id.cmp(&b.script_id))
        });
        entries
    }

    pub fn next_run_at(&self, script_id: &str) -> Option<DateTime<Utc>> {
        self.lock()
            .entries
            .get(script_id)
            .and_then(CustomJobEntry::next_run_at)
    }

    #[cfg(test)]
    fn earliest_deadline(&self) -> Option<DateTime<Utc>> {
        self.lock()
            .entries
            .values()
            .filter_map(CustomJobEntry::next_run_at)
            .min()
    }

    /// Pick the earliest deadline and record it as the one armed timer.
    fn arm(&self) -> Option<DateTime<Utc>> {
        let mut state = self.lock();
        let deadline = state
            .entries
            .values()
            .filter_map(CustomJobEntry::next_run_at)
            .min();
        state.armed = deadline;
        deadline
    }

    /// The deadline the worker's single timer is currently armed for.
    pub fn armed_deadline(&self) -> Option<DateTime<Utc>> {
        self.lock().armed
    }

    /// Take every fire due at `now` and advance those scripts only.
    pub fn take_due(&self, now: DateTime<Utc>) -> Vec<DueCustomJob> {
        let mut due = Vec::new();
        let mut state = self.lock();
        for entry in state.entries.values_mut() {
            if entry.startup_fire.is_some_and(|at| at <= now) {
                entry.startup_fire = None;
                due.push(DueCustomJob {
                    script_id: entry.script_id.clone(),
                    trigger_source: JobTriggerSource::ScheduledStartup,
                    execution_mode: entry.execution_mode,
                });
            }
            if let Some(scheduled) = entry.next_fire.filter(|at| *at <= now) {
                entry.anchor = scheduled;
                entry.next_fire = next_fire(&entry.schedule, scheduled, Some(now));
                due.push(DueCustomJob {
                    script_id: entry.script_id.clone(),
                    trigger_source: schedule_trigger_source(&entry.schedule),
                    execution_mode: entry.execution_mode,
                });
            }
        }
        due.sort_by(|a, b| a.script_id.cmp(&b.script_id));
        due
    }
}

/// Where the worker loads the schedule set from at start.
#[async_trait]
pub trait CustomJobSource: Send + Sync {
    async fn load_scheduled_scripts(&self) -> AppResult<Vec<PostProcessingScript>>;
}

/// Starts a run for a due script. Must return promptly; the run itself
/// executes in the background.
#[async_trait]
pub trait CustomJobDispatcher: Send + Sync {
    async fn dispatch(&self, due: DueCustomJob);
}

/// The scheduler worker. Loads the schedule set once, then sleeps until the
/// earliest deadline (or parks when there is none), waking only for that
/// deadline, a reload, or shutdown.
pub async fn run_custom_job_scheduler(
    scheduler: CustomJobScheduler,
    source: &dyn CustomJobSource,
    dispatcher: &dyn CustomJobDispatcher,
    token: tokio_util::sync::CancellationToken,
) {
    info!("custom job scheduler started");
    match source.load_scheduled_scripts().await {
        Ok(scripts) => scheduler.apply(scripts),
        Err(error) => warn!(error = %error, "failed to load scheduled scripts"),
    }
    scheduler.arm_startup_fires(scheduler.clock.now() + CUSTOM_JOB_STARTUP_DELAY);

    loop {
        let deadline = scheduler.arm();
        let reload = scheduler.reload.notified();
        tokio::select! {
            _ = token.cancelled() => {
                info!("custom job scheduler shutting down");
                scheduler.lock().armed = None;
                return;
            }
            _ = reload => {}
            _ = async {
                match deadline {
                    Some(deadline) => {
                        tokio::time::sleep_until(scheduler.clock.instant_for(deadline)).await
                    }
                    None => std::future::pending().await,
                }
            } => {
                for due in scheduler.take_due(scheduler.clock.now()) {
                    dispatcher.dispatch(due).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use scryer_domain::ScriptType;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 2, 12, 0, 0).unwrap()
    }

    fn script(id: &str, schedule: ScriptSchedule) -> PostProcessingScript {
        PostProcessingScript {
            id: id.to_string(),
            name: format!("fixture {id}"),
            description: String::new(),
            script_type: ScriptType::Inline,
            script_content: "echo fixture".to_string(),
            applied_facets: vec![],
            execution_mode: ExecutionMode::Blocking,
            timeout_secs: 60,
            priority: 0,
            enabled: true,
            debug: false,
            language: Default::default(),
            trigger: ScriptTrigger::Schedule,
            schedule: Some(schedule),
            run_on_startup: false,
            created_at: base(),
            updated_at: base(),
        }
    }

    fn every(seconds: i64) -> ScriptSchedule {
        ScriptSchedule::Interval {
            every_seconds: seconds,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_interval_anchors_on_when_it_was_scheduled_then_on_each_fire() {
        let scheduler = CustomJobScheduler::new(SchedulerClock::tokio_at(base()));
        scheduler.apply(vec![script("a", every(600))]);
        assert_eq!(
            scheduler.next_run_at("a"),
            Some(base() + chrono::Duration::seconds(600))
        );

        // A fire noticed late still anchors on the scheduled instant, so the
        // cadence does not drift by the wake latency.
        let fired = scheduler.take_due(base() + chrono::Duration::seconds(605));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].trigger_source, JobTriggerSource::ScheduledInterval);
        assert_eq!(
            scheduler.next_run_at("a"),
            Some(base() + chrono::Duration::seconds(1200))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn missed_fires_collapse_into_one_and_the_next_is_after_now() {
        let scheduler = CustomJobScheduler::new(SchedulerClock::tokio_at(base()));
        scheduler.apply(vec![script("a", every(60))]);
        let late = base() + chrono::Duration::seconds(60 * 10 + 30);
        assert_eq!(scheduler.take_due(late).len(), 1);
        let next = scheduler.next_run_at("a").expect("rescheduled");
        assert!(next > late, "next fire {next} must be after {late}");
    }

    #[tokio::test(start_paused = true)]
    async fn an_unchanged_schedule_keeps_its_anchor_across_a_reload() {
        let scheduler = CustomJobScheduler::new(SchedulerClock::tokio_at(base()));
        scheduler.apply(vec![script("a", every(600))]);
        tokio::time::advance(std::time::Duration::from_secs(300)).await;
        scheduler.apply(vec![script("a", every(600))]);
        assert_eq!(
            scheduler.next_run_at("a"),
            Some(base() + chrono::Duration::seconds(600))
        );
        // A schedule edit re-anchors at the reload.
        scheduler.apply(vec![script("a", every(120))]);
        assert_eq!(
            scheduler.next_run_at("a"),
            Some(base() + chrono::Duration::seconds(300 + 120))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn manual_schedules_never_have_a_deadline() {
        let scheduler = CustomJobScheduler::new(SchedulerClock::tokio_at(base()));
        scheduler.apply(vec![script("m", ScriptSchedule::Manual)]);
        assert_eq!(scheduler.entries().len(), 1);
        assert_eq!(scheduler.next_run_at("m"), None);
        assert_eq!(scheduler.earliest_deadline(), None);
        assert!(
            scheduler
                .take_due(base() + chrono::Duration::days(400))
                .is_empty()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn disabled_and_import_scripts_are_not_scheduled() {
        let scheduler = CustomJobScheduler::new(SchedulerClock::tokio_at(base()));
        let mut disabled = script("d", every(60));
        disabled.enabled = false;
        let mut import = script("i", every(60));
        import.trigger = ScriptTrigger::PostImport;
        scheduler.apply(vec![disabled, import]);
        assert!(scheduler.entries().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_startup_fire_runs_once_alongside_the_schedule() {
        let scheduler = CustomJobScheduler::new(SchedulerClock::tokio_at(base()));
        let mut on_start = script("s", every(3600));
        on_start.run_on_startup = true;
        scheduler.apply(vec![on_start]);
        scheduler.arm_startup_fires(base() + CUSTOM_JOB_STARTUP_DELAY);
        assert_eq!(
            scheduler.next_run_at("s"),
            Some(base() + CUSTOM_JOB_STARTUP_DELAY)
        );
        let fired = scheduler.take_due(base() + CUSTOM_JOB_STARTUP_DELAY);
        assert_eq!(
            fired
                .iter()
                .map(|due| due.trigger_source)
                .collect::<Vec<_>>(),
            vec![JobTriggerSource::ScheduledStartup]
        );
        assert_eq!(
            scheduler.next_run_at("s"),
            Some(base() + chrono::Duration::seconds(3600))
        );
    }

    #[derive(Default)]
    struct CountingSource {
        scripts: Mutex<Vec<PostProcessingScript>>,
        loads: AtomicUsize,
    }

    #[async_trait]
    impl CustomJobSource for CountingSource {
        async fn load_scheduled_scripts(&self) -> AppResult<Vec<PostProcessingScript>> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(self.scripts.lock().unwrap().clone())
        }
    }

    struct RecordingDispatcher {
        fires: tokio::sync::mpsc::UnboundedSender<(DateTime<Utc>, DueCustomJob)>,
        clock: SchedulerClock,
    }

    #[async_trait]
    impl CustomJobDispatcher for RecordingDispatcher {
        async fn dispatch(&self, due: DueCustomJob) {
            let _ = self.fires.send((self.clock.now(), due));
        }
    }

    /// Spawn the worker over `source`; returns its fire stream and handles.
    fn spawn_worker(
        scheduler: &CustomJobScheduler,
        source: Arc<CountingSource>,
    ) -> (
        tokio::sync::mpsc::UnboundedReceiver<(DateTime<Utc>, DueCustomJob)>,
        tokio_util::sync::CancellationToken,
        tokio::task::JoinHandle<()>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let token = tokio_util::sync::CancellationToken::new();
        let dispatcher = RecordingDispatcher {
            fires: tx,
            clock: scheduler.clock(),
        };
        let worker_scheduler = scheduler.clone();
        let worker_token = token.clone();
        let handle = tokio::spawn(async move {
            run_custom_job_scheduler(worker_scheduler, source.as_ref(), &dispatcher, worker_token)
                .await;
        });
        (rx, token, handle)
    }

    /// Bounded wait for the next fire; a missed fire fails instead of hanging.
    async fn next_fire_event(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<(DateTime<Utc>, DueCustomJob)>,
    ) -> (DateTime<Utc>, DueCustomJob) {
        tokio::time::timeout(std::time::Duration::from_secs(24 * 3600), rx.recv())
            .await
            .expect("a fire within the bound")
            .expect("worker alive")
    }

    #[tokio::test(start_paused = true)]
    async fn the_worker_fires_an_interval_at_its_instants_and_reloads_after_a_toggle() {
        let scheduler = CustomJobScheduler::new(SchedulerClock::tokio_at(base()));
        let source = Arc::new(CountingSource::default());
        *source.scripts.lock().unwrap() = vec![script("a", every(600))];
        let (mut fires, token, handle) = spawn_worker(&scheduler, source.clone());

        let (at, due) = next_fire_event(&mut fires).await;
        assert_eq!(at, base() + chrono::Duration::seconds(600));
        assert_eq!(due.script_id, "a");
        let (at, _) = next_fire_event(&mut fires).await;
        assert_eq!(at, base() + chrono::Duration::seconds(1200));
        assert_eq!(
            source.loads.load(Ordering::SeqCst),
            1,
            "fires never re-list the scripts"
        );

        // Toggling the script off is a configuration change: the change path
        // reloads the set and the worker re-arms with no deadline.
        let mut disabled = script("a", every(600));
        disabled.enabled = false;
        scheduler.apply(vec![disabled]);
        assert_eq!(scheduler.next_run_at("a"), None);
        tokio::time::advance(std::time::Duration::from_secs(3600)).await;
        assert!(fires.try_recv().is_err(), "a disabled script never fires");

        // Toggling it back on anchors at the reload.
        let reenabled_at = scheduler.clock().now();
        scheduler.apply(vec![script("a", every(600))]);
        let (at, _) = next_fire_event(&mut fires).await;
        assert_eq!(at, reenabled_at + chrono::Duration::seconds(600));

        token.cancel();
        handle.await.expect("worker exits");
        assert_eq!(source.loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_worker_loads_once_and_does_nothing_until_a_fire() {
        let scheduler = CustomJobScheduler::new(SchedulerClock::tokio_at(base()));
        let source = Arc::new(CountingSource::default());
        *source.scripts.lock().unwrap() = vec![script("a", every(3600))];
        let (mut fires, token, handle) = spawn_worker(&scheduler, source.clone());

        // Let the worker load and park on its timer.
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(3599)).await;
        assert_eq!(source.loads.load(Ordering::SeqCst), 1);
        assert!(
            fires.try_recv().is_err(),
            "nothing fires before the deadline"
        );

        let (at, _) = next_fire_event(&mut fires).await;
        assert_eq!(at, base() + chrono::Duration::seconds(3600));
        assert_eq!(source.loads.load(Ordering::SeqCst), 1);

        token.cancel();
        handle.await.expect("worker exits");
    }

    #[tokio::test(start_paused = true)]
    async fn one_task_and_one_timer_serve_every_script() {
        let scheduler = CustomJobScheduler::new(SchedulerClock::tokio_at(base()));
        let source = Arc::new(CountingSource::default());
        *source.scripts.lock().unwrap() = vec![
            script("a", every(300)),
            script("b", every(500)),
            script("c", every(700)),
        ];
        let tasks_before = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        let (mut fires, token, handle) = spawn_worker(&scheduler, source.clone());

        let seconds = |n: i64| base() + chrono::Duration::seconds(n);
        let expected = [
            (300, "a"),
            (500, "b"),
            (600, "a"),
            (700, "c"),
            (900, "a"),
            (1000, "b"),
        ];
        for (at_seconds, id) in expected {
            let (at, due) = next_fire_event(&mut fires).await;
            assert_eq!((at, due.script_id.as_str()), (seconds(at_seconds), id));
            // One worker task, whatever the number of scripts.
            assert_eq!(
                tokio::runtime::Handle::current()
                    .metrics()
                    .num_alive_tasks(),
                tasks_before + 1
            );
            // Let the worker re-arm, then check its single timer targets the
            // earliest pending fire across all three scripts.
            tokio::task::yield_now().await;
            assert_eq!(scheduler.armed_deadline(), scheduler.earliest_deadline());
        }
        assert_eq!(source.loads.load(Ordering::SeqCst), 1);

        token.cancel();
        handle.await.expect("worker exits");
        assert_eq!(scheduler.armed_deadline(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn with_nothing_scheduled_the_worker_stays_parked() {
        let scheduler = CustomJobScheduler::new(SchedulerClock::tokio_at(base()));
        let source = Arc::new(CountingSource::default());
        *source.scripts.lock().unwrap() = vec![script("m", ScriptSchedule::Manual)];
        let (mut fires, token, handle) = spawn_worker(&scheduler, source.clone());

        tokio::task::yield_now().await;
        // A paused clock auto-advances only to a pending timer; a parked
        // worker has none, so a year passes with no wake and no store call.
        tokio::time::advance(std::time::Duration::from_secs(365 * 24 * 3600)).await;
        assert_eq!(source.loads.load(Ordering::SeqCst), 1);
        assert!(fires.try_recv().is_err());
        assert_eq!(scheduler.earliest_deadline(), None);
        assert_eq!(scheduler.armed_deadline(), None, "no timer while parked");

        token.cancel();
        handle.await.expect("worker exits");
        assert_eq!(source.loads.load(Ordering::SeqCst), 1);
    }
}
