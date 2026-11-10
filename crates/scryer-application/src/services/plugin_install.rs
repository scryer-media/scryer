use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginInstallOperationKind {
    Install,
    Upgrade,
}

impl PluginInstallOperationKind {
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Upgrade => "upgrade",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginInstallState {
    Downloading,
    Verifying,
    Installing,
    Succeeded,
    Failed,
}

impl PluginInstallState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Downloading => "Downloading",
            Self::Verifying => "Verifying",
            Self::Installing => "Installing",
            Self::Succeeded => "Plugin installed",
            Self::Failed => "Plugin install failed",
        }
    }

    pub const fn step_index(self) -> i32 {
        match self {
            Self::Downloading => 1,
            Self::Verifying => 2,
            Self::Installing => 3,
            Self::Succeeded | Self::Failed => 3,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginInstallProgressSnapshot {
    pub plugin_id: String,
    pub operation_kind: PluginInstallOperationKind,
    pub state: PluginInstallState,
    pub label: String,
    pub step_index: i32,
    pub step_count: i32,
    pub message: Option<String>,
    pub error: Option<String>,
}

impl PluginInstallProgressSnapshot {
    const STEP_COUNT: i32 = 3;

    fn new(
        plugin_id: String,
        operation_kind: PluginInstallOperationKind,
        state: PluginInstallState,
        message: Option<String>,
        error: Option<String>,
    ) -> Self {
        Self {
            plugin_id,
            operation_kind,
            state,
            label: state.label().to_string(),
            step_index: state.step_index(),
            step_count: Self::STEP_COUNT,
            message,
            error,
        }
    }

    fn with_state(
        &self,
        state: PluginInstallState,
        message: Option<String>,
        error: Option<String>,
    ) -> Self {
        Self::new(
            self.plugin_id.clone(),
            self.operation_kind,
            state,
            message,
            error,
        )
    }
}

#[derive(Debug)]
struct ActivePluginInstallOperation {
    actor_user_id: String,
    snapshot_key: (String, String),
    generation: u64,
}

#[derive(Clone, Debug)]
struct PluginInstallSnapshotHandle {
    generation: u64,
    active: bool,
    tx: tokio::sync::watch::Sender<PluginInstallProgressSnapshot>,
}

#[derive(Default)]
struct PluginInstallOrchestratorState {
    next_generation: u64,
    active_by_plugin: HashMap<String, ActivePluginInstallOperation>,
    snapshots_by_actor_plugin: HashMap<(String, String), PluginInstallSnapshotHandle>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginInstallInProgressError {
    pub plugin_id: String,
}

#[derive(Clone, Default)]
pub struct PluginInstallOrchestrator {
    state: Arc<tokio::sync::Mutex<PluginInstallOrchestratorState>>,
}

impl PluginInstallOrchestrator {
    const FINISHED_SNAPSHOT_TTL: tokio::time::Duration = tokio::time::Duration::from_secs(15);

    fn normalize_plugin_key(plugin_id: &str) -> String {
        plugin_id.trim().to_ascii_lowercase()
    }

    fn actor_snapshot_key(actor_user_id: &str, plugin_key: &str) -> (String, String) {
        (actor_user_id.to_string(), plugin_key.to_string())
    }

    pub async fn begin(
        &self,
        actor_user_id: &str,
        plugin_id: &str,
        operation_kind: PluginInstallOperationKind,
    ) -> Result<PluginInstallProgressSnapshot, PluginInstallInProgressError> {
        let plugin_key = Self::normalize_plugin_key(plugin_id);
        let snapshot_key = Self::actor_snapshot_key(actor_user_id, &plugin_key);
        let mut state = self.state.lock().await;
        if state.active_by_plugin.contains_key(&plugin_key) {
            return Err(PluginInstallInProgressError {
                plugin_id: plugin_key,
            });
        }

        state.next_generation += 1;
        let generation = state.next_generation;
        let snapshot = PluginInstallProgressSnapshot::new(
            plugin_key.clone(),
            operation_kind,
            PluginInstallState::Downloading,
            None,
            None,
        );
        let (tx, _rx) = tokio::sync::watch::channel(snapshot.clone());
        state.snapshots_by_actor_plugin.insert(
            snapshot_key.clone(),
            PluginInstallSnapshotHandle {
                generation,
                active: true,
                tx,
            },
        );
        state.active_by_plugin.insert(
            plugin_key,
            ActivePluginInstallOperation {
                actor_user_id: actor_user_id.to_string(),
                snapshot_key,
                generation,
            },
        );
        Ok(snapshot)
    }

    pub async fn subscribe(
        &self,
        actor_user_id: &str,
        plugin_id: &str,
    ) -> Option<tokio::sync::watch::Receiver<PluginInstallProgressSnapshot>> {
        let plugin_key = Self::normalize_plugin_key(plugin_id);
        let snapshot_key = Self::actor_snapshot_key(actor_user_id, &plugin_key);
        let state = self.state.lock().await;
        state
            .snapshots_by_actor_plugin
            .get(&snapshot_key)
            .map(|handle| handle.tx.subscribe())
    }

    /// Monotonic count of accepted install/upgrade operations, including expired snapshots.
    pub async fn operation_generation(&self) -> u64 {
        self.state.lock().await.next_generation
    }

    /// Plugins whose install/upgrade slot is currently held, by any actor —
    /// including the system actor that runs scheduled automatic updates.
    pub async fn active_plugin_ids(&self) -> HashSet<String> {
        let state = self.state.lock().await;
        state.active_by_plugin.keys().cloned().collect()
    }

    pub async fn transition(
        &self,
        actor_user_id: &str,
        plugin_id: &str,
        next_state: PluginInstallState,
        message: Option<String>,
        error: Option<String>,
    ) {
        let plugin_key = Self::normalize_plugin_key(plugin_id);
        let snapshot_key = Self::actor_snapshot_key(actor_user_id, &plugin_key);
        let mut state = self.state.lock().await;
        let generation = {
            let Some(handle) = state.snapshots_by_actor_plugin.get_mut(&snapshot_key) else {
                return;
            };
            let current = handle.tx.borrow().clone();
            let _ = handle
                .tx
                .send(current.with_state(next_state, message, error));
            if matches!(
                next_state,
                PluginInstallState::Succeeded | PluginInstallState::Failed
            ) {
                handle.active = false;
                Some(handle.generation)
            } else {
                None
            }
        };
        if let Some(generation) = generation {
            let should_release = state
                .active_by_plugin
                .get(&plugin_key)
                .is_some_and(|active| {
                    active.actor_user_id == actor_user_id
                        && active.generation == generation
                        && active.snapshot_key == snapshot_key
                });
            if should_release {
                state.active_by_plugin.remove(&plugin_key);
            }
            drop(state);
            self.schedule_finished_snapshot_cleanup(snapshot_key, generation);
        }
    }

    fn schedule_finished_snapshot_cleanup(&self, snapshot_key: (String, String), generation: u64) {
        let orchestrator = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Self::FINISHED_SNAPSHOT_TTL).await;
            let mut state = orchestrator.state.lock().await;
            if state
                .snapshots_by_actor_plugin
                .get(&snapshot_key)
                .is_some_and(|handle| handle.generation == generation && !handle.active)
            {
                state.snapshots_by_actor_plugin.remove(&snapshot_key);
            }
        });
    }
}
