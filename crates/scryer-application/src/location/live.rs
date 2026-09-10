//! Bounded wire snapshots and transient transfer telemetry. CRC offsets are
//! deliberately never recovery checkpoints.

use super::model::{LocationOperation, TitleCheckpointState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferTitle {
    pub title_id: String,
    pub name: String,
    pub sequence: i64,
    pub state: TitleCheckpointState,
    pub files_total: i64,
    pub files_done: i64,
    pub bytes_total: u64,
    pub copy_bytes: u64,
    pub verification_bytes: u64,
    pub current_file: Option<String>,
    pub copying: usize,
    pub verifying: usize,
    pub detail: Option<String>,
}

impl TransferTitle {
    pub fn rank(&self) -> i64 {
        if self.verifying > 0 {
            return 0;
        }
        if self.copying > 0 {
            return 1;
        }
        match self.state {
            TitleCheckpointState::Verifying => 0,
            TitleCheckpointState::Moving
            | TitleCheckpointState::Reconciling
            | TitleCheckpointState::CleaningUp => 1,
            TitleCheckpointState::Failed
            | TitleCheckpointState::Blocked
            | TitleCheckpointState::CompletedWithWarnings => 2,
            TitleCheckpointState::Pending => 3,
            _ => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct TransferVersion {
    pub generation: i64,
    pub revision: i64,
}

#[derive(Clone, Debug)]
pub struct TransferSnapshot {
    pub version: TransferVersion,
    pub operation: LocationOperation,
    pub progress_basis_points: i64,
    pub eta_seconds: Option<i64>,
    pub titles: Vec<TransferTitle>,
    pub total_count: i64,
    pub has_more: bool,
}

#[derive(Debug)]
pub struct TransferEta {
    started: Instant,
    sampled: Instant,
    last_useful: Instant,
    useful: u64,
    sampled_useful: u64,
    samples: u32,
    fast: f64,
    slow: f64,
}

impl Default for TransferEta {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl TransferEta {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            sampled: now,
            last_useful: now,
            useful: 0,
            sampled_useful: 0,
            samples: 0,
            fast: 0.0,
            slow: 0.0,
        }
    }

    pub fn add_useful(&mut self, bytes: u64, now: Instant) {
        self.useful = self.useful.saturating_add(bytes);
        if bytes > 0 {
            self.last_useful = now;
        }
        let seconds = now.duration_since(self.sampled).as_secs_f64();
        if seconds < 1.0 {
            return;
        }
        let delta = self.useful.saturating_sub(self.sampled_useful);
        let rate = delta as f64 / seconds;
        if self.samples == 0 && delta > 0 {
            self.fast = rate;
            self.slow = rate;
        } else {
            self.fast += (1.0 - (-seconds / 30.0).exp()) * (rate - self.fast);
            self.slow += (1.0 - (-seconds / 180.0).exp()) * (rate - self.slow);
        }
        if delta > 0 {
            self.samples += 1;
        }
        self.sampled = now;
        self.sampled_useful = self.useful;
    }

    pub fn estimate(&self, remaining: u64, now: Instant) -> Option<i64> {
        if self.samples < 3
            || now.duration_since(self.started) < Duration::from_secs(15)
            || now.duration_since(self.last_useful) >= Duration::from_secs(30)
        {
            return None;
        }
        let rate = self.fast * 0.7 + self.slow * 0.3;
        (rate.is_finite() && rate > 0.0)
            .then(|| (remaining as f64 / rate).ceil().min(i64::MAX as f64) as i64)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FileTelemetry {
    pub copied: u64,
    pub verified: u64,
    pub copy_credit: u64,
    pub verify_credit: u64,
    pub phase: Option<scryer_domain::ImportTransferPhase>,
    pub done: bool,
    pub size: u64,
}

#[derive(Debug, Default)]
struct OperationTelemetry {
    files: BTreeMap<(String, String), FileTelemetry>,
    resumed_bytes: u64,
    finalized: u64,
    finalization_seconds: f64,
    last_finalized: Option<Instant>,
    abandoned_work: BTreeMap<(String, String), u64>,
    high_water: i64,
    eta: TransferEta,
}

#[derive(Debug, Default)]
struct HubState {
    version: TransferVersion,
    operations: BTreeMap<String, OperationTelemetry>,
    last_emitted: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct TransferHub {
    state: Arc<Mutex<HubState>>,
    changed: tokio::sync::watch::Sender<TransferVersion>,
}

impl Default for TransferHub {
    fn default() -> Self {
        let (changed, _) = tokio::sync::watch::channel(TransferVersion::default());
        Self {
            state: Arc::new(Mutex::new(HubState::default())),
            changed,
        }
    }
}

impl TransferHub {
    pub fn begin(&self, operation: &str, resumed_bytes: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let telemetry = state.operations.entry(operation.to_owned()).or_default();
        // Rebuild transient work from durable proofs on every attempt. Keeping
        // old file credits would count settled titles twice on in-process resume.
        *telemetry = OperationTelemetry {
            resumed_bytes,
            high_water: telemetry.high_water,
            ..OperationTelemetry::default()
        };
    }

    pub fn title_finalized(&self, operation: &str, elapsed: Duration) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let telemetry = state.operations.entry(operation.to_owned()).or_default();
        telemetry.finalized += 1;
        telemetry.finalization_seconds += elapsed.as_secs_f64();
        telemetry.last_finalized = Some(Instant::now());
    }

    pub fn file_failed(&self, operation: &str, title: &str, path: &str, size: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let op = state.operations.entry(operation.to_owned()).or_default();
        let key = (title.to_owned(), path.to_owned());
        let completed = op.files.get_mut(&key).map_or(0, |file| {
            file.done = true;
            file.copied.saturating_add(file.verified)
        });
        // Failed work is no longer scheduled. Exclude it from ETA without
        // granting progress credit or suppressing healthy siblings' estimates.
        op.abandoned_work
            .insert(key, size.saturating_mul(2).saturating_sub(completed));
        drop(state);
        self.changed();
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<TransferVersion> {
        self.changed.subscribe()
    }
    pub fn version(&self) -> TransferVersion {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).version
    }
    pub fn set_generation(&self, generation: i64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.version.generation = generation;
        self.changed.send_replace(state.version);
    }

    pub fn changed(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.version.revision = state.version.revision.saturating_add(1);
        state.last_emitted = Some(Instant::now());
        self.changed.send_replace(state.version);
    }

    pub fn file_update(
        &self,
        operation: &str,
        title: &str,
        path: &str,
        size: u64,
        phase: scryer_domain::ImportTransferPhase,
        bytes: u64,
    ) {
        use scryer_domain::ImportTransferPhase;
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let telemetry = state.operations.entry(operation.to_owned()).or_default();
        let file = telemetry
            .files
            .entry((title.to_owned(), path.to_owned()))
            .or_default();
        telemetry
            .abandoned_work
            .remove(&(title.to_owned(), path.to_owned()));
        file.done = false;
        let phase_changed = file.phase != Some(phase);
        file.size = size;
        file.phase = Some(phase);
        let bytes = bytes.min(size);
        let useful = match phase {
            ImportTransferPhase::Copying => {
                file.copied = bytes;
                if phase_changed {
                    file.verified = 0;
                }
                let useful = bytes.saturating_sub(file.copy_credit);
                file.copy_credit = file.copy_credit.max(bytes);
                useful
            }
            ImportTransferPhase::Verifying => {
                // Adoption and interrupted placement already have destination
                // bytes. This instantaneous credit is excluded from throughput.
                file.copied = size;
                file.verified = bytes;
                let useful = bytes.saturating_sub(file.verify_credit);
                file.verify_credit = file.verify_credit.max(bytes);
                useful
            }
            _ => 0,
        };
        telemetry.eta.add_useful(useful, now);
        // No per-chunk fanout or persistent writes.
        if phase_changed {
            state.version.revision = state.version.revision.saturating_add(1);
            state.last_emitted = Some(now);
            self.changed.send_replace(state.version);
        }
    }

    pub fn file_done(&self, operation: &str, title: &str, path: &str, size: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let file = state
            .operations
            .entry(operation.to_owned())
            .or_default()
            .files
            .entry((title.to_owned(), path.to_owned()))
            .or_default();
        file.size = size;
        file.copied = size;
        file.verified = size;
        file.done = true;
        drop(state);
        self.changed();
    }

    pub fn overlay(&self, operation: &str, row: &mut TransferTitle) {
        if matches!(
            row.state,
            TitleCheckpointState::Completed
                | TitleCheckpointState::CompletedWithWarnings
                | TitleCheckpointState::Blocked
                | TitleCheckpointState::Skipped
        ) {
            row.copying = 0;
            row.verifying = 0;
            row.current_file = None;
            return;
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(telemetry) = state.operations.get(operation) else {
            return;
        };
        let files = telemetry
            .files
            .range((row.title_id.clone(), String::new())..)
            .take_while(|((title, _), _)| title == &row.title_id);
        let mut copied = 0u64;
        let mut verified = 0u64;
        row.copying = 0;
        row.verifying = 0;
        row.current_file = None;
        for ((_, path), file) in files {
            copied = copied.saturating_add(file.copied);
            verified = verified.saturating_add(file.verified);
            if !file.done {
                match file.phase {
                    Some(scryer_domain::ImportTransferPhase::Verifying) => row.verifying += 1,
                    Some(scryer_domain::ImportTransferPhase::Copying) => row.copying += 1,
                    _ => {}
                }
                row.current_file = Some(path.clone());
            }
        }
        row.copy_bytes = row.copy_bytes.max(copied);
        row.verification_bytes = row.verification_bytes.max(verified);
    }

    pub fn progress(
        &self,
        operation: &LocationOperation,
        stored_high_water: i64,
    ) -> (i64, Option<i64>) {
        let success = matches!(
            operation.state,
            super::model::LocationOperationState::Completed
                | super::model::LocationOperationState::CompletedWithWarnings
        );
        if operation.state.is_terminal() {
            return (
                if success {
                    10_000
                } else {
                    stored_high_water.clamp(0, 9_999)
                },
                None,
            );
        }
        if operation.state == super::model::LocationOperationState::Queued {
            return (stored_high_water.clamp(0, 9_999), None);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let telemetry = state.operations.entry(operation.id.clone()).or_default();
        let total = operation.counters.bytes_total.max(0) as u64;
        let settled = operation.counters.bytes_processed.max(0) as u64;
        let work = telemetry.files.values().fold(0u64, |sum, file| {
            sum.saturating_add(file.copied)
                .saturating_add(file.verified)
        });
        let processed = work
            .saturating_add(telemetry.resumed_bytes.saturating_mul(2))
            .max(settled.saturating_mul(2));
        let mut value = if total > 0 {
            (processed as f64 / (total as f64 * 2.0) * 10_000.0) as i64
        } else if operation.counters.titles_total > 0 {
            operation.counters.titles_processed * 10_000 / operation.counters.titles_total
        } else {
            0
        };
        value = if success {
            10_000
        } else {
            value.clamp(0, 9_999)
        };
        telemetry.high_water = telemetry.high_water.max(stored_high_water).max(value);
        let now = Instant::now();
        telemetry.eta.add_useful(0, now);
        let abandoned = telemetry
            .abandoned_work
            .values()
            .fold(0u64, |sum, work| sum.saturating_add(*work));
        let remaining = total
            .saturating_mul(2)
            .saturating_sub(processed)
            .saturating_sub(abandoned);
        let metadata_tail = if telemetry.finalized > 0 {
            ((operation.counters.titles_total - operation.counters.titles_processed).max(0) as f64
                * telemetry.finalization_seconds
                / telemetry.finalized as f64)
                .ceil() as i64
        } else {
            0
        };
        let eta = if operation.started_at.is_none() || operation.cancel_requested {
            None
        } else if remaining == 0
            && telemetry.finalized >= 3
            && now.duration_since(telemetry.eta.started) >= Duration::from_secs(15)
            && telemetry
                .last_finalized
                .is_some_and(|at| now.duration_since(at) < Duration::from_secs(30))
        {
            Some(metadata_tail)
        } else {
            telemetry
                .eta
                .estimate(remaining, now)
                .map(|seconds| seconds.max(metadata_tail))
        };
        (telemetry.high_water, eta)
    }

    pub fn finish(&self, operation: &str) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .operations
            .remove(operation);
        self.changed();
    }
}

impl crate::AppUseCase {
    pub async fn initialize_transfer_generation(&self) -> crate::AppResult<()> {
        let registry = &self.runtime.library.location_runners;
        let generation = registry
            .transfer_generation
            .get_or_try_init(|| {
                self.services
                    .library
                    .location_operations
                    .allocate_transfer_generation()
            })
            .await?;
        if registry.transfers.version().generation != *generation {
            registry.transfers.set_generation(*generation);
        }
        Ok(())
    }

    pub fn subscribe_location_transfers(&self) -> tokio::sync::watch::Receiver<TransferVersion> {
        self.runtime.library.location_runners.transfers.subscribe()
    }

    pub async fn location_transfer_title_detail(
        &self,
        actor: &scryer_domain::User,
        operation_id: &str,
        title_id: &str,
    ) -> crate::AppResult<Option<String>> {
        let store = &self.services.library.location_operations;
        let Some(operation) = store.get_location_operation(operation_id).await? else {
            return Ok(None);
        };
        self.require_location_operation_permission(actor, &operation)
            .await?;
        Ok(store
            .transfer_title(operation_id, title_id)
            .await?
            .and_then(|row| row.detail))
    }

    pub(crate) async fn seed_transfer_titles(
        &self,
        operation_id: &str,
        plan: &super::root_move::RootMoveExecutionPlan,
        selection: &[String],
    ) -> crate::AppResult<()> {
        let store = &self.services.library.location_operations;
        let sequences: BTreeMap<_, _> = selection
            .iter()
            .enumerate()
            .map(|(index, id)| (id.as_str(), index as i64))
            .collect();
        let planned: std::collections::BTreeSet<_> = plan
            .titles
            .iter()
            .map(|title| title.title_id.as_str())
            .collect();
        let mut summaries = Vec::with_capacity(plan.titles.len().max(selection.len()));
        for title in &plan.titles {
            summaries.push(TransferTitle {
                title_id: title.title_id.clone(),
                name: title.title_name.clone(),
                sequence: sequences
                    .get(title.title_id.as_str())
                    .copied()
                    .unwrap_or(title.sequence),
                state: TitleCheckpointState::Pending,
                files_total: title.files.len() as i64,
                files_done: 0,
                bytes_total: title.files.iter().map(|file| file.size_bytes).sum(),
                copy_bytes: 0,
                verification_bytes: 0,
                current_file: None,
                copying: 0,
                verifying: 0,
                detail: None,
            });
        }
        let unplanned: Vec<_> = selection
            .iter()
            .filter(|id| !planned.contains(id.as_str()))
            .cloned()
            .collect();
        let mut names = BTreeMap::new();
        for chunk in unplanned.chunks(500) {
            names.extend(
                self.services
                    .catalog
                    .titles
                    .get_by_ids(chunk)
                    .await?
                    .into_iter()
                    .map(|title| (title.id, title.name)),
            );
        }
        for (sequence, id) in selection.iter().enumerate() {
            if planned.contains(id.as_str()) {
                continue;
            }
            let name = names.get(id).cloned().unwrap_or_else(|| id.clone());
            summaries.push(TransferTitle {
                title_id: id.clone(),
                name,
                sequence: sequence as i64,
                state: TitleCheckpointState::Skipped,
                files_total: 0,
                files_done: 0,
                bytes_total: 0,
                copy_bytes: 0,
                verification_bytes: 0,
                current_file: None,
                copying: 0,
                verifying: 0,
                detail: None,
            });
        }
        store.seed_transfer_titles(operation_id, &summaries).await
    }

    pub async fn location_transfer_snapshot(
        &self,
        actor: &scryer_domain::User,
        operation_id: &str,
        offset: Option<i64>,
        limit: i64,
    ) -> crate::AppResult<Option<TransferSnapshot>> {
        let store = &self.services.library.location_operations;
        let registry = &self.runtime.library.location_runners;
        let Some(operation) = store.get_location_operation(operation_id).await? else {
            return Ok(None);
        };
        self.require_location_operation_permission(actor, &operation)
            .await?;
        let generation = registry
            .transfer_generation
            .get_or_try_init(|| store.allocate_transfer_generation())
            .await?;
        if registry.transfers.version().generation != *generation {
            registry.transfers.set_generation(*generation);
        }
        // Old operations materialize the compact read model once. Normal live
        // snapshots never deserialize their file plan.
        if !store.transfer_titles_initialized(operation_id).await? {
            if store.transfer_title_count(operation_id).await? == 0
                && let Some(json) = store.get_location_operation_plan_json(operation_id).await?
                && let Ok(plan) =
                    serde_json::from_str::<super::root_move::RootMoveExecutionPlan>(&json)
            {
                self.seed_transfer_titles(operation_id, &plan, &[]).await?;
                for checkpoint in store.list_location_title_checkpoints(operation_id).await? {
                    if let Some(mut row) = store
                        .transfer_title(operation_id, &checkpoint.title_id)
                        .await?
                    {
                        row.state = checkpoint.state;
                        row.files_done = checkpoint.files_verified;
                        row.copy_bytes = checkpoint.bytes_verified.max(0) as u64;
                        row.verification_bytes = row.copy_bytes;
                        row.detail = checkpoint.detail;
                        store.upsert_transfer_title(operation_id, &row).await?;
                    }
                }
            }
            store.mark_transfer_titles_initialized(operation_id).await?;
        }
        // Tag the projection with its starting version. Concurrent writes may
        // make it slightly stale; the next notification supplies a newer view.
        // Reads must neither invalidate each other nor fail under write churn.
        {
            let version = registry.transfers.version();
            let Some(operation) = store.get_location_operation(operation_id).await? else {
                return Ok(None);
            };
            let high_water = store.transfer_high_water(operation_id).await?;
            let total_count = store.transfer_title_count(operation_id).await?;
            let mut titles = match offset {
                Some(offset) => {
                    store
                        .transfer_title_page(operation_id, offset, limit)
                        .await?
                }
                None => Vec::new(),
            };
            for title in &mut titles {
                registry.transfers.overlay(operation_id, title);
            }
            let (_, eta_seconds) = registry.transfers.progress(&operation, high_water);
            // Never display aggregate work ahead of its durable high-water
            // checkpoint: even a page reload after a crash cannot regress it.
            let progress_basis_points = if matches!(
                operation.state,
                super::model::LocationOperationState::Completed
                    | super::model::LocationOperationState::CompletedWithWarnings
            ) {
                10_000
            } else {
                high_water.clamp(0, 9_999)
            };
            let has_more =
                offset.is_some_and(|offset| offset.max(0) + (titles.len() as i64) < total_count);
            Ok(Some(TransferSnapshot {
                version,
                operation,
                progress_basis_points,
                eta_seconds,
                titles,
                total_count,
                has_more,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::model::{
        LocationExecutionMode, LocationOperationState, LocationOperationType, VerificationDepth,
    };
    use scryer_domain::ImportTransferPhase;

    #[tokio::test]
    async fn snapshot_reads_survive_churn_and_do_not_advance_revisions() {
        use crate::location::test_support::{InMemoryLocationOperationStore, queued_operation};
        let (mut app, user) = crate::lib_tests::bootstrap();
        let store = Arc::new(InMemoryLocationOperationStore::new());
        store.insert_operation(queued_operation(
            "op",
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
            VerificationDepth::Full,
        ));
        app.services.library.location_operations = store.clone();
        app.initialize_transfer_generation().await.unwrap();
        let hub = app.runtime.library.location_runners.transfers.clone();
        let mut subscriber = hub.subscribe();
        let initial = hub.version();
        let (summary, page) = tokio::join!(
            app.location_transfer_snapshot(&user, "op", None, 50),
            app.location_transfer_snapshot(&user, "op", Some(0), 50),
        );
        assert_eq!(summary.unwrap().unwrap().version, initial);
        assert_eq!(page.unwrap().unwrap().version, initial);
        assert_eq!(hub.version(), initial);
        assert!(!subscriber.has_changed().unwrap());
        let churn = hub.clone();
        *store.operation_read_hook.lock().unwrap() = Some(Box::new(move || {
            for index in 0..100 {
                churn.file_done("op", "title", &index.to_string(), 1);
            }
        }));
        let snapshot = app
            .location_transfer_snapshot(&user, "op", Some(0), 50)
            .await
            .unwrap()
            .unwrap();
        assert!(snapshot.version > initial);
        assert!(snapshot.version < hub.version());
        assert!(subscriber.has_changed().unwrap());
        subscriber.borrow_and_update();
        *store.operation_read_hook.lock().unwrap() = None;
        let newest = app
            .location_transfer_snapshot(&user, "op", None, 50)
            .await
            .unwrap()
            .unwrap();
        assert!(newest.version > snapshot.version);
        assert_eq!(newest.version, hub.version());
        assert!(!subscriber.has_changed().unwrap());
    }

    #[test]
    fn failed_and_blocked_work_does_not_hide_healthy_work_eta() {
        let mut operation = crate::location::test_support::queued_operation(
            "op",
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
            VerificationDepth::Full,
        );
        operation.state = LocationOperationState::Moving;
        operation.started_at = Some(chrono::Utc::now());
        operation.counters.bytes_total = 1000;
        let hub = TransferHub::default();
        hub.file_update(
            "op",
            "failed",
            "file",
            100,
            ImportTransferPhase::Copying,
            50,
        );
        hub.file_failed("op", "failed", "file", 100);
        hub.file_failed("op", "blocked", "", 200);
        hub.file_update(
            "op",
            "healthy",
            "file",
            700,
            ImportTransferPhase::Copying,
            200,
        );
        let now = Instant::now();
        let mut eta = TransferEta::new(now - Duration::from_secs(20));
        for seconds in [5, 10, 15, 20] {
            eta.add_useful(500, now - Duration::from_secs(20 - seconds));
        }
        hub.state
            .lock()
            .unwrap()
            .operations
            .get_mut("op")
            .unwrap()
            .eta = eta;
        // 250 actual bytes earn progress; 550 abandoned work bytes earn none.
        // Healthy work has 1,200 bytes left at 100 useful bytes/second.
        assert_eq!(hub.progress(&operation, 0), (1250, Some(12)));
        hub.file_failed("op", "failed", "file", 100);
        assert_eq!(hub.progress(&operation, 0), (1250, Some(12)));
        hub.state
            .lock()
            .unwrap()
            .operations
            .get_mut("op")
            .unwrap()
            .eta
            .last_useful = now - Duration::from_secs(31);
        assert_eq!(hub.progress(&operation, 0), (1250, None));
    }

    #[test]
    fn unequal_files_retries_and_restart_keep_high_water_without_claiming_verification() {
        let mut operation = crate::location::test_support::queued_operation(
            "op",
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
            VerificationDepth::Full,
        );
        operation.state = LocationOperationState::Moving;
        operation.started_at = Some(chrono::Utc::now());
        operation.counters.bytes_total = 1000;
        let hub = TransferHub::default();
        hub.file_done("op", "title", "small", 100);
        assert_eq!(hub.progress(&operation, 0).0, 1000);
        hub.file_update(
            "op",
            "title",
            "large",
            900,
            ImportTransferPhase::Copying,
            900,
        );
        assert_eq!(hub.progress(&operation, 0).0, 5500);
        hub.file_update(
            "op",
            "title",
            "large",
            900,
            ImportTransferPhase::Verifying,
            450,
        );
        assert_eq!(hub.progress(&operation, 0).0, 7750);
        hub.file_update(
            "op",
            "title",
            "large",
            900,
            ImportTransferPhase::Verifying,
            0,
        );
        assert_eq!(hub.progress(&operation, 0).0, 7750);
        let restarted = TransferHub::default();
        assert_eq!(restarted.progress(&operation, 7750), (7750, None));
        assert!(
            restarted.state.lock().unwrap().operations["op"]
                .files
                .is_empty()
        );
        hub.file_done("op", "title", "large", 900);
        assert_eq!(hub.progress(&operation, 0).0, 9999);
        operation.state = LocationOperationState::Completed;
        assert_eq!(hub.progress(&operation, 9999).0, 10000);
    }

    #[test]
    fn retries_and_instant_hardlinks_do_not_inflate_useful_throughput() {
        let hub = TransferHub::default();
        hub.file_update(
            "op",
            "title",
            "file",
            100,
            ImportTransferPhase::Copying,
            100,
        );
        hub.file_update("op", "title", "file", 100, ImportTransferPhase::Copying, 0);
        hub.file_update(
            "op",
            "title",
            "file",
            100,
            ImportTransferPhase::Copying,
            100,
        );
        hub.file_update(
            "op",
            "title",
            "file",
            100,
            ImportTransferPhase::Verifying,
            50,
        );
        hub.file_update(
            "op",
            "title",
            "file",
            100,
            ImportTransferPhase::Verifying,
            0,
        );
        hub.file_update(
            "op",
            "title",
            "file",
            100,
            ImportTransferPhase::Verifying,
            100,
        );
        hub.file_done("op", "title", "hardlink", 1_000_000);
        assert_eq!(hub.state.lock().unwrap().operations["op"].eta.useful, 200);
    }

    #[test]
    fn copy_retry_restores_remaining_crc_work_without_recounting_throughput() {
        let hub = TransferHub::default();
        hub.file_update(
            "op",
            "title",
            "file",
            100,
            ImportTransferPhase::Copying,
            100,
        );
        hub.file_update(
            "op",
            "title",
            "file",
            100,
            ImportTransferPhase::Verifying,
            50,
        );
        hub.file_update("op", "title", "file", 100, ImportTransferPhase::Copying, 0);
        let state = hub.state.lock().unwrap();
        let operation = &state.operations["op"];
        let file = &operation.files[&("title".into(), "file".into())];
        assert_eq!((file.copied, file.verified), (0, 0));
        assert_eq!(operation.eta.useful, 150);
    }

    #[test]
    fn same_process_resume_rebuilds_credits_and_preserves_only_display_high_water() {
        let mut operation = crate::location::test_support::queued_operation(
            "op",
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
            VerificationDepth::Full,
        );
        operation.state = LocationOperationState::Moving;
        operation.started_at = Some(chrono::Utc::now());
        operation.counters.bytes_total = 200;
        operation.counters.bytes_processed = 100;
        let hub = TransferHub::default();
        hub.file_done("op", "first", "file", 100);
        assert_eq!(hub.progress(&operation, 0).0, 5000);
        hub.file_failed("op", "second", "file", 100);
        hub.title_finalized("op", Duration::from_secs(2));
        hub.begin("op", 100);
        assert_eq!(hub.progress(&operation, 5000), (5000, None));
        {
            let state = hub.state.lock().unwrap();
            let telemetry = &state.operations["op"];
            assert!(telemetry.files.is_empty());
            assert!(telemetry.abandoned_work.is_empty());
            assert_eq!(telemetry.finalized, 0);
            assert_eq!(telemetry.eta.samples, 0);
        }
        hub.file_update(
            "op",
            "second",
            "file",
            100,
            ImportTransferPhase::Verifying,
            50,
        );
        assert_eq!(hub.progress(&operation, 5000).0, 8750);
        hub.begin("op", 100);
        assert_eq!(hub.progress(&operation, 8750), (8750, None));
    }

    #[test]
    fn failed_title_still_exposes_a_waiting_siblings_later_copy_and_verification() {
        let hub = TransferHub::default();
        hub.file_update(
            "op",
            "title",
            "failed",
            100,
            ImportTransferPhase::Copying,
            0,
        );
        hub.file_failed("op", "title", "failed", 100);
        hub.file_update(
            "op",
            "title",
            "waiting",
            100,
            ImportTransferPhase::Waiting,
            0,
        );
        let mut row = TransferTitle {
            title_id: "title".into(),
            name: "Title".into(),
            sequence: 0,
            state: TitleCheckpointState::Failed,
            files_total: 2,
            files_done: 0,
            bytes_total: 200,
            copy_bytes: 0,
            verification_bytes: 0,
            current_file: None,
            copying: 0,
            verifying: 0,
            detail: Some("first file failed".into()),
        };
        hub.overlay("op", &mut row);
        assert_eq!(row.rank(), 2);
        hub.file_update(
            "op",
            "title",
            "waiting",
            100,
            ImportTransferPhase::Copying,
            25,
        );
        hub.overlay("op", &mut row);
        assert_eq!((row.copying, row.verifying, row.rank()), (1, 0, 1));
        assert_eq!(row.current_file.as_deref(), Some("waiting"));
        hub.file_update(
            "op",
            "title",
            "waiting",
            100,
            ImportTransferPhase::Verifying,
            50,
        );
        hub.overlay("op", &mut row);
        assert_eq!((row.copying, row.verifying, row.rank()), (0, 1, 0));
        hub.file_done("op", "title", "waiting", 100);
        hub.overlay("op", &mut row);
        assert_eq!((row.copying, row.verifying, row.rank()), (0, 0, 2));
        assert!(row.current_file.is_none());
        assert!(row.detail.is_some());
    }

    #[test]
    fn hot_order_prioritizes_overlapping_verification_over_copying() {
        let make = |state| TransferTitle {
            title_id: "title".into(),
            name: "name".into(),
            sequence: 0,
            state,
            files_total: 5,
            files_done: 0,
            bytes_total: 500,
            copy_bytes: 0,
            verification_bytes: 0,
            current_file: None,
            copying: 0,
            verifying: 0,
            detail: None,
        };
        let mut mixed = make(TitleCheckpointState::Moving);
        mixed.copying = 2;
        mixed.verifying = 3;
        assert_eq!(mixed.rank(), 0);
        let states = [
            TitleCheckpointState::Verifying,
            TitleCheckpointState::Moving,
            TitleCheckpointState::Failed,
            TitleCheckpointState::Pending,
            TitleCheckpointState::Completed,
        ];
        assert_eq!(states.map(|state| make(state).rank()), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn eta_measures_concurrent_aggregate_wall_time_and_crc_tail() {
        let start = Instant::now();
        let mut aggregate = TransferEta::new(start);
        let mut batched = TransferEta::new(start);
        for second in 1..=20 {
            let now = start + Duration::from_secs(second);
            aggregate.add_useful(200, now);
            batched.add_useful(100, now);
            batched.add_useful(100, now);
        }
        let estimate = aggregate
            .estimate(2000, start + Duration::from_secs(20))
            .unwrap();
        assert_eq!(estimate, 10);
        assert!(
            (batched
                .estimate(2000, start + Duration::from_secs(20))
                .unwrap()
                - estimate)
                .abs()
                <= 5
        );
        for second in 21..=80 {
            aggregate.add_useful(50, start + Duration::from_secs(second));
        }
        assert!(
            aggregate
                .estimate(2000, start + Duration::from_secs(80))
                .unwrap()
                > estimate
        );
    }

    #[test]
    fn eta_warms_up_adapts_and_expires_without_countdown() {
        let start = Instant::now();
        let mut eta = TransferEta::new(start);
        assert_eq!(eta.estimate(1000, start), None);
        for second in 1..=20 {
            eta.add_useful(100, start + Duration::from_secs(second));
        }
        assert_eq!(
            eta.estimate(1000, start + Duration::from_secs(20)),
            Some(10)
        );
        for second in 21..=50 {
            eta.add_useful(10, start + Duration::from_secs(second));
        }
        assert!(eta.estimate(1000, start + Duration::from_secs(50)).unwrap() > 10);
        assert_eq!(eta.estimate(1000, start + Duration::from_secs(80)), None);
    }
}
