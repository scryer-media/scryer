use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use scryer_mediainfo::{AnalysisProfile, AnalyzeOptions, MediaAnalysis, analyze_file_with_options};
use serde_json::Value;

const MAX_DEPTH: usize = 16;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::parse(env::args().skip(1))?;
    let ffprobe = config
        .ffprobe
        .clone()
        .or_else(ffprobe_bin)
        .ok_or("ffprobe not found; pass --ffprobe <path> or set FFPROBE")?;

    let selections = if config.files.is_empty() {
        discover_by_type(&config.root, config.per_type)?
    } else {
        group_explicit_files(config.files)
    };
    if selections.values().all(Vec::is_empty) {
        return Err(format!("no media files found under {}", config.root.display()).into());
    }

    println!("Root: {}", config.root.display());
    println!("Per type: {}", config.per_type);
    println!("ffprobe: {}", ffprobe.display());
    println!();
    println!(
        "{:<5} {:>10} {:>10} {:>12} {:>10} {:>8} file",
        "type", "fast_ms", "rich_ms", "scryer_ms", "ffprobe_ms", "ratio"
    );

    let mut summary = Summary::default();
    for (kind, files) in selections {
        if files.len() < config.per_type {
            eprintln!(
                "warning: found only {} {} files under {}",
                files.len(),
                kind,
                config.root.display()
            );
        }

        for path in files {
            let native = time_native_fast_then_rich(&path);
            let ffprobe_timing = time_sonarr_ffprobe(&ffprobe, &path);
            match (native, ffprobe_timing) {
                (Ok(native), Ok(ffprobe_duration)) => {
                    let ratio = ffprobe_duration.as_secs_f64()
                        / native.total_duration.as_secs_f64().max(0.000_001);
                    summary.record(kind, native.total_duration, ffprobe_duration);
                    println!(
                        "{:<5} {:>10.2} {:>10} {:>12.2} {:>10.2} {:>8.1} {}",
                        kind,
                        ms(native.fast_duration),
                        native
                            .rich_duration
                            .map(|duration| format!("{:.2}", ms(duration)))
                            .unwrap_or_else(|| "-".to_owned()),
                        ms(native.total_duration),
                        ms(ffprobe_duration),
                        ratio,
                        path.display()
                    );
                }
                (Err(error), _) => {
                    eprintln!("native error: {} -> {}", path.display(), error);
                }
                (_, Err(error)) => {
                    eprintln!("ffprobe error: {} -> {}", path.display(), error);
                }
            }
        }
    }

    println!();
    summary.print();

    Ok(())
}

struct Config {
    root: PathBuf,
    per_type: usize,
    ffprobe: Option<PathBuf>,
    files: Vec<PathBuf>,
}

impl Config {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut root = None;
        let mut per_type = 5;
        let mut ffprobe = None;
        let mut files = Vec::new();
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--per-type" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--per-type requires a value".to_owned())?;
                    per_type = value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid --per-type value: {value}"))?;
                }
                "--ffprobe" => {
                    ffprobe = Some(PathBuf::from(
                        args.next()
                            .ok_or_else(|| "--ffprobe requires a path".to_owned())?,
                    ));
                }
                "--file" => {
                    files.push(PathBuf::from(
                        args.next()
                            .ok_or_else(|| "--file requires a path".to_owned())?,
                    ));
                }
                "--help" | "-h" => {
                    return Err(
                        "usage: simple_raw_timings [root] [--per-type n] [--ffprobe path] [--file path...]"
                            .to_owned(),
                    );
                }
                value if value.starts_with('-') => {
                    return Err(format!("unknown argument: {value}"));
                }
                value => {
                    if root.replace(PathBuf::from(value)).is_some() {
                        return Err("only one root path may be provided".to_owned());
                    }
                }
            }
        }

        Ok(Self {
            root: root.unwrap_or_else(|| PathBuf::from("/Volumes/Media")),
            per_type,
            ffprobe,
            files,
        })
    }
}

struct NativeTiming {
    fast_duration: Duration,
    rich_duration: Option<Duration>,
    total_duration: Duration,
}

fn time_native_fast_then_rich(path: &Path) -> Result<NativeTiming, String> {
    if media_type(path) == Some("mkv") {
        let rich_start = Instant::now();
        analyze_file_with_options(
            path,
            AnalyzeOptions {
                profile: AnalysisProfile::DefaultRich,
            },
        )
        .map_err(|error| error.to_string())?;
        let rich_duration = rich_start.elapsed();
        return Ok(NativeTiming {
            fast_duration: rich_duration,
            rich_duration: None,
            total_duration: rich_duration,
        });
    }

    let fast_start = Instant::now();
    let fast = analyze_file_with_options(
        path,
        AnalyzeOptions {
            profile: AnalysisProfile::Fast,
        },
    )
    .map_err(|error| error.to_string())?;
    let fast_duration = fast_start.elapsed();
    if fast_analysis_has_adequate_facts(&fast) {
        return Ok(NativeTiming {
            fast_duration,
            rich_duration: None,
            total_duration: fast_duration,
        });
    }

    let rich_start = Instant::now();
    analyze_file_with_options(
        path,
        AnalyzeOptions {
            profile: AnalysisProfile::DefaultRich,
        },
    )
    .map_err(|error| error.to_string())?;
    let rich_duration = rich_start.elapsed();

    Ok(NativeTiming {
        fast_duration,
        rich_duration: Some(rich_duration),
        total_duration: fast_duration + rich_duration,
    })
}

fn fast_analysis_has_adequate_facts(analysis: &MediaAnalysis) -> bool {
    if !scryer_mediainfo::is_valid_video(analysis) {
        return false;
    }
    if analysis.video_width.is_none()
        || analysis.video_height.is_none()
        || analysis.duration_seconds.is_none()
        || analysis.video_frame_rate.is_none()
    {
        return false;
    }
    if !analysis.audio_streams.is_empty() && analysis.audio_codec.is_none() {
        return false;
    }
    if analysis.audio_streams.iter().any(|stream| {
        stream.codec.is_none()
            || stream.channels.is_none()
            || audio_codec_needs_profile(stream.codec.as_deref()) && stream.profile.is_none()
    }) {
        return false;
    }
    if analysis
        .subtitle_streams
        .iter()
        .any(|stream| stream.codec.is_none())
        || analysis.subtitle_codecs.len() < analysis.subtitle_streams.len()
    {
        return false;
    }

    !hevc_hdr_may_need_rich_confirmation(analysis)
}

fn audio_codec_needs_profile(codec: Option<&str>) -> bool {
    matches!(codec, Some("eac3" | "truehd" | "dts"))
}

fn hevc_hdr_may_need_rich_confirmation(analysis: &MediaAnalysis) -> bool {
    analysis.video_codec.as_deref() == Some("hevc")
        && analysis.video_hdr_format.as_deref() != Some("Dolby Vision")
        && (analysis
            .video_bit_depth
            .is_some_and(|bit_depth| bit_depth >= 10)
            || matches!(
                analysis.video_hdr_format.as_deref(),
                Some("HDR10" | "HLG" | "HDR10+")
            ))
}

fn time_sonarr_ffprobe(ffprobe: &Path, path: &Path) -> Result<Duration, String> {
    let start = Instant::now();
    sonarr_ffprobe_probe(ffprobe, path)?;
    Ok(start.elapsed())
}

fn sonarr_ffprobe_probe(ffprobe: &Path, path: &Path) -> Result<(), String> {
    let analysis = run_sonarr_analysis_json(ffprobe, path, &["-probesize", "50000000"])?;

    let analysis = if primary_audio_channel_layout_missing(&analysis) {
        run_sonarr_analysis_json(
            ffprobe,
            path,
            &["-probesize", "150000000", "-analyzeduration", "150000000"],
        )?
    } else {
        analysis
    };

    if let Some((video_stream_ordinal, primary_video)) = primary_video_stream(&analysis)
        && primary_video.get("color_transfer").and_then(Value::as_str) == Some("smpte2084")
    {
        run_sonarr_frames_json(ffprobe, path, video_stream_ordinal)?;
    }

    Ok(())
}

fn run_sonarr_analysis_json(
    ffprobe: &Path,
    path: &Path,
    custom_args: &[&str],
) -> Result<Value, String> {
    let output = Command::new(ffprobe)
        .arg("-loglevel")
        .arg("error")
        .arg("-print_format")
        .arg("json")
        .arg("-show_format")
        .arg("-sexagesimal")
        .arg("-show_streams")
        .arg("-show_chapters")
        .arg(path)
        .args(custom_args)
        .output()
        .map_err(|error| error.to_string())?;

    parse_ffprobe_output(output)
}

fn run_sonarr_frames_json(
    ffprobe: &Path,
    path: &Path,
    video_stream_ordinal: usize,
) -> Result<Value, String> {
    let select_stream = format!("v:{video_stream_ordinal}");
    let output = Command::new(ffprobe)
        .arg("-loglevel")
        .arg("error")
        .arg("-print_format")
        .arg("json")
        .arg("-show_frames")
        .arg("-v")
        .arg("quiet")
        .arg("-sexagesimal")
        .arg(path)
        .arg("-read_intervals")
        .arg("%+#1")
        .arg("-select_streams")
        .arg(select_stream)
        .output()
        .map_err(|error| error.to_string())?;

    parse_ffprobe_output(output)
}

fn parse_ffprobe_output(output: std::process::Output) -> Result<Value, String> {
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("failed to parse ffprobe JSON: {error}"))
}

fn primary_audio_channel_layout_missing(json: &Value) -> bool {
    json.get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"))
        .and_then(|stream| stream.get("channel_layout"))
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::is_empty)
        .unwrap_or(true)
}

fn primary_video_stream(json: &Value) -> Option<(usize, &Value)> {
    json.get("streams")?
        .as_array()?
        .iter()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("video"))
        .filter(|stream| {
            !matches!(
                stream.get("codec_name").and_then(Value::as_str),
                Some("mjpeg" | "png")
            )
        })
        .enumerate()
        .next()
}

fn discover_by_type(
    root: &Path,
    per_type: usize,
) -> std::io::Result<BTreeMap<&'static str, Vec<PathBuf>>> {
    let mut files = empty_media_groups();
    collect_by_type(root, 0, per_type, &mut files)?;
    Ok(files)
}

fn group_explicit_files(paths: Vec<PathBuf>) -> BTreeMap<&'static str, Vec<PathBuf>> {
    let mut files = empty_media_groups();
    for path in paths {
        if let Some(kind) = media_type(&path) {
            files.get_mut(kind).expect("known media type").push(path);
        }
    }
    files
}

fn empty_media_groups() -> BTreeMap<&'static str, Vec<PathBuf>> {
    scryer_media_types::formats::MEDIA_FORMATS
        .iter()
        .filter(|entry| entry.handling == scryer_media_types::formats::MediaHandling::Native)
        .map(|entry| (entry.extension, Vec::new()))
        .collect()
}

fn collect_by_type(
    root: &Path,
    depth: usize,
    per_type: usize,
    files: &mut BTreeMap<&'static str, Vec<PathBuf>>,
) -> std::io::Result<()> {
    if depth > MAX_DEPTH || files.values().all(|paths| paths.len() >= per_type) {
        return Ok(());
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return Ok(()),
        Err(error) => return Err(error),
    };

    for entry in entries {
        if files.values().all(|paths| paths.len() >= per_type) {
            break;
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => continue,
            Err(error) => return Err(error),
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().to_string();

        if file_type.is_dir() {
            if !is_ignored_dir_name(&name) {
                collect_by_type(&path, depth + 1, per_type, files)?;
            }
            continue;
        }

        if !file_type.is_file() || is_sample_name(&name) {
            continue;
        }
        let Some(kind) = media_type(&path) else {
            continue;
        };
        let paths = files.get_mut(kind).expect("known media type");
        if paths.len() < per_type {
            paths.push(path);
        }
    }

    Ok(())
}

fn media_type(path: &Path) -> Option<&'static str> {
    scryer_media_types::formats::media_format_for_path(path)
        .filter(|entry| entry.handling == scryer_media_types::formats::MediaHandling::Native)
        .map(|entry| entry.extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_every_supported_explicit_media_type() {
        let paths = ["sample.wmv", "sample.ogv", "sample.flv"]
            .map(PathBuf::from)
            .to_vec();
        let groups = group_explicit_files(paths);

        for media_type in ["wmv", "ogv", "flv"] {
            assert_eq!(groups[media_type].len(), 1, "{media_type} bucket");
        }
    }
}

fn is_ignored_dir_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with('.') || matches!(lower.as_str(), "@eadir" | ".@__thumb" | "plex versions")
}

fn is_sample_name(name: &str) -> bool {
    name.to_ascii_lowercase().contains("sample")
}

fn ffprobe_bin() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = env::var("FFPROBE") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("ffprobe"),
        PathBuf::from("/opt/homebrew/bin/ffprobe"),
        PathBuf::from("/usr/local/bin/ffprobe"),
    ]);

    candidates.into_iter().find(|candidate| {
        Command::new(candidate)
            .arg("-version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    })
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[derive(Default)]
struct Summary {
    by_type: BTreeMap<&'static str, TypeSummary>,
}

impl Summary {
    fn record(&mut self, kind: &'static str, native: Duration, ffprobe: Duration) {
        self.by_type
            .entry(kind)
            .or_default()
            .record(native, ffprobe);
    }

    fn print(&self) {
        println!(
            "{:<5} {:>5} {:>12} {:>12} {:>8}",
            "type", "n", "scryer_ms", "ffprobe_ms", "ratio"
        );
        for (kind, summary) in &self.by_type {
            let native = summary.native_total.as_secs_f64() / summary.count as f64;
            let ffprobe = summary.ffprobe_total.as_secs_f64() / summary.count as f64;
            println!(
                "{:<5} {:>5} {:>12.2} {:>12.2} {:>8.1}",
                kind,
                summary.count,
                native * 1000.0,
                ffprobe * 1000.0,
                ffprobe / native.max(0.000_001)
            );
        }
    }
}

#[derive(Default)]
struct TypeSummary {
    count: usize,
    native_total: Duration,
    ffprobe_total: Duration,
}

impl TypeSummary {
    fn record(&mut self, native: Duration, ffprobe: Duration) {
        self.count += 1;
        self.native_total += native;
        self.ffprobe_total += ffprobe;
    }
}
