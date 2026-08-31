use std::sync::Arc;

/// Process-restart callback supplied by the executable host.
///
/// The application crate owns this small boundary so an upgrade can schedule
/// its restart without depending on an HTTP or GraphQL layer.
#[derive(Clone)]
pub struct ApplicationUpgradeRestartHandle {
    schedule_fn: Arc<dyn Fn() + Send + Sync>,
    exit_fn: Arc<dyn Fn() + Send + Sync>,
}

impl ApplicationUpgradeRestartHandle {
    pub fn new(schedule: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            schedule_fn: Arc::new(schedule),
            exit_fn: Arc::new(|| {}),
        }
    }

    pub fn new_with_exit(
        schedule: impl Fn() + Send + Sync + 'static,
        exit: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            schedule_fn: Arc::new(schedule),
            exit_fn: Arc::new(exit),
        }
    }

    pub fn schedule_restart(&self) {
        (self.schedule_fn)();
    }

    /// Request a delayed exit without launching a replacement process.
    /// Windows upgrade helpers use this after they have been detached.
    pub fn schedule_exit(&self) {
        (self.exit_fn)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn exit_only_callback_does_not_schedule_a_restart() {
        let restarted = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let handle = ApplicationUpgradeRestartHandle::new_with_exit(
            {
                let restarted = restarted.clone();
                move || restarted.store(true, Ordering::SeqCst)
            },
            {
                let exited = exited.clone();
                move || exited.store(true, Ordering::SeqCst)
            },
        );

        handle.schedule_exit();
        assert!(exited.load(Ordering::SeqCst));
        assert!(!restarted.load(Ordering::SeqCst));
    }
}
