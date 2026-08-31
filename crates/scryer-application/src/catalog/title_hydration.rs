use super::*;
use crate::catalog_workflow::{
    HYDRATION_BULK_BATCH_SIZE, HydrationSource, HydrationTarget, extract_tvdb_id, movie_title_ref,
};
use crate::polling_worker::PollingWorker;
use std::time::Duration;
use tracing::{debug, info, warn};

const TITLE_HYDRATION_MAX_BATCH: usize = HYDRATION_BULK_BATCH_SIZE;
const TITLE_HYDRATION_IDLE_POLL_INTERVAL: Duration = Duration::from_secs(30);
const TITLE_HYDRATION_RETRY_BASE: Duration = Duration::from_secs(10);
const TITLE_HYDRATION_RETRY_MAX: Duration = Duration::from_secs(300);
const TITLE_HYDRATION_MAX_ATTEMPTS: i64 = 12;
const MOVIE_SMG_IDENTITY_BACKFILL_MAX_BATCH: usize = 200;
const MOVIE_SMG_IDENTITY_BACKFILL_MAX_ATTEMPTS: i64 = 5;
const MOVIE_SMG_IDENTITY_BACKFILL_TICK_INTERVAL: Duration = Duration::from_secs(5);
const MOVIE_SMG_IDENTITY_BACKFILL_RESUME_AFTER_KEY: &str =
    "catalog.movie_smg_identity_backfill_resume_after";

fn active_scan_facet_labels(facets: &[MediaFacet]) -> Vec<&'static str> {
    facets.iter().map(MediaFacet::as_str).collect()
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct MovieSmgIdentityBackfillSummary {
    pub(crate) linked: usize,
    pub(crate) unresolved: usize,
    pub(crate) errors: usize,
}

pub(crate) enum MovieSmgIdentityBackfillTick {
    Completed(MovieSmgIdentityBackfillSummary),
    NotSupported,
    Cancelled,
    Failed(crate::AppError),
}

impl AppUseCase {
    async fn movie_smg_identity_backfill_resume_position(&self) -> AppResult<Option<String>> {
        let value_json = self
            .services
            .config
            .settings
            .get_setting_json_explicit(
                SETTINGS_SCOPE_SYSTEM,
                MOVIE_SMG_IDENTITY_BACKFILL_RESUME_AFTER_KEY,
                None,
            )
            .await?;
        Ok(value_json
            .and_then(|value_json| serde_json::from_str::<String>(&value_json).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()))
    }

    async fn store_movie_smg_identity_backfill_resume_position(
        &self,
        position: Option<&str>,
    ) -> AppResult<()> {
        let value_json = serde_json::to_string(position.unwrap_or_default())
            .map_err(|error| crate::AppError::Repository(error.to_string()))?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                MOVIE_SMG_IDENTITY_BACKFILL_RESUME_AFTER_KEY,
                None,
                value_json,
                "system",
                None,
            )
            .await
    }
}

pub(crate) async fn run_movie_smg_identity_backfill_tick(
    app: &AppUseCase,
    token: &tokio_util::sync::CancellationToken,
    limit: usize,
) -> MovieSmgIdentityBackfillTick {
    if limit == 0 {
        return MovieSmgIdentityBackfillTick::Completed(MovieSmgIdentityBackfillSummary::default());
    }

    let after_id = tokio::select! {
        _ = token.cancelled() => return MovieSmgIdentityBackfillTick::Cancelled,
        result = app.movie_smg_identity_backfill_resume_position() => match result {
            Ok(after_id) => after_id,
            Err(error) => return MovieSmgIdentityBackfillTick::Failed(error),
        },
    };
    let titles = tokio::select! {
        _ = token.cancelled() => return MovieSmgIdentityBackfillTick::Cancelled,
        result = app.services.catalog.titles.list_movie_titles_missing_smg_id_after_id(after_id.as_deref(), limit) => match result {
            Ok(titles) => titles,
            Err(error) => return MovieSmgIdentityBackfillTick::Failed(error),
        },
    };

    if titles.is_empty() {
        return MovieSmgIdentityBackfillTick::Completed(MovieSmgIdentityBackfillSummary::default());
    }

    let next_cursor = titles
        .last()
        .map(|title| title.id.clone())
        .and_then(|candidate| {
            after_id
                .as_deref()
                .filter(|after_id| candidate.as_str() <= *after_id)
                .map(str::to_string)
                .or(Some(candidate))
        });
    let mut summary = MovieSmgIdentityBackfillSummary::default();
    let mut unresolved_title_ids = Vec::new();
    let candidates = titles
        .into_iter()
        .filter_map(|title| match movie_title_ref(&title) {
            Some(reference) => Some((title, reference)),
            None => {
                unresolved_title_ids.push(title.id);
                None
            }
        })
        .collect::<Vec<_>>();
    let references = candidates
        .iter()
        .map(|(_, reference)| reference.clone())
        .collect::<Vec<_>>();

    if !references.is_empty() {
        let resolutions = tokio::select! {
            _ = token.cancelled() => return MovieSmgIdentityBackfillTick::Cancelled,
            result = app.services.library.metadata_gateway.resolve_movie_titles(&references, false) => match result {
                Ok(resolutions) => resolutions,
                Err(error) if crate::catalog_workflow::movie_title_queries_not_supported(&error) => {
                    return MovieSmgIdentityBackfillTick::NotSupported;
                }
                Err(error) => return MovieSmgIdentityBackfillTick::Failed(error),
            },
        };
        let resolutions = resolutions
            .into_iter()
            .map(|resolution| (resolution.ref_index, resolution))
            .collect::<std::collections::HashMap<_, _>>();

        for (index, (title, _)) in candidates.iter().enumerate() {
            let Some(resolution) = resolutions.get(&index) else {
                unresolved_title_ids.push(title.id.clone());
                continue;
            };
            let Some(smg_id) = resolution.resolved.then_some(resolution.smg_id).flatten() else {
                unresolved_title_ids.push(title.id.clone());
                continue;
            };
            let persisted = tokio::select! {
                _ = token.cancelled() => return MovieSmgIdentityBackfillTick::Cancelled,
                result = app.services.catalog.titles.persist_smg_id(&title.id, smg_id, resolution.redirected_from) => result,
            };
            match persisted {
                Ok(()) => summary.linked += 1,
                Err(error) => {
                    summary.errors += 1;
                    warn!(
                        title_id = %title.id,
                        smg_id,
                        error = %error,
                        "movie SMG identity backfill: failed to persist title id"
                    );
                }
            }
        }
    }

    for title_id in unresolved_title_ids {
        summary.unresolved += 1;
        let recorded = tokio::select! {
            _ = token.cancelled() => return MovieSmgIdentityBackfillTick::Cancelled,
            result = app.services.catalog.titles.record_movie_smg_identity_backfill_unresolved(&title_id) => result,
        };
        if let Err(error) = recorded {
            summary.errors += 1;
            warn!(
                title_id = %title_id,
                max_attempts = MOVIE_SMG_IDENTITY_BACKFILL_MAX_ATTEMPTS,
                error = %error,
                "movie SMG identity backfill: failed to record unresolved identity attempt"
            );
        }
    }

    if let Err(error) = tokio::select! {
        _ = token.cancelled() => return MovieSmgIdentityBackfillTick::Cancelled,
        result = app.store_movie_smg_identity_backfill_resume_position(next_cursor.as_deref()) => result,
    } {
        summary.errors += 1;
        warn!(error = %error, "movie SMG identity backfill: failed to persist cursor");
    }
    MovieSmgIdentityBackfillTick::Completed(summary)
}

async fn run_movie_smg_identity_backfill_phase(
    app: &AppUseCase,
    token: &tokio_util::sync::CancellationToken,
    enabled: &mut bool,
    last_tick: &mut Option<std::time::Instant>,
) -> bool {
    if !*enabled {
        return true;
    }
    if last_tick
        .is_some_and(|last_tick| last_tick.elapsed() < MOVIE_SMG_IDENTITY_BACKFILL_TICK_INTERVAL)
    {
        return true;
    }
    *last_tick = Some(std::time::Instant::now());

    match run_movie_smg_identity_backfill_tick(app, token, MOVIE_SMG_IDENTITY_BACKFILL_MAX_BATCH)
        .await
    {
        MovieSmgIdentityBackfillTick::Completed(summary) => {
            if summary.linked > 0 {
                metrics::counter!("scryer_movie_smg_identity_backfill_linked_total")
                    .increment(summary.linked as u64);
            }
            if summary.unresolved > 0 {
                metrics::counter!("scryer_movie_smg_identity_backfill_unresolved_total")
                    .increment(summary.unresolved as u64);
            }
            if summary.errors > 0 {
                metrics::counter!("scryer_movie_smg_identity_backfill_errors_total")
                    .increment(summary.errors as u64);
            }
            if summary.linked > 0 || summary.unresolved > 0 || summary.errors > 0 {
                info!(
                    linked = summary.linked,
                    unresolved = summary.unresolved,
                    errors = summary.errors,
                    "movie SMG identity backfill batch complete"
                );
            }
            true
        }
        MovieSmgIdentityBackfillTick::NotSupported => {
            *enabled = false;
            warn!(
                "movie SMG identity backfill disabled because the metadata gateway does not support title-id queries"
            );
            true
        }
        MovieSmgIdentityBackfillTick::Cancelled => false,
        MovieSmgIdentityBackfillTick::Failed(error) => {
            metrics::counter!("scryer_movie_smg_identity_backfill_errors_total").increment(1);
            warn!(error = %error, "movie SMG identity backfill batch failed");
            true
        }
    }
}

pub async fn start_background_title_hydration_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    let worker = PollingWorker::new("title_hydration", token.clone());
    let mut movie_smg_identity_backfill_enabled = true;
    let mut movie_smg_identity_backfill_last_tick = None;
    info!(
        max_batch = TITLE_HYDRATION_MAX_BATCH,
        idle_poll_secs = TITLE_HYDRATION_IDLE_POLL_INTERVAL.as_secs(),
        retry_base_secs = TITLE_HYDRATION_RETRY_BASE.as_secs(),
        retry_max_secs = TITLE_HYDRATION_RETRY_MAX.as_secs(),
        max_attempts = TITLE_HYDRATION_MAX_ATTEMPTS,
        "background title hydration loop started"
    );

    loop {
        let blocked_facets = app
            .runtime
            .library
            .library_scan_tracker
            .active_facets()
            .await;
        let due_titles = match app
            .services
            .catalog
            .titles
            .list_titles_due_for_hydration(TITLE_HYDRATION_MAX_BATCH, &blocked_facets)
            .await
        {
            Ok(due_titles) => due_titles,
            Err(error) => {
                worker.warn_error("list_due_titles", &error);
                if !worker.wait_for_sleep(Duration::from_secs(2)).await {
                    return;
                }
                continue;
            }
        };

        metrics::gauge!("scryer_title_metadata_hydration_pending").set(due_titles.len() as f64);

        if due_titles.is_empty() {
            if !blocked_facets.contains(&MediaFacet::Movie)
                && !run_movie_smg_identity_backfill_phase(
                    &app,
                    &token,
                    &mut movie_smg_identity_backfill_enabled,
                    &mut movie_smg_identity_backfill_last_tick,
                )
                .await
            {
                return;
            }
            if blocked_facets.is_empty() {
                if !worker
                    .wait_for_wake_or_timeout(
                        &app.runtime.catalog.title_hydration_wake,
                        TITLE_HYDRATION_IDLE_POLL_INTERVAL,
                    )
                    .await
                {
                    return;
                }
            } else {
                metrics::counter!("scryer_title_metadata_hydration_scan_owned_yields_total")
                    .increment(1);
                debug!(
                    blocked_facets = ?active_scan_facet_labels(&blocked_facets),
                    "title hydration loop: yielding while library scan owns active facet"
                );
                if !worker
                    .wait_for_future_or_wake_or_timeout(
                        &app.runtime.catalog.title_hydration_wake,
                        app.runtime
                            .library
                            .library_scan_tracker
                            .wait_for_active_facets_change(&blocked_facets),
                        TITLE_HYDRATION_IDLE_POLL_INTERVAL,
                    )
                    .await
                {
                    return;
                }
            }
            continue;
        }

        let blocked_facets_before_dispatch = app
            .runtime
            .library
            .library_scan_tracker
            .active_facets()
            .await;
        if blocked_facets_before_dispatch != blocked_facets {
            metrics::counter!("scryer_title_metadata_hydration_scan_owned_rechecks_total")
                .increment(1);
            debug!(
                blocked_facets = ?active_scan_facet_labels(&blocked_facets),
                blocked_facets_before_dispatch =
                    ?active_scan_facet_labels(&blocked_facets_before_dispatch),
                "title hydration loop: active scan facets changed before dispatch"
            );
            continue;
        }

        let mut original_attempts = std::collections::HashMap::with_capacity(due_titles.len());
        let mut targets = Vec::with_capacity(due_titles.len());
        for due_title in due_titles {
            original_attempts.insert(
                due_title.title.id.clone(),
                (due_title.attempt_count, due_title.title.facet.clone()),
            );
            let requested_movie_ref = movie_title_ref(&due_title.title);
            let hydratable = match due_title.title.facet {
                MediaFacet::Movie => requested_movie_ref.is_some(),
                MediaFacet::Series | MediaFacet::Anime => {
                    extract_tvdb_id(&due_title.title).is_some()
                }
            };
            if !hydratable {
                warn!(
                    hydration_source = HydrationSource::BackgroundDue.as_str(),
                    facet = due_title.title.facet.as_str(),
                    title_id = %due_title.title.id,
                    title_name = %due_title.title.name,
                    "title hydration loop: clearing retry state because title has no supported external id"
                );
                if let Err(error) = app
                    .services
                    .catalog
                    .titles
                    .clear_title_metadata_hydration_retry_state(&due_title.title.id)
                    .await
                {
                    warn!(
                        hydration_source = HydrationSource::BackgroundDue.as_str(),
                        title_id = %due_title.title.id,
                        error = %error,
                        "title hydration loop: failed to clear retry state for title without a supported external id"
                    );
                }
                original_attempts.remove(&due_title.title.id);
                continue;
            }
            targets.push(HydrationTarget {
                title: due_title.title,
                requested_tvdb_id: None,
                requested_movie_ref,
                sync_wanted_after_completion: true,
                source: HydrationSource::BackgroundDue,
            });
        }

        if targets.is_empty() {
            if !blocked_facets.contains(&MediaFacet::Movie)
                && !run_movie_smg_identity_backfill_phase(
                    &app,
                    &token,
                    &mut movie_smg_identity_backfill_enabled,
                    &mut movie_smg_identity_backfill_last_tick,
                )
                .await
            {
                return;
            }
            continue;
        }

        debug!(
            hydration_source = HydrationSource::BackgroundDue.as_str(),
            count = targets.len(),
            "title hydration loop: processing batch"
        );

        for _ in 0..targets.len() {
            metrics::counter!("scryer_title_metadata_hydration_attempts_total").increment(1);
        }

        let title_ids = targets
            .iter()
            .map(|target| target.title.id.clone())
            .collect::<Vec<_>>();

        match app.hydrate_titles_bulk(targets).await {
            Ok(outcome) => {
                for title_id in outcome.hydrated_titles.keys() {
                    metrics::counter!("scryer_title_metadata_hydration_success_total").increment(1);
                    original_attempts.remove(title_id);
                }

                for title_id in outcome.deferred_titles {
                    if let Err(error) = app
                        .services
                        .catalog
                        .titles
                        .clear_title_metadata_hydration_retry_state(&title_id)
                        .await
                    {
                        warn!(
                            hydration_source = HydrationSource::BackgroundDue.as_str(),
                            title_id = %title_id,
                            error = %error,
                            "title hydration loop: failed to park title unsupported by the legacy metadata gateway"
                        );
                    }
                    original_attempts.remove(&title_id);
                }

                for (title_id, _) in outcome.failed_titles {
                    metrics::counter!("scryer_title_metadata_hydration_failure_total").increment(1);
                    if let Some((previous_attempt_count, facet)) =
                        original_attempts.remove(&title_id)
                    {
                        schedule_title_hydration_retry(
                            &app,
                            &title_id,
                            &facet,
                            previous_attempt_count,
                        )
                        .await;
                    }
                }

                for title_id in original_attempts.keys() {
                    if let Err(error) = app
                        .services
                        .catalog
                        .titles
                        .mark_title_metadata_hydration_due_now(title_id)
                        .await
                    {
                        warn!(
                            hydration_source = HydrationSource::BackgroundDue.as_str(),
                            title_id = %title_id,
                            error = %error,
                            "title hydration loop: failed to keep unreported title due"
                        );
                    }
                }
            }
            Err(error) => {
                warn!(
                    hydration_source = HydrationSource::BackgroundDue.as_str(),
                    error = %error,
                    title_ids = ?title_ids,
                    "title hydration loop: batch failed"
                );
                for title_id in title_ids {
                    metrics::counter!("scryer_title_metadata_hydration_failure_total").increment(1);
                    if let Some((previous_attempt_count, facet)) =
                        original_attempts.get(&title_id).cloned()
                    {
                        schedule_title_hydration_retry(
                            &app,
                            &title_id,
                            &facet,
                            previous_attempt_count,
                        )
                        .await;
                    }
                }
            }
        }
        if !blocked_facets.contains(&MediaFacet::Movie)
            && !run_movie_smg_identity_backfill_phase(
                &app,
                &token,
                &mut movie_smg_identity_backfill_enabled,
                &mut movie_smg_identity_backfill_last_tick,
            )
            .await
        {
            return;
        }
    }
}

async fn schedule_title_hydration_retry(
    app: &AppUseCase,
    title_id: &str,
    facet: &MediaFacet,
    previous_attempt_count: i64,
) {
    let Some((next_attempt_at, next_attempt_count)) =
        next_title_hydration_retry(app.runtime.environment.now(), previous_attempt_count)
    else {
        metrics::counter!("scryer_title_metadata_hydration_terminal_failures_total").increment(1);
        warn!(
            hydration_source = HydrationSource::BackgroundDue.as_str(),
            facet = facet.as_str(),
            title_id = %title_id,
            max_attempts = TITLE_HYDRATION_MAX_ATTEMPTS,
            "title hydration loop: reached max retry attempts, clearing retry state"
        );
        if let Err(error) = app
            .services
            .catalog
            .titles
            .clear_title_metadata_hydration_retry_state(title_id)
            .await
        {
            warn!(
                hydration_source = HydrationSource::BackgroundDue.as_str(),
                facet = facet.as_str(),
                title_id = %title_id,
                error = %error,
                "title hydration loop: failed to clear terminal retry state"
            );
        }
        return;
    };

    if let Err(error) = app
        .services
        .catalog
        .titles
        .schedule_title_metadata_hydration_retry(
            title_id,
            &next_attempt_at.to_rfc3339(),
            next_attempt_count,
        )
        .await
    {
        warn!(
            hydration_source = HydrationSource::BackgroundDue.as_str(),
            facet = facet.as_str(),
            title_id = %title_id,
            attempt_count = next_attempt_count,
            next_attempt_at = %next_attempt_at,
            error = %error,
            "title hydration loop: failed to schedule retry"
        );
    }
}

fn next_title_hydration_retry(
    now: chrono::DateTime<chrono::Utc>,
    previous_attempt_count: i64,
) -> Option<(chrono::DateTime<chrono::Utc>, i64)> {
    let next_attempt_count = previous_attempt_count.saturating_add(1);
    if next_attempt_count >= TITLE_HYDRATION_MAX_ATTEMPTS {
        return None;
    }

    let retry_delay = title_hydration_retry_delay(next_attempt_count);
    let next_attempt_at = now
        + chrono::Duration::from_std(retry_delay)
            .unwrap_or_else(|_| chrono::Duration::seconds(300));
    Some((next_attempt_at, next_attempt_count))
}

fn title_hydration_retry_delay(attempt_count: i64) -> Duration {
    let exponent = attempt_count.saturating_sub(1).clamp(0, 30) as u32;
    let multiplier = 1u32.checked_shl(exponent).unwrap_or(u32::MAX);
    let delay = TITLE_HYDRATION_RETRY_BASE.saturating_mul(multiplier);
    delay.min(TITLE_HYDRATION_RETRY_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_title_hydration_retry_stops_after_max_attempts() {
        let now = chrono::Utc::now();
        assert!(next_title_hydration_retry(now, TITLE_HYDRATION_MAX_ATTEMPTS - 1).is_none());
    }

    #[test]
    fn next_title_hydration_retry_uses_backoff_and_clamps_to_max() {
        let now = chrono::Utc::now();
        let (next_attempt_at, next_attempt_count) =
            next_title_hydration_retry(now, 10).expect("retry should still schedule");
        assert_eq!(next_attempt_count, 11);
        assert_eq!(
            next_attempt_at - now,
            chrono::Duration::from_std(TITLE_HYDRATION_RETRY_MAX)
                .expect("chrono duration should convert")
        );
    }
}
