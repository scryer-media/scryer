//! Audio labels and ranking shared by catalog summaries and import scoring.

pub fn normalize_audio_codec_for_release(
    codec: Option<&str>,
    profile: Option<&str>,
) -> Option<String> {
    let profile_lower = profile.unwrap_or_default().to_ascii_lowercase();
    let codec_lower = codec.unwrap_or_default().to_ascii_lowercase();

    if profile_lower.contains("dolby truehd") && profile_lower.contains("atmos") {
        return Some("TrueHD Atmos".into());
    }
    if profile_lower.contains("dolby digital plus") && profile_lower.contains("atmos") {
        return Some("EAC3 Atmos".into());
    }
    if profile_lower.contains("dts:x") {
        return Some("DTS:X".into());
    }
    if profile_lower.contains("dts-hd ma") {
        return Some("DTS-HD MA".into());
    }
    if profile_lower.contains("dts-hd hra") {
        return Some("DTS-HD".into());
    }
    if profile_lower.contains("dts") {
        return Some("DTS".into());
    }

    if codec_lower.contains("truehd") {
        return Some("TrueHD".into());
    }
    if codec_lower.contains("e-ac-3") || codec_lower.contains("eac3") || codec_lower.contains("dd+")
    {
        return Some("EAC3".into());
    }
    if codec_lower.contains("ac-3") || codec_lower.contains("ac3") {
        return Some("AC3".into());
    }
    if codec_lower.contains("dts-hd ma") || codec_lower.contains("dts-hd master") {
        return Some("DTS-HD MA".into());
    }
    if codec_lower.contains("dts-hd") {
        return Some("DTS-HD".into());
    }
    if codec_lower.contains("dts") {
        return Some("DTS".into());
    }
    if codec_lower.contains("flac") {
        return Some("FLAC".into());
    }
    if codec_lower.contains("aac") {
        return Some("AAC".into());
    }
    if codec_lower.contains("mp3") || codec_lower.contains("mpeg audio") {
        return Some("MP3".into());
    }
    if codec_lower.contains("opus") {
        return Some("Opus".into());
    }
    if codec_lower.contains("vorbis") {
        return Some("Vorbis".into());
    }
    if codec_lower.contains("pcm") || codec_lower.contains("lpcm") {
        return Some("PCM".into());
    }

    None
}

pub fn audio_codec_rank_for_release_label(label: &str) -> i32 {
    match label {
        "TrueHD Atmos" => 100,
        "DTS:X" => 95,
        "TrueHD" => 90,
        "DTS-HD MA" => 85,
        "FLAC" => 80,
        "EAC3 Atmos" => 75,
        "EAC3" => 70,
        "DTS-HD" => 65,
        "DTS" => 60,
        "AC3" => 50,
        "AAC" | "Opus" => 40,
        "MP3" | "Vorbis" => 30,
        "PCM" => 20,
        _ => 10,
    }
}
