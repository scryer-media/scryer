use std::path::{Path, PathBuf};

use super::language::normalize_subtitle_language_code;
use super::provider::{SubtitleFile, SubtitleProvider};
use crate::{AppError, AppResult};

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
    let allowed = ["srt", "ass", "ssa", "sub", "vtt", "idx"];
    if !allowed.contains(&fmt) {
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
    let language = normalize_language(language)?;
    let file = provider.download(provider_file_id).await?;
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
            Path::new("/data/tv/My Show S01E02.mkv"),
            "spa",
            "srt",
            true,
            false,
        );
        assert_eq!(
            path,
            PathBuf::from("/data/tv/My Show S01E02.spa.forced.srt")
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
            Path::new("/data/tv/Show.S01E01.mkv"),
            "fre",
            "srt",
            true,
            true,
        );
        assert_eq!(path, PathBuf::from("/data/tv/Show.S01E01.fre.forced.srt"));
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
