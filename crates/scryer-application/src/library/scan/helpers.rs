use super::*;
use crate::library::library::library_scan_cancel_requested;
use crate::library_scan_coordinator::{LibraryScanCoordinator, LibraryScanProjectionReplay};
use tokio_util::sync::CancellationToken;

const LIBRARY_SCAN_DISCOVERY_WORK_QUEUE_CAPACITY: usize = 16;

pub(crate) fn spawn_library_discovery_queue<T>(
    app: AppUseCase,
    session_id: String,
    mut discovered_batches: tokio::sync::mpsc::Receiver<AppResult<Vec<T>>>,
    track_file_total: bool,
    mark_complete_on_drain: bool,
    cancel_token: Option<CancellationToken>,
) -> tokio::sync::mpsc::Receiver<AppResult<Vec<T>>>
where
    T: Send + 'static,
{
    let (queued_batches_tx, queued_batches_rx) =
        tokio::sync::mpsc::channel(LIBRARY_SCAN_DISCOVERY_WORK_QUEUE_CAPACITY);

    tokio::spawn(async move {
        let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.clone());
        while let Some(batch_result) =
            await_cancellable(cancel_token.as_ref(), discovered_batches.recv())
                .await
                .flatten()
        {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                return;
            }
            let batch = match batch_result {
                Ok(batch) => batch,
                Err(error) => {
                    let _ = queued_batches_tx.send(Err(error)).await;
                    return;
                }
            };

            if batch.is_empty() {
                continue;
            }

            coordinator
                .register_discovery_batch(batch.len(), track_file_total)
                .await;
            coordinator.publish_progress().await;
            let Some(send_result) =
                await_cancellable(cancel_token.as_ref(), queued_batches_tx.send(Ok(batch))).await
            else {
                return;
            };
            if send_result.is_err() {
                return;
            }
        }

        if mark_complete_on_drain {
            coordinator.mark_discovery_complete(track_file_total).await;
            coordinator.publish_progress().await;
        }
    });

    queued_batches_rx
}

pub(crate) fn require_directory_library_path(library_path: &str) -> AppResult<&Path> {
    let root = Path::new(library_path);
    if !root.is_dir() {
        return Err(AppError::Validation(format!(
            "library path is not a directory: {library_path}"
        )));
    }

    Ok(root)
}

pub(crate) struct LibraryScanSessionDropGuard {
    app: AppUseCase,
    session_id: String,
    armed: bool,
}

impl LibraryScanSessionDropGuard {
    pub(crate) fn new(app: AppUseCase, session_id: String) -> Self {
        Self {
            app,
            session_id,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for LibraryScanSessionDropGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let app = self.app.clone();
        let session_id = self.session_id.clone();
        tokio::spawn(async move {
            LibraryScanCoordinator::new(app, session_id).fail().await;
        });
    }
}

pub(crate) async fn wait_for_projected_library_scan_session(
    app: &AppUseCase,
    session_id: &str,
) -> AppResult<LibraryScanSession> {
    let mut receiver = app.runtime.library.library_scan_tracker.subscribe();
    // One fold that survives the loop. Each pass folds only the events
    // appended since the previous one, which is the same projection a fresh
    // replay from sequence 0 would produce - see `LibraryScanProjectionReplay`.
    let mut replay = LibraryScanProjectionReplay::new(session_id);

    loop {
        if let Some(session) = app
            .runtime
            .library
            .library_scan_tracker
            .get_session(session_id)
            .await
            && (matches!(
                session.status,
                LibraryScanStatus::Failed | LibraryScanStatus::Canceled
            ) || matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            ) || session.is_ready_to_complete())
        {
            return Ok(session);
        }

        let projected_session = replay.advance(app).await?;
        if let Some(session) = projected_session
            && (matches!(
                session.status,
                LibraryScanStatus::Failed | LibraryScanStatus::Canceled
            ) || matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            ) || session.is_ready_to_complete())
        {
            return Ok(session);
        }

        match receiver.recv().await {
            Ok(session) => {
                if session.session_id == session_id
                    && (matches!(
                        session.status,
                        LibraryScanStatus::Failed | LibraryScanStatus::Canceled
                    ) || matches!(
                        session.status,
                        LibraryScanStatus::Completed | LibraryScanStatus::Warning
                    ) || session.is_ready_to_complete())
                {
                    return Ok(session);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                if let Some(session) = replay.advance(app).await?
                    && (matches!(
                        session.status,
                        LibraryScanStatus::Failed | LibraryScanStatus::Canceled
                    ) || matches!(
                        session.status,
                        LibraryScanStatus::Completed | LibraryScanStatus::Warning
                    ) || session.is_ready_to_complete())
                {
                    return Ok(session);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return Err(AppError::Repository(
                    "library scan tracker broadcast closed before completion".into(),
                ));
            }
        }
    }
}
