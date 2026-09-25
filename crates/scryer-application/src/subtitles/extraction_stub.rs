use std::sync::Arc;

use super::provider::SubtitleFile;
use crate::{AppError, AppResult, ArchiveExtractorPluginProvider};

const SUPPORTED_SUBTITLE_FORMATS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx"];

#[derive(Debug, Clone, Default)]
pub struct SubtitleExtractionContext {
    pub language: Option<String>,
    pub episode: Option<i32>,
    pub absolute_episode: Option<i32>,
    /// The episode's number inside its anime community entry, which is how a
    /// cour's subtitle archive usually numbers its members.
    pub community_episode: Option<i32>,
}

pub fn is_supported_subtitle_format(format: &str) -> bool {
    SUPPORTED_SUBTITLE_FORMATS.contains(&normalize_extension(format).as_str())
}

pub async fn normalize_downloaded_subtitle(
    file: SubtitleFile,
    _context: SubtitleExtractionContext,
) -> AppResult<SubtitleFile> {
    normalize_downloaded_subtitle_with_archive_provider(file, _context, None).await
}

pub async fn normalize_downloaded_subtitle_with_archive_provider(
    mut file: SubtitleFile,
    _context: SubtitleExtractionContext,
    _archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
) -> AppResult<SubtitleFile> {
    let format = final_subtitle_format(&file).ok_or_else(|| {
        AppError::Validation(format!(
            "unsupported subtitle artifact format: {}",
            file.filename
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or(file.format.as_str())
        ))
    })?;
    file.format = format;
    Ok(file)
}

fn final_subtitle_format(file: &SubtitleFile) -> Option<String> {
    extension_for_filename(file.filename.as_deref())
        .filter(|format| is_supported_subtitle_format(format))
        .or_else(|| {
            is_supported_subtitle_format(&file.format).then(|| normalize_extension(&file.format))
        })
}

fn extension_for_filename(filename: Option<&str>) -> Option<String> {
    filename
        .and_then(|value| value.rsplit_once('.').map(|(_, ext)| ext))
        .map(normalize_extension)
}

fn normalize_extension(format: &str) -> String {
    format.trim().trim_start_matches('.').to_ascii_lowercase()
}
