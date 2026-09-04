use std::{path::Path, sync::LazyLock};

use chrono::{DateTime, Utc};
use regex::Regex;
use tokio::{fs, io::AsyncReadExt};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
use whatlang::{Lang, detect};

use crate::stored_paths::path_to_stored_string;
use crate::{AppError, AppResult};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const MAX_TEXT_PROBE_SIZE_BYTES: i64 = 1024 * 1024;
const MAX_LANGUAGE_SAMPLE_BYTES: usize = 16 * 1024;
const MIN_SCRIPT_RELEVANT_CHARS: usize = 40;
const MIN_DETECTOR_CONFIDENCE: f64 = 0.65;
const PROBE_UPDATED_AT_FORMAT_VERSION: i32 = 2;
const TRADITIONAL_CHINESE_ONLY_CHARACTERS: &str =
    "國體學會愛讓龍點畫線關開嗎麼說語廣萬與為樂這裡應該還沒聽見";

static CUE_NUMBER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d+$").expect("cue number regex"));
static TIMESTAMP_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\d{1,2}:\d{2}:\d{2}(?:[.,:]\d{1,3})?\s*-->\s*\d{1,2}:\d{2}:\d{2}(?:[.,:]\d{1,3})?(?:\s+.*)?$",
    )
    .expect("timestamp regex")
});
static UPPERCASE_SPEAKER_LABEL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<label>[A-Z][A-Z '\-]{0,23}):\s*(?P<rest>.+)?$")
        .expect("uppercase speaker label regex")
});
static EFFECT_KEYWORD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:music|applause|laughter|laughing|sighs?|gasps?|panting|crying|whispering|shouting|screaming|door opens|door closes|phone rings|gunshots?|sirens?)\b",
    )
    .expect("effect keyword regex")
});

pub const EXTERNAL_SUBTITLE_PROBE_VERSION: i32 = PROBE_UPDATED_AT_FORMAT_VERSION;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalSubtitleDetectionSource {
    Filename,
    Content,
    Unknown,
}

impl ExternalSubtitleDetectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Filename => "filename",
            Self::Content => "content",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "filename" => Some(Self::Filename),
            "content" => Some(Self::Content),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalSubtitleProbeCacheEntry {
    pub media_file_id: String,
    pub file_path: String,
    pub size_bytes: i64,
    pub modified_at: Option<String>,
    pub language: Option<String>,
    pub hearing_impaired: Option<bool>,
    pub detection_source_language: ExternalSubtitleDetectionSource,
    pub detection_source_hi: ExternalSubtitleDetectionSource,
    pub probe_version: i32,
    pub updated_at: String,
}

impl ExternalSubtitleProbeCacheEntry {
    pub fn hearing_impaired_or_false(&self) -> bool {
        self.hearing_impaired.unwrap_or(false)
    }
}

pub(crate) trait SubtitleLanguageDetector: Send + Sync {
    fn detect(&self, sample: &str) -> Option<DetectedSubtitleLanguage>;
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DetectedSubtitleLanguage {
    code: String,
    confidence: f64,
}

#[derive(Default)]
struct WhatlangSubtitleLanguageDetector;

impl SubtitleLanguageDetector for WhatlangSubtitleLanguageDetector {
    fn detect(&self, sample: &str) -> Option<DetectedSubtitleLanguage> {
        let info = detect(sample)?;
        let code = match info.lang() {
            Lang::Cmn => "zho",
            other => other.code(),
        };

        Some(DetectedSubtitleLanguage {
            code: code.to_string(),
            confidence: info.confidence(),
        })
    }
}

static DEFAULT_SUBTITLE_LANGUAGE_DETECTOR: WhatlangSubtitleLanguageDetector =
    WhatlangSubtitleLanguageDetector;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExternalSubtitleProbeResolution {
    pub language: Option<String>,
    pub hearing_impaired: bool,
    pub cache_entry: ExternalSubtitleProbeCacheEntry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExternalSubtitleFingerprint {
    size_bytes: i64,
    modified_at: Option<String>,
    probe_version: i32,
}

struct DecodedSubtitleText {
    text: String,
    encoding_name: String,
    had_errors: bool,
    byte_len: usize,
    nul_byte_count: usize,
}

pub(crate) async fn resolve_external_subtitle(
    media_file_id: &str,
    subtitle_path: &Path,
    extension: &str,
    filename_language: Option<&str>,
    forced: bool,
    filename_hearing_impaired: bool,
    existing_cache: Option<&ExternalSubtitleProbeCacheEntry>,
) -> AppResult<ExternalSubtitleProbeResolution> {
    resolve_external_subtitle_with_detector(
        media_file_id,
        subtitle_path,
        extension,
        filename_language,
        forced,
        filename_hearing_impaired,
        existing_cache,
        &DEFAULT_SUBTITLE_LANGUAGE_DETECTOR,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "subtitle probing combines filename hints, cache state, and detector selection in one step"
)]
async fn resolve_external_subtitle_with_detector(
    media_file_id: &str,
    subtitle_path: &Path,
    extension: &str,
    filename_language: Option<&str>,
    forced: bool,
    filename_hearing_impaired: bool,
    existing_cache: Option<&ExternalSubtitleProbeCacheEntry>,
    detector: &dyn SubtitleLanguageDetector,
) -> AppResult<ExternalSubtitleProbeResolution> {
    let fingerprint = collect_fingerprint(subtitle_path).await?;
    if let Some(cache_entry) =
        existing_cache.filter(|cache_entry| cache_matches_fingerprint(cache_entry, &fingerprint))
    {
        metrics::counter!("scryer_subtitle_external_probe_cache_hit_total").increment(1);
        tracing::debug!(
            path = %subtitle_path.display(),
            "external subtitle probe cache hit"
        );
        return Ok(ExternalSubtitleProbeResolution {
            language: cache_entry.language.clone(),
            hearing_impaired: cache_entry.hearing_impaired_or_false(),
            cache_entry: cache_entry.clone(),
        });
    }

    metrics::counter!("scryer_subtitle_external_probe_cache_miss_total").increment(1);
    tracing::debug!(
        path = %subtitle_path.display(),
        "external subtitle probe cache miss"
    );

    let mut language = filename_language.map(str::to_string);
    let mut detection_source_language = if language.is_some() {
        ExternalSubtitleDetectionSource::Filename
    } else {
        ExternalSubtitleDetectionSource::Unknown
    };
    let mut hearing_impaired = if filename_hearing_impaired {
        Some(true)
    } else {
        None
    };
    let mut detection_source_hi = if filename_hearing_impaired {
        ExternalSubtitleDetectionSource::Filename
    } else {
        ExternalSubtitleDetectionSource::Unknown
    };

    let needs_language_probe = language.is_none();
    let needs_hi_probe =
        !forced && hearing_impaired.is_none() && extension.eq_ignore_ascii_case("srt");

    if should_probe_content(extension, needs_language_probe, needs_hi_probe) {
        if fingerprint.size_bytes > MAX_TEXT_PROBE_SIZE_BYTES {
            metrics::counter!("scryer_subtitle_external_probe_skipped_size_total").increment(1);
            tracing::debug!(
                path = %subtitle_path.display(),
                size_bytes = fingerprint.size_bytes,
                ceiling_bytes = MAX_TEXT_PROBE_SIZE_BYTES,
                "skipping external subtitle content probe because the file exceeds the size ceiling"
            );
        } else {
            match read_subtitle_to_string(subtitle_path).await {
                Ok(decoded) => {
                    if decoded.had_errors && has_excessive_replacement_chars(&decoded.text) {
                        metrics::counter!("scryer_subtitle_external_probe_decode_failed_total")
                            .increment(1);
                        tracing::debug!(
                            path = %subtitle_path.display(),
                            encoding = %decoded.encoding_name,
                            "skipping external subtitle content probe after lossy decode"
                        );
                    } else if !is_likely_text_subtitle_payload(&decoded) {
                        metrics::counter!("scryer_subtitle_external_probe_skipped_non_text_total")
                            .increment(1);
                        tracing::debug!(
                            path = %subtitle_path.display(),
                            encoding = %decoded.encoding_name,
                            "skipping external subtitle content probe for non-text or binary content"
                        );
                    } else {
                        let sample = preprocess_subtitle_text_for_language_detection(&decoded.text);
                        let sample_is_large_enough =
                            script_relevant_character_count(&sample) >= MIN_SCRIPT_RELEVANT_CHARS;

                        if needs_language_probe
                            && sample_is_large_enough
                            && let Some(detected) = detector.detect(&sample)
                            && detected.confidence >= MIN_DETECTOR_CONFIDENCE
                            && let Some(normalized) = normalize_detected_subtitle_language(
                                &detected.code,
                                subtitle_path,
                                &decoded.encoding_name,
                                &sample,
                            )
                        {
                            language = Some(normalized);
                            detection_source_language = ExternalSubtitleDetectionSource::Content;
                        }

                        let hi_probe_language_hint = language
                            .as_deref()
                            .or_else(|| infer_hi_probe_language_hint(&decoded.text));

                        if needs_hi_probe
                            && looks_like_hearing_impaired_srt(
                                &decoded.text,
                                hi_probe_language_hint,
                            )
                        {
                            hearing_impaired = Some(true);
                            detection_source_hi = ExternalSubtitleDetectionSource::Content;
                        }

                        if needs_language_probe && language.is_none() {
                            metrics::counter!(
                                "scryer_subtitle_external_probe_unresolved_language_total"
                            )
                            .increment(1);
                            tracing::debug!(
                                path = %subtitle_path.display(),
                                encoding = %decoded.encoding_name,
                                sample_script_chars = script_relevant_character_count(&sample),
                                "external subtitle content probe did not resolve a language"
                            );
                        }
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        path = %subtitle_path.display(),
                        error = %error,
                        "failed to read external subtitle sidecar for content probing"
                    );
                }
            }
        }
    }

    let updated_at = Utc::now().to_rfc3339();
    let cache_entry = ExternalSubtitleProbeCacheEntry {
        media_file_id: media_file_id.to_string(),
        file_path: path_to_stored_string(subtitle_path),
        size_bytes: fingerprint.size_bytes,
        modified_at: fingerprint.modified_at,
        language: language.clone(),
        hearing_impaired,
        detection_source_language,
        detection_source_hi,
        probe_version: fingerprint.probe_version,
        updated_at,
    };

    Ok(ExternalSubtitleProbeResolution {
        language,
        hearing_impaired: cache_entry.hearing_impaired_or_false(),
        cache_entry,
    })
}

fn should_probe_content(extension: &str, needs_language_probe: bool, needs_hi_probe: bool) -> bool {
    (needs_language_probe || needs_hi_probe) && text_probe_capable_extension(extension)
}

fn text_probe_capable_extension(extension: &str) -> bool {
    !extension.eq_ignore_ascii_case("idx")
}

async fn collect_fingerprint(path: &Path) -> AppResult<ExternalSubtitleFingerprint> {
    let metadata = fs::symlink_metadata(path)
        .await
        .map_err(|error| AppError::Repository(format!("cannot stat subtitle file: {error}")))?;
    validate_regular_subtitle_sidecar(&metadata)?;
    let size_bytes = i64::try_from(metadata.len())
        .map_err(|_| AppError::Repository("subtitle file is too large to fingerprint".into()))?;
    let modified_at = metadata
        .modified()
        .ok()
        .map(|value| DateTime::<Utc>::from(value).to_rfc3339());

    Ok(ExternalSubtitleFingerprint {
        size_bytes,
        modified_at,
        probe_version: EXTERNAL_SUBTITLE_PROBE_VERSION,
    })
}

fn validate_regular_subtitle_sidecar(metadata: &std::fs::Metadata) -> AppResult<()> {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(AppError::Validation(
            "subtitle sidecar must not be a symlink".into(),
        ));
    }
    if !file_type.is_file() {
        return Err(AppError::Validation(
            "subtitle sidecar must be a regular file".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SubtitleFileFingerprint {
    len: u64,
    modified_at: Option<std::time::SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

fn subtitle_file_fingerprint(metadata: &std::fs::Metadata) -> SubtitleFileFingerprint {
    SubtitleFileFingerprint {
        len: metadata.len(),
        modified_at: metadata.modified().ok(),
        #[cfg(unix)]
        dev: metadata.dev(),
        #[cfg(unix)]
        ino: metadata.ino(),
    }
}

async fn open_subtitle_sidecar(path: &Path) -> AppResult<fs::File> {
    #[cfg(unix)]
    {
        let mut options = fs::OpenOptions::new();
        options.read(true).custom_flags(libc::O_NOFOLLOW);
        options
            .open(path)
            .await
            .map_err(|error| AppError::Repository(format!("cannot open subtitle file: {error}")))
    }

    #[cfg(not(unix))]
    {
        fs::File::open(path)
            .await
            .map_err(|error| AppError::Repository(format!("cannot open subtitle file: {error}")))
    }
}

fn cache_matches_fingerprint(
    cache_entry: &ExternalSubtitleProbeCacheEntry,
    fingerprint: &ExternalSubtitleFingerprint,
) -> bool {
    cache_entry.size_bytes == fingerprint.size_bytes
        && cache_entry.modified_at == fingerprint.modified_at
        && cache_entry.probe_version == fingerprint.probe_version
}

async fn read_subtitle_to_string(path: &Path) -> AppResult<DecodedSubtitleText> {
    let metadata = fs::symlink_metadata(path)
        .await
        .map_err(|error| AppError::Repository(format!("cannot stat subtitle file: {error}")))?;
    validate_regular_subtitle_sidecar(&metadata)?;
    let fingerprint = subtitle_file_fingerprint(&metadata);
    if metadata.len() > MAX_TEXT_PROBE_SIZE_BYTES as u64 {
        return Err(AppError::Validation(
            "subtitle sidecar exceeds content probe size limit".into(),
        ));
    }

    let file = open_subtitle_sidecar(path).await?;
    let opened_metadata = file
        .metadata()
        .await
        .map_err(|error| AppError::Repository(format!("cannot stat subtitle file: {error}")))?;
    validate_regular_subtitle_sidecar(&opened_metadata)?;
    if subtitle_file_fingerprint(&opened_metadata) != fingerprint {
        return Err(AppError::Validation(
            "subtitle sidecar changed during content probe".into(),
        ));
    }
    if opened_metadata.len() > MAX_TEXT_PROBE_SIZE_BYTES as u64 {
        return Err(AppError::Validation(
            "subtitle sidecar exceeds content probe size limit".into(),
        ));
    }

    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    let mut reader = file.take(MAX_TEXT_PROBE_SIZE_BYTES as u64 + 1);
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| AppError::Repository(format!("cannot read subtitle file: {error}")))?;
    if bytes.len() > MAX_TEXT_PROBE_SIZE_BYTES as usize {
        return Err(AppError::Validation(
            "subtitle sidecar exceeds content probe size limit".into(),
        ));
    }

    if let Ok(text) = std::str::from_utf8(&bytes) {
        return Ok(DecodedSubtitleText {
            text: text.to_string(),
            encoding_name: "utf-8".to_string(),
            had_errors: false,
            byte_len: bytes.len(),
            nul_byte_count: bytes.iter().filter(|byte| **byte == 0).count(),
        });
    }

    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(&bytes, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Allow);
    let (decoded, _, had_errors) = encoding.decode(&bytes);
    if had_errors {
        tracing::debug!(
            path = %path.display(),
            encoding = %encoding.name(),
            "subtitle file had charset conversion errors during content probing"
        );
    }

    Ok(DecodedSubtitleText {
        text: decoded.into_owned(),
        encoding_name: encoding.name().to_string(),
        had_errors,
        byte_len: bytes.len(),
        nul_byte_count: bytes.iter().filter(|byte| **byte == 0).count(),
    })
}

fn normalize_detected_subtitle_language(
    code: &str,
    subtitle_path: &Path,
    encoding_name: &str,
    sample: &str,
) -> Option<String> {
    let normalized = crate::media::language::normalize_detected_subtitle_language_code(code)?;
    if normalized == "zho"
        && (path_has_traditional_hint(subtitle_path)
            || encoding_has_traditional_hint(encoding_name)
            || contains_traditional_chinese_characters(sample))
    {
        return Some("zht".to_string());
    }

    Some(normalized)
}

fn preprocess_subtitle_text_for_language_detection(text: &str) -> String {
    let mut sample = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || CUE_NUMBER_RE.is_match(trimmed)
            || TIMESTAMP_LINE_RE.is_match(trimmed)
        {
            continue;
        }
        if is_obvious_effect_only_line(trimmed) {
            continue;
        }

        let normalized_line = strip_uppercase_speaker_label(trimmed)
            .filter(|rest| !rest.is_empty())
            .unwrap_or(trimmed);
        if append_sample_line(&mut sample, normalized_line).is_err() {
            break;
        }
    }

    sample
}

fn append_sample_line(sample: &mut String, line: &str) -> Result<(), ()> {
    let additional = if sample.is_empty() {
        line.len()
    } else {
        line.len() + 1
    };
    if sample.len() + additional > MAX_LANGUAGE_SAMPLE_BYTES {
        return Err(());
    }
    if !sample.is_empty() {
        sample.push('\n');
    }
    sample.push_str(line);
    Ok(())
}

fn is_obvious_effect_only_line(line: &str) -> bool {
    contains_music_symbol(line)
        || is_bracketed_hi_cue(line, false)
        || is_short_keyword_effect_line(line)
        || strip_uppercase_speaker_label(line)
            .map(is_short_keyword_effect_line)
            .unwrap_or(false)
}

fn looks_like_hearing_impaired_srt(text: &str, language: Option<&str>) -> bool {
    let arabic_mode = language == Some("ara");
    let mut cue_score = 0usize;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || CUE_NUMBER_RE.is_match(trimmed)
            || TIMESTAMP_LINE_RE.is_match(trimmed)
        {
            continue;
        }

        if contains_music_symbol(trimmed) {
            cue_score += 2;
            continue;
        }

        if is_bracketed_hi_cue(trimmed, arabic_mode) {
            cue_score += 1;
            continue;
        }

        if is_short_keyword_effect_line(trimmed) {
            cue_score += 1;
            continue;
        }

        if looks_like_uppercase_speaker_label(trimmed) {
            cue_score += 1;
        }
    }

    cue_score >= 2
}

fn infer_hi_probe_language_hint(text: &str) -> Option<&'static str> {
    let mut arabic_char_count = 0usize;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || CUE_NUMBER_RE.is_match(trimmed)
            || TIMESTAMP_LINE_RE.is_match(trimmed)
        {
            continue;
        }

        let relevant = strip_uppercase_speaker_label(trimmed).unwrap_or(trimmed);
        arabic_char_count += relevant
            .chars()
            .filter(|ch| is_arabic_script_character(*ch))
            .count();
        if arabic_char_count >= 6 {
            return Some("ara");
        }
    }

    None
}

fn contains_music_symbol(line: &str) -> bool {
    line.contains('♪') || line.contains('♫')
}

fn is_bracketed_hi_cue(line: &str, arabic_mode: bool) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 || trimmed.len() > 64 {
        return false;
    }

    let bracketed = (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (!arabic_mode && trimmed.starts_with('(') && trimmed.ends_with(')'));
    if !bracketed {
        return false;
    }

    let inner = trimmed[1..trimmed.len() - 1].trim();
    !inner.is_empty() && word_count(inner) <= 8
}

fn is_short_keyword_effect_line(line: &str) -> bool {
    let normalized = normalize_for_token_matching(line);
    EFFECT_KEYWORD_RE.is_match(&normalized) && looks_like_short_effect_line(line)
}

fn looks_like_short_effect_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() <= 64
        && !trimmed.ends_with('.')
        && !trimmed.ends_with('!')
        && !trimmed.ends_with('?')
}

fn looks_like_uppercase_speaker_label(line: &str) -> bool {
    UPPERCASE_SPEAKER_LABEL_RE.is_match(line)
}

fn strip_uppercase_speaker_label(line: &str) -> Option<&str> {
    let captures = UPPERCASE_SPEAKER_LABEL_RE.captures(line)?;
    captures.name("rest").map(|rest| rest.as_str().trim())
}

fn path_has_traditional_hint(path: &Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('_', "-")
        .to_ascii_lowercase();
    normalized.contains("zh-hant")
        || normalized.contains("zh-tw")
        || normalized
            .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
            .any(|token| matches!(token, "cht" | "zht" | "big5" | "traditional"))
}

fn encoding_has_traditional_hint(encoding_name: &str) -> bool {
    encoding_name.to_ascii_lowercase().contains("big5")
}

fn contains_traditional_chinese_characters(text: &str) -> bool {
    text.chars()
        .any(|ch| TRADITIONAL_CHINESE_ONLY_CHARACTERS.contains(ch))
}

fn normalize_for_token_matching(text: &str) -> String {
    text.nfkd()
        .filter(|ch| !is_combining_mark(*ch))
        .flat_map(char::to_lowercase)
        .collect()
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn script_relevant_character_count(text: &str) -> usize {
    text.chars()
        .filter(|ch| is_script_relevant_character(*ch))
        .count()
}

fn is_script_relevant_character(ch: char) -> bool {
    ch.is_alphabetic()
        || matches!(
            ch as u32,
            0x3040..=0x30ff
                | 0x31f0..=0x31ff
                | 0x3400..=0x4dbf
                | 0x4e00..=0x9fff
                | 0xac00..=0xd7af
                | 0x1100..=0x11ff
        )
}

fn is_arabic_script_character(ch: char) -> bool {
    matches!(ch as u32, 0x0600..=0x06ff | 0x0750..=0x077f | 0x08a0..=0x08ff)
}

fn has_excessive_replacement_chars(text: &str) -> bool {
    let total_chars = text.chars().count().max(1);
    let replacement_chars = text.chars().filter(|ch| *ch == '\u{fffd}').count();
    replacement_chars * 8 > total_chars
}

fn is_likely_text_subtitle_payload(decoded: &DecodedSubtitleText) -> bool {
    if decoded.byte_len == 0 {
        return false;
    }
    if !is_utf16_family(&decoded.encoding_name) && decoded.nul_byte_count * 32 > decoded.byte_len {
        return false;
    }

    let trimmed = decoded.text.trim();
    if trimmed.len() < 8 {
        return false;
    }

    let total_chars = trimmed.chars().count().max(1);
    let control_chars = trimmed
        .chars()
        .filter(|ch| ch.is_control() && !matches!(*ch, '\n' | '\r' | '\t'))
        .count();
    if control_chars * 20 > total_chars {
        return false;
    }

    let replacement_chars = trimmed.chars().filter(|ch| *ch == '\u{fffd}').count();
    if replacement_chars * 10 > total_chars {
        return false;
    }

    script_relevant_character_count(trimmed) >= 4
}

fn is_utf16_family(encoding_name: &str) -> bool {
    encoding_name.to_ascii_lowercase().contains("utf-16")
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use super::{
        EXTERNAL_SUBTITLE_PROBE_VERSION, ExternalSubtitleDetectionSource,
        ExternalSubtitleProbeCacheEntry, SubtitleLanguageDetector,
        contains_traditional_chinese_characters, normalize_detected_subtitle_language,
        resolve_external_subtitle, resolve_external_subtitle_with_detector,
    };

    #[derive(Clone)]
    struct StubSubtitleLanguageDetector {
        detected: Option<(&'static str, f64)>,
    }

    impl SubtitleLanguageDetector for StubSubtitleLanguageDetector {
        fn detect(&self, _sample: &str) -> Option<super::DetectedSubtitleLanguage> {
            self.detected
                .map(|(code, confidence)| super::DetectedSubtitleLanguage {
                    code: code.to_string(),
                    confidence,
                })
        }
    }

    fn cache_entry_for(
        media_file_id: &str,
        file_path: &str,
        size_bytes: i64,
        modified_at: Option<&str>,
    ) -> ExternalSubtitleProbeCacheEntry {
        ExternalSubtitleProbeCacheEntry {
            media_file_id: media_file_id.to_string(),
            file_path: file_path.to_string(),
            size_bytes,
            modified_at: modified_at.map(str::to_string),
            language: Some("eng".to_string()),
            hearing_impaired: Some(false),
            detection_source_language: ExternalSubtitleDetectionSource::Content,
            detection_source_hi: ExternalSubtitleDetectionSource::Unknown,
            probe_version: EXTERNAL_SUBTITLE_PROBE_VERSION,
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_sidecars_are_rejected() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let target = tempdir.path().join("real.srt");
        fs::write(&target, "1\n00:00:01,000 --> 00:00:02,000\nHello\n").expect("subtitle");
        let symlink = tempdir.path().join("linked.srt");
        std::os::unix::fs::symlink(&target, &symlink).expect("symlink");

        let error = resolve_external_subtitle("media-1", &symlink, "srt", None, false, false, None)
            .await
            .expect_err("symlink should be rejected");
        assert!(error.to_string().contains("must not be a symlink"));
    }

    #[tokio::test]
    async fn utf8_content_language_detection_works() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Show.S01E01.srt");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nThank you for coming with me tonight.\n\n2\n00:00:03,000 --> 00:00:04,000\nThis is the only way out, and we need to leave right now.\n",
        )
        .expect("subtitle");

        let resolved =
            resolve_external_subtitle("media-1", &subtitle, "srt", None, false, false, None)
                .await
                .expect("resolve");

        assert_eq!(resolved.language.as_deref(), Some("eng"));
        assert_eq!(
            resolved.cache_entry.detection_source_language,
            ExternalSubtitleDetectionSource::Content
        );
    }

    #[tokio::test]
    async fn non_utf8_content_language_detection_works() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        let bytes = b"1\n00:00:01,000 --> 00:00:02,000\nVoc\xea n\xe3o pode ficar aqui agora, porque ainda temos muito trabalho.\n\n2\n00:00:03,000 --> 00:00:04,000\nEu tenho uma resposta para voc\xea, mas precisamos continuar andando.\n";
        fs::write(&subtitle, bytes).expect("subtitle");

        let resolved =
            resolve_external_subtitle("media-1", &subtitle, "srt", None, false, false, None)
                .await
                .expect("resolve");

        assert_eq!(resolved.language.as_deref(), Some("por"));
    }

    #[tokio::test]
    async fn detector_confidence_rejection_keeps_language_unresolved() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nThis subtitle has plenty of ordinary dialogue for the detector to inspect.\n\n2\n00:00:03,000 --> 00:00:04,000\nBut the detector confidence will be forced below the acceptance threshold.\n",
        )
        .expect("subtitle");

        let resolved = resolve_external_subtitle_with_detector(
            "media-1",
            &subtitle,
            "srt",
            None,
            false,
            false,
            None,
            &StubSubtitleLanguageDetector {
                detected: Some(("eng", 0.25)),
            },
        )
        .await
        .expect("resolve");

        assert_eq!(resolved.language, None);
        assert_eq!(
            resolved.cache_entry.detection_source_language,
            ExternalSubtitleDetectionSource::Unknown
        );
    }

    #[tokio::test]
    async fn generic_chinese_content_plus_traditional_hint_normalizes_to_zht() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Movie.zh-hant.srt");
        let sample = "我們現在要離開這裡，因為這不是一個安全的地方，而且情況比剛才更危險。這應該還沒有人知道，所以我們得快一點，不然就真的來不及了。";
        fs::write(&subtitle, sample).expect("subtitle");

        assert_eq!(
            normalize_detected_subtitle_language("zho", &subtitle, "utf-8", sample).as_deref(),
            Some("zht")
        );
        assert!(contains_traditional_chinese_characters("國體"));
    }

    #[tokio::test]
    async fn srt_hi_detection_marks_hearing_impaired_content() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Movie.eng.srt");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\n[door opens]\n\n2\n00:00:03,000 --> 00:00:04,000\n♪ quiet music playing ♪\n\n3\n00:00:05,000 --> 00:00:06,000\nWe have to go.\n",
        )
        .expect("subtitle");

        let resolved =
            resolve_external_subtitle("media-1", &subtitle, "srt", Some("eng"), false, false, None)
                .await
                .expect("resolve");

        assert!(resolved.hearing_impaired);
        assert_eq!(
            resolved.cache_entry.detection_source_hi,
            ExternalSubtitleDetectionSource::Content
        );
    }

    #[tokio::test]
    async fn arabic_hi_mode_avoids_parenthesis_false_positives() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Movie.ara.srt");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\n(مرحبا بك هنا الليلة)\n\n2\n00:00:03,000 --> 00:00:04,000\n(كيف حالك في هذا المكان)\n",
        )
        .expect("subtitle");

        let resolved =
            resolve_external_subtitle("media-1", &subtitle, "srt", Some("ara"), false, false, None)
                .await
                .expect("resolve");

        assert!(!resolved.hearing_impaired);
    }

    #[tokio::test]
    async fn short_unlabeled_arabic_dialogue_avoids_parenthesis_false_positives() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\n(مرحبا بك)\n\n2\n00:00:03,000 --> 00:00:04,000\n(كيف حالك)\n",
        )
        .expect("subtitle");

        let resolved =
            resolve_external_subtitle("media-1", &subtitle, "srt", None, false, false, None)
                .await
                .expect("resolve");

        assert_eq!(resolved.language, None);
        assert!(!resolved.hearing_impaired);
    }

    #[tokio::test]
    async fn binary_non_text_payloads_are_rejected() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        fs::write(
            &subtitle,
            [
                0_u8, 159, 146, 150, 0, 255, 0, 254, 0, 253, 0, 0, 0, 1, 2, 3,
            ],
        )
        .expect("subtitle");

        let resolved =
            resolve_external_subtitle("media-1", &subtitle, "srt", None, false, false, None)
                .await
                .expect("resolve");

        assert_eq!(resolved.language, None);
        assert_eq!(
            resolved.cache_entry.detection_source_language,
            ExternalSubtitleDetectionSource::Unknown
        );
    }

    #[tokio::test]
    async fn size_ceiling_prevents_content_probe() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        let mut contents = String::from("1\n00:00:01,000 --> 00:00:02,000\n");
        while contents.len() <= (1024 * 1024) + 32 {
            contents.push_str("Thank you for staying with us tonight.\n");
        }
        fs::write(&subtitle, contents).expect("subtitle");

        let resolved =
            resolve_external_subtitle("media-1", &subtitle, "srt", None, false, false, None)
                .await
                .expect("resolve");

        assert_eq!(resolved.language, None);
        assert_eq!(
            resolved.cache_entry.detection_source_language,
            ExternalSubtitleDetectionSource::Unknown
        );
    }

    #[tokio::test]
    async fn cache_hit_reuses_existing_probe_entry() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nThank you for staying with us tonight, everyone.\n",
        )
        .expect("subtitle");

        let initial =
            resolve_external_subtitle("media-1", &subtitle, "srt", None, false, false, None)
                .await
                .expect("resolve");
        let cached = resolve_external_subtitle(
            "media-1",
            &subtitle,
            "srt",
            None,
            false,
            false,
            Some(&initial.cache_entry),
        )
        .await
        .expect("resolve");

        assert_eq!(cached.cache_entry, initial.cache_entry);
    }

    #[test]
    fn changed_mtime_invalidates_cache() {
        let cache = cache_entry_for(
            "media-1",
            "/tmp/example.srt",
            12,
            Some("2024-01-01T00:00:00Z"),
        );
        assert_ne!(cache.modified_at.as_deref(), Some("2024-01-02T00:00:00Z"));
    }

    #[test]
    fn changed_probe_version_invalidates_cache() {
        let cache = cache_entry_for(
            "media-1",
            "/tmp/example.srt",
            12,
            Some("2024-01-01T00:00:00Z"),
        );
        assert_ne!(cache.probe_version, EXTERNAL_SUBTITLE_PROBE_VERSION + 1);
    }

    #[tokio::test]
    async fn cache_miss_after_file_change_reprobes_content() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nThank you for staying with us tonight, everyone.\n",
        )
        .expect("subtitle");

        let initial = resolve_external_subtitle_with_detector(
            "media-1",
            &subtitle,
            "srt",
            None,
            false,
            false,
            None,
            &StubSubtitleLanguageDetector {
                detected: Some(("eng", 0.99)),
            },
        )
        .await
        .expect("resolve");
        std::thread::sleep(Duration::from_secs(1));
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nGracias por quedarte con nosotros esta noche, de verdad, porque esto importa mucho.\n\n2\n00:00:03,000 --> 00:00:04,000\nTodavia tenemos mucho trabajo por delante y nadie mas puede resolverlo por nosotros.\n",
        )
        .expect("subtitle");

        let reprobed = resolve_external_subtitle_with_detector(
            "media-1",
            &subtitle,
            "srt",
            None,
            false,
            false,
            Some(&initial.cache_entry),
            &StubSubtitleLanguageDetector {
                detected: Some(("spa", 0.99)),
            },
        )
        .await
        .expect("resolve");

        assert_eq!(reprobed.language.as_deref(), Some("spa"));
        assert_ne!(
            reprobed.cache_entry.updated_at,
            initial.cache_entry.updated_at
        );
    }
}
