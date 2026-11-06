use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use xtask_support::{TaskContext, command_available, ok, step, warn};

#[derive(Args)]
pub(crate) struct MediaFixturesArgs {
    #[command(subcommand)]
    pub(crate) command: MediaFixturesCommand,
}

#[derive(Subcommand)]
pub(crate) enum MediaFixturesCommand {
    Generate(MediaFixturesGenerateArgs),
}

#[derive(Args)]
pub(crate) struct MediaFixturesGenerateArgs {
    #[arg(
        long,
        help = "Only rewrite media-fixtures.toml; do not invoke ffmpeg or touch media files"
    )]
    pub(crate) manifest_only: bool,
    #[arg(
        long = "fixture",
        value_name = "NAME",
        help = "Generate only the named fixture; repeat to select multiple fixtures"
    )]
    pub(crate) fixtures: Vec<String>,
    #[arg(
        long,
        value_name = "PATH",
        default_value = "ffmpeg",
        help = "FFmpeg executable used for encoder checks and fixture generation"
    )]
    pub(crate) ffmpeg: std::path::PathBuf,
    #[arg(
        long,
        value_name = "PATH",
        default_value = "speexenc",
        help = "speexenc executable used for the Speex FLV fixture"
    )]
    pub(crate) speexenc: std::path::PathBuf,
}

#[derive(Clone)]
struct FixtureCase {
    name: String,
    container: &'static str,
    video_codec: &'static str,
    width: i32,
    height: i32,
    fps: i32,
    audio_codecs: Vec<String>,
    source_audio_codecs: Vec<&'static str>,
    audio_channels: Vec<i32>,
    audio_languages: Vec<String>,
    source_audio_languages: Vec<&'static str>,
    subtitle_stream_count: usize,
    generated: bool,
    duration_seconds: Option<f64>,
    min_duration_seconds: i32,
    valid_video: bool,
    derive_ts_layout: Option<DerivedTsLayout>,
    asf_wave_extensible: bool,
}

#[derive(Clone)]
struct DerivedTsLayout {
    source: &'static str,
    raw_packet_size: usize,
    sync_offset: usize,
}

const WIDTHS: [i32; 4] = [64, 80, 96, 112];
const HEIGHTS: [i32; 4] = [36, 44, 54, 64];
const RATES: [i32; 5] = [12, 15, 24, 25, 30];
const CHANNELS: [i32; 3] = [1, 2, 6];
const LANGS: [&str; 4] = ["eng", "jpn", "spa", "und"];

fn select_cases<'a>(
    cases: &'a [FixtureCase],
    requested: &[String],
) -> Result<Vec<&'a FixtureCase>> {
    for name in requested {
        if !cases.iter().any(|case| &case.name == name) {
            bail!("unknown media fixture {name}");
        }
    }
    if requested.is_empty() {
        Ok(cases.iter().collect())
    } else {
        Ok(cases
            .iter()
            .filter(|case| requested.contains(&case.name))
            .collect())
    }
}

fn require_ffmpeg_encoder(ffmpeg: &Path, encoder: &str, installation_hint: &str) -> Result<()> {
    let output = Command::new(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .with_context(|| format!("failed to query encoders from {}", ffmpeg.display()))?;
    if !output.status.success() {
        bail!("ffmpeg -encoders failed with {}", output.status);
    }
    let listing = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    let available = ffmpeg_encoder_listing_contains(&listing, encoder);
    if !available {
        bail!("ffmpeg encoder {encoder} is required; {installation_hint}");
    }
    Ok(())
}

fn ffmpeg_encoder_listing_contains(listing: &[u8], encoder: &str) -> bool {
    listing.split(|byte| *byte == b'\n').any(|line| {
        std::str::from_utf8(line)
            .ok()
            .and_then(|line| line.split_whitespace().nth(1))
            == Some(encoder)
    })
}

pub(crate) fn generate(ctx: &TaskContext, args: &MediaFixturesGenerateArgs) -> Result<()> {
    step("Generating scryer-mediainfo media fixtures");

    let tests_dir = ctx.path("crates/scryer-mediainfo/tests");
    let media_dir = tests_dir.join("media");
    let manifest_path = tests_dir.join("media-fixtures.toml");
    fs::create_dir_all(&media_dir)
        .with_context(|| format!("failed to create {}", media_dir.display()))?;

    let mut cases = build_matrix();
    cases.extend(build_dense_simd_cases());
    let selected = select_cases(&cases, &args.fixtures)?;

    let ffprobe_available = if args.manifest_only {
        false
    } else {
        let status = Command::new(&args.ffmpeg)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .with_context(|| format!("failed to execute {}", args.ffmpeg.display()))?;
        if !status.success() {
            bail!("{} -version failed with {status}", args.ffmpeg.display());
        }
        if selected.iter().any(|case| case.video_codec == "theora") {
            require_ffmpeg_encoder(
                &args.ffmpeg,
                "libtheora",
                "install or rebuild FFmpeg with libtheora encoder support",
            )?;
        }
        if selected
            .iter()
            .any(|case| case.source_audio_codecs.contains(&"speex"))
        {
            let status = Command::new(&args.speexenc)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .with_context(|| format!("failed to execute {}", args.speexenc.display()))?;
            if !status.success() {
                bail!("{} --version failed with {status}", args.speexenc.display());
            }
        }
        let available = command_available("ffprobe").unwrap_or(false);
        if !available {
            warn("ffprobe was not found; generated fixtures will not get ffprobe validation");
        }
        available
    };

    write_manifest(&manifest_path, &cases)?;

    if args.manifest_only {
        ok(format!("wrote {}", manifest_path.display()));
        return Ok(());
    }

    for (index, case) in selected.iter().enumerate() {
        generate_case(
            ctx,
            &media_dir,
            case,
            &args.ffmpeg,
            &args.speexenc,
            ffprobe_available,
        )
        .with_context(|| format!("failed to generate {} at case {}", case.name, index + 1))?;
    }

    if args.fixtures.is_empty() {
        write_simd_scan_fixture(&media_dir)?;
    }
    ok(format!(
        "generated {} media fixtures in {}",
        selected.len(),
        media_dir.display()
    ));
    Ok(())
}

fn dims(index: usize) -> (i32, i32) {
    (WIDTHS[index % WIDTHS.len()], HEIGHTS[index % HEIGHTS.len()])
}

fn rate(index: usize) -> i32 {
    RATES[index % RATES.len()]
}

fn channel_layout(channels: i32) -> &'static str {
    match channels {
        1 => "mono",
        2 => "stereo",
        6 => "5.1",
        _ => "stereo",
    }
}

fn supported_channels(codec: &str, requested: i32) -> i32 {
    if codec == "vorbis" {
        return 2;
    }
    if matches!(codec, "mp3" | "mp2" | "vorbis" | "wmav1" | "wmav2") {
        return requested.min(2);
    }
    requested
}

struct FixtureFlags {
    subtitle: bool,
    dual: bool,
}

fn build_case(
    prefix: &str,
    index: usize,
    ext: &'static str,
    container: &'static str,
    video: &'static str,
    audio: &[&'static str],
    flags: FixtureFlags,
) -> FixtureCase {
    let (width, height) = dims(index);
    let mut audios = audio.to_vec();
    if flags.dual && audios.len() == 1 {
        let extra = if ext == "webm" {
            if audios[0] != "vorbis" {
                "vorbis"
            } else {
                "opus"
            }
        } else if ext == "mov" {
            "aac"
        } else if matches!(ext, "mp4" | "m4v" | "mov") {
            if audios[0] == "aac" { "ac3" } else { "aac" }
        } else if audios[0] != "aac" {
            "aac"
        } else {
            "mp3"
        };
        audios.push(extra);
    }

    let mut channels = audios
        .iter()
        .enumerate()
        .map(|(stream, codec)| {
            supported_channels(codec, CHANNELS[(index + stream) % CHANNELS.len()])
        })
        .collect::<Vec<_>>();
    if ext == "webm" {
        channels.fill(2);
    }

    let source_audio_languages = (0..audios.len())
        .map(|stream| LANGS[(index + stream) % LANGS.len()])
        .collect::<Vec<_>>();
    let audio_languages = source_audio_languages
        .iter()
        .copied()
        .filter(|language| *language != "und" && !matches!(container, "avi" | "flv"))
        .map(str::to_string)
        .collect::<Vec<_>>();

    FixtureCase {
        name: format!("matrix_{prefix}_{index:03}.{ext}"),
        container,
        video_codec: video,
        width,
        height,
        fps: rate(index),
        audio_codecs: audios.iter().copied().map(str::to_string).collect(),
        source_audio_codecs: audios,
        audio_channels: channels,
        audio_languages,
        source_audio_languages,
        subtitle_stream_count: usize::from(flags.subtitle),
        generated: true,
        duration_seconds: None,
        min_duration_seconds: 1,
        valid_video: true,
        derive_ts_layout: None,
        asf_wave_extensible: false,
    }
}

fn build_matrix() -> Vec<FixtureCase> {
    let mut cases = Vec::new();

    let mkv_pairs = product(
        &["h264", "mpeg4", "mjpeg", "mpeg2video", "vp9"],
        &["aac", "mp3", "flac", "ac3", "eac3", "opus", "vorbis"],
    );
    for i in 0..43 {
        let (video, audio) = mkv_pairs[i % mkv_pairs.len()];
        cases.push(build_case(
            "mkv",
            i + 1,
            "mkv",
            "matroska",
            video,
            &[audio],
            FixtureFlags {
                subtitle: i % 9 == 0,
                dual: i % 7 == 0,
            },
        ));
    }

    let webm_pairs = product(&["vp8", "vp9"], &["opus", "vorbis"]);
    for i in 0..12 {
        let (video, audio) = webm_pairs[i % webm_pairs.len()];
        cases.push(build_case(
            "webm",
            i + 1,
            "webm",
            "webm",
            video,
            &[audio],
            FixtureFlags {
                subtitle: false,
                dual: i % 5 == 0,
            },
        ));
    }

    let mp4_exts = ["mp4", "m4v", "mov"];
    let mp4_pairs = product(&["h264", "mpeg4", "vp9"], &["aac", "ac3", "eac3", "opus"]);
    for i in 0..60 {
        let ext = mp4_exts[i % mp4_exts.len()];
        let (mut video, mut audio) = mp4_pairs[i % mp4_pairs.len()];
        if ext == "mov" {
            video = "h264";
            if audio != "aac" {
                audio = "aac";
            }
        }
        let container = if ext == "mov" { "mov" } else { "mp4" };
        cases.push(build_case(
            "mp4",
            i + 1,
            ext,
            container,
            video,
            &[audio],
            FixtureFlags {
                subtitle: i % 10 == 0,
                dual: i % 8 == 0,
            },
        ));
    }

    let avi_pairs = product(
        &["mpeg4", "mjpeg", "mpeg2video", "h264"],
        &["mp3", "pcm_s16le", "ac3", "aac"],
    );
    for i in 0..40 {
        let (video, audio) = avi_pairs[i % avi_pairs.len()];
        cases.push(build_case(
            "avi",
            i + 1,
            "avi",
            "avi",
            video,
            &[audio],
            FixtureFlags {
                subtitle: false,
                dual: i % 6 == 0,
            },
        ));
    }

    let ts_exts = ["ts", "m2ts"];
    let ts_pairs = product(
        &["h264", "mpeg2video"],
        &["aac", "mp2", "ac3", "eac3", "dts"],
    );
    for i in 0..45 {
        let ext = ts_exts[i % ts_exts.len()];
        let (video, audio) = ts_pairs[i % ts_pairs.len()];
        cases.push(build_case(
            "ts",
            i + 1,
            ext,
            "mpegts",
            video,
            &[audio],
            FixtureFlags {
                subtitle: false,
                dual: i % 9 == 0,
            },
        ));
    }

    const WMV_SPECS: &[(&str, &str, &[&str], &[i32])] = &[
        ("wmv_wmv1_wmav1.wmv", "wmv1", &["wmav1"], &[1]),
        ("wmv_wmv2_wmav2.wmv", "wmv2", &["wmav2"], &[2]),
        ("wmv_wmv2_pcm_s24le.wmv", "wmv2", &["pcm_s24le"], &[2]),
        ("wmv_wmv1_mp3_mono.wmv", "wmv1", &["mp3"], &[1]),
        ("wmv_wmv2_pcm_u8_mono.wmv", "wmv2", &["pcm_u8"], &[1]),
        (
            "wmv_wmv1_pcm_s16le_stereo.wmv",
            "wmv1",
            &["pcm_s16le"],
            &[2],
        ),
        (
            "wmv_wmv2_pcm_s32le_stereo.wmv",
            "wmv2",
            &["pcm_s32le"],
            &[2],
        ),
        (
            "wmv_wmv1_pcm_f32le_stereo.wmv",
            "wmv1",
            &["pcm_f32le"],
            &[2],
        ),
        (
            "wmv_wmv2_pcm_s24le_surround.wmv",
            "wmv2",
            &["pcm_s24le"],
            &[6],
        ),
        ("wmv_wmv1_aac_surround.wmv", "wmv1", &["aac"], &[6]),
        ("wmv_wmv2_ac3_surround.wmv", "wmv2", &["ac3"], &[6]),
        (
            "wmv_wmv1_dual_wma.wmv",
            "wmv1",
            &["wmav1", "wmav2"],
            &[1, 2],
        ),
        ("wmv_wmv2_video_only.wmv", "wmv2", &[], &[]),
    ];
    for (index, &(name, video, audio, channels)) in WMV_SPECS.iter().enumerate() {
        let mut case = build_case(
            "wmv",
            index + 1,
            "wmv",
            "asf",
            video,
            audio,
            FixtureFlags {
                subtitle: false,
                dual: false,
            },
        );
        case.name = name.into();
        case.audio_channels = channels.to_vec();
        case.asf_wave_extensible = name == "wmv_wmv2_pcm_s24le_surround.wmv";
        cases.push(case);
    }

    const OGV_SPECS: &[(&str, &[&str], &[i32])] = &[
        ("ogv_theora_vorbis.ogv", &["vorbis"], &[2]),
        ("ogv_theora_opus.ogv", &["opus"], &[6]),
        ("ogv_theora_opus_mono.ogv", &["opus"], &[1]),
        ("ogv_theora_opus_stereo.ogv", &["opus"], &[2]),
        ("ogv_theora_opus_surround.ogv", &["opus"], &[6]),
        (
            "ogv_theora_dual_vorbis_opus.ogv",
            &["vorbis", "opus"],
            &[2, 1],
        ),
        ("ogv_theora_dual_opus.ogv", &["opus", "opus"], &[1, 6]),
        ("ogv_theora_dual_vorbis.ogv", &["vorbis", "vorbis"], &[2, 2]),
        ("ogv_theora_vorbis_alt.ogv", &["vorbis"], &[2]),
        ("ogv_theora_video_only.ogv", &[], &[]),
    ];
    for (index, &(name, audio, channels)) in OGV_SPECS.iter().enumerate() {
        let mut case = build_case(
            "ogv",
            index + 1,
            "ogv",
            "ogg",
            "theora",
            audio,
            FixtureFlags {
                subtitle: false,
                dual: false,
            },
        );
        case.name = name.into();
        case.audio_channels = channels.to_vec();
        cases.push(case);
    }

    const FLV_SPECS: &[(&str, &str, &[&str], &[i32])] = &[
        ("flv_flv1_mp3.flv", "flv1", &["mp3"], &[2]),
        ("flv_h264_aac.flv", "h264", &["aac"], &[6]),
        ("flv_flv1_pcm_s16le.flv", "flv1", &["pcm_s16le"], &[2]),
        ("flv_h264_mp3.flv", "h264", &["mp3"], &[1]),
        ("flv_flv1_pcm_u8.flv", "flv1", &["pcm_u8"], &[1]),
        ("flv_flv1_adpcm_swf.flv", "flv1", &["adpcm_swf"], &[1]),
        ("flv_flv1_nellymoser.flv", "flv1", &["nellymoser"], &[1]),
        ("flv_flv1_aac_mono.flv", "flv1", &["aac"], &[1]),
        ("flv_h264_pcm_alaw.flv", "h264", &["pcm_alaw"], &[1]),
        ("flv_flv1_pcm_mulaw.flv", "flv1", &["pcm_mulaw"], &[1]),
        ("flv_flv1_speex.flv", "flv1", &["speex"], &[1]),
        ("flv_flv1_video_only.flv", "flv1", &[], &[]),
    ];
    for (index, &(name, video, audio, channels)) in FLV_SPECS.iter().enumerate() {
        let mut case = build_case(
            "flv",
            index + 1,
            "flv",
            "flv",
            video,
            audio,
            FixtureFlags {
                subtitle: false,
                dual: false,
            },
        );
        case.name = name.into();
        case.audio_channels = channels.to_vec();
        cases.push(case);
    }

    assert_eq!(cases.len(), 235);
    cases
}

fn product(
    left: &'static [&'static str],
    right: &'static [&'static str],
) -> Vec<(&'static str, &'static str)> {
    let mut pairs = Vec::with_capacity(left.len() * right.len());
    for lhs in left {
        for rhs in right {
            pairs.push((*lhs, *rhs));
        }
    }
    pairs
}

fn build_dense_idx1_case() -> FixtureCase {
    FixtureCase {
        name: "avi_idx1_dense_mpeg4.avi".to_string(),
        container: "avi",
        video_codec: "mpeg4",
        width: 32,
        height: 18,
        fps: 120,
        audio_codecs: Vec::new(),
        source_audio_codecs: Vec::new(),
        audio_channels: Vec::new(),
        audio_languages: Vec::new(),
        source_audio_languages: Vec::new(),
        subtitle_stream_count: 0,
        generated: true,
        duration_seconds: Some(8.0),
        min_duration_seconds: 8,
        valid_video: true,
        derive_ts_layout: None,
        asf_wave_extensible: false,
    }
}

fn build_dense_simd_case(
    name: &'static str,
    container: &'static str,
    video: &'static str,
    audio: &'static str,
) -> FixtureCase {
    FixtureCase {
        name: name.to_string(),
        container,
        video_codec: video,
        width: 32,
        height: 18,
        fps: 120,
        audio_codecs: vec![audio.to_string()],
        source_audio_codecs: vec![audio],
        audio_channels: vec![2],
        audio_languages: vec!["eng".to_string()],
        source_audio_languages: vec!["eng"],
        subtitle_stream_count: 0,
        generated: true,
        duration_seconds: Some(8.0),
        min_duration_seconds: 7,
        valid_video: true,
        derive_ts_layout: None,
        asf_wave_extensible: false,
    }
}

fn build_derived_ts_layout_case(
    name: &'static str,
    raw_packet_size: usize,
    sync_offset: usize,
) -> FixtureCase {
    let mut case = build_dense_simd_case(name, "mpegts", "h264", "aac");
    case.derive_ts_layout = Some(DerivedTsLayout {
        source: "simd_dense_h264_aac.ts",
        raw_packet_size,
        sync_offset,
    });
    case
}

fn build_dense_simd_cases() -> Vec<FixtureCase> {
    vec![
        build_dense_idx1_case(),
        build_dense_simd_case("simd_dense_h264_aac.ts", "mpegts", "h264", "aac"),
        build_derived_ts_layout_case("simd_dense_h264_aac_192.m2ts", 192, 4),
        build_derived_ts_layout_case("simd_dense_h264_aac_204.ts", 204, 0),
        build_dense_simd_case("simd_dense_mpeg2_mp2.ts", "mpegts", "mpeg2video", "mp2"),
        build_dense_simd_case("simd_dense_h264_ac3.ts", "mpegts", "h264", "ac3"),
        build_dense_simd_case("simd_dense_h264_dts.ts", "mpegts", "h264", "dts"),
        build_dense_simd_case("simd_dense_vp9_opus.webm", "webm", "vp9", "opus"),
        build_dense_simd_case("simd_dense_h264_aac.mp4", "mp4", "h264", "aac"),
    ]
}

fn write_manifest(manifest_path: &Path, cases: &[FixtureCase]) -> Result<()> {
    fs::write(manifest_path, manifest_contents(cases))
        .with_context(|| format!("failed to write {}", manifest_path.display()))
}

fn manifest_contents(cases: &[FixtureCase]) -> String {
    let mut manifest = String::from(
        "# Generated by cargo xtask media-fixtures generate.\n\
         # This manifest is the source of truth for the committed matrix fixtures.\n\n",
    );

    for (index, case) in cases.iter().enumerate() {
        if index > 0 {
            manifest.push('\n');
        }
        manifest.push_str("[[fixtures]]\n");
        push_toml_str(&mut manifest, "name", &case.name);
        push_toml_bool(&mut manifest, "generated", case.generated);
        push_toml_str(&mut manifest, "container", case.container);
        push_toml_str(&mut manifest, "video_codec", case.video_codec);
        push_toml_i32(&mut manifest, "width", case.width);
        push_toml_i32(&mut manifest, "height", case.height);
        push_toml_i32(&mut manifest, "fps", case.fps);
        push_toml_str_array(&mut manifest, "audio_codecs", &case.audio_codecs);
        push_toml_i32_array(&mut manifest, "audio_channels", &case.audio_channels);
        push_toml_str_array(&mut manifest, "audio_languages", &case.audio_languages);
        push_toml_usize(
            &mut manifest,
            "subtitle_stream_count",
            case.subtitle_stream_count,
        );
        push_toml_i32(
            &mut manifest,
            "min_duration_seconds",
            case.min_duration_seconds,
        );
        push_toml_bool(&mut manifest, "valid_video", case.valid_video);
    }

    manifest
}

fn push_toml_str(output: &mut String, key: &str, value: &str) {
    output.push_str(key);
    output.push_str(" = \"");
    output.push_str(&toml_escape(value));
    output.push_str("\"\n");
}

fn push_toml_bool(output: &mut String, key: &str, value: bool) {
    output.push_str(key);
    output.push_str(" = ");
    output.push_str(if value { "true" } else { "false" });
    output.push('\n');
}

fn push_toml_i32(output: &mut String, key: &str, value: i32) {
    output.push_str(key);
    output.push_str(" = ");
    output.push_str(&value.to_string());
    output.push('\n');
}

fn push_toml_usize(output: &mut String, key: &str, value: usize) {
    output.push_str(key);
    output.push_str(" = ");
    output.push_str(&value.to_string());
    output.push('\n');
}

fn push_toml_str_array(output: &mut String, key: &str, values: &[String]) {
    output.push_str(key);
    output.push_str(" = [");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push('"');
        output.push_str(&toml_escape(value));
        output.push('"');
    }
    output.push_str("]\n");
}

fn push_toml_i32_array(output: &mut String, key: &str, values: &[i32]) {
    output.push_str(key);
    output.push_str(" = [");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(&value.to_string());
    }
    output.push_str("]\n");
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn derive_ts_layout_case(
    media_dir: &Path,
    case: &FixtureCase,
    spec: &DerivedTsLayout,
) -> Result<()> {
    let source = media_dir.join(spec.source);
    let out = media_dir.join(&case.name);
    if spec.sync_offset + 188 > spec.raw_packet_size {
        bail!("invalid TS layout for {}", case.name);
    }

    let source_bytes =
        fs::read(&source).with_context(|| format!("failed to read {}", source.display()))?;
    let mut out_bytes = Vec::new();
    let usable_len = source_bytes.len() - (source_bytes.len() % 188);
    let mut packet_count = 0usize;
    for offset in (0..usable_len).step_by(188) {
        let packet = &source_bytes[offset..offset + 188];
        if packet[0] != 0x47 {
            bail!(
                "{} lost TS sync at packet {}",
                source.display(),
                packet_count
            );
        }

        let mut raw_packet = vec![0_u8; spec.raw_packet_size];
        raw_packet[spec.sync_offset..spec.sync_offset + 188].copy_from_slice(packet);
        out_bytes.extend(raw_packet);
        packet_count += 1;
    }

    if packet_count == 0 {
        bail!("{} did not contain TS packets", source.display());
    }

    fs::write(&out, out_bytes).with_context(|| format!("failed to write {}", out.display()))
}

fn read_u16_le_at(data: &[u8], offset: usize) -> Result<u16> {
    let Some(bytes) = data.get(offset..offset + 2) else {
        bail!("truncated ASF field at offset {offset}");
    };
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u32_le_at(data: &[u8], offset: usize) -> Result<u32> {
    let Some(bytes) = data.get(offset..offset + 4) else {
        bail!("truncated ASF field at offset {offset}");
    };
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u64_le_at(data: &[u8], offset: usize) -> Result<u64> {
    let Some(bytes) = data.get(offset..offset + 8) else {
        bail!("truncated ASF field at offset {offset}");
    };
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

fn write_le_at(data: &mut [u8], offset: usize, value: &[u8]) -> Result<()> {
    let Some(target) = data.get_mut(offset..offset + value.len()) else {
        bail!("truncated ASF field at offset {offset}");
    };
    target.copy_from_slice(value);
    Ok(())
}

fn rewrite_asf_pcm_as_wave_extensible(path: &Path) -> Result<()> {
    const FILE_PROPERTIES_GUID: [u8; 16] = [
        0xa1, 0xdc, 0xab, 0x8c, 0x47, 0xa9, 0xcf, 0x11, 0x8e, 0xe4, 0x00, 0xc0, 0x0c, 0x20, 0x53,
        0x65,
    ];
    const STREAM_PROPERTIES_GUID: [u8; 16] = [
        0x91, 0x07, 0xdc, 0xb7, 0xb7, 0xa9, 0xcf, 0x11, 0x8e, 0xe6, 0x00, 0xc0, 0x0c, 0x20, 0x53,
        0x65,
    ];
    const AUDIO_MEDIA_GUID: [u8; 16] = [
        0x40, 0x9e, 0x69, 0xf8, 0x4d, 0x5b, 0xcf, 0x11, 0xa8, 0xfd, 0x00, 0x80, 0x5f, 0x5c, 0x44,
        0x2b,
    ];
    const EXTENSIBLE_BYTES: u64 = 22;

    let mut data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let header_size = usize::try_from(read_u64_le_at(&data, 16)?)
        .context("ASF header size does not fit in memory")?;
    if header_size > data.len() || header_size < 30 {
        bail!("invalid ASF header size in {}", path.display());
    }

    let mut position = 30_usize;
    let mut file_size_offset = None;
    let mut target = None;
    while position < header_size {
        let object_end = usize::try_from(read_u64_le_at(&data, position + 16)?)
            .ok()
            .and_then(|size| position.checked_add(size))
            .filter(|end| *end <= header_size)
            .ok_or_else(|| anyhow::anyhow!("invalid ASF object size at offset {position}"))?;
        let guid = &data[position..position + 16];
        if guid == FILE_PROPERTIES_GUID {
            file_size_offset = Some(position + 40);
        } else if guid == STREAM_PROPERTIES_GUID
            && data.get(position + 24..position + 40) == Some(AUDIO_MEDIA_GUID.as_slice())
        {
            let type_size_offset = position + 64;
            let type_size = usize::try_from(read_u32_le_at(&data, type_size_offset)?)
                .context("ASF type-specific data size does not fit in memory")?;
            let type_data_offset = position + 78;
            let format_tag = read_u16_le_at(&data, type_data_offset)?;
            if type_size >= 40
                && format_tag == 0xfffe
                && read_u16_le_at(&data, type_data_offset + 24)? == 0x0001
            {
                return Ok(());
            }
            if type_size == 18
                && format_tag == 0x0001
                && read_u16_le_at(&data, type_data_offset + 2)? > 2
                && read_u16_le_at(&data, type_data_offset + 14)? > 16
            {
                target = Some((position + 16, type_size_offset, type_data_offset));
            }
        }
        position = object_end;
    }

    let file_size_offset = file_size_offset.context("ASF file properties object not found")?;
    let (object_size_offset, type_size_offset, type_data_offset) =
        target.context("eligible ASF PCM stream not found")?;
    let old_file_size = read_u64_le_at(&data, file_size_offset)?;
    let old_header_size = read_u64_le_at(&data, 16)?;
    let old_object_size = read_u64_le_at(&data, object_size_offset)?;
    let old_type_size = read_u32_le_at(&data, type_size_offset)?;
    let bits_per_sample = read_u16_le_at(&data, type_data_offset + 14)?;

    write_le_at(&mut data, type_data_offset, &0xfffe_u16.to_le_bytes())?;
    write_le_at(&mut data, type_data_offset + 16, &22_u16.to_le_bytes())?;
    write_le_at(
        &mut data,
        file_size_offset,
        &old_file_size
            .checked_add(EXTENSIBLE_BYTES)
            .context("ASF file size overflow")?
            .to_le_bytes(),
    )?;
    write_le_at(
        &mut data,
        16,
        &old_header_size
            .checked_add(EXTENSIBLE_BYTES)
            .context("ASF header size overflow")?
            .to_le_bytes(),
    )?;
    write_le_at(
        &mut data,
        object_size_offset,
        &old_object_size
            .checked_add(EXTENSIBLE_BYTES)
            .context("ASF stream properties size overflow")?
            .to_le_bytes(),
    )?;
    write_le_at(
        &mut data,
        type_size_offset,
        &old_type_size
            .checked_add(u32::try_from(EXTENSIBLE_BYTES).unwrap())
            .context("ASF type-specific data size overflow")?
            .to_le_bytes(),
    )?;

    let mut extension = Vec::with_capacity(EXTENSIBLE_BYTES as usize);
    extension.extend_from_slice(&bits_per_sample.to_le_bytes());
    extension.extend_from_slice(&0x3f_u32.to_le_bytes());
    extension.extend_from_slice(&[
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b,
        0x71,
    ]);
    data.splice(type_data_offset + 18..type_data_offset + 18, extension);

    fs::write(path, data).with_context(|| format!("failed to write {}", path.display()))
}

fn generate_speex_input(
    ffmpeg: &Path,
    speexenc: &Path,
    media_dir: &Path,
    case: &FixtureCase,
    duration: &str,
) -> Result<std::path::PathBuf> {
    let stem = output_stem(&case.name)?;
    let wav_path = media_dir.join(format!(".{stem}.speex.wav"));
    let speex_path = media_dir.join(format!(".{stem}.speex.spx"));
    let mut wav_command = Command::new(ffmpeg);
    wav_command
        .arg("-y")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(format!(
            "sine=frequency=440:sample_rate=16000:duration={duration}"
        ))
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(&wav_path);
    if let Err(error) = crate::run_checked(&mut wav_command) {
        let _ = fs::remove_file(&wav_path);
        return Err(error);
    }

    let mut speex_command = Command::new(speexenc);
    speex_command.arg(&wav_path).arg(&speex_path);
    let result = crate::run_checked(&mut speex_command);
    let _ = fs::remove_file(&wav_path);
    if let Err(error) = result {
        let _ = fs::remove_file(&speex_path);
        return Err(error);
    }
    Ok(speex_path)
}

fn generate_case(
    ctx: &TaskContext,
    media_dir: &Path,
    case: &FixtureCase,
    ffmpeg: &Path,
    speexenc: &Path,
    ffprobe_available: bool,
) -> Result<()> {
    let out = media_dir.join(&case.name);
    if let Some(spec) = &case.derive_ts_layout {
        derive_ts_layout_case(media_dir, case, spec)?;
        if ffprobe_available {
            verify_with_ffprobe(ctx, &out)?;
        }
        return Ok(());
    }

    let duration = case.duration_seconds.unwrap_or(1.4).to_string();
    let speex_path = case
        .source_audio_codecs
        .contains(&"speex")
        .then(|| generate_speex_input(ffmpeg, speexenc, media_dir, case, &duration))
        .transpose()?;
    let mut command = Command::new(ffmpeg);
    command
        .arg("-y")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(format!(
            "testsrc2=size={}x{}:rate={}:duration={duration}",
            case.width, case.height, case.fps
        ));

    for (stream, channels) in case.audio_channels.iter().enumerate() {
        if case.source_audio_codecs.get(stream) == Some(&"speex") {
            command.arg("-i").arg(
                speex_path
                    .as_ref()
                    .context("Speex input was not generated")?,
            );
        } else {
            command
                .arg("-f")
                .arg("lavfi")
                .arg("-t")
                .arg(&duration)
                .arg("-i")
                .arg(format!("anullsrc=r=48000:cl={}", channel_layout(*channels)));
        }
    }

    let subtitle_path = if case.subtitle_stream_count > 0 {
        let path = media_dir.join(format!(".{}.srt", output_stem(&case.name)?));
        fs::write(&path, "1\n00:00:00,000 --> 00:00:01,000\nfixture\n")
            .with_context(|| format!("failed to write {}", path.display()))?;
        command.arg("-f").arg("srt").arg("-i").arg(&path);
        Some(path)
    } else {
        None
    };

    command.arg("-map").arg("0:v:0");
    for stream in 0..case.audio_channels.len() {
        command.arg("-map").arg(format!("{}:a:0", stream + 1));
    }
    if subtitle_path.is_some() {
        command
            .arg("-map")
            .arg(format!("{}:s:0", case.audio_channels.len() + 1));
    }

    append_video_encoder(&mut command, case.video_codec)?;
    for (stream, codec) in case.source_audio_codecs.iter().enumerate() {
        append_audio_encoder(&mut command, codec, stream)?;
        command
            .arg(format!("-ac:a:{stream}"))
            .arg(case.audio_channels[stream].to_string());
        command
            .arg(format!("-metadata:s:a:{stream}"))
            .arg(format!("language={}", case.source_audio_languages[stream]));
    }

    if subtitle_path.is_some() {
        if matches!(output_ext(&case.name)?, "mp4" | "m4v" | "mov") {
            command
                .arg("-c:s")
                .arg("mov_text")
                .arg("-metadata:s:s:0")
                .arg("language=eng");
        } else {
            command
                .arg("-c:s")
                .arg("srt")
                .arg("-metadata:s:s:0")
                .arg("language=eng");
        }
    }

    let extension = output_ext(&case.name)?;
    append_muxer_format(&mut command, extension)?;
    command.arg("-shortest").arg(&out);

    let generate_result = crate::run_checked(&mut command);
    if let Some(path) = subtitle_path {
        let _ = fs::remove_file(path);
    }
    if let Some(path) = speex_path {
        let _ = fs::remove_file(path);
    }
    generate_result?;
    if case.asf_wave_extensible {
        rewrite_asf_pcm_as_wave_extensible(&out)?;
    }

    if ffprobe_available {
        verify_with_ffprobe(ctx, &out)?;
    }
    Ok(())
}

fn append_video_encoder(command: &mut Command, codec: &str) -> Result<()> {
    match codec {
        "h264" => {
            command
                .arg("-c:v")
                .arg("libx264")
                .arg("-preset")
                .arg("ultrafast")
                .arg("-tune")
                .arg("zerolatency")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "mpeg4" => {
            command
                .arg("-c:v")
                .arg("mpeg4")
                .arg("-q:v")
                .arg("8")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "mjpeg" => {
            command
                .arg("-c:v")
                .arg("mjpeg")
                .arg("-q:v")
                .arg("8")
                .arg("-pix_fmt")
                .arg("yuvj420p");
        }
        "mpeg2video" => {
            command
                .arg("-c:v")
                .arg("mpeg2video")
                .arg("-q:v")
                .arg("8")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "vp8" => {
            command
                .arg("-c:v")
                .arg("libvpx")
                .arg("-deadline")
                .arg("realtime")
                .arg("-cpu-used")
                .arg("8")
                .arg("-b:v")
                .arg("80k")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "vp9" => {
            command
                .arg("-c:v")
                .arg("libvpx-vp9")
                .arg("-deadline")
                .arg("realtime")
                .arg("-cpu-used")
                .arg("8")
                .arg("-b:v")
                .arg("80k")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "wmv1" | "wmv2" => {
            command
                .arg("-c:v")
                .arg(codec)
                .arg("-q:v")
                .arg("8")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "theora" => {
            command
                .arg("-c:v")
                .arg("libtheora")
                .arg("-q:v")
                .arg("5")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "flv1" => {
            command
                .arg("-c:v")
                .arg("flv")
                .arg("-q:v")
                .arg("8")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        _ => bail!("unsupported video codec {codec}"),
    }
    Ok(())
}

fn append_audio_encoder(command: &mut Command, codec: &str, stream: usize) -> Result<()> {
    let stream_spec = format!(":a:{stream}");
    match codec {
        "aac" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("aac")
                .arg(format!("-b{stream_spec}"))
                .arg("64k");
        }
        "mp3" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("libmp3lame")
                .arg(format!("-b{stream_spec}"))
                .arg("64k");
        }
        "pcm_u8" | "pcm_s16le" | "pcm_alaw" | "pcm_mulaw" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg(codec)
                .arg(format!("-ar{stream_spec}"))
                .arg("44100");
        }
        "pcm_s24le" | "pcm_s32le" | "pcm_f32le" => {
            command.arg(format!("-c{stream_spec}")).arg(codec);
        }
        "adpcm_swf" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("adpcm_swf")
                .arg(format!("-ar{stream_spec}"))
                .arg("44100");
        }
        "nellymoser" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("nellymoser")
                .arg(format!("-ar{stream_spec}"))
                .arg("16000");
        }
        "speex" => {
            command.arg(format!("-c{stream_spec}")).arg("copy");
        }
        "ac3" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("ac3")
                .arg(format!("-b{stream_spec}"))
                .arg("96k");
        }
        "eac3" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("eac3")
                .arg(format!("-b{stream_spec}"))
                .arg("96k");
        }
        "flac" => {
            command.arg(format!("-c{stream_spec}")).arg("flac");
        }
        "opus" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("libopus")
                .arg(format!("-b{stream_spec}"))
                .arg("64k");
        }
        "vorbis" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("vorbis")
                .arg(format!("-strict{stream_spec}"))
                .arg("-2");
        }
        "mp2" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("mp2")
                .arg(format!("-b{stream_spec}"))
                .arg("96k");
        }
        "dts" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("dca")
                .arg(format!("-strict{stream_spec}"))
                .arg("-2");
        }
        "wmav1" | "wmav2" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg(codec)
                .arg(format!("-b{stream_spec}"))
                .arg("64k");
        }
        _ => bail!("unsupported audio codec {codec}"),
    }
    Ok(())
}

fn append_muxer_format(command: &mut Command, extension: &str) -> Result<()> {
    match extension {
        "webm" => command.arg("-f").arg("webm"),
        "mp4" | "m4v" => command.arg("-f").arg("mp4"),
        "mov" => command.arg("-f").arg("mov"),
        "ts" | "m2ts" => command.arg("-f").arg("mpegts"),
        "avi" => command.arg("-f").arg("avi"),
        "wmv" => command.arg("-f").arg("asf"),
        "ogv" => command.arg("-f").arg("ogg"),
        "flv" => command.arg("-f").arg("flv"),
        _ => bail!("unsupported fixture extension {extension}"),
    };
    Ok(())
}

fn verify_with_ffprobe(ctx: &TaskContext, path: &Path) -> Result<()> {
    let mut command = ctx.command("ffprobe");
    command
        .arg("-v")
        .arg("error")
        .arg("-show_streams")
        .arg("-show_format")
        .arg(path)
        .stdout(Stdio::null());
    crate::run_checked(&mut command).with_context(|| format!("ffprobe rejected {}", path.display()))
}

fn output_ext(name: &str) -> Result<&str> {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .ok_or_else(|| anyhow::anyhow!("fixture name has no UTF-8 extension: {name}"))
}

fn output_stem(name: &str) -> Result<&str> {
    Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| anyhow::anyhow!("fixture name has no UTF-8 stem: {name}"))
}

fn write_simd_scan_fixture(media_dir: &Path) -> Result<()> {
    let mut data = Vec::new();

    late(&mut data, b"\xA7", 1024);
    late(&mut data, b"\x47", 512);
    late(&mut data, b"\x00\x00\x01\xB3", 1024);
    late(&mut data, b"\x00\x00\x00\x01\x67", 1024);
    late(&mut data, b"\xFF\xF1\x50\x80", 512);
    late(&mut data, b"\x56\xE0\x00\x00", 512);
    late(&mut data, b"\xFF\xE2\x00\x00", 512);
    late(&mut data, b"\x0B\x77\x00\x00", 512);
    late(&mut data, b"\x7F\xFE\x80\x01", 512);
    late(&mut data, b"\x1F\x43\xB6\x75", 1024);
    late(&mut data, b"\xA3", 512);
    late(&mut data, b"\x75\xA1", 512);
    late(&mut data, b"\xE7", 512);
    late(&mut data, b"\x4E\x01\x50\x00", 1024);
    late(&mut data, b"\xB5\x00\x3C\x00\x01\x04", 1024);
    late(&mut data, b"\x00\x00\x03\x01", 512);
    late(&mut data, b"\x00\x00\x00\x0Cmoov\x00\x00\x00\x00", 1024);

    let trailing_padding = (16 - (data.len() % 16)) % 16;
    if trailing_padding != 0 {
        pad(&mut data, trailing_padding, 0x55);
    }
    data.extend_from_slice(b"02wb");
    data.extend_from_slice(&0_u32.to_le_bytes());
    data.extend_from_slice(&128_u32.to_le_bytes());
    data.extend_from_slice(&64_u32.to_le_bytes());

    let out = media_dir.join("simd_scan_dense.bin");
    fs::write(&out, data).with_context(|| format!("failed to write {}", out.display()))
}

fn late(data: &mut Vec<u8>, payload: &[u8], pad_len: usize) {
    pad(data, pad_len, 0x55);
    data.extend_from_slice(payload);
}

fn pad(data: &mut Vec<u8>, length: usize, byte: u8) {
    data.extend(std::iter::repeat_n(byte, length));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_manifest_matches_committed_fixture_manifest() {
        let mut cases = build_matrix();
        cases.extend(build_dense_simd_cases());
        assert_eq!(cases.len(), 244);

        let manifest_path =
            xtask_support::repo_root().join("crates/scryer-mediainfo/tests/media-fixtures.toml");
        let committed = fs::read_to_string(&manifest_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", manifest_path.display()));

        assert_eq!(manifest_contents(&cases), committed);
    }

    #[test]
    fn rejects_unknown_selective_fixture_before_generation() {
        let cases = build_matrix();
        let error = match select_cases(&cases, &["does-not-exist".into()]) {
            Ok(_) => panic!("unknown fixture unexpectedly selected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("unknown media fixture"));
    }

    #[test]
    fn failed_speex_source_generation_removes_partial_wav() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "scryer-speex-cleanup-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&temp_dir).unwrap();
        let case = build_matrix()
            .into_iter()
            .find(|case| case.name == "flv_flv1_speex.flv")
            .unwrap();
        let current_exe = std::env::current_exe().unwrap();

        assert!(generate_speex_input(&current_exe, &current_exe, &temp_dir, &case, "0.1").is_err());
        assert!(!temp_dir.join(".flv_flv1_speex.speex.wav").exists());
        fs::remove_dir(temp_dir).unwrap();
    }

    #[test]
    fn committed_asf_fixture_uses_wave_format_extensible() {
        const AUDIO_MEDIA_GUID: [u8; 16] = [
            0x40, 0x9e, 0x69, 0xf8, 0x4d, 0x5b, 0xcf, 0x11, 0xa8, 0xfd, 0x00, 0x80, 0x5f, 0x5c,
            0x44, 0x2b,
        ];
        let path = xtask_support::repo_root()
            .join("crates/scryer-mediainfo/tests/media/wmv_wmv2_pcm_s24le_surround.wmv");
        let data = fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let stream_type_offset = data
            .windows(AUDIO_MEDIA_GUID.len())
            .position(|window| window == AUDIO_MEDIA_GUID)
            .expect("ASF audio stream GUID");
        let type_data_offset = stream_type_offset + 54;

        assert_eq!(read_u32_le_at(&data, stream_type_offset + 40).unwrap(), 40);
        assert_eq!(read_u16_le_at(&data, type_data_offset).unwrap(), 0xfffe);
        assert_eq!(
            read_u16_le_at(&data, type_data_offset + 24).unwrap(),
            0x0001
        );
    }

    #[test]
    fn recognizes_exact_ffmpeg_encoder_names() {
        let listing = b" V....D libtheora  libtheora Theora encoder\n";
        assert!(ffmpeg_encoder_listing_contains(listing, "libtheora"));
        assert!(!ffmpeg_encoder_listing_contains(listing, "theora"));
    }
}
