use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use scryer_mediainfo::{AnalysisProfile, AnalyzeOptions, analyze_file, analyze_file_with_options};
use serde_json::Value;

const MAX_DEPTH: usize = 3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Volumes/Media/Movies"));
    let limit = args.next().and_then(|value| value.parse::<usize>().ok());

    let ffprobe = ffprobe_bin().ok_or("ffprobe not found on PATH")?;
    let mut files = discover_representative_files(&root)?;
    if let Some(limit) = limit {
        files.truncate(limit);
    }

    if files.is_empty() {
        return Err(format!(
            "no representative video files found under {}",
            root.display()
        )
        .into());
    }

    println!("Root: {}", root.display());
    println!("Representative files: {}", files.len());
    println!("ffprobe: {}", ffprobe.display());
    println!();

    let mut rich_durations = Vec::with_capacity(files.len());
    let mut parity_durations = Vec::with_capacity(files.len());
    let mut sonarr_ffprobe_durations = Vec::with_capacity(files.len());
    let mut rich_failures = Vec::new();
    let mut parity_failures = Vec::new();
    let mut sonarr_ffprobe_failures = Vec::new();

    for (index, path) in files.iter().enumerate() {
        if index % 25 == 0 {
            println!(
                "Progress: {}/{} ({})",
                index,
                files.len(),
                path.file_name()
                    .unwrap_or_else(|| OsStr::new("?"))
                    .to_string_lossy()
            );
        }

        match index % 3 {
            0 => {
                run_native(path, None, &mut rich_durations, &mut rich_failures);
                run_native(
                    path,
                    Some(AnalysisProfile::FfprobeParity),
                    &mut parity_durations,
                    &mut parity_failures,
                );
                run_sonarr_ffprobe(
                    &ffprobe,
                    path,
                    &mut sonarr_ffprobe_durations,
                    &mut sonarr_ffprobe_failures,
                );
            }
            1 => {
                run_sonarr_ffprobe(
                    &ffprobe,
                    path,
                    &mut sonarr_ffprobe_durations,
                    &mut sonarr_ffprobe_failures,
                );
                run_native(path, None, &mut rich_durations, &mut rich_failures);
                run_native(
                    path,
                    Some(AnalysisProfile::FfprobeParity),
                    &mut parity_durations,
                    &mut parity_failures,
                );
            }
            _ => {
                run_native(
                    path,
                    Some(AnalysisProfile::FfprobeParity),
                    &mut parity_durations,
                    &mut parity_failures,
                );
                run_sonarr_ffprobe(
                    &ffprobe,
                    path,
                    &mut sonarr_ffprobe_durations,
                    &mut sonarr_ffprobe_failures,
                );
                run_native(path, None, &mut rich_durations, &mut rich_failures);
            }
        }
    }

    println!();
    print_summary(
        "scryer_mediainfo::analyze_file [default-rich]",
        &rich_durations,
        &rich_failures,
    );
    println!();
    print_summary(
        "scryer_mediainfo::analyze_file [sonarr-ffprobe-parity]",
        &parity_durations,
        &parity_failures,
    );
    println!();
    print_summary(
        "ffprobe (Sonarr-style)",
        &sonarr_ffprobe_durations,
        &sonarr_ffprobe_failures,
    );

    Ok(())
}

fn run_native(
    path: &Path,
    profile: Option<AnalysisProfile>,
    durations: &mut Vec<(PathBuf, Duration)>,
    failures: &mut Vec<(PathBuf, String)>,
) {
    let start = Instant::now();
    let result = match profile {
        Some(profile) => analyze_file_with_options(path, AnalyzeOptions { profile }),
        None => analyze_file(path),
    };
    match result {
        Ok(_) => durations.push((path.to_path_buf(), start.elapsed())),
        Err(error) => failures.push((path.to_path_buf(), error.to_string())),
    }
}

fn run_sonarr_ffprobe(
    ffprobe: &Path,
    path: &Path,
    durations: &mut Vec<(PathBuf, Duration)>,
    failures: &mut Vec<(PathBuf, String)>,
) {
    let start = Instant::now();
    match sonarr_ffprobe_probe(ffprobe, path) {
        Ok(()) => durations.push((path.to_path_buf(), start.elapsed())),
        Err(error) => failures.push((path.to_path_buf(), error)),
    }
}

fn sonarr_ffprobe_probe(ffprobe: &Path, path: &Path) -> Result<(), String> {
    let mut analysis = run_ffprobe_json(
        ffprobe,
        path,
        &[
            "-show_streams",
            "-show_format",
            "-print_format",
            "json",
            "-probesize",
            "50000000",
        ],
    )?;

    if primary_audio_channel_layout(&analysis).is_none() {
        analysis = run_ffprobe_json(
            ffprobe,
            path,
            &[
                "-show_streams",
                "-show_format",
                "-print_format",
                "json",
                "-probesize",
                "150000000",
                "-analyzeduration",
                "150000000",
            ],
        )?;
    }

    if let Some((video_stream_ordinal, primary_video)) = primary_video_stream(&analysis)
        && primary_video.get("color_transfer").and_then(Value::as_str) == Some("smpte2084")
    {
        let select_stream = format!("v:{video_stream_ordinal}");
        let args = vec![
            "-show_frames".to_owned(),
            "-print_format".to_owned(),
            "json".to_owned(),
            "-read_intervals".to_owned(),
            "%+#1".to_owned(),
            "-select_streams".to_owned(),
            select_stream,
        ];
        let _ = run_ffprobe_json_owned(ffprobe, path, &args)?;
    }

    Ok(())
}

fn run_ffprobe_json(ffprobe: &Path, path: &Path, args: &[&str]) -> Result<Value, String> {
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    run_ffprobe_json_owned(ffprobe, path, &args)
}

fn run_ffprobe_json_owned(ffprobe: &Path, path: &Path, args: &[String]) -> Result<Value, String> {
    let output = Command::new(ffprobe)
        .arg("-v")
        .arg("error")
        .args(args)
        .arg(path)
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("failed to parse ffprobe JSON: {error}"))
}

fn primary_audio_channel_layout(json: &Value) -> Option<&str> {
    json.get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"))
        .and_then(|stream| stream.get("channel_layout"))
        .and_then(Value::as_str)
        .filter(|layout| !layout.is_empty())
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

fn print_summary(label: &str, durations: &[(PathBuf, Duration)], failures: &[(PathBuf, String)]) {
    let mut sorted = durations
        .iter()
        .map(|(path, duration)| (path.clone(), *duration))
        .collect::<Vec<_>>();
    sorted.sort_by_key(|(_, duration)| *duration);

    let total = sorted
        .iter()
        .fold(Duration::ZERO, |acc, (_, duration)| acc + *duration);
    let count = sorted.len();
    let average = if count == 0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(total.as_secs_f64() / count as f64)
    };

    println!("{label}");
    println!("  successes: {}", count);
    println!("  failures: {}", failures.len());
    println!("  total:    {:.3}s", total.as_secs_f64());
    println!("  average:  {:.3}s", average.as_secs_f64());
    if count > 0 {
        println!(
            "  median:   {:.3}s",
            percentile_duration(&sorted, 0.50).as_secs_f64()
        );
        println!(
            "  p95:      {:.3}s",
            percentile_duration(&sorted, 0.95).as_secs_f64()
        );
        println!("  slowest:");
        for (path, duration) in sorted.iter().rev().take(5) {
            println!("    {:.3}s  {}", duration.as_secs_f64(), path.display());
        }
    }
    if !failures.is_empty() {
        println!("  sample failures:");
        for (path, error) in failures.iter().take(5) {
            println!("    {} -> {}", path.display(), error);
        }
    }
}

fn percentile_duration(sorted: &[(PathBuf, Duration)], percentile: f64) -> Duration {
    let index = ((sorted.len().saturating_sub(1) as f64) * percentile).round() as usize;
    sorted[index].1
}

fn discover_representative_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());

    let mut files = Vec::new();
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().to_string();

        if file_type.is_dir() {
            if is_ignored_dir_name(&name) {
                continue;
            }
            if let Some(file) = discover_primary_video_in_dir(&path, 0)? {
                files.push(file);
            }
        } else if file_type.is_file() && is_video_file(&path) && !is_sample_name(&name) {
            files.push(path);
        }
    }
    Ok(files)
}

fn discover_primary_video_in_dir(dir: &Path, depth: usize) -> io::Result<Option<PathBuf>> {
    let mut candidates = Vec::new();
    collect_video_candidates(dir, depth, &mut candidates)?;
    if candidates.is_empty() {
        return Ok(None);
    }

    candidates.sort_by(|a, b| {
        a.is_sample
            .cmp(&b.is_sample)
            .then_with(|| b.size.cmp(&a.size))
            .then_with(|| a.path.cmp(&b.path))
    });

    Ok(candidates
        .into_iter()
        .next()
        .map(|candidate| candidate.path))
}

fn collect_video_candidates(
    dir: &Path,
    depth: usize,
    candidates: &mut Vec<VideoCandidate>,
) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }

    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().to_string();

        if file_type.is_dir() {
            if is_ignored_dir_name(&name) || is_ignored_movie_subdir_name(&name) {
                continue;
            }
            collect_video_candidates(&path, depth + 1, candidates)?;
            continue;
        }

        if file_type.is_file() && is_video_file(&path) {
            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            candidates.push(VideoCandidate {
                path,
                size,
                is_sample: is_sample_name(&name),
            });
        }
    }

    Ok(())
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

fn is_video_file(path: &Path) -> bool {
    scryer_media_types::formats::media_format_for_path(path)
        .is_some_and(|entry| entry.handling == scryer_media_types::formats::MediaHandling::Native)
}

fn is_ignored_dir_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with('.') || matches!(lower.as_str(), "@eadir" | ".@__thumb" | "plex versions")
}

fn is_ignored_movie_subdir_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "extras"
            | "extrafanart"
            | "behind the scenes"
            | "deleted scenes"
            | "featurettes"
            | "interviews"
            | "other"
            | "scenes"
            | "samples"
            | "shorts"
            | "trailers"
    )
}

fn is_sample_name(name: &str) -> bool {
    name.to_ascii_lowercase().contains("sample")
}

struct VideoCandidate {
    path: PathBuf,
    size: u64,
    is_sample: bool,
}
