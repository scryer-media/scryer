//! Shared comparison for serialized parser and application analysis contracts.
use serde_json::Value;

fn number(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}
pub fn reference_bitrate_bps(stream: &Value) -> Option<f64> {
    if stream["reference_packet_accounting"]["complete"] == true
        && let Some(bitrate) = number(&stream["reference_packet_accounting"]["bitrate_bps"])
            .filter(|value| value.is_finite() && *value > 0.0)
    {
        return Some(bitrate);
    }
    let bitrate = number(&stream["bit_rate"])?;
    // FFprobe 8.1.1 exposes MPEG-1's all-ones bitrate code as a rate. That
    // sequence value means unspecified/VBR; it is not a 104.8572 Mbps stream.
    if stream["codec_name"] == "mpeg1video" && bitrate == f64::from(0x3ffff_u32 * 400) {
        return None;
    }
    (bitrate.is_finite() && bitrate > 0.0).then_some(bitrate)
}
pub fn complete_video_packet_accounting(stream: &Value, packets: &Value) -> Option<Value> {
    fn unsigned(value: &Value) -> Option<u64> {
        value.as_u64().or_else(|| value.as_str()?.parse().ok())
    }
    fn signed(value: &Value) -> Option<i64> {
        value.as_i64().or_else(|| value.as_str()?.parse().ok())
    }
    if stream["codec_type"] != "video" || stream["index"].is_null() {
        return None;
    }
    let expected = unsigned(&stream["nb_frames"])?;
    let packets = packets.as_array()?;
    if !(1..=4096).contains(&expected) || packets.len() > 4096 {
        return None;
    }
    let mut bytes = 0_u64;
    let mut times = Vec::new();
    for packet in packets
        .iter()
        .filter(|packet| packet["stream_index"] == stream["index"])
    {
        bytes = bytes.checked_add(unsigned(&packet["size"])?)?;
        let start = signed(&packet["pts"]).or_else(|| signed(&packet["dts"]))?;
        let duration = signed(&packet["duration"]).filter(|value| *value > 0)?;
        times.push((start, start.checked_add(duration)?));
    }
    if times.len() as u64 != expected {
        return None;
    }
    times.sort_unstable();
    if times.windows(2).any(|pair| pair[0].1 != pair[1].0) {
        return None;
    }
    let duration_ticks = times.last()?.1.checked_sub(times.first()?.0)?;
    let duration_seconds = duration_ticks as f64 * rational(&stream["time_base"])?;
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 || bytes == 0 {
        return None;
    }
    Some(serde_json::json!({
        "complete": true, "packets": expected, "bytes": bytes,
        "duration_seconds": duration_seconds,
        "bitrate_bps": bytes.checked_mul(8)? as f64 / duration_seconds,
    }))
}

fn rational(value: &Value) -> Option<f64> {
    if let Some(text) = value.as_str() {
        let (n, d) = text.split_once('/').or_else(|| text.split_once(':'))?;
        let (n, d) = (n.parse::<f64>().ok()?, d.parse::<f64>().ok()?);
        (d != 0.0).then_some(n / d)
    } else {
        let (n, d) = (number(&value["numerator"])?, number(&value["denominator"])?);
        (d != 0.0).then_some(n / d)
    }
}
fn near(
    out: &mut Vec<String>,
    label: &str,
    native: Option<f64>,
    reference: Option<f64>,
    tolerance: f64,
) {
    if let Some(reference) = reference.filter(|value| value.is_finite()) {
        if native.is_none_or(|native| !native.is_finite() || (native - reference).abs() > tolerance)
        {
            out.push(format!("{label}: native={native:?} reference={reference}"));
        }
    }
}
fn same(out: &mut Vec<String>, label: &str, native: &Value, reference: &Value) {
    if !reference.is_null() && reference.as_str() != Some("unknown") && native != reference {
        out.push(format!("{label}: native={native} reference={reference}"));
    }
}
fn color_code(field: &str, name: &str) -> Option<u32> {
    let names: &[(u32, &str)] = match field {
        "primaries" => &[
            (1, "bt709"),
            (4, "bt470m"),
            (5, "bt470bg"),
            (6, "smpte170m"),
            (7, "smpte240m"),
            (8, "film"),
            (9, "bt2020"),
            (10, "smpte428"),
            (11, "smpte431"),
            (12, "smpte432"),
            (22, "jedec-p22"),
        ],
        "transfer" => &[
            (1, "bt709"),
            (4, "gamma22"),
            (5, "gamma28"),
            (6, "smpte170m"),
            (7, "smpte240m"),
            (8, "linear"),
            (9, "log100"),
            (10, "log316"),
            (11, "iec61966-2-4"),
            (12, "bt1361e"),
            (13, "iec61966-2-1"),
            (14, "bt2020-10"),
            (15, "bt2020-12"),
            (16, "smpte2084"),
            (17, "smpte428"),
            (18, "arib-std-b67"),
        ],
        "matrix" => &[
            (0, "gbr"),
            (1, "bt709"),
            (4, "fcc"),
            (5, "bt470bg"),
            (6, "smpte170m"),
            (7, "smpte240m"),
            (8, "ycgco"),
            (9, "bt2020nc"),
            (10, "bt2020c"),
            (11, "smpte2085"),
            (12, "chroma-derived-nc"),
            (13, "chroma-derived-c"),
            (14, "ictcp"),
        ],
        _ => return None,
    };
    names
        .iter()
        .find_map(|(code, label)| (*label == name).then_some(*code))
}

fn pixel_depth(format: &str) -> Option<f64> {
    let format = format
        .strip_suffix("le")
        .or_else(|| format.strip_suffix("be"))
        .unwrap_or(format);
    for family in [
        "yuv420p", "yuv422p", "yuv444p", "yuv440p", "yuva420p", "yuva422p", "yuva444p", "gbrp",
        "gbrap", "gray",
    ] {
        if let Some(suffix) = format.strip_prefix(family) {
            return if suffix.is_empty() {
                Some(8.0)
            } else {
                suffix.parse().ok()
            };
        }
    }
    match format {
        "nv12" | "nv21" | "rgb24" | "bgr24" | "rgba" | "bgra" | "yuvj420p" | "yuvj422p"
        | "yuvj444p" => Some(8.0),
        "rgb48" | "bgr48" | "rgba64" | "bgra64" | "p016" => Some(16.0),
        "p010" => Some(10.0),
        "p012" => Some(12.0),
        _ => None,
    }
}

pub fn reference_bitrate_is_estimate(reference: &Value, stream: &Value) -> bool {
    // These FFmpeg audio parsers average frame-header rates when the container
    // has no bitrate declaration. Compare native bounded estimates in that case.
    // MP4 sample tables and explicit Matroska statistics remain accountable.
    let format = reference["format"]["format_name"]
        .as_str()
        .unwrap_or_default();
    let has_statistics = stream["tags"].as_object().is_some_and(|tags| {
        tags.iter().any(|(key, value)| {
            (key.eq_ignore_ascii_case("BPS") || key.to_ascii_uppercase().starts_with("BPS-"))
                && number(value).is_some_and(|rate| rate > 0.0)
        })
    });
    matches!(stream["codec_name"].as_str(), Some("aac" | "eac3" | "mp1" | "mp2" | "mp3"))
        && !has_statistics
        // A Matroska WAVEFORMATEX codec tag can carry a declared byte rate.
        && (!format.split(',').any(|name| name == "matroska")
            || stream["codec_tag"].as_str().is_none_or(|tag| matches!(tag, "0x0000" | "0x00000000")))
        && format
            .split(',')
            .any(|name| matches!(name, "matroska" | "mpegts" | "mpeg"))
}

pub fn compare(native: &Value, reference: &Value) -> Vec<String> {
    let mut differences = Vec::new();
    let details = &native["details"];
    let native_streams = details["streams"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let streams = reference["streams"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    for kind in ["video", "audio", "subtitle"] {
        let expected: Vec<_> = streams
            .iter()
            .filter(|stream| stream["codec_type"] == kind)
            .collect();
        let actual: Vec<_> = native_streams
            .iter()
            .filter(|stream| stream["kind"] == kind)
            .collect();
        if expected.len() != actual.len() {
            differences.push(format!(
                "{kind} inventory: native={} reference={}",
                actual.len(),
                expected.len()
            ));
        }
        for (ordinal, stream) in expected.iter().enumerate() {
            let Some(actual) = actual.get(ordinal) else {
                continue;
            };
            let metadata = &actual["metadata"];
            let label = format!("{kind}[{ordinal}]");
            same(
                &mut differences,
                &format!("{label}.codec"),
                &actual["codec"],
                &stream["codec_name"],
            );
            // FFprobe's sample_fmt is its selected decoder's output format:
            // AAC can use floating-point or fixed-point decoders, and 24-bit
            // PCM is expanded into 32-bit output. It is not an encoded format
            // declaration. Native sample formats are checked by authored
            // expected-value fixtures, not by inventing a decoder choice here.
            for (field, reference_field) in [
                ("profile", "profile"),
                ("pixel_format", "pix_fmt"),
                ("channel_layout", "channel_layout"),
            ] {
                same(
                    &mut differences,
                    &format!("{label}.{field}"),
                    &metadata[field],
                    &stream[reference_field],
                );
            }
            for field in ["width", "height", "channels"] {
                near(
                    &mut differences,
                    &format!("{label}.{field}"),
                    number(&actual[field]),
                    number(&stream[field]),
                    0.0,
                );
            }
            for (field, reference_field) in [("level", "level"), ("sample_rate", "sample_rate")] {
                near(
                    &mut differences,
                    &format!("{label}.{field}"),
                    number(&metadata[field]),
                    number(&stream[reference_field]).filter(|value| *value >= 0.0),
                    0.0,
                );
            }
            if let Some(format) = stream["pix_fmt"].as_str() {
                near(
                    &mut differences,
                    &format!("{label}.bit_depth"),
                    number(&metadata["bit_depth"]),
                    pixel_depth(format),
                    0.0,
                );
            }
            for (field, reference_field) in [
                ("primaries", "color_primaries"),
                ("transfer", "color_transfer"),
                ("matrix", "color_space"),
            ] {
                near(
                    &mut differences,
                    &format!("{label}.color.{field}"),
                    number(&metadata["color"][field]),
                    stream[reference_field]
                        .as_str()
                        .and_then(|name| color_code(field, name))
                        .map(f64::from),
                    0.0,
                );
            }
            if let Some(range @ ("tv" | "pc")) = stream["color_range"].as_str() {
                same(
                    &mut differences,
                    &format!("{label}.color.full_range"),
                    &metadata["color"]["full_range"],
                    &Value::Bool(range == "pc"),
                );
            }
            for field in ["sample_aspect_ratio", "display_aspect_ratio"] {
                near(
                    &mut differences,
                    &format!("{label}.{field}"),
                    rational(&metadata[field]),
                    rational(&stream[field]).filter(|value| *value > 0.0),
                    0.001,
                );
            }
            if kind == "video" {
                let declared =
                    rational(&metadata["declared_frame_rate"]).filter(|rate| *rate > 0.0);
                let average = rational(&stream["avg_frame_rate"]).filter(|rate| *rate > 0.0);
                let nominal = rational(&stream["r_frame_rate"]).filter(|rate| *rate > 0.0);
                let reference_rate = average.or(nominal);
                near(
                    &mut differences,
                    &format!("{label}.rational_frame_rate"),
                    declared.or_else(|| rational(&metadata["observed_frame_rate"])),
                    reference_rate,
                    0.05,
                );
                same(
                    &mut differences,
                    &format!("{label}.field_order"),
                    &metadata["field_order"],
                    &stream["field_order"],
                );
            }
            let bitrate = reference_bitrate_bps(stream);
            let measured = number(&metadata["bitrate_bps"]);
            let accepts_estimate = reference_bitrate_is_estimate(reference, stream);
            let native_bitrate = measured.or_else(|| {
                accepts_estimate
                    .then(|| number(&metadata["estimated_bitrate_bps"]))
                    .flatten()
            });
            near(
                &mut differences,
                &format!("{label}.bitrate_bps"),
                native_bitrate,
                bitrate,
                bitrate.unwrap_or(0.0) * 0.02 + 1000.0,
            );
            if bitrate.is_some()
                && (measured.is_some() || !accepts_estimate)
                && matches!(
                    metadata["bitrate_provenance"].as_str(),
                    None | Some("unknown" | "legacy" | "estimated")
                )
            {
                differences.push(format!("{label}.bitrate_provenance is not an accountable stream measurement/declaration"));
            }
            if let Some(language) = stream["tags"]["language"]
                .as_str()
                .filter(|value| !value.is_empty() && *value != "und")
            {
                // ASF's reference demuxer converts RFC 1766 tags to ISO 639
                // and discards regions. Native fixtures separately require
                // the exact original tag; this comparison checks its meaning.
                let asf_alias = reference["format"]["format_name"] == "asf"
                    && metadata["original_language"]
                        .as_str()
                        .is_some_and(|original| {
                            let primary = original
                                .split('-')
                                .next()
                                .unwrap_or(original)
                                .to_ascii_lowercase();
                            isolang::Language::from_639_1(&primary)
                                .is_some_and(|value| value.to_639_3() == language)
                        });
                if !asf_alias {
                    same(
                        &mut differences,
                        &format!("{label}.original_language"),
                        &metadata["original_language"],
                        &Value::String(language.into()),
                    );
                }
            }
            // FFprobe's zero role flags often mean no signal was found. Positive
            // roles are required facts; they may never disappear in projection.
            for (field, reference_field) in [
                ("default", "default"),
                ("forced", "forced"),
                ("commentary", "comment"),
                ("original", "original"),
                ("hearing_impaired", "hearing_impaired"),
                ("visual_impaired", "visual_impaired"),
                ("attached_picture", "attached_pic"),
                ("still_image", "still_image"),
            ] {
                if stream["disposition"][reference_field] == 1 {
                    same(
                        &mut differences,
                        &format!("{label}.disposition.{field}"),
                        &metadata["disposition"][field],
                        &Value::Bool(true),
                    );
                }
            }
            let frame_sides = reference["sampled_frames"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|frame| frame["stream_index"] == stream["index"])
                .flat_map(|frame| frame["side_data_list"].as_array().into_iter().flatten());
            for side in stream["side_data_list"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(frame_sides)
            {
                match side["side_data_type"].as_str().unwrap_or_default() {
                    "Mastering display metadata" => {
                        for field in [
                            "red_x",
                            "red_y",
                            "green_x",
                            "green_y",
                            "blue_x",
                            "blue_y",
                            "white_point_x",
                            "white_point_y",
                            "min_luminance",
                            "max_luminance",
                        ] {
                            let target = match field {
                                "white_point_x" => "white_x",
                                "white_point_y" => "white_y",
                                _ => field,
                            };
                            near(
                                &mut differences,
                                &format!("{label}.mastering.{target}"),
                                number(&metadata["color"]["mastering_display"][target]),
                                rational(&side[field]),
                                0.0001,
                            );
                        }
                    }
                    "Content light level metadata" => {
                        for (field, source) in
                            [("max_cll", "max_content"), ("max_fall", "max_average")]
                        {
                            near(
                                &mut differences,
                                &format!("{label}.content_light.{field}"),
                                number(&metadata["color"]["content_light"][field]),
                                number(&side[source]),
                                0.0,
                            );
                        }
                    }
                    "DOVI configuration record" => {
                        same(
                            &mut differences,
                            &format!("{label}.hdr.dolby_vision"),
                            &metadata["hdr"]["dolby_vision"],
                            &Value::Bool(true),
                        );
                        for (field, source) in [
                            ("profile", "dv_profile"),
                            ("level", "dv_level"),
                            (
                                "base_layer_compatibility_id",
                                "dv_bl_signal_compatibility_id",
                            ),
                        ] {
                            near(
                                &mut differences,
                                &format!("{label}.dovi.{field}"),
                                number(&metadata["hdr"]["dovi"][field]),
                                number(&side[source]),
                                0.0,
                            );
                        }
                    }
                    name if name.contains("2094-40") => same(
                        &mut differences,
                        &format!("{label}.hdr.hdr10plus"),
                        &metadata["hdr"]["hdr10plus"],
                        &Value::Bool(true),
                    ),
                    "Display Matrix" => near(
                        &mut differences,
                        &format!("{label}.rotation_degrees"),
                        number(&metadata["rotation_degrees"]),
                        number(&side["rotation"]),
                        0.01,
                    ),
                    _ => {}
                }
            }
        }
    }
    let chapters = details["chapters"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    for (index, chapter) in reference["chapters"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let actual = chapters.get(index).unwrap_or(&Value::Null);
        near(
            &mut differences,
            &format!("chapter[{index}].start_seconds"),
            number(&actual["start_seconds"]),
            number(&chapter["start_time"]),
            0.001,
        );
        same(
            &mut differences,
            &format!("chapter[{index}].title"),
            &actual["title"],
            &chapter["tags"]["title"],
        );
    }
    differences.sort();
    differences.dedup();
    differences
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mpeg1_bitrate_sentinel_is_not_a_reference_measurement() {
        let mut stream = serde_json::json!({"codec_name":"mpeg1video", "bit_rate":"104857200"});
        assert_eq!(reference_bitrate_bps(&stream), None);
        stream["bit_rate"] = "104856800".into();
        assert_eq!(reference_bitrate_bps(&stream), Some(104856800.0));
        let reference = serde_json::json!({"streams":[stream]});
        let native = serde_json::json!({"details":{"streams":[{"kind":"video","codec":"mpeg1video","metadata":{}}]}});
        assert!(compare(&native, &serde_json::json!({"streams":[{"codec_type":"video", "codec_name":"mpeg1video", "bit_rate":"104856800"}]})).iter().any(|error| error.contains("bitrate_bps")));
        let mut other_codec = reference["streams"][0].clone();
        other_codec["codec_name"] = "h264".into();
        other_codec["bit_rate"] = "104857200".into();
        assert_eq!(reference_bitrate_bps(&other_codec), Some(104857200.0));
    }

    #[test]
    fn estimated_reference_bitrates_still_require_native_values_and_preserve_declarations() {
        for codec in ["aac", "eac3", "mp1", "mp2", "mp3"] {
            let mut reference = serde_json::json!({"format":{"format_name":"mpegts"},"streams":[{
                "codec_type":"audio", "codec_name":codec, "bit_rate":"64000"
            }]});
            assert!(reference_bitrate_is_estimate(
                &reference,
                &reference["streams"][0]
            ));
            let mut native = serde_json::json!({"details":{"streams":[{"kind":"audio","codec":codec,"metadata":{}}]}});
            assert!(
                compare(&native, &reference)
                    .iter()
                    .any(|error| error.contains("bitrate_bps"))
            );
            native["details"]["streams"][0]["metadata"]["estimated_bitrate_bps"] = 64000.into();
            assert!(
                !compare(&native, &reference)
                    .iter()
                    .any(|error| error.contains("bitrate"))
            );
            native["details"]["streams"][0]["metadata"]["estimated_bitrate_bps"] = 8000.into();
            assert!(
                compare(&native, &reference)
                    .iter()
                    .any(|error| error.contains("bitrate_bps"))
            );
            for format in ["matroska,webm", "mpegts", "mpeg"] {
                reference["format"]["format_name"] = format.into();
                assert!(reference_bitrate_is_estimate(
                    &reference,
                    &reference["streams"][0]
                ));
            }
            reference["format"]["format_name"] = "mpegts".into();
            reference["streams"][0]["codec_tag"] = "0x000f".into();
            assert!(
                reference_bitrate_is_estimate(&reference, &reference["streams"][0]),
                "a TS codec tag is a stream type, not a WAVEFORMATEX declaration"
            );
            reference["streams"][0]["tags"] = serde_json::json!({"BPS":"64000"});
            assert!(!reference_bitrate_is_estimate(
                &reference,
                &reference["streams"][0]
            ));
            reference["streams"][0]["tags"] = Value::Null;
            reference["format"]["format_name"] = "mov,mp4,m4a,3gp,3g2,mj2".into();
            assert!(!reference_bitrate_is_estimate(
                &reference,
                &reference["streams"][0]
            ));
            reference["format"]["format_name"] = "matroska,webm".into();
            reference["streams"][0]["codec_tag"] = "0x0055".into();
            assert!(!reference_bitrate_is_estimate(
                &reference,
                &reference["streams"][0]
            ));
        }
    }

    #[test]
    fn asf_reference_language_normalization_requires_a_matching_original() {
        let mut native = serde_json::json!({"details":{"streams":[{"kind":"audio","codec":"mp3","metadata":{"original_language":"en-US"}}]}});
        let mut reference = serde_json::json!({"format":{"format_name":"asf"},"streams":[{"codec_type":"audio","codec_name":"mp3","tags":{"language":"eng"}}]});
        assert!(compare(&native, &reference).is_empty());
        for original in [Value::Null, "".into(), "ja-JP".into()] {
            native["details"]["streams"][0]["metadata"]["original_language"] = original;
            assert!(
                compare(&native, &reference)
                    .iter()
                    .any(|error| error.contains("original_language"))
            );
        }
        native["details"]["streams"][0]["metadata"]["original_language"] = "en-US".into();
        reference["format"]["format_name"] = "matroska,webm".into();
        assert!(
            compare(&native, &reference)
                .iter()
                .any(|error| error.contains("original_language"))
        );
    }

    #[test]
    fn reference_packet_accounting_requires_complete_contiguous_stream_timing() {
        let mut stream = serde_json::json!({"index":0,"codec_type":"video","codec_name":"mjpeg","nb_frames":"2","time_base":"1/25","bit_rate":"800000"});
        let packets = serde_json::json!([
            {"stream_index":0,"pts":1,"duration":1,"size":"3000"},
            {"stream_index":1,"pts":0,"duration":1,"size":"9000"},
            {"stream_index":0,"pts":0,"duration":1,"size":"1000"}
        ]);
        let accounting = complete_video_packet_accounting(&stream, &packets).unwrap();
        assert_eq!(accounting["bytes"], 4000);
        assert_eq!(accounting["packets"], 2);
        assert_eq!(accounting["duration_seconds"], 0.08);
        assert_eq!(accounting["bitrate_bps"], 400000.0);
        stream["reference_packet_accounting"] = accounting;
        assert_eq!(reference_bitrate_bps(&stream), Some(400000.0));
        assert_eq!(
            stream["bit_rate"], "800000",
            "retain the original reference declaration"
        );
        let reference = serde_json::json!({"streams":[stream.clone()]});
        let native = serde_json::json!({"details":{"streams":[{"kind":"video","codec":"mjpeg","metadata":{}}]}});
        assert!(
            compare(&native, &reference)
                .iter()
                .any(|error| error.contains("bitrate_bps"))
        );
        assert!(
            complete_video_packet_accounting(&stream, &serde_json::json!([packets[0].clone()]))
                .is_none()
        );
        for field in ["duration", "size", "pts"] {
            let mut incomplete = packets.clone();
            incomplete[0][field] = Value::Null;
            assert!(complete_video_packet_accounting(&stream, &incomplete).is_none());
        }
        for pts in [0, 2] {
            let mut discontinuous = packets.clone();
            discontinuous[0]["pts"] = pts.into();
            assert!(complete_video_packet_accounting(&stream, &discontinuous).is_none());
        }
        stream["nb_frames"] = "4097".into();
        assert!(complete_video_packet_accounting(&stream, &packets).is_none());
    }

    #[test]
    fn nominal_frame_rate_comparison_keeps_bounded_observations_separate() {
        let mut native = serde_json::json!({"details":{"streams":[{"kind":"video","codec":"vp9","metadata":{
            "declared_frame_rate":{"numerator":120,"denominator":1},
            "observed_frame_rate":{"numerator":2750,"denominator":23}
        }}]}});
        let mut reference = serde_json::json!({"streams":[{"codec_type":"video","codec_name":"vp9","r_frame_rate":"240/1","avg_frame_rate":"120/1"}]});
        assert!(compare(&native, &reference).is_empty());
        reference["streams"][0]["avg_frame_rate"] = "0/0".into();
        reference["streams"][0]["r_frame_rate"] = "120/1".into();
        assert!(compare(&native, &reference).is_empty());
        native["details"]["streams"][0]["metadata"]["declared_frame_rate"] =
            serde_json::json!({"numerator":60,"denominator":1});
        assert!(
            compare(&native, &reference)
                .iter()
                .any(|error| error.contains("rational_frame_rate"))
        );
        native["details"]["streams"][0]["metadata"] = Value::Null;
        assert!(
            compare(&native, &reference)
                .iter()
                .any(|error| error.contains("rational_frame_rate"))
        );
    }

    #[test]
    fn missing_required_rich_values_are_mismatches() {
        let native = serde_json::json!({"details":{"streams":[{"kind":"video","codec":"hevc","metadata":{}}]}});
        let reference = serde_json::json!({"streams":[{"codec_type":"video","codec_name":"hevc","profile":"Main 10","pix_fmt":"yuv420p10le","color_transfer":"smpte2084","disposition":{"comment":1}}]});
        let errors = compare(&native, &reference);
        for field in [
            "profile",
            "pixel_format",
            "bit_depth",
            "color.transfer",
            "commentary",
        ] {
            assert!(
                errors.iter().any(|error| error.contains(field)),
                "{field}: {errors:?}"
            );
        }
    }
}
