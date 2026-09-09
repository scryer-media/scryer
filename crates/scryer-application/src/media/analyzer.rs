use std::path::PathBuf;

use async_trait::async_trait;

#[cfg(feature = "runtime-media-analysis")]
use crate::nice_thread;
use crate::{AppError, AppResult, MediaAnalysisOutcome, MediaAnalyzer};

pub struct NativeMediaAnalyzer;

#[cfg(any(feature = "runtime-media-analysis", test))]
#[derive(Clone, PartialEq, Eq)]
struct AnalysisKey {
    path: PathBuf,
    source: Option<super::discs::MediaSourceVersion>,
    selection: scryer_media_types::DiscSelection,
}

#[cfg(any(feature = "runtime-media-analysis", test))]
struct AnalysisFlight<T> {
    key: AnalysisKey,
    result: tokio::sync::watch::Sender<Option<Result<T, String>>>,
}

#[cfg(any(feature = "runtime-media-analysis", test))]
struct AnalysisFlights<T> {
    active: std::sync::Mutex<Vec<std::sync::Weak<AnalysisFlight<T>>>>,
}

#[cfg(any(feature = "runtime-media-analysis", test))]
impl<T> Default for AnalysisFlights<T> {
    fn default() -> Self {
        Self {
            active: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[cfg(any(feature = "runtime-media-analysis", test))]
impl<T: Clone + Send + Sync + 'static> AnalysisFlights<T> {
    fn start(
        &self,
        key: AnalysisKey,
        operation: impl FnOnce() -> AppResult<T> + Send + 'static,
    ) -> std::sync::Arc<AnalysisFlight<T>> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        active.retain(|entry| entry.strong_count() > 0);
        if let Some(flight) = active
            .iter()
            .filter_map(std::sync::Weak::upgrade)
            .find(|flight| flight.key == key)
        {
            return flight;
        }
        let flight = std::sync::Arc::new(AnalysisFlight {
            key,
            result: tokio::sync::watch::channel(None).0,
        });
        active.push(std::sync::Arc::downgrade(&flight));
        let running = flight.clone();
        // The job owns the flight until the blocking read ends. Dropping the
        // initiating request must not allow a second reader to start its work.
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(operation)
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            running.result.send_replace(Some(result));
        });
        flight
    }
}

#[cfg(any(feature = "runtime-media-analysis", test))]
impl<T: Clone> AnalysisFlight<T> {
    async fn outcome(&self) -> AppResult<T> {
        let mut result = self.result.subscribe();
        loop {
            if let Some(outcome) = result.borrow_and_update().clone() {
                return outcome.map_err(AppError::Repository);
            }
            result
                .changed()
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
        }
    }
}

#[cfg(feature = "runtime-media-analysis")]
static NATIVE_PROBE_FLIGHTS: std::sync::LazyLock<
    AnalysisFlights<Result<scryer_mediainfo::MediaAnalysis, String>>,
> = std::sync::LazyLock::new(AnalysisFlights::default);

/// Share the physical probe across catalog scans and final import validation.
#[cfg(feature = "runtime-media-analysis")]
pub(crate) async fn probe_native_catalog(
    path: PathBuf,
    selection: scryer_media_types::DiscSelection,
    operation: crate::media::metrics::ProbeOperation,
) -> AppResult<Result<scryer_mediainfo::MediaAnalysis, String>> {
    let source_path = path.clone();
    let key = AnalysisKey {
        path: path.clone(),
        source: super::discs::MediaSourceVersion::read(&path).await.ok(),
        selection: selection.clone(),
    };
    let flight = NATIVE_PROBE_FLIGHTS.start(key.clone(), move || {
        nice_thread();
        let started = std::time::Instant::now();
        let result = if scryer_domain::is_disc_image(&path) {
            scryer_mediainfo::analyze_disc_file(&path, selection)
        } else {
            scryer_mediainfo::analyze_catalog_file(&path)
        };
        crate::media::metrics::record_probe(
            operation,
            started.elapsed(),
            result
                .as_ref()
                .ok()
                .map(|analysis| &analysis.details.report),
        );
        Ok(result.map_err(|error| error.to_string()))
    });
    let outcome = flight.outcome().await?;
    if key.source
        != super::discs::MediaSourceVersion::read(&source_path)
            .await
            .ok()
    {
        return Err(AppError::Validation(
            "media source changed during inspection".into(),
        ));
    }
    Ok(outcome)
}

#[cfg(all(test, feature = "runtime-media-analysis"))]
pub(crate) async fn hold_native_probe_for_test(
    path: PathBuf,
) -> (std::sync::mpsc::Sender<()>, impl Fn() -> usize) {
    let key = AnalysisKey {
        source: super::discs::MediaSourceVersion::read(&path).await.ok(),
        path: path.clone(),
        selection: Default::default(),
    };
    let (release, held) = std::sync::mpsc::channel();
    let flight = NATIVE_PROBE_FLIGHTS.start(key, move || {
        held.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|error| AppError::Repository(error.to_string()))?;
        Ok(scryer_mediainfo::analyze_catalog_file(&path).map_err(|error| error.to_string()))
    });
    (release, move || std::sync::Arc::strong_count(&flight))
}

#[async_trait]
impl MediaAnalyzer for NativeMediaAnalyzer {
    async fn diagnose_file(
        &self,
        path: PathBuf,
        selection: scryer_media_types::DiscSelection,
    ) -> AppResult<scryer_media_types::StructuralDiagnostics> {
        #[cfg(feature = "runtime-media-analysis")]
        {
            tokio::task::spawn_blocking(move || {
                nice_thread();
                let started = std::time::Instant::now();
                let result = scryer_mediainfo::diagnostics::diagnose_file(
                    &path,
                    scryer_mediainfo::diagnostics::DiagnosticsOptions {
                        disc_selection: selection,
                        ..Default::default()
                    },
                );
                crate::media::metrics::record_probe(
                    crate::media::metrics::ProbeOperation::Diagnostics,
                    started.elapsed(),
                    result.as_ref().ok().map(|diagnostics| &diagnostics.report),
                );
                result.map_err(|error| AppError::Validation(error.to_string()))
            })
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?
        }
        #[cfg(not(feature = "runtime-media-analysis"))]
        {
            let _ = (path, selection);
            Err(AppError::Validation(
                "native diagnostics are not compiled into this target".into(),
            ))
        }
    }

    async fn analyze_file(&self, path: PathBuf) -> AppResult<MediaAnalysisOutcome> {
        self.analyze_file_with_selection(path, Default::default())
            .await
    }

    async fn analyze_file_with_selection(
        &self,
        path: PathBuf,
        selection: scryer_media_types::DiscSelection,
    ) -> AppResult<MediaAnalysisOutcome> {
        #[cfg(not(feature = "runtime-media-analysis"))]
        let _ = selection;
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("strm"))
        {
            return Ok(MediaAnalysisOutcome::Valid(Box::new(
                crate::post_download_gate::build_stream_pointer_media_file_analysis(),
            )));
        }
        #[cfg(feature = "runtime-media-analysis")]
        let result = probe_native_catalog(
            path,
            selection,
            crate::media::metrics::ProbeOperation::Catalog,
        )
        .await?;
        #[cfg(feature = "runtime-media-analysis")]
        match result {
            Ok(analysis)
                if scryer_mediainfo::is_valid_video(&analysis)
                    && (analysis.container_format.as_deref() != Some("iso")
                        || analysis.details.report.status
                            == scryer_media_types::ProbeStatus::Complete) =>
            {
                Ok(MediaAnalysisOutcome::Valid(Box::new(
                    crate::post_download_gate::build_media_file_analysis(&analysis),
                )))
            }
            Ok(analysis)
                if analysis.container_format.as_deref() == Some("iso")
                    || analysis.video_codec.is_some()
                    || analysis.details.selected_video_id.is_some()
                    || analysis.details.report.budget_exhausted
                    || matches!(
                        analysis.details.report.status,
                        scryer_media_types::ProbeStatus::Unsupported
                            | scryer_media_types::ProbeStatus::Encrypted
                    ) =>
            {
                Ok(MediaAnalysisOutcome::Inconclusive(Box::new(
                    crate::post_download_gate::build_media_file_analysis(&analysis),
                )))
            }
            Ok(_) => Ok(MediaAnalysisOutcome::Invalid(
                "file is not a valid video".to_string(),
            )),
            Err(error) => Ok(MediaAnalysisOutcome::Invalid(error.to_string())),
        }
        #[cfg(not(feature = "runtime-media-analysis"))]
        Ok(MediaAnalysisOutcome::Invalid(
            "native media analysis is not compiled into this target".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn matching_analysis_waiters_share_work_after_initiator_cancellation() {
        let flights = AnalysisFlights::default();
        let key = AnalysisKey {
            path: PathBuf::from("same-media.iso"),
            source: None,
            selection: Default::default(),
        };
        let (started, ready) = tokio::sync::oneshot::channel();
        let (resume, blocked) = std::sync::mpsc::channel();
        let first = flights.start(key.clone(), move || {
            let _ = started.send(());
            blocked
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            Ok(MediaAnalysisOutcome::Invalid("shared result".into()))
        });
        let request = tokio::spawn(async move { first.outcome().await });
        ready.await.unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        let second = flights.start(key, || panic!("duplicate native reader"));
        resume.send(()).unwrap();
        assert!(
            matches!(second.outcome().await.unwrap(), MediaAnalysisOutcome::Invalid(message) if message == "shared result")
        );
        drop(second);
        assert!(
            flights
                .active
                .lock()
                .unwrap()
                .iter()
                .all(|entry| entry.upgrade().is_none())
        );
    }

    #[tokio::test]
    async fn analysis_flights_separate_source_versions_and_disc_choices() {
        let flights = AnalysisFlights::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disc.iso");
        std::fs::write(&path, b"original").unwrap();
        let key = AnalysisKey {
            path: path.clone(),
            source: Some(
                super::super::discs::MediaSourceVersion::read(&path)
                    .await
                    .unwrap(),
            ),
            selection: Default::default(),
        };
        let (resume, blocked) = std::sync::mpsc::channel();
        let original = flights.start(key.clone(), move || {
            blocked
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            Ok(MediaAnalysisOutcome::Invalid("original".into()))
        });
        let mut selected_key = key.clone();
        selected_key.selection.title_id = Some("00002".into());
        let selected = flights.start(selected_key, || {
            Ok(MediaAnalysisOutcome::Invalid("different cut".into()))
        });
        assert!(
            matches!(selected.outcome().await.unwrap(), MediaAnalysisOutcome::Invalid(message) if message == "different cut")
        );
        std::fs::write(&path, b"replacement source").unwrap();
        let replaced_key = AnalysisKey {
            source: Some(
                super::super::discs::MediaSourceVersion::read(&path)
                    .await
                    .unwrap(),
            ),
            ..key
        };
        let replaced = flights.start(replaced_key, || {
            Ok(MediaAnalysisOutcome::Invalid("new source".into()))
        });
        assert!(
            matches!(replaced.outcome().await.unwrap(), MediaAnalysisOutcome::Invalid(message) if message == "new source")
        );
        resume.send(()).unwrap();
        assert!(
            matches!(original.outcome().await.unwrap(), MediaAnalysisOutcome::Invalid(message) if message == "original")
        );
    }

    #[tokio::test]
    async fn analysis_worker_panics_wake_waiters_and_allow_retry() {
        let flights = AnalysisFlights::default();
        let key = AnalysisKey {
            path: PathBuf::from("broken-media.iso"),
            source: None,
            selection: Default::default(),
        };
        let failed = flights.start(key.clone(), || panic!("fixture reader failure"));
        let follower = flights.start(key.clone(), || panic!("duplicate reader"));
        assert!(failed.outcome().await.is_err());
        assert!(follower.outcome().await.is_err());
        drop((failed, follower));
        let retry = flights.start(key, || Ok(MediaAnalysisOutcome::Invalid("retry".into())));
        assert!(
            matches!(retry.outcome().await.unwrap(), MediaAnalysisOutcome::Invalid(message) if message == "retry")
        );
    }

    #[tokio::test]
    async fn analyze_file_returns_minimal_valid_analysis_for_strm() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Example.Movie.2024.strm");
        std::fs::write(&path, b"https://nzbdav.example/stream/Example.Movie.2024")
            .expect("write strm");

        let analyzer = NativeMediaAnalyzer;
        let outcome = analyzer.analyze_file(path).await.expect("analyze strm");

        match outcome {
            MediaAnalysisOutcome::Valid(analysis) => {
                assert_eq!(analysis.container_format.as_deref(), Some("strm"));
                assert!(analysis.audio_streams.is_empty());
                assert!(analysis.subtitle_streams.is_empty());
            }
            MediaAnalysisOutcome::Invalid(error) => {
                panic!("expected valid strm analysis, got invalid: {error}");
            }
            MediaAnalysisOutcome::Inconclusive(_) => panic!("unexpected incomplete stream pointer"),
        }
    }
}
