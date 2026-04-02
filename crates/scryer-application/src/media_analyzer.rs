use std::path::PathBuf;

use async_trait::async_trait;

use crate::{AppError, AppResult, MediaAnalysisOutcome, MediaAnalyzer, nice_thread};

pub struct NativeMediaAnalyzer;

#[async_trait]
impl MediaAnalyzer for NativeMediaAnalyzer {
    async fn analyze_file(&self, path: PathBuf) -> AppResult<MediaAnalysisOutcome> {
        tokio::task::spawn_blocking(move || {
            nice_thread();
            match scryer_mediainfo::analyze_file(&path) {
                Ok(analysis) if scryer_mediainfo::is_valid_video(&analysis) => {
                    Ok(MediaAnalysisOutcome::Valid(Box::new(
                        crate::post_download_gate::build_media_file_analysis(&analysis),
                    )))
                }
                Ok(_) => Ok(MediaAnalysisOutcome::Invalid(
                    "file is not a valid video".to_string(),
                )),
                Err(error) => Ok(MediaAnalysisOutcome::Invalid(error.to_string())),
            }
        })
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
    }
}
