use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use scryer_mediainfo::{AnalysisProfile, AnalyzeOptions, MediaAnalysis, analyze_file_with_options};
use serde_json::{Value, json};

const MAX_DEPTH: usize = 16;
const PROGRESS_INTERVAL: usize = 25;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::parse(env::args().skip(1))?;
    let ffprobe = config
        .ffprobe
        .clone()
        .or_else(ffprobe_bin)
        .ok_or("ffprobe not found; pass --ffprobe <path> or set FFPROBE")?;

    let report_path = config.report.unwrap_or_else(default_report_path);
    if let Some(parent) = report_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut report = BufWriter::new(File::create(&report_path)?);
    let mut summary = Summary::default();

    println!("Root: {}", config.root.display());
    if config.files.is_empty() {
        println!("Files: streaming discovery");
    } else {
        println!("Files: explicit list ({})", config.files.len());
    }
    println!("ffprobe: {}", ffprobe.display());
    println!("Report: {}", report_path.display());

    let mut processed = 0;
    if config.files.is_empty() {
        walk_media_files(
            &config.root,
            0,
            config.limit,
            &mut processed,
            &mut |index, path| {
                process_path(
                    &mut report,
                    &mut summary,
                    &ffprobe,
                    index,
                    path,
                    config.sonarr_channel_layout_retry,
                )
            },
        )?;
    } else {
        for (index, path) in config
            .files
            .iter()
            .take(config.limit.unwrap_or(usize::MAX))
            .enumerate()
        {
            process_path(
                &mut report,
                &mut summary,
                &ffprobe,
                index,
                path,
                config.sonarr_channel_layout_retry,
            )?;
            processed += 1;
        }
    }
    report.flush()?;

    println!("Progress: {} done", processed);
    println!();
    summary.print();

    if summary.has_failures() {
        std::process::exit(1);
    }

    Ok(())
}

struct Config {
    root: PathBuf,
    files: Vec<PathBuf>,
    limit: Option<usize>,
    report: Option<PathBuf>,
    ffprobe: Option<PathBuf>,
    sonarr_channel_layout_retry: bool,
}

impl Config {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut root = None;
        let mut limit = None;
        let mut report = None;
        let mut ffprobe = None;
        let mut files = Vec::new();
        let mut sonarr_channel_layout_retry = false;
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--limit" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--limit requires a value".to_owned())?;
                    limit = Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| format!("invalid --limit value: {value}"))?,
                    );
                }
                "--report" => {
                    report = Some(PathBuf::from(
                        args.next()
                            .ok_or_else(|| "--report requires a path".to_owned())?,
                    ));
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
                "--sonarr-channel-layout-retry" => {
                    sonarr_channel_layout_retry = true;
                }
                "--help" | "-h" => {
                    return Err(
                        "usage: ffprobe_corpus_parity [root] [--limit n] [--report path] [--ffprobe path] [--file path...] [--sonarr-channel-layout-retry]"
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
            files,
            limit,
            report,
            ffprobe,
            sonarr_channel_layout_retry,
        })
    }
}

fn process_path<W: Write>(
    report: &mut W,
    summary: &mut Summary,
    ffprobe: &Path,
    index: usize,
    path: &Path,
    sonarr_channel_layout_retry: bool,
) -> io::Result<()> {
    if index == 0 || index.is_multiple_of(PROGRESS_INTERVAL) {
        println!("Progress: {} {}", index, path.display());
    }

    write_started(report, index, path)?;
    let outcome = compare_file(ffprobe, path, sonarr_channel_layout_retry);
    summary.record(&outcome);
    write_outcome(report, index, path, &outcome)
}

#[derive(Default)]
struct Summary {
    files: usize,
    passed: usize,
    fast_ok: usize,
    rich_needed: usize,
    rich_ok: usize,
    mismatched_files: usize,
    mismatch_count: usize,
    native_failures: usize,
    ffprobe_failures: usize,
    mismatch_fields: BTreeMap<String, usize>,
}

impl Summary {
    fn record(&mut self, outcome: &FileOutcome) {
        self.files += 1;
        let stage = outcome.stage();
        if stage.fast_ok {
            self.fast_ok += 1;
        }
        if stage.rich_needed {
            self.rich_needed += 1;
        }
        if stage.rich_ok {
            self.rich_ok += 1;
        }
        match outcome {
            FileOutcome::Ok(_) => self.passed += 1,
            FileOutcome::NativeError(_, _) => self.native_failures += 1,
            FileOutcome::FfprobeError(_, _) => self.ffprobe_failures += 1,
            FileOutcome::Mismatch(_, mismatches) => {
                self.mismatched_files += 1;
                self.mismatch_count += mismatches.len();
                for mismatch in mismatches {
                    *self
                        .mismatch_fields
                        .entry(mismatch.field.clone())
                        .or_default() += 1;
                }
            }
        }
    }

    fn has_failures(&self) -> bool {
        self.mismatched_files > 0 || self.native_failures > 0 || self.ffprobe_failures > 0
    }

    fn print(&self) {
        println!("Summary");
        println!("  files:            {}", self.files);
        println!("  passed:           {}", self.passed);
        println!("  fast ok:          {}", self.fast_ok);
        println!("  rich needed:      {}", self.rich_needed);
        println!("  rich ok:          {}", self.rich_ok);
        println!("  mismatched files: {}", self.mismatched_files);
        println!("  mismatches:       {}", self.mismatch_count);
        println!("  native failures:  {}", self.native_failures);
        println!("  ffprobe failures: {}", self.ffprobe_failures);

        if !self.mismatch_fields.is_empty() {
            println!("  mismatch fields:");
            for (field, count) in &self.mismatch_fields {
                println!("    {field}: {count}");
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct StageStatus {
    fast_ok: bool,
    rich_needed: bool,
    rich_ok: bool,
}

enum FileOutcome {
    Ok(StageStatus),
    NativeError(StageStatus, String),
    FfprobeError(StageStatus, String),
    Mismatch(StageStatus, Vec<Mismatch>),
}

impl FileOutcome {
    fn stage(&self) -> StageStatus {
        match self {
            FileOutcome::Ok(stage)
            | FileOutcome::NativeError(stage, _)
            | FileOutcome::FfprobeError(stage, _)
            | FileOutcome::Mismatch(stage, _) => *stage,
        }
    }
}

struct Mismatch {
    field: String,
    native: String,
    ffprobe: String,
}

fn compare_file(ffprobe: &Path, path: &Path, sonarr_channel_layout_retry: bool) -> FileOutcome {
    let fast = match analyze_file_with_options(
        path,
        AnalyzeOptions {
            profile: AnalysisProfile::Fast,
        },
    ) {
        Ok(analysis) => analysis,
        Err(fast_error) => {
            let stage = StageStatus {
                fast_ok: false,
                rich_needed: true,
                rich_ok: false,
            };
            return match analyze_file_with_options(
                path,
                AnalyzeOptions {
                    profile: AnalysisProfile::FfprobeParity,
                },
            ) {
                Ok(analysis) => compare_file_with_analysis(
                    ffprobe,
                    path,
                    sonarr_channel_layout_retry,
                    analysis,
                    StageStatus {
                        rich_ok: true,
                        ..stage
                    },
                ),
                Err(rich_error) => FileOutcome::NativeError(
                    stage,
                    format!("fast: {fast_error}; rich: {rich_error}"),
                ),
            };
        }
    };

    let fast_ok = fast_analysis_has_adequate_facts(&fast);
    let rich_needed = !fast_ok;
    let mut stage = StageStatus {
        fast_ok,
        rich_needed,
        rich_ok: !rich_needed,
    };
    let analysis = if rich_needed {
        match analyze_file_with_options(
            path,
            AnalyzeOptions {
                profile: AnalysisProfile::FfprobeParity,
            },
        ) {
            Ok(analysis) => {
                stage.rich_ok = true;
                analysis
            }
            Err(error) => return FileOutcome::NativeError(stage, error.to_string()),
        }
    } else {
        fast
    };

    compare_file_with_analysis(ffprobe, path, sonarr_channel_layout_retry, analysis, stage)
}

fn compare_file_with_analysis(
    ffprobe: &Path,
    path: &Path,
    sonarr_channel_layout_retry: bool,
    analysis: MediaAnalysis,
    stage: StageStatus,
) -> FileOutcome {
    let ffprobe = match sonarr_ffprobe_json(ffprobe, path, sonarr_channel_layout_retry) {
        Ok(json) => json,
        Err(error) => return FileOutcome::FfprobeError(stage, error),
    };

    compare_analysis_to_ffprobe(analysis, ffprobe, stage)
}

fn compare_analysis_to_ffprobe(
    analysis: MediaAnalysis,
    ffprobe: Value,
    stage: StageStatus,
) -> FileOutcome {
    let video = ffprobe_primary_video_stream(&ffprobe);
    let audio = ffprobe_primary_stream(&ffprobe, "audio");
    let container_format = analysis.container_format.as_deref();
    let mut mismatches = Vec::new();

    compare_option(
        &mut mismatches,
        "video_codec",
        &analysis.video_codec,
        video
            .and_then(|stream| stream.get("codec_name"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    );
    compare_option(
        &mut mismatches,
        "audio_codec",
        &analysis.audio_codec,
        audio
            .and_then(|stream| stream.get("codec_name"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    );
    compare_option(
        &mut mismatches,
        "video_width",
        &analysis.video_width,
        ffprobe_optional_i32(video, "width"),
    );
    compare_option(
        &mut mismatches,
        "video_height",
        &analysis.video_height,
        ffprobe_optional_i32(video, "height"),
    );
    compare_option(
        &mut mismatches,
        "audio_channels",
        &analysis.audio_channels,
        ffprobe_optional_i32(audio, "channels"),
    );

    compare_bitrate(
        &mut mismatches,
        "video_bitrate_kbps",
        analysis.video_bitrate_kbps,
        ffprobe_bitrate_kbps(video),
    );
    compare_bitrate(
        &mut mismatches,
        "audio_bitrate_kbps",
        analysis.audio_bitrate_kbps,
        ffprobe_bitrate_kbps(audio),
    );
    compare_frame_rate(
        &mut mismatches,
        analysis
            .video_frame_rate
            .as_deref()
            .and_then(|fps| fps.parse::<f64>().ok()),
        ffprobe_frame_rate(video),
    );
    compare_i32(
        &mut mismatches,
        "duration_seconds",
        analysis.duration_seconds.unwrap_or_default(),
        ffprobe_sonarr_duration_seconds(&ffprobe).unwrap_or_default(),
        1,
    );
    let native_audio_languages = analysis.audio_languages.clone();
    compare_vec(
        &mut mismatches,
        "audio_languages",
        &native_audio_languages,
        ffprobe_languages_for_compare(
            container_format,
            &native_audio_languages,
            ffprobe_languages(&ffprobe, "audio"),
        ),
    );

    let native_subtitle_languages = analysis.subtitle_languages.clone();
    compare_vec(
        &mut mismatches,
        "subtitle_languages",
        &native_subtitle_languages,
        ffprobe_languages_for_compare(
            container_format,
            &native_subtitle_languages,
            ffprobe_languages(&ffprobe, "subtitle"),
        ),
    );

    compare_vec(
        &mut mismatches,
        "subtitle_codecs",
        &analysis.subtitle_codecs,
        ffprobe_subtitle_codecs(&ffprobe),
    );

    let probe_audio_streams = ffprobe_streams(&ffprobe, "audio");
    compare_i32(
        &mut mismatches,
        "audio_stream_count",
        analysis.audio_streams.len() as i32,
        probe_audio_streams.len() as i32,
        0,
    );
    for (index, (native, probe)) in analysis
        .audio_streams
        .iter()
        .zip(probe_audio_streams.iter())
        .enumerate()
    {
        let probe_language = ffprobe_language_for_compare(
            container_format,
            native.language.as_deref(),
            probe
                .get("tags")
                .and_then(|tags| tags.get("language"))
                .and_then(Value::as_str),
        );

        compare_option(
            &mut mismatches,
            &format!("audio_stream[{index}].codec"),
            &native.codec,
            probe
                .get("codec_name")
                .and_then(Value::as_str)
                .map(str::to_owned),
        );
        compare_option(
            &mut mismatches,
            &format!("audio_stream[{index}].channels"),
            &native.channels,
            ffprobe_optional_i32(Some(probe), "channels"),
        );
        compare_option(
            &mut mismatches,
            &format!("audio_stream[{index}].language"),
            &native.language,
            probe_language,
        );
        compare_bitrate(
            &mut mismatches,
            &format!("audio_stream[{index}].bitrate_kbps"),
            native.bitrate_kbps,
            ffprobe_bitrate_kbps(Some(probe)),
        );
    }

    let probe_subtitle_streams = ffprobe_streams(&ffprobe, "subtitle");
    compare_i32(
        &mut mismatches,
        "subtitle_stream_count",
        analysis.subtitle_streams.len() as i32,
        probe_subtitle_streams.len() as i32,
        0,
    );
    for (index, (native, probe)) in analysis
        .subtitle_streams
        .iter()
        .zip(probe_subtitle_streams.iter())
        .enumerate()
    {
        let probe_language = ffprobe_language_for_compare(
            container_format,
            native.language.as_deref(),
            probe
                .get("tags")
                .and_then(|tags| tags.get("language"))
                .and_then(Value::as_str),
        );
        let disposition = probe.get("disposition");
        let probe_forced = disposition
            .and_then(|disp| disp.get("forced"))
            .and_then(Value::as_i64)
            .unwrap_or_default()
            != 0;
        let probe_default = disposition
            .and_then(|disp| disp.get("default"))
            .and_then(Value::as_i64)
            .unwrap_or_default()
            != 0;

        compare_option(
            &mut mismatches,
            &format!("subtitle_stream[{index}].codec"),
            &native.codec,
            probe
                .get("codec_name")
                .and_then(Value::as_str)
                .map(str::to_owned),
        );
        compare_option(
            &mut mismatches,
            &format!("subtitle_stream[{index}].language"),
            &native.language,
            probe_language,
        );
        compare_bool(
            &mut mismatches,
            &format!("subtitle_stream[{index}].forced"),
            native.forced,
            probe_forced,
        );
        compare_bool(
            &mut mismatches,
            &format!("subtitle_stream[{index}].default"),
            native.default,
            probe_default,
        );
    }

    if mismatches.is_empty() {
        FileOutcome::Ok(stage)
    } else {
        FileOutcome::Mismatch(stage, mismatches)
    }
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

fn compare_option<T: std::fmt::Debug + PartialEq>(
    mismatches: &mut Vec<Mismatch>,
    field: &str,
    native: &Option<T>,
    ffprobe: Option<T>,
) {
    if *native != ffprobe {
        mismatches.push(Mismatch {
            field: field.to_owned(),
            native: format!("{native:?}"),
            ffprobe: format!("{ffprobe:?}"),
        });
    }
}

fn compare_vec<T: std::fmt::Debug + PartialEq>(
    mismatches: &mut Vec<Mismatch>,
    field: &str,
    native: &[T],
    ffprobe: Vec<T>,
) {
    if native != ffprobe {
        mismatches.push(Mismatch {
            field: field.to_owned(),
            native: format!("{native:?}"),
            ffprobe: format!("{ffprobe:?}"),
        });
    }
}

fn compare_bitrate(
    mismatches: &mut Vec<Mismatch>,
    field: &str,
    native: Option<i32>,
    ffprobe: Option<i32>,
) {
    if let (Some(native), Some(ffprobe)) = (native, ffprobe)
        && (native - ffprobe).abs() > 16
    {
        mismatches.push(Mismatch {
            field: field.to_owned(),
            native: native.to_string(),
            ffprobe: ffprobe.to_string(),
        });
    }
}

fn compare_frame_rate(mismatches: &mut Vec<Mismatch>, native: Option<f64>, ffprobe: Option<f64>) {
    if let (Some(native), Some(ffprobe)) = (native, ffprobe)
        && (native - ffprobe).abs() > 0.05
    {
        mismatches.push(Mismatch {
            field: "video_frame_rate".to_owned(),
            native: format!("{native:.3}"),
            ffprobe: format!("{ffprobe:.3}"),
        });
    }
}

fn compare_i32(
    mismatches: &mut Vec<Mismatch>,
    field: &str,
    native: i32,
    ffprobe: i32,
    tolerance: i32,
) {
    if (native - ffprobe).abs() > tolerance {
        mismatches.push(Mismatch {
            field: field.to_owned(),
            native: native.to_string(),
            ffprobe: ffprobe.to_string(),
        });
    }
}

fn compare_bool(mismatches: &mut Vec<Mismatch>, field: &str, native: bool, ffprobe: bool) {
    if native != ffprobe {
        mismatches.push(Mismatch {
            field: field.to_owned(),
            native: native.to_string(),
            ffprobe: ffprobe.to_string(),
        });
    }
}

fn write_started<W: Write>(writer: &mut W, index: usize, path: &Path) -> io::Result<()> {
    writeln!(
        writer,
        "{}",
        json!({
            "index": index,
            "path": path,
            "status": "started",
        })
    )?;
    writer.flush()
}

fn write_outcome<W: Write>(
    writer: &mut W,
    index: usize,
    path: &Path,
    outcome: &FileOutcome,
) -> io::Result<()> {
    let stage = outcome.stage();
    let value = match outcome {
        FileOutcome::Ok(_) => json!({
            "index": index,
            "path": path,
            "status": "ok",
            "fast_ok": stage.fast_ok,
            "rich_needed": stage.rich_needed,
            "rich_ok": stage.rich_ok,
        }),
        FileOutcome::NativeError(_, error) => json!({
            "index": index,
            "path": path,
            "status": "native_error",
            "fast_ok": stage.fast_ok,
            "rich_needed": stage.rich_needed,
            "rich_ok": stage.rich_ok,
            "error": error,
        }),
        FileOutcome::FfprobeError(_, error) => json!({
            "index": index,
            "path": path,
            "status": "ffprobe_error",
            "fast_ok": stage.fast_ok,
            "rich_needed": stage.rich_needed,
            "rich_ok": stage.rich_ok,
            "error": error,
        }),
        FileOutcome::Mismatch(_, mismatches) => json!({
            "index": index,
            "path": path,
            "status": "mismatch",
            "fast_ok": stage.fast_ok,
            "rich_needed": stage.rich_needed,
            "rich_ok": stage.rich_ok,
            "mismatches": mismatches.iter().map(|mismatch| {
                json!({
                    "field": mismatch.field,
                    "native": mismatch.native,
                    "ffprobe": mismatch.ffprobe,
                })
            }).collect::<Vec<_>>(),
        }),
    };
    writeln!(writer, "{value}")?;
    writer.flush()
}

fn sonarr_ffprobe_json(
    ffprobe: &Path,
    file: &Path,
    retry_missing_channel_layout: bool,
) -> Result<Value, String> {
    let json = run_sonarr_analysis_json(ffprobe, file, &["-probesize", "50000000"])?;

    if retry_missing_channel_layout && primary_audio_channel_layout_missing(&json) {
        return run_sonarr_analysis_json(
            ffprobe,
            file,
            &["-probesize", "150000000", "-analyzeduration", "150000000"],
        );
    }

    Ok(json)
}

fn primary_audio_channel_layout_missing(json: &Value) -> bool {
    ffprobe_primary_stream(json, "audio")
        .and_then(|stream| stream.get("channel_layout"))
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::is_empty)
        .unwrap_or(true)
}

fn run_sonarr_analysis_json(
    ffprobe: &Path,
    file: &Path,
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
        .args(custom_args)
        .arg("-show_chapters")
        .arg(file)
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("failed to parse ffprobe JSON: {error}"))
}

fn ffprobe_primary_stream<'a>(json: &'a Value, codec_type: &str) -> Option<&'a Value> {
    json.get("streams")?
        .as_array()?
        .iter()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some(codec_type))
}

fn ffprobe_primary_video_stream(json: &Value) -> Option<&Value> {
    let video_streams = ffprobe_streams(json, "video");
    if video_streams.len() <= 1 {
        return video_streams.into_iter().next();
    }

    video_streams
        .iter()
        .copied()
        .find(|stream| {
            !matches!(
                stream.get("codec_name").and_then(Value::as_str),
                Some("mjpeg" | "png")
            )
        })
        .or_else(|| video_streams.into_iter().next())
}

fn ffprobe_streams<'a>(json: &'a Value, codec_type: &str) -> Vec<&'a Value> {
    json.get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some(codec_type))
        .collect()
}

fn ffprobe_languages(json: &Value, codec_type: &str) -> Vec<String> {
    json.get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some(codec_type))
        .filter_map(|stream| {
            stream
                .get("tags")
                .and_then(|tags| tags.get("language"))
                .and_then(Value::as_str)
                .filter(|lang| !lang.is_empty() && *lang != "und")
                .map(str::to_owned)
        })
        .collect()
}

fn ffprobe_subtitle_codecs(json: &Value) -> Vec<String> {
    json.get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("subtitle"))
        .filter_map(|stream| {
            stream
                .get("codec_name")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

fn ffprobe_languages_for_compare(
    container_format: Option<&str>,
    native_languages: &[String],
    ffprobe_languages: Vec<String>,
) -> Vec<String> {
    if native_languages.is_empty()
        && matches!(container_format, Some("matroska") | Some("webm"))
        && ffprobe_languages.iter().all(|lang| lang == "eng")
    {
        Vec::new()
    } else {
        ffprobe_languages
    }
}

fn ffprobe_language_for_compare(
    container_format: Option<&str>,
    native_language: Option<&str>,
    ffprobe_language: Option<&str>,
) -> Option<String> {
    if native_language.is_none()
        && matches!(container_format, Some("matroska") | Some("webm"))
        && ffprobe_language == Some("eng")
    {
        return None;
    }

    ffprobe_language
        .filter(|lang| !lang.is_empty() && *lang != "und")
        .map(str::to_owned)
}

fn ffprobe_optional_i32(value: Option<&Value>, key: &str) -> Option<i32> {
    value
        .and_then(|stream| stream.get(key))
        .and_then(Value::as_i64)
        .map(|value| value as i32)
}

fn ffprobe_bitrate_kbps(value: Option<&Value>) -> Option<i32> {
    value
        .and_then(|stream| stream.get("bit_rate"))
        .and_then(Value::as_str)
        .and_then(|bitrate| bitrate.parse::<i64>().ok())
        .map(|bitrate| (bitrate / 1000) as i32)
}

fn ffprobe_frame_rate(stream: Option<&Value>) -> Option<f64> {
    let rate = stream
        .and_then(|stream| stream.get("avg_frame_rate"))
        .and_then(Value::as_str)
        .filter(|rate| *rate != "0/0")
        .or_else(|| {
            stream
                .and_then(|stream| stream.get("r_frame_rate"))
                .and_then(Value::as_str)
                .filter(|rate| *rate != "0/0")
        })?;

    let (num, den) = rate.split_once('/')?;
    let num = num.parse::<f64>().ok()?;
    let den = den.parse::<f64>().ok()?;
    if den == 0.0 { None } else { Some(num / den) }
}

fn ffprobe_sonarr_duration_seconds(json: &Value) -> Option<i32> {
    let audio = ffprobe_primary_stream(json, "audio").and_then(ffprobe_stream_duration);
    let video = ffprobe_primary_video_stream(json).and_then(ffprobe_stream_duration);
    let format = json
        .get("format")
        .and_then(|format| format.get("duration"))
        .and_then(Value::as_str)
        .and_then(parse_ffprobe_duration)?;

    Some(best_sonarr_runtime(audio, video, format).round() as i32)
}

fn ffprobe_stream_duration(stream: &Value) -> Option<f64> {
    stream
        .get("duration")
        .and_then(Value::as_str)
        .and_then(parse_ffprobe_duration)
        .filter(|duration| *duration > 0.0)
}

fn best_sonarr_runtime(audio: Option<f64>, video: Option<f64>, format: f64) -> f64 {
    if video.unwrap_or_default() == 0.0 {
        if audio.unwrap_or_default() == 0.0 {
            format
        } else {
            audio.unwrap_or(format)
        }
    } else {
        video.unwrap_or(format)
    }
}

fn parse_ffprobe_duration(duration: &str) -> Option<f64> {
    if let Ok(seconds) = duration.parse::<f64>() {
        return Some(seconds);
    }

    let (hours, rest) = duration.split_once(':')?;
    let (minutes, seconds) = rest.split_once(':')?;
    Some(
        hours.parse::<f64>().ok()? * 3600.0
            + minutes.parse::<f64>().ok()? * 60.0
            + seconds.parse::<f64>().ok()?,
    )
}

fn walk_media_files(
    root: &Path,
    depth: usize,
    limit: Option<usize>,
    processed: &mut usize,
    on_file: &mut impl FnMut(usize, &Path) -> io::Result<()>,
) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    if limit.is_some_and(|limit| *processed >= limit) {
        return Ok(());
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Ok(()),
        Err(error) => return Err(error),
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => continue,
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
                walk_media_files(&path, depth + 1, limit, processed, on_file)?;
            }
        } else if file_type.is_file() && is_media_file(&path) {
            if limit.is_some_and(|limit| *processed >= limit) {
                break;
            }
            let index = *processed;
            *processed += 1;
            on_file(index, &path)?;
        }
    }

    Ok(())
}

fn is_media_file(path: &Path) -> bool {
    scryer_media_types::formats::media_format_for_path(path)
        .is_some_and(|entry| entry.handling == scryer_media_types::formats::MediaHandling::Native)
}

fn is_ignored_dir_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with('.')
        || matches!(
            lower.as_str(),
            "@eadir" | ".@__thumb" | "plex" | "plex versions" | "plex media server"
        )
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

fn default_report_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("ffprobe-corpus-parity.jsonl")
}
