use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::extraction::{
    SubtitleExtractionContext, is_supported_subtitle_format,
    normalize_downloaded_subtitle_with_archive_provider,
};
use super::language::normalize_subtitle_language_code;
use super::provider::{SubtitleFile, SubtitleProvider};
use crate::{AppError, AppResult, ArchiveExtractorPluginProvider};

#[derive(Clone, Default)]
pub struct SubtitleDownloadSelection {
    pub episode: Option<i32>,
    pub absolute_episode: Option<i32>,
    pub archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
}

impl std::fmt::Debug for SubtitleDownloadSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubtitleDownloadSelection")
            .field("episode", &self.episode)
            .field("absolute_episode", &self.absolute_episode)
            .field("archive_provider", &self.archive_provider.is_some())
            .finish()
    }
}

/// Normalize a language code and ensure it's safe for use in filenames.
fn normalize_language(lang: &str) -> AppResult<String> {
    let normalized = normalize_subtitle_language_code(lang)
        .ok_or_else(|| AppError::Validation(format!("invalid subtitle language code: {lang:?}")))?;

    if normalized.len() < 2
        || normalized.len() > 3
        || !normalized.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(AppError::Validation(format!(
            "invalid subtitle language code: {lang:?}"
        )));
    }

    Ok(normalized)
}

/// Validate that a subtitle format is safe for use in filenames.
fn validate_format(fmt: &str) -> AppResult<()> {
    if !is_supported_subtitle_format(fmt) {
        return Err(AppError::Validation(format!(
            "unsupported subtitle format: {fmt:?}"
        )));
    }
    Ok(())
}

/// Save a downloaded subtitle file to disk next to the video file.
///
/// Naming convention: `{video_stem}.{language}.{format}`
/// e.g., `Movie.2024.1080p.BluRay.eng.srt`
///
/// If `forced` is true: `{video_stem}.{language}.forced.{format}`
/// If `hearing_impaired` is true: `{video_stem}.{language}.hi.{format}`
pub fn build_subtitle_path(
    video_path: &Path,
    language: &str,
    format: &str,
    forced: bool,
    hearing_impaired: bool,
) -> PathBuf {
    let stem = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let parent = video_path.parent().unwrap_or(Path::new("."));

    let suffix = if forced {
        format!("{language}.forced")
    } else if hearing_impaired {
        format!("{language}.hi")
    } else {
        language.to_string()
    };

    parent.join(format!("{stem}.{suffix}.{format}"))
}

/// Download a subtitle from a provider and save it to disk.
pub async fn download_and_save(
    provider: &dyn SubtitleProvider,
    provider_file_id: &str,
    video_path: &Path,
    language: &str,
    forced: bool,
    hearing_impaired: bool,
) -> AppResult<(PathBuf, SubtitleFile)> {
    download_and_save_with_selection(
        provider,
        provider_file_id,
        video_path,
        language,
        forced,
        hearing_impaired,
        SubtitleDownloadSelection::default(),
    )
    .await
}

/// Download a subtitle from a provider, normalize raw/compressed artifacts,
/// and save the final subtitle next to the video file.
pub async fn download_and_save_with_selection(
    provider: &dyn SubtitleProvider,
    provider_file_id: &str,
    video_path: &Path,
    language: &str,
    forced: bool,
    hearing_impaired: bool,
    selection: SubtitleDownloadSelection,
) -> AppResult<(PathBuf, SubtitleFile)> {
    let language = normalize_language(language)?;
    let file = normalize_downloaded_subtitle_with_archive_provider(
        provider.download(provider_file_id).await?,
        SubtitleExtractionContext {
            language: Some(language.clone()),
            episode: selection.episode,
            absolute_episode: selection.absolute_episode,
        },
        selection.archive_provider,
    )
    .await?;
    validate_format(&file.format)?;
    let dest = build_subtitle_path(
        video_path,
        &language,
        &file.format,
        forced,
        hearing_impaired,
    );

    // Ensure parent directory exists
    if let Some(parent) = dest.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::Repository(format!(
                "cannot create subtitle directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    std::fs::write(&dest, &file.content).map_err(|e| {
        AppError::Repository(format!(
            "cannot write subtitle file {}: {e}",
            dest.display()
        ))
    })?;

    tracing::info!(
        path = %dest.display(),
        language = %language,
        provider = provider.name(),
        "subtitle downloaded and saved"
    );

    Ok((dest, file))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "runtime-archives")]
    use crate::subtitles::{SubtitleMatch, SubtitleQuery};
    #[cfg(feature = "runtime-archives")]
    use std::io::Write;

    #[cfg(feature = "runtime-archives")]
    struct StaticProvider {
        content: Vec<u8>,
        format: String,
        filename: Option<String>,
        content_type: Option<String>,
    }

    #[cfg(feature = "runtime-archives")]
    #[async_trait::async_trait]
    impl SubtitleProvider for StaticProvider {
        async fn search(&self, _query: &SubtitleQuery) -> AppResult<Vec<SubtitleMatch>> {
            unreachable!("search is not used in these tests")
        }

        async fn download(&self, _provider_file_id: &str) -> AppResult<SubtitleFile> {
            Ok(SubtitleFile {
                content: self.content.clone(),
                format: self.format.clone(),
                filename: self.filename.clone(),
                content_type: self.content_type.clone(),
            })
        }

        fn name(&self) -> &str {
            "static"
        }
    }

    #[cfg(feature = "runtime-archives")]
    #[tokio::test]
    async fn download_and_save_extracts_compressed_subtitle_before_writing() {
        let subtitle_content =
            b"[Script Info]\nTitle: Test\n\n[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hello\n";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(subtitle_content).unwrap();
        let gzip_content = encoder.finish().unwrap();

        let provider = StaticProvider {
            content: gzip_content,
            format: "ass".to_string(),
            filename: Some("release.ass.gz".to_string()),
            content_type: Some("application/gzip".to_string()),
        };
        let temp = tempfile::tempdir().unwrap();
        let video_path = temp.path().join("Movie.mkv");
        std::fs::write(&video_path, b"fake video").unwrap();

        let (dest, file) =
            download_and_save(&provider, "provider-file", &video_path, "eng", false, false)
                .await
                .unwrap();

        assert_eq!(
            dest.file_name().and_then(|name| name.to_str()),
            Some("Movie.eng.ass")
        );
        assert_eq!(file.format, "ass");
        assert_eq!(std::fs::read(dest).unwrap(), subtitle_content);
    }

    #[test]
    fn subtitle_path_basic() {
        let path = build_subtitle_path(
            Path::new("/data/movies/Movie.2024.1080p.BluRay.mkv"),
            "eng",
            "srt",
            false,
            false,
        );
        assert_eq!(
            path,
            PathBuf::from("/data/movies/Movie.2024.1080p.BluRay.eng.srt")
        );
    }

    #[test]
    fn subtitle_path_forced() {
        let path = build_subtitle_path(
            Path::new("/data/movies/Movie.mkv"),
            "spa",
            "srt",
            true,
            false,
        );
        assert_eq!(path, PathBuf::from("/data/movies/Movie.spa.forced.srt"));
    }

    #[test]
    fn subtitle_path_hi() {
        let path = build_subtitle_path(
            Path::new("/data/movies/Movie.mkv"),
            "eng",
            "ass",
            false,
            true,
        );
        assert_eq!(path, PathBuf::from("/data/movies/Movie.eng.hi.ass"));
    }

    // ── Spaces in filename ──────────────────────────────────────────

    #[test]
    fn subtitle_path_with_spaces() {
        let path = build_subtitle_path(
            Path::new("/data/movies/My Great Movie 2024.mkv"),
            "eng",
            "srt",
            false,
            false,
        );
        assert_eq!(
            path,
            PathBuf::from("/data/movies/My Great Movie 2024.eng.srt")
        );
    }

    #[test]
    fn subtitle_path_with_spaces_forced() {
        let path = build_subtitle_path(
            Path::new("/data/series/My Show S01E02.mkv"),
            "spa",
            "srt",
            true,
            false,
        );
        assert_eq!(
            path,
            PathBuf::from("/data/series/My Show S01E02.spa.forced.srt")
        );
    }

    // ── Periods in filename (release-style names) ───────────────────

    #[test]
    fn subtitle_path_with_periods_in_filename() {
        let path = build_subtitle_path(
            Path::new("/data/movies/Movie.2024.1080p.BluRay.x264-GROUP.mkv"),
            "eng",
            "srt",
            false,
            false,
        );
        // file_stem takes everything before the last dot
        assert_eq!(
            path,
            PathBuf::from("/data/movies/Movie.2024.1080p.BluRay.x264-GROUP.eng.srt")
        );
    }

    #[test]
    fn subtitle_path_with_periods_hi() {
        let path = build_subtitle_path(
            Path::new("/data/movies/Some.Movie.2024.2160p.WEB-DL.DDP5.1.DV.HDR.H.265-NTb.mkv"),
            "eng",
            "srt",
            false,
            true,
        );
        assert_eq!(
            path,
            PathBuf::from(
                "/data/movies/Some.Movie.2024.2160p.WEB-DL.DDP5.1.DV.HDR.H.265-NTb.eng.hi.srt"
            )
        );
    }

    // ── No parent directory ─────────────────────────────────────────

    #[test]
    fn subtitle_path_no_parent_directory() {
        // Path::new("video.mkv").parent() returns Some("") not None,
        // so the result is just "video.eng.srt" without "./" prefix.
        let path = build_subtitle_path(Path::new("video.mkv"), "eng", "srt", false, false);
        assert_eq!(path, PathBuf::from("video.eng.srt"));
    }

    #[test]
    fn subtitle_path_no_parent_forced() {
        let path = build_subtitle_path(Path::new("video.mkv"), "jpn", "ass", true, false);
        assert_eq!(path, PathBuf::from("video.jpn.forced.ass"));
    }

    // ── Forced + hearing_impaired (forced takes precedence) ─────────

    #[test]
    fn subtitle_path_forced_takes_precedence_over_hi() {
        let path = build_subtitle_path(
            Path::new("/data/movies/Movie.mkv"),
            "spa",
            "srt",
            true,
            true,
        );
        // When both forced and HI are true, forced wins (checked first in the if-else)
        assert_eq!(path, PathBuf::from("/data/movies/Movie.spa.forced.srt"));
        // Verify it does NOT contain ".hi."
        let path_str = path.to_string_lossy();
        assert!(
            !path_str.contains(".hi."),
            "forced should take precedence, got: {path_str}"
        );
    }

    #[test]
    fn subtitle_path_forced_precedence_different_language() {
        let path = build_subtitle_path(
            Path::new("/data/series/Show.S01E01.mkv"),
            "fre",
            "srt",
            true,
            true,
        );
        assert_eq!(
            path,
            PathBuf::from("/data/series/Show.S01E01.fre.forced.srt")
        );
    }

    // ── Various formats ─────────────────────────────────────────────

    #[test]
    fn subtitle_path_sub_format() {
        let path = build_subtitle_path(
            Path::new("/data/movies/Movie.mkv"),
            "eng",
            "sub",
            false,
            false,
        );
        assert_eq!(path, PathBuf::from("/data/movies/Movie.eng.sub"));
    }

    #[test]
    fn subtitle_path_ssa_format() {
        let path = build_subtitle_path(
            Path::new("/data/movies/Movie.mkv"),
            "jpn",
            "ssa",
            false,
            false,
        );
        assert_eq!(path, PathBuf::from("/data/movies/Movie.jpn.ssa"));
    }
}
