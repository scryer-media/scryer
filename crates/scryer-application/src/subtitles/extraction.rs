use std::cmp::Reverse;
use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::path::{Component, Path};
use std::sync::Arc;

use flate2::read::GzDecoder;
use scryer_plugin_sdk::{
    ArchivePluginFormat, ArchivePluginOperation, ArchivePluginProcessRequest,
    ArchivePluginProcessResponse, ArchivePluginStatus,
};

use super::provider::SubtitleFile;
use crate::{AppError, AppResult, ArchiveExtractorClient, ArchiveExtractorPluginProvider};

const MAX_RECURSION_DEPTH: usize = 3;
const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
const MAX_EXPANDED_BYTES: usize = 128 * 1024 * 1024;
const MAX_ARCHIVE_FILES: usize = 512;

const SUPPORTED_SUBTITLE_FORMATS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx"];

#[derive(Debug, Clone, Default)]
pub struct SubtitleExtractionContext {
    pub language: Option<String>,
    pub episode: Option<i32>,
    pub absolute_episode: Option<i32>,
}

struct ArchiveCandidate {
    filename: String,
    file: SubtitleFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactKind {
    Zip,
    Tar,
    SevenZip,
    Rar,
    Gzip,
    Zstd,
    Xz,
}

pub fn is_supported_subtitle_format(format: &str) -> bool {
    SUPPORTED_SUBTITLE_FORMATS.contains(&normalize_extension(format).as_str())
}

pub async fn normalize_downloaded_subtitle(
    file: SubtitleFile,
    context: SubtitleExtractionContext,
) -> AppResult<SubtitleFile> {
    normalize_downloaded_subtitle_with_archive_provider(file, context, None).await
}

pub async fn normalize_downloaded_subtitle_with_archive_provider(
    file: SubtitleFile,
    context: SubtitleExtractionContext,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
) -> AppResult<SubtitleFile> {
    if let Some((archive_type, format)) = plugin_subtitle_archive_format(&file)
        && let Some(client) =
            archive_provider.and_then(|provider| provider.client_for_format(format))
    {
        return normalize_with_archive_plugin(file, context, archive_type, format, client).await;
    }

    tokio::task::spawn_blocking(move || normalize_sync(file, &context, 0))
        .await
        .map_err(|error| {
            AppError::Repository(format!("subtitle extraction task failed: {error}"))
        })?
}

async fn normalize_with_archive_plugin(
    file: SubtitleFile,
    context: SubtitleExtractionContext,
    archive_type: ArtifactKind,
    format: ArchivePluginFormat,
    client: Arc<dyn ArchiveExtractorClient>,
) -> AppResult<SubtitleFile> {
    let temp_dir = tempfile::tempdir().map_err(|error| {
        AppError::Repository(format!(
            "failed to create subtitle archive scratch dir: {error}"
        ))
    })?;
    let input_name = safe_archive_filename(&file, archive_type);
    let input_path = temp_dir.path().join(input_name);
    fs::write(&input_path, &file.content).map_err(|error| {
        AppError::Repository(format!(
            "failed to write subtitle archive scratch file: {error}"
        ))
    })?;
    let output_dir = temp_dir.path().join("output");
    fs::create_dir_all(&output_dir).map_err(|error| {
        AppError::Repository(format!(
            "failed to create subtitle archive output dir: {error}"
        ))
    })?;

    let response = client
        .process(ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::ExtractArchive {
                archive_path: input_path.to_string_lossy().into_owned(),
                output_dir: output_dir.to_string_lossy().into_owned(),
                format,
                password: None,
            },
        })
        .await?;
    handle_subtitle_archive_plugin_response(response)?;

    let candidates = tokio::task::spawn_blocking({
        let output_dir = output_dir.clone();
        move || collect_plugin_output_candidates(&output_dir)
    })
    .await
    .map_err(|error| {
        AppError::Repository(format!("subtitle archive candidate task failed: {error}"))
    })??;

    select_archive_candidate(candidates, &context, 0)
}

fn plugin_subtitle_archive_format(
    file: &SubtitleFile,
) -> Option<(ArtifactKind, ArchivePluginFormat)> {
    match detect_artifact_kind(file) {
        Some(ArtifactKind::Zip) => Some((ArtifactKind::Zip, ArchivePluginFormat::Zip)),
        Some(ArtifactKind::SevenZip) => {
            Some((ArtifactKind::SevenZip, ArchivePluginFormat::SevenZip))
        }
        Some(ArtifactKind::Rar) => Some((ArtifactKind::Rar, ArchivePluginFormat::Rar)),
        Some(ArtifactKind::Xz) => Some((ArtifactKind::Xz, ArchivePluginFormat::Xz)),
        _ => None,
    }
}

fn safe_archive_filename(file: &SubtitleFile, archive_type: ArtifactKind) -> String {
    let extension = match archive_type {
        ArtifactKind::Zip => "zip",
        ArtifactKind::SevenZip => "7z",
        ArtifactKind::Rar => "rar",
        ArtifactKind::Xz => "xz",
        _ => "archive",
    };
    let filename = file
        .filename
        .as_deref()
        .and_then(|name| Path::new(name).file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("subtitle_archive");
    if Path::new(filename).extension().is_some() {
        filename.to_string()
    } else {
        format!("{filename}.{extension}")
    }
}

fn handle_subtitle_archive_plugin_response(
    response: ArchivePluginProcessResponse,
) -> AppResult<()> {
    match response.status {
        ArchivePluginStatus::Ok => Ok(()),
        ArchivePluginStatus::UnsupportedFormat => Err(AppError::Validation(
            "archive plugin does not support this subtitle archive format".to_string(),
        )),
        ArchivePluginStatus::PasswordRequired => Err(AppError::Validation(
            "subtitle archive requires a password".to_string(),
        )),
        ArchivePluginStatus::PasswordInvalid => Err(AppError::Validation(
            "subtitle archive password is invalid".to_string(),
        )),
        ArchivePluginStatus::Failed => {
            let message = response
                .message
                .or(response.error_code)
                .unwrap_or_else(|| "archive plugin subtitle extraction failed".to_string());
            Err(AppError::Repository(message))
        }
    }
}

fn normalize_sync(
    file: SubtitleFile,
    context: &SubtitleExtractionContext,
    depth: usize,
) -> AppResult<SubtitleFile> {
    if depth > MAX_RECURSION_DEPTH {
        return Err(AppError::Validation(
            "subtitle artifact nesting is too deep".to_string(),
        ));
    }
    if file.content.len() > MAX_ARTIFACT_BYTES {
        return Err(AppError::Validation(format!(
            "subtitle artifact is too large: {} bytes",
            file.content.len()
        )));
    }

    match detect_artifact_kind(&file) {
        Some(ArtifactKind::Gzip) => {
            let filename = file.filename;
            let format = file.format;
            let content = read_limited(GzDecoder::new(Cursor::new(file.content)))?;
            normalize_sync(
                inner_file_from_parts(filename, format, content, ".gz"),
                context,
                depth + 1,
            )
        }
        Some(ArtifactKind::Zstd) => {
            let filename = file.filename;
            let format = file.format;
            let decoder =
                zstd::stream::read::Decoder::new(Cursor::new(file.content)).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to initialize Zstandard subtitle decoder: {error}"
                    ))
                })?;
            let content = read_limited(decoder)?;
            normalize_sync(
                inner_file_from_parts(filename, format, content, ".zst"),
                context,
                depth + 1,
            )
        }
        Some(ArtifactKind::Xz) => Err(AppError::archive_extraction_plugin_required(None)),
        Some(ArtifactKind::Zip) => Err(AppError::archive_extraction_plugin_required(None)),
        Some(ArtifactKind::Tar) => {
            select_archive_candidate(extract_tar_candidates(file.content)?, context, depth)
        }
        Some(ArtifactKind::SevenZip) => Err(AppError::archive_extraction_plugin_required(None)),
        Some(ArtifactKind::Rar) => Err(AppError::archive_extraction_plugin_required(None)),
        None => finalize_subtitle(file),
    }
}

fn finalize_subtitle(mut file: SubtitleFile) -> AppResult<SubtitleFile> {
    let Some(format) = final_subtitle_format(&file) else {
        return Err(AppError::Validation(format!(
            "unsupported subtitle artifact format: {}",
            file.filename
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or(file.format.as_str())
        )));
    };

    if !file.content.iter().any(|byte| !byte.is_ascii_whitespace()) {
        return Err(AppError::Repository(
            "subtitle download returned empty content".to_string(),
        ));
    }

    file.format = format;
    file.content_type = subtitle_content_type(file.format.as_str()).map(str::to_string);
    Ok(file)
}

fn select_archive_candidate(
    candidates: Vec<ArchiveCandidate>,
    context: &SubtitleExtractionContext,
    depth: usize,
) -> AppResult<SubtitleFile> {
    let mut normalized = Vec::new();
    for candidate in candidates {
        if let Ok(file) = normalize_sync(candidate.file, context, depth + 1) {
            normalized.push((candidate.filename, file));
        }
    }

    if normalized.is_empty() {
        return Err(AppError::Validation(
            "subtitle archive did not contain a supported subtitle file".to_string(),
        ));
    }

    normalized.sort_by_key(|(filename, file)| Reverse(candidate_rank(filename, file, context)));

    if normalized.len() > 1 {
        let first = candidate_rank(&normalized[0].0, &normalized[0].1, context);
        let second = candidate_rank(&normalized[1].0, &normalized[1].1, context);
        if first == second {
            return Err(AppError::Validation(format!(
                "subtitle archive has multiple equally ranked subtitle files: {}, {}",
                normalized[0].0, normalized[1].0
            )));
        }
    }

    Ok(normalized.remove(0).1)
}

fn candidate_rank(
    filename: &str,
    file: &SubtitleFile,
    context: &SubtitleExtractionContext,
) -> (i32, i32, i32, usize) {
    let mut episode_score = 0;
    if let Some(episode) = context.episode
        && filename_matches_number(filename, episode)
    {
        episode_score += 2;
    }
    if let Some(absolute_episode) = context.absolute_episode
        && filename_matches_number(filename, absolute_episode)
    {
        episode_score += 1;
    }

    let language_score = context
        .language
        .as_deref()
        .is_some_and(|language| filename_matches_language(filename, language))
        as i32;
    let format_score = subtitle_format_rank(file.format.as_str());
    let size_score = file.content.len().min(1024 * 1024);

    (episode_score, language_score, format_score, size_score)
}

fn extract_tar_candidates(content: Vec<u8>) -> AppResult<Vec<ArchiveCandidate>> {
    let mut archive = tar::Archive::new(Cursor::new(content));
    let mut candidates = Vec::new();
    let mut expanded_bytes = 0usize;
    let mut entry_count = 0usize;

    for entry in archive
        .entries()
        .map_err(|error| AppError::Repository(format!("invalid tar subtitle archive: {error}")))?
    {
        entry_count += 1;
        if entry_count > MAX_ARCHIVE_FILES {
            return Err(AppError::Validation(
                "subtitle archive contains too many files".to_string(),
            ));
        }
        let mut entry = entry.map_err(|error| {
            AppError::Repository(format!("failed to read tar subtitle entry: {error}"))
        })?;
        let path = entry.path().map_err(|error| {
            AppError::Repository(format!("failed to read tar subtitle entry path: {error}"))
        })?;
        let name = path.to_string_lossy().to_string();
        if !is_safe_relative_path(&name) || !is_extractable_subtitle_artifact(&name) {
            continue;
        }
        let content = read_limited(&mut entry)?;
        expanded_bytes = checked_expanded_size(expanded_bytes, content.len())?;
        candidates.push(candidate(name, content));
    }

    Ok(candidates)
}

fn collect_plugin_output_candidates(output_dir: &Path) -> AppResult<Vec<ArchiveCandidate>> {
    let mut candidates = Vec::new();
    let mut expanded_bytes = 0usize;
    let mut entry_count = 0usize;
    let mut stack = vec![output_dir.to_path_buf()];

    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|error| {
            AppError::Repository(format!(
                "failed to read archive plugin output directory '{}': {error}",
                dir.display()
            ))
        })? {
            let entry = entry.map_err(|error| {
                AppError::Repository(format!(
                    "failed to read archive plugin output entry: {error}"
                ))
            })?;
            let file_type = entry.file_type().map_err(|error| {
                AppError::Repository(format!(
                    "failed to read archive plugin output file type: {error}"
                ))
            })?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            entry_count += 1;
            if entry_count > MAX_ARCHIVE_FILES {
                return Err(AppError::Validation(
                    "subtitle archive contains too many files".to_string(),
                ));
            }

            let path = entry.path();
            let relative = path.strip_prefix(output_dir).map_err(|_| {
                AppError::Validation(format!(
                    "archive plugin output escaped output root: {}",
                    path.display()
                ))
            })?;
            let name = relative.to_string_lossy().to_string();
            if !is_safe_relative_path(&name) || !is_extractable_subtitle_artifact(&name) {
                continue;
            }

            let mut file = File::open(&path).map_err(|error| {
                AppError::Repository(format!(
                    "failed to open archive plugin subtitle output '{}': {error}",
                    path.display()
                ))
            })?;
            let content = read_limited(&mut file)?;
            expanded_bytes = checked_expanded_size(expanded_bytes, content.len())?;
            candidates.push(candidate(name, content));
        }
    }

    Ok(candidates)
}

fn candidate(filename: String, content: Vec<u8>) -> ArchiveCandidate {
    let format = extension_for_filename(&filename)
        .or_else(|| detect_subtitle_format_from_content(&content))
        .unwrap_or_else(|| "bin".to_string());
    ArchiveCandidate {
        filename: filename.clone(),
        file: SubtitleFile {
            content,
            format,
            filename: Some(filename),
            content_type: None,
        },
    }
}

fn inner_file_from_parts(
    parent_filename: Option<String>,
    parent_format: String,
    content: Vec<u8>,
    suffix: &str,
) -> SubtitleFile {
    let filename = compressed_tar_alias(parent_filename.as_deref(), suffix).or_else(|| {
        parent_filename
            .as_deref()
            .and_then(|name| strip_extension_suffix(name, suffix))
    });
    let format = filename
        .as_deref()
        .and_then(extension_for_filename)
        .filter(|format| format != "gz" && format != "zst" && format != "xz")
        .unwrap_or(parent_format);
    SubtitleFile {
        content,
        format,
        filename,
        content_type: None,
    }
}

fn detect_artifact_kind(file: &SubtitleFile) -> Option<ArtifactKind> {
    if let Some(filename) = file.filename.as_deref()
        && let Some(kind) = artifact_kind_from_filename(filename)
    {
        return Some(kind);
    }
    if let Some(kind) = artifact_kind_from_content_type(file.content_type.as_deref()) {
        return Some(kind);
    }
    artifact_kind_from_magic(&file.content)
}

fn artifact_kind_from_filename(filename: &str) -> Option<ArtifactKind> {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        Some(ArtifactKind::Zip)
    } else if lower.ends_with(".tar") {
        Some(ArtifactKind::Tar)
    } else if lower.ends_with(".7z") {
        Some(ArtifactKind::SevenZip)
    } else if lower.ends_with(".rar") {
        Some(ArtifactKind::Rar)
    } else if lower.ends_with(".gz") || lower.ends_with(".tgz") {
        Some(ArtifactKind::Gzip)
    } else if lower.ends_with(".zst") || lower.ends_with(".tzst") {
        Some(ArtifactKind::Zstd)
    } else if lower.ends_with(".xz") || lower.ends_with(".txz") {
        Some(ArtifactKind::Xz)
    } else {
        None
    }
}

fn artifact_kind_from_content_type(content_type: Option<&str>) -> Option<ArtifactKind> {
    let lower = content_type?.to_ascii_lowercase();
    if lower.contains("zip") {
        Some(ArtifactKind::Zip)
    } else if lower.contains("x-tar") || lower.contains("tar") {
        Some(ArtifactKind::Tar)
    } else if lower.contains("7z") {
        Some(ArtifactKind::SevenZip)
    } else if lower.contains("rar") {
        Some(ArtifactKind::Rar)
    } else if lower.contains("gzip") || lower.contains("x-gzip") {
        Some(ArtifactKind::Gzip)
    } else if lower.contains("zstd") || lower.contains("zst") {
        Some(ArtifactKind::Zstd)
    } else if lower.contains("xz") || lower.contains("lzma") {
        Some(ArtifactKind::Xz)
    } else {
        None
    }
}

fn artifact_kind_from_magic(content: &[u8]) -> Option<ArtifactKind> {
    if content.starts_with(b"PK\x03\x04")
        || content.starts_with(b"PK\x05\x06")
        || content.starts_with(b"PK\x07\x08")
    {
        Some(ArtifactKind::Zip)
    } else if content.starts_with(b"7z\xBC\xAF\x27\x1C") {
        Some(ArtifactKind::SevenZip)
    } else if content.starts_with(b"Rar!\x1A\x07\x00")
        || content.starts_with(b"Rar!\x1A\x07\x01\x00")
    {
        Some(ArtifactKind::Rar)
    } else if content.starts_with(b"\x1F\x8B") {
        Some(ArtifactKind::Gzip)
    } else if content.starts_with(b"\x28\xB5\x2F\xFD") {
        Some(ArtifactKind::Zstd)
    } else if content.starts_with(b"\xFD\x37\x7A\x58\x5A\x00") {
        Some(ArtifactKind::Xz)
    } else if content.len() > 262 && &content[257..262] == b"ustar" {
        Some(ArtifactKind::Tar)
    } else {
        None
    }
}

fn final_subtitle_format(file: &SubtitleFile) -> Option<String> {
    file.filename
        .as_deref()
        .and_then(extension_for_filename)
        .filter(|format| is_supported_subtitle_format(format))
        .or_else(|| {
            is_supported_subtitle_format(&file.format).then(|| normalize_extension(&file.format))
        })
        .or_else(|| detect_subtitle_format_from_content(&file.content))
}

fn detect_subtitle_format_from_content(content: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(content)
        .ok()?
        .trim_start_matches('\u{feff}')
        .trim_start();
    if text.starts_with("WEBVTT") {
        return Some("vtt".to_string());
    }
    if text.starts_with("[Script Info]") || text.contains("\nDialogue:") {
        return Some("ass".to_string());
    }
    if text.contains("-->") {
        return Some("srt".to_string());
    }
    None
}

fn subtitle_format_rank(format: &str) -> i32 {
    match normalize_extension(format).as_str() {
        "ass" => 60,
        "ssa" => 55,
        "srt" => 50,
        "vtt" => 40,
        "sub" => 10,
        "idx" => 5,
        _ => 0,
    }
}

fn subtitle_content_type(format: &str) -> Option<&'static str> {
    match normalize_extension(format).as_str() {
        "srt" => Some("application/x-subrip"),
        "ass" | "ssa" => Some("text/x-ssa"),
        "vtt" => Some("text/vtt"),
        "sub" | "idx" => Some("application/octet-stream"),
        _ => None,
    }
}

fn is_extractable_subtitle_artifact(filename: &str) -> bool {
    extension_for_filename(filename)
        .as_deref()
        .is_some_and(is_supported_subtitle_format)
        || artifact_kind_from_filename(filename).is_some()
}

fn extension_for_filename(filename: &str) -> Option<String> {
    Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(normalize_extension)
}

fn normalize_extension(extension: &str) -> String {
    extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

fn strip_extension_suffix(filename: &str, suffix: &str) -> Option<String> {
    filename
        .to_ascii_lowercase()
        .ends_with(suffix)
        .then(|| filename[..filename.len() - suffix.len()].to_string())
}

fn filename_matches_number(filename: &str, number: i32) -> bool {
    let lower = filename.to_ascii_lowercase();
    let plain = number.to_string();
    let padded = format!("{number:02}");
    lower.contains(&format!("e{padded}"))
        || lower.contains(&format!("ep{padded}"))
        || lower.contains(&format!("episode {number}"))
        || split_filename_tokens(&lower)
            .iter()
            .any(|token| token == &plain || token == &padded)
}

fn filename_matches_language(filename: &str, language: &str) -> bool {
    let normalized = language.to_ascii_lowercase();
    let fallback = [normalized.as_str()];
    let aliases: &[&str] = match normalized.as_str() {
        "eng" | "en" => &["eng", "en", "english"],
        "jpn" | "ja" => &["jpn", "ja", "japanese"],
        "spa" | "es" => &["spa", "es", "spanish"],
        "fre" | "fr" => &["fre", "fra", "fr", "french"],
        "ger" | "de" => &["ger", "deu", "de", "german"],
        "ita" | "it" => &["ita", "it", "italian"],
        "por" | "pt" => &["por", "pt", "portuguese"],
        _ => &fallback,
    };
    let tokens = split_filename_tokens(&filename.to_ascii_lowercase());
    aliases
        .iter()
        .any(|alias| tokens.iter().any(|token| token == alias))
}

fn compressed_tar_alias(filename: Option<&str>, suffix: &str) -> Option<String> {
    let filename = filename?;
    match suffix {
        ".gz" => strip_extension_suffix(filename, ".tgz").map(|name| format!("{name}.tar")),
        ".zst" => strip_extension_suffix(filename, ".tzst").map(|name| format!("{name}.tar")),
        ".xz" => strip_extension_suffix(filename, ".txz").map(|name| format!("{name}.tar")),
        _ => None,
    }
}

fn split_filename_tokens(filename: &str) -> Vec<String> {
    filename
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn is_safe_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn read_limited<R: Read>(reader: R) -> AppResult<Vec<u8>> {
    let mut limited = reader.take((MAX_EXPANDED_BYTES + 1) as u64);
    let mut out = Vec::new();
    limited.read_to_end(&mut out).map_err(|error| {
        AppError::Repository(format!("failed to read subtitle artifact: {error}"))
    })?;
    ensure_expanded_size(out.len())?;
    Ok(out)
}

fn ensure_expanded_size(size: usize) -> AppResult<()> {
    if size > MAX_EXPANDED_BYTES {
        return Err(AppError::Validation(format!(
            "subtitle artifact expands beyond the {} byte limit",
            MAX_EXPANDED_BYTES
        )));
    }
    Ok(())
}

fn checked_expanded_size(current: usize, next: usize) -> AppResult<usize> {
    let total = current.saturating_add(next);
    ensure_expanded_size(total)?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    const ASS_CONTENT: &[u8] = b"[Script Info]\nTitle: Test\n\n[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hello\n";
    const SRT_CONTENT: &[u8] = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n";

    fn subtitle_file(filename: &str, content: Vec<u8>) -> SubtitleFile {
        SubtitleFile {
            content,
            format: extension_for_filename(filename).unwrap_or_else(|| "bin".to_string()),
            filename: Some(filename.to_string()),
            content_type: None,
        }
    }

    fn context() -> SubtitleExtractionContext {
        SubtitleExtractionContext {
            language: Some("eng".to_string()),
            episode: Some(17),
            absolute_episode: Some(17),
        }
    }

    struct RecordingArchiveClient {
        operation: Arc<Mutex<Option<ArchivePluginOperation>>>,
    }

    #[async_trait::async_trait]
    impl ArchiveExtractorClient for RecordingArchiveClient {
        async fn process(
            &self,
            request: ArchivePluginProcessRequest,
        ) -> AppResult<ArchivePluginProcessResponse> {
            let output_dir = match &request.operation {
                ArchivePluginOperation::ExtractArchive { output_dir, .. } => output_dir,
                ArchivePluginOperation::Inspect { .. } => {
                    return Ok(ArchivePluginProcessResponse {
                        status: ArchivePluginStatus::Failed,
                        files: Vec::new(),
                        expanded_bytes: None,
                        copied_bytes: None,
                        staged_bytes: None,
                        error_code: Some("unsupported_operation".to_string()),
                        message: Some("operation does not extract archive files".to_string()),
                    });
                }
            };
            let output_dir = Path::new(output_dir);
            fs::create_dir_all(output_dir).unwrap();
            fs::write(output_dir.join("Show.S01E17.eng.srt"), SRT_CONTENT).unwrap();
            *self.operation.lock().unwrap() = Some(request.operation);
            Ok(ArchivePluginProcessResponse {
                status: ArchivePluginStatus::Ok,
                files: Vec::new(),
                expanded_bytes: Some(SRT_CONTENT.len() as u64),
                copied_bytes: None,
                staged_bytes: None,
                error_code: None,
                message: None,
            })
        }
    }

    struct RecordingArchiveProvider {
        client: Arc<dyn ArchiveExtractorClient>,
        formats: Vec<ArchivePluginFormat>,
    }

    impl ArchiveExtractorPluginProvider for RecordingArchiveProvider {
        fn client_for_format(
            &self,
            format: ArchivePluginFormat,
        ) -> Option<Arc<dyn ArchiveExtractorClient>> {
            self.formats
                .contains(&format)
                .then(|| Arc::clone(&self.client))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["recording".to_string()]
        }
    }

    fn tar_with_entry(name: &str, content: &[u8]) -> Vec<u8> {
        let mut tar = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar);
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_cksum();
            builder.append_data(&mut header, name, content).unwrap();
            builder.finish().unwrap();
        }
        tar
    }

    #[tokio::test]
    async fn raw_subtitle_passes_through() {
        let normalized = normalize_downloaded_subtitle(
            subtitle_file("release.ass", ASS_CONTENT.to_vec()),
            context(),
        )
        .await
        .unwrap();

        assert_eq!(normalized.format, "ass");
        assert_eq!(normalized.content, ASS_CONTENT);
    }

    #[tokio::test]
    async fn gzip_subtitle_decompresses() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(SRT_CONTENT).unwrap();
        let gz = encoder.finish().unwrap();

        let normalized =
            normalize_downloaded_subtitle(subtitle_file("release.srt.gz", gz), context())
                .await
                .unwrap();

        assert_eq!(normalized.format, "srt");
        assert_eq!(normalized.content, SRT_CONTENT);
    }

    #[tokio::test]
    async fn zstd_subtitle_decompresses() {
        let zst = zstd::encode_all(Cursor::new(SRT_CONTENT), 0).unwrap();
        let normalized =
            normalize_downloaded_subtitle(subtitle_file("release.srt.zst", zst), context())
                .await
                .unwrap();

        assert_eq!(normalized.format, "srt");
        assert_eq!(normalized.content, SRT_CONTENT);
    }

    #[tokio::test]
    async fn xz_subtitle_requires_archive_plugin() {
        let err = normalize_downloaded_subtitle(
            subtitle_file("release.ass.xz", b"xz".to_vec()),
            context(),
        )
        .await
        .unwrap_err();

        assert!(matches!(
            err,
            AppError::ArchiveExtractionPluginRequired { .. }
        ));
    }

    #[tokio::test]
    async fn xz_subtitle_uses_archive_plugin_when_supported() {
        let operation = Arc::new(Mutex::new(None));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(RecordingArchiveClient {
            operation: Arc::clone(&operation),
        });
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client,
                formats: vec![ArchivePluginFormat::Xz],
            });

        let normalized = normalize_downloaded_subtitle_with_archive_provider(
            subtitle_file("release.ass.xz", b"xz".to_vec()),
            context(),
            Some(provider),
        )
        .await
        .unwrap();

        assert_eq!(normalized.format, "srt");
        assert_eq!(normalized.content, SRT_CONTENT);
        let recorded = operation.lock().unwrap().clone().unwrap();
        assert!(matches!(
            recorded,
            ArchivePluginOperation::ExtractArchive {
                format: ArchivePluginFormat::Xz,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn zip_subtitle_requires_archive_plugin() {
        let err =
            normalize_downloaded_subtitle(subtitle_file("release.zip", b"zip".to_vec()), context())
                .await
                .unwrap_err();

        assert!(matches!(
            err,
            AppError::ArchiveExtractionPluginRequired { .. }
        ));
    }

    #[tokio::test]
    async fn sevenz_subtitle_requires_archive_plugin() {
        let err =
            normalize_downloaded_subtitle(subtitle_file("release.7z", b"7z".to_vec()), context())
                .await
                .unwrap_err();

        assert!(matches!(
            err,
            AppError::ArchiveExtractionPluginRequired { .. }
        ));
    }

    #[tokio::test]
    async fn sevenz_subtitle_uses_archive_plugin_when_supported() {
        let operation = Arc::new(Mutex::new(None));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(RecordingArchiveClient {
            operation: Arc::clone(&operation),
        });
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client,
                formats: vec![ArchivePluginFormat::SevenZip],
            });

        let normalized = normalize_downloaded_subtitle_with_archive_provider(
            subtitle_file("release.7z", b"7z".to_vec()),
            context(),
            Some(provider),
        )
        .await
        .unwrap();

        assert_eq!(normalized.format, "srt");
        assert_eq!(normalized.content, SRT_CONTENT);
        let recorded = operation.lock().unwrap().clone().unwrap();
        assert!(matches!(
            recorded,
            ArchivePluginOperation::ExtractArchive {
                format: ArchivePluginFormat::SevenZip,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn tar_archive_extracts_subtitle() {
        let tar = tar_with_entry("Show.S01E17.eng.srt", SRT_CONTENT);

        let normalized =
            normalize_downloaded_subtitle(subtitle_file("release.tar", tar), context())
                .await
                .unwrap();

        assert_eq!(normalized.format, "srt");
        assert_eq!(normalized.content, SRT_CONTENT);
    }

    #[tokio::test]
    async fn compressed_tar_archive_extracts_subtitle() {
        let tar = tar_with_entry("Show.S01E17.eng.ass", ASS_CONTENT);
        let zst = zstd::encode_all(Cursor::new(tar), 0).unwrap();

        let normalized =
            normalize_downloaded_subtitle(subtitle_file("release.tar.zst", zst), context())
                .await
                .unwrap();

        assert_eq!(normalized.format, "ass");
        assert_eq!(normalized.content, ASS_CONTENT);
    }

    #[test]
    fn review_regression_tgz_aliases_to_tar_when_unwrapping_gzip() {
        let file = inner_file_from_parts(
            Some("release.tgz".to_string()),
            "gz".to_string(),
            Vec::new(),
            ".gz",
        );

        assert_eq!(file.filename.as_deref(), Some("release.tar"));
        assert_eq!(file.format, "tar");
    }

    #[tokio::test]
    async fn rar_subtitle_requires_archive_plugin() {
        let err =
            normalize_downloaded_subtitle(subtitle_file("release.rar", b"rar".to_vec()), context())
                .await
                .unwrap_err();

        assert!(matches!(
            err,
            AppError::ArchiveExtractionPluginRequired { .. }
        ));
    }
}
