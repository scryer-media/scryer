use super::*;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Timelike, Utc};
use serde::Serialize;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const SCHEDULE_POLL: Duration = Duration::from_secs(60);

pub(crate) fn artwork_window_open<T: TimeZone>(now: &DateTime<T>) -> bool {
    (3..9).contains(&now.hour())
}

/// Resolve each calendar day's opening independently so DST never becomes a 24-hour timer.
pub(crate) fn next_artwork_window<T: TimeZone>(
    now: &DateTime<T>,
    include_current: bool,
) -> Option<DateTime<Utc>> {
    if include_current && artwork_window_open(now) {
        return Some(now.with_timezone(&Utc));
    }
    let zone = now.timezone();
    let mut date = now.date_naive();
    if now.hour() >= 3 {
        date = date.succ_opt()?;
    }
    for _ in 0..3 {
        // If 03:00 does not exist, use the first valid minute within the window.
        for minute in 3 * 60..9 * 60 {
            let local = date.and_hms_opt(minute / 60, minute % 60, 0)?;
            if let Some(candidate) = zone.from_local_datetime(&local).earliest()
                && candidate > *now
            {
                return Some(candidate.with_timezone(&Utc));
            }
        }
        date = date.succ_opt()?;
    }
    None
}

pub(crate) fn artwork_workers(class: RuntimePerformanceClass) -> usize {
    match class {
        RuntimePerformanceClass::Fast => 4,
        RuntimePerformanceClass::Slow => 2,
    }
}

#[derive(Default, Serialize)]
pub(crate) struct ArtworkEncodingSummary {
    pub concurrency: usize,
    pub processed: usize,
    pub failed: usize,
    pub reason: String,
}

impl ArtworkEncodingSummary {
    pub(crate) fn text(&self) -> String {
        format!(
            "{}; {} concurrent images; {} processed, {} failed",
            self.reason, self.concurrency, self.processed, self.failed
        )
    }
}

impl AppUseCase {
    async fn has_pending_artwork(&self) -> AppResult<bool> {
        if !self
            .services
            .library
            .title_images
            .list_title_image_refresh_work(1, &[])
            .await?
            .is_empty()
        {
            return Ok(true);
        }
        Ok(!self
            .services
            .library
            .image_proxy_cache_control
            .list_cached_jpegs(1, None)
            .await?
            .is_empty())
    }

    pub(crate) async fn artwork_progress(
        &self,
        run: &JobRunRecord,
        summary: &ArtworkEncodingSummary,
    ) -> AppResult<()> {
        let mut current = run.clone();
        current.progress_json = Some(
            serde_json::to_string(summary)
                .map_err(|error| AppError::Repository(error.to_string()))?,
        );
        current.summary_text = Some(summary.text());
        current.updated_at = Utc::now();
        let current = self
            .services
            .events
            .job_runs
            .update_job_run(&current)
            .await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&current, None))
            .await;
        *self.runtime.catalog.artwork_wait_reason.write().await = summary.text();
        Ok(())
    }

    pub(crate) async fn run_artwork_encoding(
        &self,
        run: &JobRunRecord,
    ) -> AppResult<ArtworkEncodingSummary> {
        let (class, elapsed) = self.artwork_cpu_performance().await;
        let workers = artwork_workers(class);
        info!(cpu_class = %class, cpu_probe_elapsed_ms = elapsed, workers, "artwork run CPU budget selected");
        self.services
            .library
            .title_image_processor
            .configure_encoding_workers(workers)
            .await?;
        self.run_artwork_encoding_with_workers(
            run,
            workers,
            &self.runtime.catalog.artwork_shutdown,
            &|| self.runtime.environment.now().with_timezone(&Local),
        )
        .await
    }

    pub(crate) async fn run_artwork_encoding_with_workers(
        &self,
        run: &JobRunRecord,
        workers: usize,
        token: &CancellationToken,
        now: &(dyn Fn() -> DateTime<Local> + Send + Sync),
    ) -> AppResult<ArtworkEncodingSummary> {
        let manual = run.trigger_source == JobTriggerSource::Manual;
        let mut summary = ArtworkEncodingSummary {
            concurrency: workers,
            ..Default::default()
        };
        let mut skipped = Vec::new();
        let mut jpeg_cursor: Option<(String, String)> = None;
        loop {
            if token.is_cancelled() {
                summary.reason = "Stopped for shutdown; remaining artwork is pending".into();
                break;
            }
            if !manual && !artwork_window_open(&now()) {
                summary.reason = "Waiting for the next nightly window".into();
                break;
            }
            if !self
                .runtime
                .library
                .library_scan_tracker
                .list_active()
                .await
                .is_empty()
            {
                summary.reason = "Waiting for library scans".into();
                self.artwork_progress(run, &summary).await?;
                tokio::select! {
                    _ = token.cancelled() => {},
                    _ = self.runtime.library.library_scan_tracker.wait_until_idle() => {},
                    _ = tokio::time::sleep(SCHEDULE_POLL) => {},
                }
                continue;
            }
            // A waiting maintenance writer gets a turn after every bounded group.
            let maintenance = tokio::select! {
                _ = token.cancelled() => continue,
                _ = tokio::time::sleep(SCHEDULE_POLL) => continue,
                guard = self.runtime.catalog.title_image_maintenance_lock.read() => guard,
            };
            if token.is_cancelled() || (!manual && !artwork_window_open(&now())) {
                continue;
            }
            let batch = self
                .services
                .library
                .title_images
                .list_title_image_refresh_work(workers, &skipped)
                .await?;
            let jpegs = if batch.is_empty() {
                self.services
                    .library
                    .image_proxy_cache_control
                    .list_cached_jpegs(
                        workers,
                        jpeg_cursor
                            .as_ref()
                            .map(|(token, variant)| (token.as_str(), variant.as_str())),
                    )
                    .await?
            } else {
                Vec::new()
            };
            if batch.is_empty() && jpegs.is_empty() {
                summary.reason = if summary.failed == 0 {
                    "Artwork backlog drained"
                } else {
                    "Artwork drained; failed images remain pending for a later run"
                }
                .into();
                break;
            }

            summary.reason = "Encoding artwork".into();
            self.artwork_progress(run, &summary).await?;
            let mut tasks = tokio::task::JoinSet::new();
            for task in batch.into_iter().take(workers) {
                // Fetching is part of admission: decoded inputs cannot accumulate outside the budget.
                if token.is_cancelled() || (!manual && !artwork_window_open(&now())) {
                    break;
                }
                let app = self.clone();
                tasks.spawn(async move {
                    let result = app
                        .services
                        .library
                        .title_image_processor
                        .fetch_and_process_image(task.kind, &task.source_url, task.variants.clone())
                        .await;
                    let result = match result {
                        Ok(image) => app
                            .services
                            .library
                            .title_images
                            .upsert_title_image_source_result(&task.title_id, image, None)
                            .await
                            .map(|_| true),
                        Err(error) => Err(error),
                    };
                    (Some(task), result)
                });
            }
            for entry in jpegs.into_iter().take(workers) {
                if token.is_cancelled() || (!manual && !artwork_window_open(&now())) {
                    break;
                }
                jpeg_cursor = Some((entry.token.clone(), entry.variant.clone()));
                let app = self.clone();
                tasks.spawn(async move {
                    let result = app.services.library.image_proxy_cache_control
                        .optimize_cached_jpeg(entry.clone(), app.services.library.title_image_processor.clone())
                        .await;
                    if let Err(error) = &result {
                        warn!(token = %entry.token, variant = %entry.variant, %error, "cached JPEG failed; deferring");
                    }
                    (None, result)
                });
            }
            // The nightly cutoff drains admitted images; shutdown abandons them for a later run.
            let mut panic_error = None;
            loop {
                let result = tokio::select! {
                    biased;
                    _ = token.cancelled() => {
                        tasks.abort_all();
                        break;
                    },
                    result = tasks.join_next() => result,
                };
                let Some(result) = result else { break };
                match result {
                    Ok((_, Ok(true))) => summary.processed += 1,
                    Ok((_, Ok(false))) => {}
                    Ok((task, Err(error))) => {
                        if let Some(task) = task {
                            warn!(title_id = %task.title_id, %error, "artwork image failed; deferring");
                            skipped.push(task);
                        }
                        summary.failed += 1;
                    }
                    Err(error) => {
                        summary.failed += 1;
                        panic_error = Some(error);
                    }
                }
            }
            drop(maintenance);
            self.artwork_progress(run, &summary).await?;
            if let Some(error) = panic_error {
                // Without a task identity we cannot safely exclude a panicking item and keep draining.
                return Err(AppError::Repository(format!(
                    "artwork worker task failed: {error}"
                )));
            }
        }
        self.artwork_progress(run, &summary).await?;
        info!(processed = summary.processed, failed = summary.failed, workers, reason = %summary.reason, "artwork run settled");
        Ok(summary)
    }
}

pub async fn start_background_title_image_loop(app: AppUseCase, token: CancellationToken) {
    // The job runner also serves manual requests. Both kinds observe this shutdown signal.
    let shutdown = app.runtime.catalog.artwork_shutdown.clone();
    let shutdown_watch = token.clone();
    tokio::spawn(async move {
        shutdown_watch.cancelled().await;
        shutdown.cancel();
    });

    let mut pending_wake = true;
    let mut consecutive_failures = 0;
    let mut previous_open = false;
    let mut previous_date: Option<NaiveDate> = None;
    loop {
        if token.is_cancelled() {
            return;
        }
        let now = app.runtime.environment.now().with_timezone(&Local);
        let open = artwork_window_open(&now);
        if open && (!previous_open || previous_date != Some(now.date_naive())) {
            pending_wake = true;
            consecutive_failures = 0;
        }
        previous_open = open;
        previous_date = Some(now.date_naive());

        let active = app
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::ArtworkEncoding)
            .await;
        if !active {
            let reason = if !open {
                "Waiting for the nightly window"
            } else if !app
                .runtime
                .library
                .library_scan_tracker
                .list_active()
                .await
                .is_empty()
            {
                "Waiting for library scans"
            } else {
                "Waiting for pending artwork"
            };
            *app.runtime.catalog.artwork_wait_reason.write().await = reason.into();
            let next = next_artwork_window(&now, open && pending_wake);
            if let Some(next) = next
                && app
                    .runtime
                    .jobs
                    .job_run_tracker
                    .next_run_at(JobKey::ArtworkEncoding)
                    .await
                    != Some(next)
            {
                app.set_job_next_run_at(JobKey::ArtworkEncoding, next).await;
            }
            if open
                && pending_wake
                && app
                    .runtime
                    .library
                    .library_scan_tracker
                    .list_active()
                    .await
                    .is_empty()
            {
                pending_wake = false;
                let result = match app.has_pending_artwork().await {
                    Ok(true) => {
                        app.run_scheduled_job_now(
                            JobKey::ArtworkEncoding,
                            JobTriggerSource::ScheduledDaily,
                        )
                        .await
                    }
                    Ok(_) => Ok(()),
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    consecutive_failures += 1;
                    pending_wake = consecutive_failures < 3;
                    warn!(%error, consecutive_failures, retrying = pending_wake, "scheduled artwork attempt failed");
                    *app.runtime.catalog.artwork_wait_reason.write().await = if pending_wake {
                        "Waiting one minute to retry after an artwork job error"
                    } else {
                        "Waiting for the next window or artwork wake after repeated job errors"
                    }
                    .into();
                    // Keep transient failures retryable without spinning on a broken repository.
                    // Every retry returns through the wall-clock, scan, and shutdown gates.
                    tokio::select! {
                        _ = token.cancelled() => return,
                        _ = tokio::time::sleep(SCHEDULE_POLL) => {},
                    }
                } else {
                    consecutive_failures = 0;
                }
                continue;
            }
        }
        tokio::select! {
            _ = token.cancelled() => return,
            _ = app.runtime.catalog.poster_wake.notified() => pending_wake = true,
            _ = app.runtime.catalog.fanart_wake.notified() => pending_wake = true,
            _ = app.services.library.image_proxy_cache_control.wait_for_cached_jpeg() => pending_wake = true,
            _ = tokio::time::sleep(SCHEDULE_POLL) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn window_boundaries_and_next_day_follow_local_wall_clock() {
        let zone = FixedOffset::west_opt(6 * 3600).unwrap();
        for (hour, minute, open) in [(2, 59, false), (3, 0, true), (8, 59, true), (9, 0, false)] {
            let now = zone.with_ymd_and_hms(2026, 9, 7, hour, minute, 0).unwrap();
            assert_eq!(artwork_window_open(&now), open);
            let next = next_artwork_window(&now, true)
                .unwrap()
                .with_timezone(&zone);
            if open {
                assert_eq!(next, now);
            } else {
                assert_eq!(next.hour(), 3);
                assert_eq!(
                    next.date_naive(),
                    now.date_naive() + chrono::Duration::days(i64::from(hour >= 9))
                );
            }
        }
    }

    #[test]
    fn slow_or_fast_cpu_selects_the_run_budget() {
        assert_eq!(artwork_workers(RuntimePerformanceClass::Slow), 2);
        assert_eq!(artwork_workers(RuntimePerformanceClass::Fast), 4);
    }

    #[cfg(unix)]
    #[test]
    fn artwork_local_calendar_handles_dst_without_mutating_parent_timezone() {
        const CHILD: &str = "SCRYER_ARTWORK_DST_TEST";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "catalog::title_images::tests::artwork_local_calendar_handles_dst_without_mutating_parent_timezone", "--nocapture"])
                .env(CHILD, "1")
                .env("TZ", "Europe/Helsinki")
                .output().unwrap();
            assert!(
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            return;
        }
        // Helsinki skips 03:00 in spring and repeats it in autumn.
        let before_gap = Local.with_ymd_and_hms(2026, 3, 29, 2, 59, 0).unwrap();
        let opening = next_artwork_window(&before_gap, true)
            .unwrap()
            .with_timezone(&Local);
        assert_eq!(opening.hour(), 4);
        assert!(artwork_window_open(&opening));
        let next_day = next_artwork_window(&opening, false)
            .unwrap()
            .with_timezone(&Local);
        assert_eq!(next_day.hour(), 3);
        assert_eq!(
            next_day.date_naive(),
            opening.date_naive().succ_opt().unwrap()
        );

        let repeated = Local.with_ymd_and_hms(2026, 10, 25, 3, 30, 0);
        let first = repeated.earliest().unwrap();
        let second = repeated.latest().unwrap();
        assert_ne!(first.offset(), second.offset());
        assert!(artwork_window_open(&first));
        assert!(artwork_window_open(&second));
        let at_end = Local.with_ymd_and_hms(2026, 10, 25, 9, 0, 0).unwrap();
        assert!(!artwork_window_open(&at_end));
    }
}
