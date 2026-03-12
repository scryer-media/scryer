use super::*;
use std::time::Duration;
use tracing::{info, warn};

const POSTER_COLLECT_WINDOW: Duration = Duration::from_millis(50);
const POSTER_MAX_BATCH: usize = 256;
const POSTER_RETRY_BASE: Duration = Duration::from_secs(10);
const POSTER_RETRY_MAX: Duration = Duration::from_secs(300);

pub async fn start_background_poster_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    info!(
        collect_window_ms = POSTER_COLLECT_WINDOW.as_millis(),
        max_batch = POSTER_MAX_BATCH,
        retry_base_secs = POSTER_RETRY_BASE.as_secs(),
        retry_max_secs = POSTER_RETRY_MAX.as_secs(),
        "background poster loop started"
    );

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("background poster loop shutting down");
                return;
            }
            _ = app.services.poster_wake.notified() => {}
        }

        let mut retry_delay = POSTER_RETRY_BASE;
        'drain: loop {
            tokio::time::sleep(POSTER_COLLECT_WINDOW).await;

            if token.is_cancelled() {
                return;
            }

            let batch = match app
                .services
                .title_images
                .list_titles_requiring_image_refresh(TitleImageKind::Poster, POSTER_MAX_BATCH)
                .await
            {
                Ok(batch) => batch,
                Err(error) => {
                    warn!(error = %error, "poster loop: failed to list pending poster sync work");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue 'drain;
                }
            };

            if batch.is_empty() {
                info!("poster loop: no pending poster work");
                break 'drain;
            }

            let batch_len = batch.len();
            let mut had_failures = false;
            let mut success_count = 0usize;
            let mut failure_count = 0usize;
            info!(count = batch_len, "poster loop: processing batch");

            for task in batch {
                let started_at = std::time::Instant::now();
                info!(
                    title_id = %task.title_id,
                    source_url = %task.source_url,
                    cached_source_url = ?task.cached_source_url,
                    "poster loop: refreshing poster"
                );
                match app
                    .services
                    .title_image_processor
                    .fetch_and_process_image(TitleImageKind::Poster, &task.source_url)
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
                                "poster loop: failed to store processed poster"
                            );
                            had_failures = true;
                            failure_count += 1;
                        } else {
                            success_count += 1;
                            info!(
                                elapsed_ms = started_at.elapsed().as_millis(),
                                title_id = %task.title_id,
                                source_url = %task.source_url,
                                storage_mode,
                                variant_count,
                                "poster loop: cached poster"
                            );
                        }
                    }
                    Err(error) => {
                        warn!(
                            error = %error,
                            elapsed_ms = started_at.elapsed().as_millis(),
                            title_id = %task.title_id,
                            source_url = %task.source_url,
                            cached_source_url = ?task.cached_source_url,
                            "poster loop: failed to fetch/process poster"
                        );
                        had_failures = true;
                        failure_count += 1;
                    }
                }
            }

            info!(
                processed = batch_len,
                succeeded = success_count,
                failed = failure_count,
                "poster loop: batch complete"
            );

            if had_failures {
                info!(
                    retry_secs = retry_delay.as_secs(),
                    processed = batch_len,
                    succeeded = success_count,
                    failed = failure_count,
                    "poster loop: some posters failed, scheduling retry"
                );
                let new_work = tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(retry_delay) => false,
                    _ = app.services.poster_wake.notified() => true,
                };

                if new_work {
                    retry_delay = POSTER_RETRY_BASE;
                } else {
                    retry_delay = (retry_delay * 2).min(POSTER_RETRY_MAX);
                }

                continue 'drain;
            }
        }

        info!("poster loop: queue drained, parking");
    }
}
