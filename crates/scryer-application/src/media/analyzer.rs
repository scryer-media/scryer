use std::path::PathBuf;

use async_trait::async_trait;

use crate::{AppError, AppResult, MediaAnalysisOutcome, MediaAnalyzer, nice_thread};

pub struct NativeMediaAnalyzer;

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
        tokio::task::spawn_blocking(move || {
            nice_thread();
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
            let result = {
                let started = std::time::Instant::now();
                let result = if scryer_domain::is_disc_image(&path) {
                    scryer_mediainfo::analyze_disc_file(&path, selection)
                } else {
                    scryer_mediainfo::analyze_catalog_file(&path)
                };
                crate::media::metrics::record_probe(
                    crate::media::metrics::ProbeOperation::Catalog,
                    started.elapsed(),
                    result
                        .as_ref()
                        .ok()
                        .map(|analysis| &analysis.details.report),
                );
                result
            };
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
        })
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
