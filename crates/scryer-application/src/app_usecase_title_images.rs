use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{info, warn};

const IMAGE_COLLECT_WINDOW: Duration = Duration::from_millis(50);
const IMAGE_MAX_BATCH: usize = 256;
const IMAGE_RETRY_BASE: Duration = Duration::from_secs(10);
const IMAGE_RETRY_MAX: Duration = Duration::from_secs(300);
const IMAGE_CONCURRENT_WORKERS: usize = 2;

pub async fn start_background_poster_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    start_background_image_loop(app, token, TitleImageKind::Poster, "poster").await
}

pub async fn start_background_banner_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    start_background_image_loop(app, token, TitleImageKind::Banner, "banner").await
}

async fn start_background_image_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    kind: TitleImageKind,
    label: &'static str,
) {
    let wake: Arc<Notify> = match kind {
        TitleImageKind::Poster => app.services.poster_wake.clone(),
        TitleImageKind::Banner => app.services.banner_wake.clone(),
        TitleImageKind::Fanart => {
            warn!("{label} loop: no wake signal configured for this kind");
            return;
        }
    };

    info!(
        kind = label,
        collect_window_ms = IMAGE_COLLECT_WINDOW.as_millis(),
        max_batch = IMAGE_MAX_BATCH,
        concurrent_workers = IMAGE_CONCURRENT_WORKERS,
        retry_base_secs = IMAGE_RETRY_BASE.as_secs(),
        retry_max_secs = IMAGE_RETRY_MAX.as_secs(),
        "background image loop started"
    );

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!(kind = label, "background image loop shutting down");
                return;
            }
            _ = wake.notified() => {}
        }

        let mut retry_delay = IMAGE_RETRY_BASE;
        'drain: loop {
            tokio::time::sleep(IMAGE_COLLECT_WINDOW).await;

            if token.is_cancelled() {
                return;
            }

            let batch = match app
                .services
                .title_images
                .list_titles_requiring_image_refresh(kind, IMAGE_MAX_BATCH)
                .await
            {
                Ok(batch) => batch,
                Err(error) => {
                    warn!(error = %error, kind = label, "image loop: failed to list pending image sync work");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue 'drain;
                }
            };

            if batch.is_empty() {
                info!(kind = label, "image loop: no pending work");
                break 'drain;
            }

            let batch_len = batch.len();
            let success_count = AtomicUsize::new(0);
            let failure_count = AtomicUsize::new(0);
            info!(
                count = batch_len,
                kind = label,
                "image loop: processing batch"
            );

            let semaphore =
                std::sync::Arc::new(tokio::sync::Semaphore::new(IMAGE_CONCURRENT_WORKERS));
            let mut join_set = tokio::task::JoinSet::new();

            for task in batch {
                let sem = semaphore.clone();
                let app = app.clone();
                join_set.spawn(async move {
                    let _permit = sem.acquire().await.expect("semaphore should not be closed");
                    let started_at = std::time::Instant::now();
                    info!(
                        title_id = %task.title_id,
                        source_url = %task.source_url,
                        cached_source_url = ?task.cached_source_url,
                        kind = label,
                        "image loop: refreshing image"
                    );
                    match app
                        .services
                        .title_image_processor
                        .fetch_and_process_image(kind, &task.source_url)
                        .await
                    {
                        Ok(replacement) => {
                            let storage_mode = replacement.storage_mode.as_str();
                            let variant_count = replacement.variants.len();
                            if let Err(error) = app
                                .services
                                .title_images
                                .replace_title_image(&task.title_id, replacement)
                                .await
                            {
                                warn!(
                                    error = %error,
                                    elapsed_ms = started_at.elapsed().as_millis(),
                                    title_id = %task.title_id,
                                    source_url = %task.source_url,
                                    kind = label,
                                    "image loop: failed to store processed image"
                                );
                                return false;
                            }
                            info!(
                                elapsed_ms = started_at.elapsed().as_millis(),
                                title_id = %task.title_id,
                                source_url = %task.source_url,
                                storage_mode,
                                variant_count,
                                kind = label,
                                "image loop: cached image"
                            );
                            true
                        }
                        Err(error) => {
                            warn!(
                                error = %error,
                                elapsed_ms = started_at.elapsed().as_millis(),
                                title_id = %task.title_id,
                                source_url = %task.source_url,
                                cached_source_url = ?task.cached_source_url,
                                kind = label,
                                "image loop: failed to fetch/process image"
                            );
                            false
                        }
                    }
                });
            }

            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(true) => {
                        success_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(false) => {
                        failure_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(err) => {
                        warn!(error = %err, kind = label, "image loop: task panicked");
                        failure_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            let succeeded = success_count.load(Ordering::Relaxed);
            let failed = failure_count.load(Ordering::Relaxed);
            let had_failures = failed > 0;

            info!(
                processed = batch_len,
                succeeded,
                failed,
                kind = label,
                "image loop: batch complete"
            );

            if had_failures {
                info!(
                    retry_secs = retry_delay.as_secs(),
                    processed = batch_len,
                    succeeded,
                    failed,
                    kind = label,
                    "image loop: some images failed, scheduling retry"
                );
                let new_work = tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(retry_delay) => false,
                    _ = wake.notified() => true,
                };

                if new_work {
                    retry_delay = IMAGE_RETRY_BASE;
                } else {
                    retry_delay = (retry_delay * 2).min(IMAGE_RETRY_MAX);
                }

                continue 'drain;
            }
        }

        info!(kind = label, "image loop: queue drained, parking");
    }
}
