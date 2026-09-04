impl AppUseCase {
    pub async fn subscribe_download_queue(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<DownloadQueueItem>>> {
        if !self
            .has_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?
        {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }

        let (tx, rx) = broadcast::channel(32);
        let app = self.clone();
        let actor = actor.clone();
        let mut sync_rx = self.runtime.acquisition.download_queue_snapshot.subscribe();
        metrics::gauge!("scryer_download_queue_legacy_subscriptions").increment(1.0);
        tokio::spawn(async move {
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
            metrics::gauge!("scryer_download_queue_legacy_subscriptions").decrement(1.0);
        });
        Ok(rx)
    }

    pub async fn subscribe_download_queue_sync(
        &self,
        actor: &User,
    ) -> AppResult<tokio::sync::watch::Receiver<DownloadQueueSync>> {
        if !self
            .has_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?
        {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }
        Ok(self.runtime.acquisition.download_queue_snapshot.subscribe())
    }
}
