/// RAII counter for the open legacy download-queue broadcast subscriptions.
///
/// The gauge is incremented on construction and decremented on drop, so the
/// three ways a forwarding task can end — normal exit, panic, `JoinHandle`
/// abort — all decrement exactly once. `Drop` cannot run twice for one value,
/// so the balance is a type-system property rather than a review convention.
struct LegacySubscriptionGauge;

impl LegacySubscriptionGauge {
    fn new() -> Self {
        metrics::gauge!(crate::services::DOWNLOAD_QUEUE_LEGACY_SUBSCRIPTIONS).increment(1.0);
        Self
    }
}

impl Drop for LegacySubscriptionGauge {
    fn drop(&mut self) {
        metrics::gauge!(crate::services::DOWNLOAD_QUEUE_LEGACY_SUBSCRIPTIONS).decrement(1.0);
    }
}

impl AppUseCase {
    pub fn subscribe_download_queue(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<DownloadQueueItem>>> {
        if !actor
            .authorization
            .has_any_library_permission(scryer_domain::LibraryPermission::View)
        {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }

        let (tx, rx) = broadcast::channel(32);
        let app = self.clone();
        let actor = actor.clone();
        let mut sync_rx = self.runtime.acquisition.download_queue_snapshot.subscribe();
        tokio::spawn(async move {
            // Owned by the task, so an abort or a panic decrements too. The
            // hand-written decrement at the end of the body was skipped
            // whenever the task did not run to completion, and the gauge only
            // ever drifted up.
            let _subscription = LegacySubscriptionGauge::new();
            loop {
                let items = match app.list_download_queue_snapshot(&actor).await {
                    Ok(items) => items,
                    Err(error) => {
                        tracing::warn!(
                            "download queue subscription snapshot filter failed: {error}"
                        );
                        break;
                    }
                };
                if tx.send(items).is_err() || sync_rx.changed().await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }

    pub fn subscribe_download_queue_sync(
        &self,
        actor: &User,
    ) -> AppResult<tokio::sync::watch::Receiver<DownloadQueueSync>> {
        if !actor
            .authorization
            .has_any_library_permission(scryer_domain::LibraryPermission::View)
        {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }
        Ok(self.runtime.acquisition.download_queue_snapshot.subscribe())
    }
}

#[cfg(test)]
mod legacy_subscription_gauge_tests {
    use super::LegacySubscriptionGauge;
    use metrics::with_local_recorder;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

    /// Net change in the legacy-subscription gauge since the previous reading.
    ///
    /// `Snapshotter::snapshot` drains the debugging registry, so consecutive
    /// readings are deltas, not absolutes. `None` means the family was never
    /// touched at all, which is a different failure from "touched and balanced".
    fn subscription_delta(snapshotter: &Snapshotter) -> Option<f64> {
        snapshotter.snapshot().into_vec().into_iter().find_map(
            |(key, _unit, _description, value)| match value {
                DebugValue::Gauge(inner)
                    if key.key().name() == crate::services::DOWNLOAD_QUEUE_LEGACY_SUBSCRIPTIONS =>
                {
                    Some(*inner)
                }
                _ => None,
            },
        )
    }

    #[test]
    fn guard_increments_on_construction_and_decrements_on_drop() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, || {
            let guard = LegacySubscriptionGauge::new();
            assert_eq!(subscription_delta(&snapshotter), Some(1.0));
            // `Drop` runs exactly once per value, so the decrement cannot be
            // duplicated or skipped by any caller.
            drop(guard);
            assert_eq!(subscription_delta(&snapshotter), Some(-1.0));
        });
    }

    #[test]
    fn an_aborted_forwarding_task_still_decrements() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("current-thread runtime");
        with_local_recorder(&recorder, || {
            runtime.block_on(async {
                let (constructed_tx, constructed_rx) = tokio::sync::oneshot::channel();
                let handle = tokio::spawn(async move {
                    let _subscription = LegacySubscriptionGauge::new();
                    let _ = constructed_tx.send(());
                    // Stands in for the forwarding loop parked on `changed()`.
                    std::future::pending::<()>().await;
                });
                constructed_rx.await.expect("guard constructed");
                assert_eq!(subscription_delta(&snapshotter), Some(1.0));

                handle.abort();
                assert!(handle.await.unwrap_err().is_cancelled());
                assert_eq!(
                    subscription_delta(&snapshotter),
                    Some(-1.0),
                    "an aborted task must not leak an open subscription"
                );
            });
        });
    }
}
