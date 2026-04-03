use scryer_mediainfo::analyze_file;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn media_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("media")
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

fn ffprobe_json(ffprobe: &Path, file: &Path) -> Value {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
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

fn ffprobe_primary_stream<'a>(json: &'a Value, codec_type: &str) -> Option<&'a Value> {
    json.get("streams")?
        .as_array()?
        .iter()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some(codec_type))
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

fn ffprobe_streams<'a>(json: &'a Value, codec_type: &str) -> Vec<&'a Value> {
    json.get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some(codec_type))
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
        .and_then(|stream| stream.get("bit_rate"))
        .and_then(Value::as_str)
        .and_then(|bitrate| bitrate.parse::<i64>().ok())
        .map(|bitrate| (bitrate / 1000) as i32)
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
    let mut mismatches = Vec::new();

    for entry in std::fs::read_dir(media_dir()).expect("media dir should exist") {
        let path = entry.expect("fixture entry").path();
        if !path.is_file() {
            continue;
        }

        let analysis = analyze_file(&path).expect("native analysis should succeed");
        let ffprobe = ffprobe_json(&ffprobe, &path);

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

    assert!(
        mismatches.is_empty(),
        "ffprobe parity mismatches:\n{}",
        mismatches.join("\n")
    );
}
