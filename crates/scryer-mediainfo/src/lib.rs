use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single audio stream extracted from ffprobe output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStreamDetail {
    pub codec: Option<String>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    pub bitrate_kbps: Option<i32>,
}

/// Parsed media properties extracted from ffprobe output.
#[derive(Debug, Clone)]
pub struct MediaAnalysis {
    pub video_codec: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bitrate_kbps: Option<i32>,
    pub video_bit_depth: Option<i32>,
    /// "Dolby Vision", "HDR10+", "HDR10", or "HLG"
    pub video_hdr_format: Option<String>,
    /// Frame rate as a decimal string, e.g. "23.976", "24", "60"
    pub video_frame_rate: Option<String>,
    /// Codec profile, e.g. "Main 10", "High", "Main"
    pub video_profile: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<i32>,
    /// Bitrate of the primary audio stream in kbps
    pub audio_bitrate_kbps: Option<i32>,
    /// Language tags from all audio streams (BCP-47 / ISO 639-2), "und" filtered out
    pub audio_languages: Vec<String>,
    /// All audio streams with per-stream details
    pub audio_streams: Vec<AudioStreamDetail>,
    /// Language tags from all subtitle streams
    pub subtitle_languages: Vec<String>,
    /// Codec names for all subtitle streams
    pub subtitle_codecs: Vec<String>,
    pub has_multiaudio: bool,
    pub duration_seconds: Option<i32>,
    pub container_format: Option<String>,
    /// Full ffprobe JSON output verbatim
    pub raw_json: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FfprobeError {
    #[error("ffprobe process failed: {0}")]
    ProcessFailed(String),
    #[error("ffprobe output could not be parsed: {0}")]
    ParseFailed(String),
}

/// Returns `true` if the analysis describes a valid video file (has a video
/// stream and non-zero duration). Returns `false` for executables, audio-only
/// files, corrupt containers, etc.
pub fn is_valid_video(analysis: &MediaAnalysis) -> bool {
    analysis.video_codec.is_some() && analysis.duration_seconds.map(|d| d > 0).unwrap_or(false)
}

/// Looks for `ffprobe` next to the current executable. Returns `None` if not
/// found — callers should skip analysis gracefully rather than failing.
pub fn locate_ffprobe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let ffprobe = dir.join("ffprobe");
    if ffprobe.exists() {
        Some(ffprobe)
    } else {
        None
    }
}

/// Runs ffprobe on `file_path` and returns parsed analysis. The ffprobe binary
/// path must be supplied by the caller (see `locate_ffprobe()`).
pub async fn analyze_file(
    ffprobe_path: &Path,
    file_path: &Path,
) -> Result<MediaAnalysis, FfprobeError> {
    let output = tokio::process::Command::new(ffprobe_path)
        .args(["-v", "quiet", "-print_format", "json", "-show_streams", "-show_format"])
        .arg(file_path)
        .output()
        .await
        .map_err(|e| FfprobeError::ProcessFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FfprobeError::ProcessFailed(format!(
            "exit code {:?}: {}",
            output.status.code(),
            stderr.trim()
        )));
    }

    let json = String::from_utf8_lossy(&output.stdout);
    parse_ffprobe_output(&json)
}

/// Parses raw ffprobe JSON output into a `MediaAnalysis`. Pure function — no
/// process call. Used directly by tests with fixture files.
pub fn parse_ffprobe_output(json: &str) -> Result<MediaAnalysis, FfprobeError> {
    let output: FfprobeOutput =
        serde_json::from_str(json).map_err(|e| FfprobeError::ParseFailed(e.to_string()))?;

    let video_stream = output
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"));

    let audio_streams: Vec<&FfprobeStream> = output
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .collect();

    let subtitle_streams: Vec<&FfprobeStream> = output
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("subtitle"))
        .collect();

    let video_codec = video_stream.and_then(|s| s.codec_name.clone());
    let video_width = video_stream.and_then(|s| s.width);
    let video_height = video_stream.and_then(|s| s.height);
    let video_bitrate_kbps = video_stream
        .and_then(|s| s.bit_rate.as_deref())
        .and_then(|br| br.parse::<i64>().ok())
        .map(|br| (br / 1000) as i32);
    let video_bit_depth = video_stream
        .and_then(|s| s.bits_per_raw_sample.as_deref())
        .and_then(|b| b.parse::<i32>().ok());
    let video_hdr_format = video_stream.and_then(detect_hdr);
    let video_frame_rate = video_stream
        .and_then(|s| s.r_frame_rate.as_deref())
        .and_then(parse_frame_rate);
    let video_profile = video_stream.and_then(|s| s.profile.clone());

    let primary_audio = audio_streams.first().copied();
    let audio_codec = primary_audio.and_then(|s| s.codec_name.clone());
    let audio_channels = primary_audio.and_then(|s| s.channels);
    let audio_bitrate_kbps = primary_audio
        .and_then(|s| s.bit_rate.as_deref())
        .and_then(|br| br.parse::<i64>().ok())
        .map(|br| (br / 1000) as i32);

    let audio_languages: Vec<String> = audio_streams
        .iter()
        .filter_map(|s| s.tags.language.as_deref())
        .filter(|lang| !lang.is_empty() && *lang != "und")
        .map(str::to_owned)
        .collect();

    let audio_streams_detail: Vec<AudioStreamDetail> = audio_streams
        .iter()
        .map(|s| AudioStreamDetail {
            codec: s.codec_name.clone(),
            channels: s.channels,
            language: s
                .tags
                .language
                .as_deref()
                .filter(|l| !l.is_empty() && *l != "und")
                .map(str::to_owned),
            bitrate_kbps: s
                .bit_rate
                .as_deref()
                .and_then(|br| br.parse::<i64>().ok())
                .map(|br| (br / 1000) as i32),
        })
        .collect();

    let subtitle_languages: Vec<String> = subtitle_streams
        .iter()
        .filter_map(|s| s.tags.language.as_deref())
        .filter(|lang| !lang.is_empty() && *lang != "und")
        .map(str::to_owned)
        .collect();

    let subtitle_codecs: Vec<String> = subtitle_streams
        .iter()
        .filter_map(|s| s.codec_name.clone())
        .collect();

    let has_multiaudio = audio_streams.len() > 1;

    let duration_seconds = output
        .format
        .as_ref()
        .and_then(|f| f.duration.as_deref())
        .and_then(|d| d.parse::<f64>().ok())
        .map(|d| d as i32);

    let container_format = output
        .format
        .as_ref()
        .and_then(|f| f.format_name.as_deref())
        .map(|name| {
            // "matroska,webm" -> "matroska"
            name.split(',').next().unwrap_or(name).to_owned()
        });

    Ok(MediaAnalysis {
        video_codec,
        video_width,
        video_height,
        video_bitrate_kbps,
        video_bit_depth,
        video_hdr_format,
        video_frame_rate,
        video_profile,
        audio_codec,
        audio_channels,
        audio_bitrate_kbps,
        audio_languages,
        audio_streams: audio_streams_detail,
        subtitle_languages,
        subtitle_codecs,
        has_multiaudio,
        duration_seconds,
        container_format,
        raw_json: json.to_owned(),
    })
}

/// Parse a rational frame rate string like "24000/1001" or "24/1" into a
/// human-readable decimal string like "23.976" or "24".
fn parse_frame_rate(r_frame_rate: &str) -> Option<String> {
    let (num_str, den_str) = r_frame_rate.split_once('/')?;
    let num: f64 = num_str.trim().parse().ok()?;
    let den: f64 = den_str.trim().parse().ok()?;
    if den == 0.0 {
        return None;
    }
    let fps = num / den;
    if fps <= 0.0 {
        return None;
    }
    // Round to 3 decimal places; trim trailing zeros
    let s = format!("{fps:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    Some(s.to_owned())
}

fn detect_hdr(stream: &FfprobeStream) -> Option<String> {
    for side_data in &stream.side_data_list {
        if let Some(ref sdt) = side_data.side_data_type {
            if sdt == "DOVI configuration record" {
                return Some("Dolby Vision".to_owned());
            }
            if sdt.contains("SMPTE2094-40") {
                return Some("HDR10+".to_owned());
            }
        }
    }
    match stream.color_transfer.as_deref() {
        Some("smpte2084") => Some("HDR10".to_owned()),
        Some("arib-std-b67") => Some("HLG".to_owned()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Internal Serde types for ffprobe JSON deserialization
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    #[serde(default)]
    format: Option<FfprobeFormat>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    codec_name: Option<String>,
    codec_type: Option<String>,
    profile: Option<String>,
    width: Option<i32>,
    height: Option<i32>,
    r_frame_rate: Option<String>,
    // ffprobe outputs these as strings; some versions may use numbers — handle both
    #[serde(default, deserialize_with = "string_from_value_opt")]
    bit_rate: Option<String>,
    #[serde(default, deserialize_with = "string_from_value_opt")]
    bits_per_raw_sample: Option<String>,
    color_transfer: Option<String>,
    channels: Option<i32>,
    #[serde(default)]
    side_data_list: Vec<FfprobeSideData>,
    #[serde(default)]
    tags: FfprobeTags,
}

#[derive(Deserialize)]
struct FfprobeSideData {
    side_data_type: Option<String>,
}

#[derive(Deserialize, Default)]
struct FfprobeTags {
    language: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    format_name: Option<String>,
    // ffprobe outputs duration as a string like "7200.123456"
    #[serde(default, deserialize_with = "string_from_value_opt")]
    duration: Option<String>,
}

/// Deserializes a field that may be a JSON string or number into `Option<String>`.
fn string_from_value_opt<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    match Option::<Value>::deserialize(deserializer)? {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(Value::Number(n)) => Ok(Some(n.to_string())),
        Some(_) => Ok(None),
    }
}
