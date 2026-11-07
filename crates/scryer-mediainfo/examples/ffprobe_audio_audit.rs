use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use scryer_mediainfo::analyze_file;
use serde_json::Value;

const MAX_DEPTH: usize = 8;
const MAX_EXAMPLES_PER_KIND: usize = 20;

#[derive(Debug, Clone)]
struct ProbeAudioStream {
    codec: Option<String>,
    profile: Option<String>,
    channels: Option<i32>,
    bitrate_kbps: Option<i32>,
    language: Option<String>,
}

#[derive(Debug, Default)]
struct AuditStats {
    files_total: usize,
    files_with_audio: usize,
    files_with_any_mismatch: usize,
    total_native_audio_streams: usize,
    total_probe_audio_streams: usize,
    mismatch_counts: BTreeMap<&'static str, usize>,
    examples: BTreeMap<&'static str, Vec<String>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Volumes/Media/Movies"));

    let ffprobe = ffprobe_bin().ok_or("ffprobe not found on PATH")?;
    let mut files = discover_media_files(&root)?;
    files.sort();

    if files.is_empty() {
        return Err(format!("no media files found under {}", root.display()).into());
    }

    println!("Root: {}", root.display());
    println!("Files: {}", files.len());
    println!("ffprobe: {}", ffprobe.display());
    println!();

    let mut stats = AuditStats::default();

    for (index, path) in files.iter().enumerate() {
        if index % 50 == 0 {
            println!(
                "Progress: {}/{} ({})",
                index,
                files.len(),
                path.file_name()
                    .unwrap_or_else(|| OsStr::new("?"))
                    .to_string_lossy()
            );
        }

        stats.files_total += 1;

        let native = match analyze_file(path) {
            Ok(native) => native,
            Err(error) => {
                record_example(
                    &mut stats,
                    "native_failure",
                    format!("{} native failed: {error}", path.display()),
                );
                continue;
            }
        };

        let probe_json = match run_ffprobe_json(&ffprobe, path) {
            Ok(json) => json,
            Err(error) => {
                record_example(
                    &mut stats,
                    "ffprobe_failure",
                    format!("{} ffprobe failed: {error}", path.display()),
                );
                continue;
            }
        };

        let probe_audio_streams = ffprobe_audio_streams(&probe_json);
        if native.audio_streams.is_empty() && probe_audio_streams.is_empty() {
            continue;
        }

        stats.files_with_audio += 1;
        stats.total_native_audio_streams += native.audio_streams.len();
        stats.total_probe_audio_streams += probe_audio_streams.len();

        let mut file_has_mismatch = false;

        if native.audio_streams.len() != probe_audio_streams.len() {
            file_has_mismatch = true;
            increment_mismatch(
                &mut stats,
                "stream_count",
                format!(
                    "{} stream_count native={} ffprobe={}",
                    path.display(),
                    native.audio_streams.len(),
                    probe_audio_streams.len()
                ),
            );
        }

        for (stream_index, (native_stream, probe_stream)) in native
            .audio_streams
            .iter()
            .zip(probe_audio_streams.iter())
            .enumerate()
        {
            if native_stream.codec != probe_stream.codec {
                file_has_mismatch = true;
                increment_mismatch(
                    &mut stats,
                    "codec",
                    format!(
                        "{} stream {} codec native={:?} ffprobe={:?}",
                        path.display(),
                        stream_index,
                        native_stream.codec,
                        probe_stream.codec
                    ),
                );
            }

            if native_stream.profile != probe_stream.profile {
                file_has_mismatch = true;
                increment_mismatch(
                    &mut stats,
                    "profile",
                    format!(
                        "{} stream {} profile native={:?} ffprobe={:?}",
                        path.display(),
                        stream_index,
                        native_stream.profile,
                        probe_stream.profile
                    ),
                );
            }

            if native_stream.channels != probe_stream.channels {
                file_has_mismatch = true;
                increment_mismatch(
                    &mut stats,
                    "channels",
                    format!(
                        "{} stream {} channels native={:?} ffprobe={:?}",
                        path.display(),
                        stream_index,
                        native_stream.channels,
                        probe_stream.channels
                    ),
                );
            }

            if native_stream.language != probe_stream.language {
                file_has_mismatch = true;
                increment_mismatch(
                    &mut stats,
                    "language",
                    format!(
                        "{} stream {} language native={:?} ffprobe={:?}",
                        path.display(),
                        stream_index,
                        native_stream.language,
                        probe_stream.language
                    ),
                );
            }

            if let (Some(native_bitrate), Some(probe_bitrate)) =
                (native_stream.bitrate_kbps, probe_stream.bitrate_kbps)
                && (native_bitrate - probe_bitrate).abs() > 16
            {
                file_has_mismatch = true;
                increment_mismatch(
                    &mut stats,
                    "bitrate",
                    format!(
                        "{} stream {} bitrate native={} ffprobe={}",
                        path.display(),
                        stream_index,
                        native_bitrate,
                        probe_bitrate
                    ),
                );
            }
        }

        if file_has_mismatch {
            stats.files_with_any_mismatch += 1;
        }
    }

    println!();
    println!("files_total: {}", stats.files_total);
    println!("files_with_audio: {}", stats.files_with_audio);
    println!("files_with_any_mismatch: {}", stats.files_with_any_mismatch);
    println!(
        "native_audio_streams_total: {}",
        stats.total_native_audio_streams
    );
    println!(
        "ffprobe_audio_streams_total: {}",
        stats.total_probe_audio_streams
    );
    println!();
    println!("Mismatch counts:");
    for (kind, count) in &stats.mismatch_counts {
        println!("  {kind}: {count}");
    }

    println!();
    println!("Examples:");
    for (kind, examples) in &stats.examples {
        println!("  {kind}:");
        for example in examples {
            println!("    {example}");
        }
    }

    Ok(())
}

fn increment_mismatch(stats: &mut AuditStats, kind: &'static str, example: String) {
    *stats.mismatch_counts.entry(kind).or_default() += 1;
    record_example(stats, kind, example);
}

fn record_example(stats: &mut AuditStats, kind: &'static str, example: String) {
    let examples = stats.examples.entry(kind).or_default();
    if examples.len() < MAX_EXAMPLES_PER_KIND {
        examples.push(example);
    }
}

fn ffprobe_audio_streams(json: &Value) -> Vec<ProbeAudioStream> {
    json.get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"))
        .map(|stream| ProbeAudioStream {
            codec: stream
                .get("codec_name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            profile: stream
                .get("profile")
                .and_then(Value::as_str)
                .filter(|profile| !profile.is_empty() && *profile != "unknown")
                .map(str::to_owned),
            channels: stream
                .get("channels")
                .and_then(Value::as_i64)
                .map(|value| value as i32),
            bitrate_kbps: stream
                .get("bit_rate")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<i64>().ok())
                .map(|value| (value / 1000) as i32),
            language: stream
                .get("tags")
                .and_then(|tags| tags.get("language").or_else(|| tags.get("LANGUAGE")))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty() && *value != "und")
                .map(str::to_owned),
        })
        .collect()
}

fn run_ffprobe_json(ffprobe: &Path, path: &Path) -> Result<Value, String> {
    let output = Command::new(ffprobe)
        .arg("-v")
        .arg("error")
        .arg("-probesize")
        .arg("150000000")
        .arg("-analyzeduration")
        .arg("150000000")
        .arg("-show_entries")
        .arg("stream=index,codec_type,codec_name,profile,channels,bit_rate:stream_tags=language")
        .arg("-show_streams")
        .arg("-print_format")
        .arg("json")
        .arg(path)
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("failed to parse ffprobe JSON: {error}"))
}

fn ffprobe_bin() -> Option<PathBuf> {
    let candidates = ["ffprobe", "/opt/homebrew/bin/ffprobe"];
    for candidate in candidates {
        let output = Command::new(candidate).arg("-version").output().ok()?;
        if output.status.success() {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

fn discover_media_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_media_files(root, 0, &mut files)?;
    Ok(files)
}

fn collect_media_files(root: &Path, depth: usize, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_media_files(&path, depth + 1, files)?;
            continue;
        }
        if is_media_file(&path) {
            files.push(path);
        }
    }

    Ok(())
}

fn is_media_file(path: &Path) -> bool {
    scryer_media_types::formats::media_format_for_path(path)
        .is_some_and(|entry| entry.handling == scryer_media_types::formats::MediaHandling::Native)
}
