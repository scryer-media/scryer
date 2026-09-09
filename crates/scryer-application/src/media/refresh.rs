use super::discs::MediaSourceVersion;
use crate::{AppResult, AppUseCase, MediaAnalysisOutcome, MediaFileAnalysis};

impl AppUseCase {
    /// Drain a small revision queue independently of directory timestamp shortcuts.
    pub(crate) async fn refresh_stale_media_analysis(&self, library_id: &str) -> AppResult<()> {
        let Ok(guard) = self
            .runtime
            .library
            .media_analysis_refresh_lock
            .clone()
            .try_lock_owned()
        else {
            return Ok(());
        };
        let app = self.clone();
        let library_id = library_id.to_string();
        // Keep the single-flight guard until the blocking native reader finishes,
        // including when the caller's scan future is cancelled.
        tokio::spawn(async move {
            let _guard = guard;
            app.refresh_stale_media_analysis_batch(&library_id).await
        })
        .await
        .map_err(|error| crate::AppError::Repository(error.to_string()))?
    }

    async fn refresh_stale_media_analysis_batch(&self, library_id: &str) -> AppResult<()> {
        let files = &self.services.library.media_files;
        let candidates = files
            .list_media_files_needing_analysis(
                library_id,
                scryer_media_types::ANALYSIS_REVISION,
                chrono::Utc::now() - chrono::Duration::hours(24),
                20,
            )
            .await?;
        let mut progress = super::metrics::RefreshProgress::default();
        let mut seen = std::collections::HashSet::new();
        for candidate in candidates.into_iter().take(20) {
            if !seen.insert(candidate.id.clone()) {
                continue;
            }
            if self
                .runtime
                .imports
                .execution_coordinator
                .has_foreground_work()
            {
                break;
            }
            let Ok(_capacity) = self
                .runtime
                .library
                .library_scan_analysis_limit
                .clone()
                .try_acquire_owned()
            else {
                break;
            };
            let path = crate::stored_paths::stored_path_to_path_buf(&candidate.file_path);
            let Some(_destination) = self
                .runtime
                .imports
                .execution_coordinator
                .try_acquire_destination(&path)
                .await
            else {
                continue;
            };
            let Some(current) = files.get_media_file_by_id(&candidate.id).await? else {
                continue;
            };
            if current.file_path != candidate.file_path
                || current.analysis_details.revision >= scryer_media_types::ANALYSIS_REVISION
            {
                continue;
            }
            if self
                .runtime
                .imports
                .execution_coordinator
                .has_foreground_work()
            {
                break;
            }
            let started = std::time::Instant::now();
            let _flight = super::metrics::RefreshFlight::begin();
            let source_before = MediaSourceVersion::read(&path).await.ok();
            match self
                .analyze_catalogued_media_file(Some(&current.id), path.clone())
                .await
            {
                Ok(inspected) => {
                    if !MediaSourceVersion::read(&path)
                        .await
                        .is_ok_and(|source| source == inspected.source)
                    {
                        continue;
                    }
                    let expected = inspected.expected.as_ref().unwrap_or(&current);
                    match inspected.outcome {
                        MediaAnalysisOutcome::Valid(mut analysis) => {
                            if let Some(disc) = &mut analysis.details.disc {
                                let Some(title) = self
                                    .services
                                    .catalog
                                    .titles
                                    .get_by_id(&current.title_id)
                                    .await?
                                else {
                                    continue;
                                };
                                if let Err(error) =
                                    self.validate_disc_episode_mappings(&title, disc).await
                                {
                                    analysis.details.report.status =
                                        scryer_media_types::ProbeStatus::Incomplete;
                                    analysis.details.report.warnings.push(
                                        scryer_media_types::ProbeWarning {
                                            code: "disc_episode_mapping_review".into(),
                                            message: error.to_string(),
                                            ..Default::default()
                                        },
                                    );
                                    if MediaSourceVersion::read(&path)
                                        .await
                                        .is_ok_and(|source| source == inspected.source)
                                    {
                                        progress.failed += u32::from(
                                            files
                                                .record_media_analysis_attempt(expected, &analysis)
                                                .await?,
                                        );
                                    }
                                    continue;
                                }
                            }
                            if !MediaSourceVersion::read(&path)
                                .await
                                .is_ok_and(|source| source == inspected.source)
                            {
                                continue;
                            }
                            if files
                                .update_media_file_analysis_if_unchanged(expected, *analysis)
                                .await?
                            {
                                progress.updated += 1;
                            }
                        }
                        MediaAnalysisOutcome::Inconclusive(analysis) => {
                            progress.failed += u32::from(
                                files
                                    .record_media_analysis_attempt(expected, &analysis)
                                    .await?,
                            );
                        }
                        MediaAnalysisOutcome::Invalid(message) => {
                            progress.failed += u32::from(
                                self.record_failed_analysis_refresh(expected, message)
                                    .await?,
                            );
                        }
                    }
                }
                Err(error) => {
                    if MediaSourceVersion::read(&path).await.ok() == source_before {
                        progress.failed += u32::from(
                            self.record_failed_analysis_refresh(&current, error.to_string())
                                .await?,
                        );
                    }
                }
            }
            tracing::debug!(file_id = %current.id, elapsed_ms = started.elapsed().as_millis(), "stale media inspection finished");
            tokio::task::yield_now().await;
        }
        tracing::debug!(
            library_id,
            updated = progress.updated,
            failed = progress.failed,
            "stale media analysis batch finished"
        );
        Ok(())
    }

    pub(crate) async fn record_failed_analysis_refresh(
        &self,
        expected: &crate::TitleMediaFile,
        message: String,
    ) -> AppResult<bool> {
        let mut analysis = MediaFileAnalysis::default();
        analysis.details.revision = scryer_media_types::ANALYSIS_REVISION;
        analysis.details.report.status = scryer_media_types::ProbeStatus::Incomplete;
        analysis
            .details
            .report
            .warnings
            .push(scryer_media_types::ProbeWarning {
                code: "refresh_failed".into(),
                message,
                ..Default::default()
            });
        self.services
            .library
            .media_files
            .record_media_analysis_attempt(expected, &analysis)
            .await
    }
}
