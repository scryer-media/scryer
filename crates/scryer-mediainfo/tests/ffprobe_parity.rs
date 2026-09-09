use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

#[path = "support/analysis_parity.rs"]
mod analysis_parity;
#[path = "support/sparse_mkv.rs"]
mod sparse_mkv;

fn media_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("media")
}

fn is_media_fixture(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(
            "avi"
                | "mkv"
                | "webm"
                | "mp4"
                | "m4v"
                | "mov"
                | "ts"
                | "m2ts"
                | "wmv"
                | "ogv"
                | "flv"
                | "mpg"
                | "mpeg"
                | "vob"
        )
    )
}

fn ffprobe_bin() -> Option<PathBuf> {
    let candidates = ["ffprobe", "/opt/homebrew/bin/ffprobe"];
    for candidate in candidates {
        let Ok(output) = Command::new(candidate).arg("-version").output() else {
            continue;
        };
        if output.status.success() {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

fn sonarr_ffprobe_json(ffprobe: &Path, file: &Path) -> Value {
    let mut analysis = run_ffprobe_json(
        ffprobe,
        file,
        &[
            "-show_streams",
            "-show_format",
            "-show_chapters",
            "-show_programs",
            "-print_format",
            "json",
            "-probesize",
            "50000000",
        ],
    );

    if primary_audio_channel_layout(&analysis).is_none() {
        analysis = run_ffprobe_json(
            ffprobe,
            file,
            &[
                "-show_streams",
                "-show_format",
                "-show_chapters",
                "-show_programs",
                "-print_format",
                "json",
                "-probesize",
                "150000000",
                "-analyzeduration",
                "150000000",
            ],
        );
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
            "%+#16".to_owned(),
            "-select_streams".to_owned(),
            select_stream,
        ];
        let sampled = run_ffprobe_json_owned(ffprobe, file, &args);
        analysis["sampled_frames"] = sampled["frames"].clone();
    }

    if analysis["format"]["format_name"] == "avi" {
        let packet_report = run_ffprobe_json(
            ffprobe,
            file,
            &[
                "-select_streams",
                "v",
                "-show_packets",
                "-show_entries",
                "packet=stream_index,pts,dts,duration,size",
                "-read_intervals",
                "%+#4097",
                "-print_format",
                "json",
            ],
        );
        let packets = &packet_report["packets"];
        if let Some(streams) = analysis["streams"].as_array_mut() {
            for stream in streams {
                if let Some(accounting) =
                    analysis_parity::complete_video_packet_accounting(stream, packets)
                {
                    stream["reference_packet_accounting"] = accounting;
                }
            }
        }
        analysis["reference_video_packets"] = packets.clone();
    }

    analysis
}

fn run_ffprobe_json(ffprobe: &Path, file: &Path, args: &[&str]) -> Value {
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    run_ffprobe_json_owned(ffprobe, file, &args)
}

fn run_ffprobe_json_owned(ffprobe: &Path, file: &Path, args: &[String]) -> Value {
    let output = Command::new(ffprobe)
        .arg("-v")
        .arg("error")
        .args(args)
        .arg(file)
        .output()
        .expect("ffprobe should run");
    assert!(
        output.status.success(),
        "ffprobe failed for {}: {}",
        file.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("ffprobe JSON should parse")
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
        .enumerate()
        .find(|(_, stream)| {
            stream["disposition"]["attached_pic"] != 1
                && stream["disposition"]["still_image"] != 1
                && (stream["disposition"]["attached_pic"] == 0
                    || !matches!(
                        stream.get("codec_name").and_then(Value::as_str),
                        Some("mjpeg" | "png")
                    ))
        })
}

fn ffprobe_primary_stream<'a>(json: &'a Value, codec_type: &str) -> Option<&'a Value> {
    if codec_type == "video" {
        return primary_video_stream(json).map(|(_, stream)| stream);
    }
    let candidates = ffprobe_streams(json, codec_type);
    if codec_type != "audio" {
        return candidates.into_iter().next();
    }
    candidates
        .into_iter()
        .enumerate()
        .filter(|(_, stream)| stream["disposition"]["comment"] != 1)
        .max_by_key(|(ordinal, stream)| {
            (
                reference_audio_rank(stream),
                stream["channels"].as_u64().unwrap_or(0),
                std::cmp::Reverse(*ordinal),
            )
        })
        .map(|(_, stream)| stream)
}

fn reference_audio_rank(stream: &Value) -> i32 {
    match (
        stream["codec_name"].as_str().unwrap_or_default(),
        stream["profile"].as_str().unwrap_or_default(),
    ) {
        ("truehd", profile) if profile.contains("Atmos") => 100,
        ("dts", profile) if profile.contains("DTS:X") => 95,
        ("truehd", _) => 90,
        ("dts", "DTS-HD MA") => 85,
        ("flac", _) => 80,
        ("eac3", profile) if profile.contains("Atmos") => 75,
        ("eac3", _) => 70,
        ("dts", "DTS-HD HRA") => 65,
        ("dts", _) => 60,
        ("ac3", _) => 50,
        ("aac" | "opus", _) => 40,
        ("mp3" | "vorbis", _) => 30,
        (codec, _) if codec.starts_with("pcm_") => 20,
        _ => 10,
    }
}

fn belongs_to_selected_program(json: &Value, stream: &Value) -> bool {
    let Some((_, video)) = primary_video_stream(json) else {
        return true;
    };
    let program = json["programs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|program| {
            program["streams"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|member| member["index"] == video["index"])
        });
    program.is_none_or(|program| {
        program["streams"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|member| member["index"] == stream["index"])
    })
}

fn ffprobe_languages(json: &Value, codec_type: &str) -> Vec<String> {
    json.get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some(codec_type))
        .filter(|stream| belongs_to_selected_program(json, stream))
        .filter(|stream| codec_type != "audio" || stream["disposition"]["comment"] != 1)
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
        .filter(|stream| belongs_to_selected_program(json, stream))
        .filter_map(|stream| {
            stream
                .get("codec_name")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

fn ffprobe_streams<'a>(json: &'a Value, codec_type: &str) -> Vec<&'a Value> {
    json.get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some(codec_type))
        .filter(|stream| belongs_to_selected_program(json, stream))
        .collect()
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

fn ffprobe_optional_i32(value: Option<&Value>, key: &str) -> Option<i32> {
    value
        .and_then(|stream| stream.get(key))
        .and_then(Value::as_i64)
        .map(|value| value as i32)
}

fn ffprobe_bitrate_kbps(value: Option<&Value>) -> Option<i32> {
    value
        .and_then(analysis_parity::reference_bitrate_bps)
        .map(|bitrate| (bitrate / 1000.0) as i32)
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

#[test]
#[ignore = "dev-only parity harness; run manually when auditing native probe drift"]
fn compare_fixture_corpus_against_ffprobe() {
    let ffprobe = ffprobe_bin().expect("ffprobe must be installed for parity checks");
    let version = Command::new(&ffprobe)
        .arg("-version")
        .output()
        .expect("read reference version");
    eprintln!(
        "Reference: {}",
        String::from_utf8_lossy(&version.stdout)
            .lines()
            .next()
            .unwrap_or("unknown")
    );
    let requested = std::env::var("SCRYER_PARITY_FIXTURES")
        .ok()
        .filter(|names| !names.trim().is_empty())
        .map(|names| {
            names
                .split(',')
                .map(|name| name.trim().to_owned())
                .collect::<Vec<_>>()
        });
    let report_path = std::env::var_os("SCRYER_PARITY_REPORT").map(PathBuf::from);
    let mut records = Vec::new();
    let mut seen = Vec::new();
    let mut mismatches = Vec::new();

    let mut paths: Vec<_> = std::fs::read_dir(media_dir())
        .expect("media dir should exist")
        .map(|entry| entry.expect("fixture entry").path())
        .collect();
    paths.sort();
    for path in paths {
        if !path.is_file() {
            continue;
        }
        if !is_media_fixture(&path) {
            continue;
        }
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
        if requested
            .as_ref()
            .is_some_and(|names| !names.contains(&file_name))
        {
            continue;
        }
        seen.push(file_name.clone());
        let analysis = scryer_mediainfo::analyze_catalog_file(&path)
            .expect("canonical native analysis should succeed");
        let raw_reference = sonarr_ffprobe_json(&ffprobe, &path);
        let native_json = serde_json::to_value(&analysis).unwrap();
        let (ffprobe, reference_assumptions, expectation_errors) =
            if sparse_mkv::NAMES.contains(&file_name.as_str()) {
                assert!(
                    std::fs::metadata(&path).unwrap().len() <= 1024,
                    "authored sparse fixture grew; audit required"
                );
                sparse_mkv::comparison_view(
                    &file_name,
                    &std::fs::read(&path).unwrap(),
                    &raw_reference,
                    &native_json,
                )
                .expect("sparse fixture must still match its independently authored byte structure")
            } else {
                (raw_reference.clone(), Vec::new(), Vec::new())
            };
        mismatches.extend(
            expectation_errors
                .into_iter()
                .map(|error| format!("{file_name}: {error}")),
        );
        mismatches.extend(
            analysis_parity::compare(&native_json, &ffprobe)
                .into_iter()
                .map(|difference| format!("{} {difference}", path.display())),
        );
        if report_path.is_some() {
            records.push(
                serde_json::json!({"fixture": file_name, "native": native_json,
                "reference": raw_reference, "comparison_reference": ffprobe.clone(),
                "reference_assumptions": reference_assumptions}),
            );
        }

        let video = ffprobe_primary_stream(&ffprobe, "video");
        let audio = ffprobe_primary_stream(&ffprobe, "audio");
        let container_format = analysis.container_format.as_deref();

        let native_duration = analysis.duration_seconds.unwrap_or_default();
        let ffprobe_duration = ffprobe
            .get("format")
            .and_then(|format| format.get("duration"))
            .and_then(Value::as_str)
            .and_then(|duration| duration.parse::<f64>().ok())
            .map(|duration| duration.round() as i32)
            .unwrap_or_default();
        let native_num_chapters = analysis.num_chapters.unwrap_or_default();
        let ffprobe_num_chapters = ffprobe
            .get("chapters")
            .and_then(Value::as_array)
            .map(|chapters| chapters.len() as i32)
            .unwrap_or_default();

        let checks = [
            (
                "video_codec",
                analysis.video_codec.clone(),
                video
                    .and_then(|stream| stream.get("codec_name"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            ),
            (
                "audio_codec",
                analysis.audio_codec.clone(),
                audio
                    .and_then(|stream| stream.get("codec_name"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            ),
        ];

        for (field, native, probe) in checks {
            if native != probe {
                mismatches.push(format!(
                    "{} {} mismatch: native={:?} ffprobe={:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    field,
                    native,
                    probe
                ));
            }
        }

        let numeric_checks = [
            (
                "video_width",
                analysis.video_width,
                video
                    .and_then(|stream| stream.get("width"))
                    .and_then(Value::as_i64)
                    .map(|value| value as i32),
            ),
            (
                "video_height",
                analysis.video_height,
                video
                    .and_then(|stream| stream.get("height"))
                    .and_then(Value::as_i64)
                    .map(|value| value as i32),
            ),
            (
                "audio_channels",
                analysis.audio_channels,
                audio
                    .and_then(|stream| stream.get("channels"))
                    .and_then(Value::as_i64)
                    .map(|value| value as i32),
            ),
        ];

        for (field, native, probe) in numeric_checks {
            if native != probe {
                mismatches.push(format!(
                    "{} {} mismatch: native={:?} ffprobe={:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    field,
                    native,
                    probe
                ));
            }
        }

        let video_bitrate_probe = ffprobe_bitrate_kbps(video);
        missing_native(
            &mut mismatches,
            &path,
            "video_bitrate_kbps",
            analysis.video_bitrate_kbps,
            video_bitrate_probe,
        );
        if let (Some(native), Some(probe)) = (analysis.video_bitrate_kbps, video_bitrate_probe)
            && (native - probe).abs() > 16
        {
            mismatches.push(format!(
                "{} video_bitrate_kbps mismatch: native={} ffprobe={}",
                path.file_name().unwrap().to_string_lossy(),
                native,
                probe
            ));
        }

        let audio_bitrate_probe = ffprobe_bitrate_kbps(audio);
        if !audio
            .is_some_and(|stream| analysis_parity::reference_bitrate_is_estimate(&ffprobe, stream))
        {
            missing_native(
                &mut mismatches,
                &path,
                "audio_bitrate_kbps",
                analysis.audio_bitrate_kbps,
                audio_bitrate_probe,
            );
        }
        if let (Some(native), Some(probe)) = (analysis.audio_bitrate_kbps, audio_bitrate_probe)
            && (native - probe).abs() > 16
        {
            mismatches.push(format!(
                "{} audio_bitrate_kbps mismatch: native={} ffprobe={}",
                path.file_name().unwrap().to_string_lossy(),
                native,
                probe
            ));
        }

        missing_native(
            &mut mismatches,
            &path,
            "video_frame_rate",
            analysis
                .video_frame_rate
                .as_deref()
                .and_then(|value| value.parse::<f64>().ok()),
            ffprobe_frame_rate(video),
        );
        missing_native(
            &mut mismatches,
            &path,
            "duration_seconds",
            analysis.details.duration_seconds,
            ffprobe
                .get("format")
                .and_then(|format| format.get("duration"))
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<f64>().ok()),
        );
        if let (Some(native), Some(probe)) = (
            analysis
                .video_frame_rate
                .as_deref()
                .and_then(|fps| fps.parse::<f64>().ok()),
            ffprobe_frame_rate(video),
        ) && (native - probe).abs() > 0.05
        {
            mismatches.push(format!(
                "{} video_frame_rate mismatch: native={:.3} ffprobe={:.3}",
                path.file_name().unwrap().to_string_lossy(),
                native,
                probe
            ));
        }

        if (native_duration - ffprobe_duration).abs() > 1 {
            mismatches.push(format!(
                "{} duration mismatch: native={} ffprobe={}",
                path.file_name().unwrap().to_string_lossy(),
                native_duration,
                ffprobe_duration
            ));
        }

        if native_num_chapters != ffprobe_num_chapters {
            mismatches.push(format!(
                "{} num_chapters mismatch: native={} ffprobe={}",
                path.file_name().unwrap().to_string_lossy(),
                native_num_chapters,
                ffprobe_num_chapters
            ));
        }

        let native_audio_languages = analysis.audio_languages.clone();
        let probe_audio_languages = ffprobe_languages_for_compare(
            container_format,
            &native_audio_languages,
            ffprobe_languages(&ffprobe, "audio"),
        );
        if native_audio_languages != probe_audio_languages {
            mismatches.push(format!(
                "{} audio_languages mismatch: native={:?} ffprobe={:?}",
                path.file_name().unwrap().to_string_lossy(),
                native_audio_languages,
                probe_audio_languages
            ));
        }

        let native_subtitle_languages = analysis.subtitle_languages.clone();
        let probe_subtitle_languages = ffprobe_languages_for_compare(
            container_format,
            &native_subtitle_languages,
            ffprobe_languages(&ffprobe, "subtitle"),
        );
        if native_subtitle_languages != probe_subtitle_languages {
            mismatches.push(format!(
                "{} subtitle_languages mismatch: native={:?} ffprobe={:?}",
                path.file_name().unwrap().to_string_lossy(),
                native_subtitle_languages,
                probe_subtitle_languages
            ));
        }

        let native_subtitle_codecs = analysis.subtitle_codecs.clone();
        let probe_subtitle_codecs = ffprobe_subtitle_codecs(&ffprobe);
        if native_subtitle_codecs != probe_subtitle_codecs {
            mismatches.push(format!(
                "{} subtitle_codecs mismatch: native={:?} ffprobe={:?}",
                path.file_name().unwrap().to_string_lossy(),
                native_subtitle_codecs,
                probe_subtitle_codecs
            ));
        }

        let probe_audio_streams = ffprobe_streams(&ffprobe, "audio");
        if analysis.audio_streams.len() != probe_audio_streams.len() {
            mismatches.push(format!(
                "{} audio_stream count mismatch: native={} ffprobe={}",
                path.file_name().unwrap().to_string_lossy(),
                analysis.audio_streams.len(),
                probe_audio_streams.len()
            ));
        }

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
            let probe_channels = ffprobe_optional_i32(Some(probe), "channels");
            let probe_bitrate = ffprobe_bitrate_kbps(Some(probe));
            if !analysis_parity::reference_bitrate_is_estimate(&ffprobe, probe) {
                missing_native(
                    &mut mismatches,
                    &path,
                    &format!("audio_stream[{index}].bitrate_kbps"),
                    native.bitrate_kbps,
                    probe_bitrate,
                );
            }

            if native.codec.as_deref() != probe.get("codec_name").and_then(Value::as_str) {
                mismatches.push(format!(
                    "{} audio_stream[{}] codec mismatch: native={:?} ffprobe={:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    index,
                    native.codec,
                    probe.get("codec_name").and_then(Value::as_str)
                ));
            }
            if native.channels != probe_channels {
                mismatches.push(format!(
                    "{} audio_stream[{}] channels mismatch: native={:?} ffprobe={:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    index,
                    native.channels,
                    probe_channels
                ));
            }
            if native.language != probe_language {
                mismatches.push(format!(
                    "{} audio_stream[{}] language mismatch: native={:?} ffprobe={:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    index,
                    native.language,
                    probe_language
                ));
            }
            if let (Some(native_bitrate), Some(probe_bitrate)) =
                (native.bitrate_kbps, probe_bitrate)
                && (native_bitrate - probe_bitrate).abs() > 16
            {
                mismatches.push(format!(
                    "{} audio_stream[{}] bitrate mismatch: native={} ffprobe={}",
                    path.file_name().unwrap().to_string_lossy(),
                    index,
                    native_bitrate,
                    probe_bitrate
                ));
            }
        }

        let probe_subtitle_streams = ffprobe_streams(&ffprobe, "subtitle");
        if analysis.subtitle_streams.len() != probe_subtitle_streams.len() {
            mismatches.push(format!(
                "{} subtitle_stream count mismatch: native={} ffprobe={}",
                path.file_name().unwrap().to_string_lossy(),
                analysis.subtitle_streams.len(),
                probe_subtitle_streams.len()
            ));
        }

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

            if native.codec.as_deref() != probe.get("codec_name").and_then(Value::as_str) {
                mismatches.push(format!(
                    "{} subtitle_stream[{}] codec mismatch: native={:?} ffprobe={:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    index,
                    native.codec,
                    probe.get("codec_name").and_then(Value::as_str)
                ));
            }
            if native.language != probe_language {
                mismatches.push(format!(
                    "{} subtitle_stream[{}] language mismatch: native={:?} ffprobe={:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    index,
                    native.language,
                    probe_language
                ));
            }
            if native.forced != probe_forced {
                mismatches.push(format!(
                    "{} subtitle_stream[{}] forced mismatch: native={} ffprobe={}",
                    path.file_name().unwrap().to_string_lossy(),
                    index,
                    native.forced,
                    probe_forced
                ));
            }
            if native.default != probe_default {
                mismatches.push(format!(
                    "{} subtitle_stream[{}] default mismatch: native={} ffprobe={}",
                    path.file_name().unwrap().to_string_lossy(),
                    index,
                    native.default,
                    probe_default
                ));
            }
        }
    }

    if let Some(requested) = requested {
        for name in requested {
            if !seen.contains(&name) {
                mismatches.push(format!("requested parity fixture was not found: {name}"));
            }
        }
    }
    if let Some(path) = report_path {
        let report = serde_json::json!({"analysis_revision": scryer_media_types::ANALYSIS_REVISION,
            "ffprobe_version": String::from_utf8_lossy(&version.stdout), "fixtures": records, "mismatches": mismatches});
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap())
            .expect("write differential report");
    }
    assert!(
        !seen.is_empty(),
        "parity run must compare at least one fixture"
    );
    assert!(
        mismatches.is_empty(),
        "ffprobe parity mismatches:\n{}",
        mismatches.join("\n")
    );
}

#[test]
fn sparse_mkv_authored_bytes_require_unknowns_and_explicit_variants_require_facts() {
    for name in sparse_mkv::NAMES {
        let sparse = sparse_mkv::authored_bytes(name, false).unwrap();
        assert_eq!(
            std::fs::read(media_dir().join(name)).unwrap(),
            sparse,
            "{name}: authored fixture audit"
        );
        let negative = scryer_mediainfo::analyze_catalog_file(&media_dir().join(name)).unwrap();
        let negative = serde_json::to_value(negative).unwrap();
        assert!(
            sparse_mkv::unknown_fact_errors(&negative).is_empty(),
            "{name}"
        );
        let explicit = scryer_mediainfo::analyze_source(
            &mut std::io::Cursor::new(sparse_mkv::authored_bytes(name, true).unwrap()),
            "mkv",
            scryer_mediainfo::AnalyzeOptions::default(),
        )
        .unwrap();
        for stream in &explicit.details.streams {
            assert_eq!(stream.metadata.original_language.as_deref(), Some("eng"));
            assert_eq!(
                stream.metadata.language_provenance,
                scryer_media_types::Provenance::Container
            );
        }
        assert_eq!(explicit.video_frame_rate.as_deref(), Some("25"));
        assert_eq!(
            explicit.details.streams[0].metadata.declared_frame_rate,
            scryer_media_types::Rational::new(25, 1)
        );
        assert_eq!(
            explicit.details.streams[1]
                .metadata
                .channel_layout
                .as_deref(),
            Some("stereo")
        );
        assert_eq!(
            explicit.details.streams[1].metadata.sample_rate,
            Some(48_000)
        );
    }
}

#[test]
fn sparse_mkv_reference_defaults_require_exact_authored_evidence() {
    let name = sparse_mkv::NAMES[0];
    let bytes = std::fs::read(media_dir().join(name)).unwrap();
    let native = serde_json::to_value(
        scryer_mediainfo::analyze_catalog_file(&media_dir().join(name)).unwrap(),
    )
    .unwrap();
    let reference = serde_json::json!({"streams":[
        {"codec_type":"video","tags":{"language":"eng"},"r_frame_rate":"1000/1"},
        {"codec_type":"audio","tags":{"language":"eng"},"channel_layout":"stereo"},
    ]});
    let (comparison, assumptions, errors) =
        sparse_mkv::comparison_view(name, &bytes, &reference, &native).unwrap();
    assert!(errors.is_empty());
    assert_eq!(assumptions.len(), 4);
    assert!(comparison["streams"][0]["r_frame_rate"].is_null());
    assert_eq!(reference["streams"][0]["r_frame_rate"], "1000/1");
    let explicit = sparse_mkv::authored_bytes(name, true).unwrap();
    assert!(sparse_mkv::comparison_view(name, &explicit, &reference, &native).is_err());
    let (unrelated, assumptions, _) =
        sparse_mkv::comparison_view("another.mkv", &bytes, &reference, &native).unwrap();
    assert_eq!(unrelated, reference);
    assert!(assumptions.is_empty());
    let mut changed_reference = reference.clone();
    changed_reference["streams"][0]["r_frame_rate"] = serde_json::json!("24/1");
    assert!(
        !sparse_mkv::comparison_view(name, &bytes, &changed_reference, &native)
            .unwrap()
            .2
            .is_empty()
    );
    for (path, fabricated) in [
        ("/video_frame_rate", serde_json::json!("1000.000")),
        (
            "/details/streams/0/metadata/original_language",
            serde_json::json!("eng"),
        ),
        (
            "/details/streams/1/metadata/channel_layout",
            serde_json::json!("stereo"),
        ),
    ] {
        let mut invalid = native.clone();
        *invalid.pointer_mut(path).unwrap() = fabricated;
        assert!(
            !sparse_mkv::comparison_view(name, &bytes, &reference, &invalid)
                .unwrap()
                .2
                .is_empty(),
            "{path}"
        );
    }
}

#[test]
#[ignore = "development reference requires FFprobe"]
fn compare_explicit_sparse_mkv_variants_against_ffprobe() {
    let ffprobe = ffprobe_bin().expect("FFprobe is required for this explicit comparison");
    struct FixtureDirectory(PathBuf);
    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = FixtureDirectory(std::env::temp_dir().join(format!(
        "scryer-explicit-mkv-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
    )));
    std::fs::create_dir(&directory.0).unwrap();
    for name in sparse_mkv::NAMES {
        let path = directory.0.join(name);
        std::fs::write(&path, sparse_mkv::authored_bytes(name, true).unwrap()).unwrap();
        let native =
            serde_json::to_value(scryer_mediainfo::analyze_catalog_file(&path).unwrap()).unwrap();
        let reference = sonarr_ffprobe_json(&ffprobe, &path);
        let differences = analysis_parity::compare(&native, &reference);
        assert!(differences.is_empty(), "{name}: {differences:?}");
        for path in [
            "/details/streams/0/metadata/original_language",
            "/details/streams/1/metadata/original_language",
            "/details/streams/1/metadata/channel_layout",
            "/details/streams/0/metadata/declared_frame_rate",
        ] {
            let mut missing = native.clone();
            *missing.pointer_mut(path).unwrap() = Value::Null;
            assert!(
                !analysis_parity::compare(&missing, &reference).is_empty(),
                "{name}: missing {path} must fail"
            );
        }
    }
}

fn missing_native<T: std::fmt::Debug>(
    mismatches: &mut Vec<String>,
    path: &Path,
    field: &str,
    native: Option<T>,
    reference: Option<T>,
) {
    if native.is_none()
        && let Some(reference) = reference
    {
        mismatches.push(format!(
            "{} {field} missing: native=None ffprobe={reference:?}",
            path.display()
        ));
    }
}

#[test]
fn reference_selection_keeps_cover_programs_and_commentary_out_of_summary() {
    let reference = serde_json::json!({"streams":[
        {"index":0,"codec_type":"video","codec_name":"png","disposition":{"attached_pic":1}},
        {"index":1,"codec_type":"video","codec_name":"hevc"},
        {"index":2,"codec_type":"audio","codec_name":"truehd","channels":8,"disposition":{"comment":1}},
        {"index":3,"codec_type":"audio","codec_name":"aac","channels":2,"tags":{"language":"eng"}},
        {"index":4,"codec_type":"audio","codec_name":"aac","channels":6},
        {"index":5,"codec_type":"audio","codec_name":"aac","channels":6},
        {"index":6,"codec_type":"audio","codec_name":"truehd","channels":8,"tags":{"language":"fra"}}
    ],"programs":[{"streams":[{"index":1},{"index":2},{"index":3},{"index":4},{"index":5}]},{"streams":[{"index":6}]}]});
    assert_eq!(primary_video_stream(&reference).unwrap().0, 1);
    assert_eq!(
        ffprobe_primary_stream(&reference, "video").unwrap()["index"],
        1
    );
    assert_eq!(
        ffprobe_primary_stream(&reference, "audio").unwrap()["index"],
        4
    );
    assert_eq!(ffprobe_languages(&reference, "audio"), ["eng"]);
    assert_eq!(ffprobe_streams(&reference, "audio").len(), 4);
}

#[test]
fn parity_distinguishes_frame_estimates_from_accountable_bitrate() {
    let mut reference = serde_json::json!({"format":{"format_name":"matroska,webm"},"streams":[
        {"codec_type":"audio","codec_name":"eac3","bit_rate":"96000"}
    ]});
    let mut native = serde_json::json!({"details":{"streams":[
        {"kind":"audio","codec":"eac3","metadata":{"estimated_bitrate_bps":96000}}
    ]}});
    assert!(analysis_parity::compare(&native, &reference).is_empty());
    native["details"]["streams"][0]["metadata"]["estimated_bitrate_bps"] = Value::Null;
    assert!(
        analysis_parity::compare(&native, &reference)
            .iter()
            .any(|gap| gap.contains("bitrate_bps"))
    );
    native["details"]["streams"][0]["metadata"]["estimated_bitrate_bps"] = 96000.into();
    reference["streams"][0]["tags"] = serde_json::json!({"BPS":"96000"});
    assert!(
        analysis_parity::compare(&native, &reference)
            .iter()
            .any(|gap| gap.contains("bitrate_provenance"))
    );
    reference["streams"][0]["tags"] = Value::Null;
    reference["format"]["format_name"] = "mov,mp4".into();
    assert!(
        analysis_parity::compare(&native, &reference)
            .iter()
            .any(|gap| gap.contains("bitrate_provenance"))
    );
}

#[test]
fn parity_counts_missing_native_required_values_as_mismatches() {
    let mut missing = Vec::new();
    missing_native(
        &mut missing,
        Path::new("fixture.mp4"),
        "duration",
        None,
        Some(2.0),
    );
    assert_eq!(missing.len(), 1);
    missing_native(
        &mut missing,
        Path::new("fixture.mp4"),
        "duration",
        Some(2.0),
        None,
    );
    assert_eq!(
        missing.len(),
        1,
        "reference absence does not invalidate measured native facts"
    );
}
