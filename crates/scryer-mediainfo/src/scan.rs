use std::borrow::Cow;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use std::sync::atomic::{AtomicU8, Ordering};

use core::range::Range;

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const ACCEL_UNSET: u8 = 0;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const ACCEL_AUTO: u8 = 1;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const ACCEL_OFF: u8 = 2;

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
static ACCEL_MODE: AtomicU8 = AtomicU8::new(ACCEL_UNSET);

#[allow(dead_code)]
pub(crate) fn find_byte_from(data: &[u8], needle: u8, start: usize) -> Option<usize> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            if let Some(offset) = unsafe { x86_64::find_byte_avx2(data, needle) } {
                return Some(start + offset);
            }
            return None;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            if let Some(offset) = unsafe { aarch64::find_byte_neon(data, needle) } {
                return Some(start + offset);
            }
            return None;
        }
    }

    scalar::find_byte_from(data, needle, 0).map(|offset| start + offset)
}

#[allow(dead_code)]
pub(crate) fn find_any_byte_from(data: &[u8], needles: &[u8], start: usize) -> Option<usize> {
    let data = data.get(start..)?;
    if needles.is_empty() {
        return None;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            if let Some(offset) = unsafe { x86_64::find_any_byte_avx2(data, needles) } {
                return Some(start + offset);
            }
            return None;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            if let Some(offset) = unsafe { aarch64::find_any_byte_neon(data, needles) } {
                return Some(start + offset);
            }
            return None;
        }
    }

    scalar::find_any_byte_from(data, needles, 0).map(|offset| start + offset)
}

pub(crate) fn find_mpeg_start_code(data: &[u8], code: u8) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_mpeg_start_code_avx2(data, code) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_mpeg_start_code_neon(data, code) };
        }
    }

    scalar::find_mpeg_start_code_from(data, code, 0)
}

pub(crate) fn find_annexb_start_code(data: &[u8], start: usize) -> Option<Range<usize>> {
    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_annexb_start_code_avx2(data, start) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_annexb_start_code_neon(data, start) };
        }
    }

    scalar::find_annexb_start_code_from(data, start)
}

/// Locate the exact three-byte prefix, including inside a four-byte Annex-B prefix.
/// Keeping that offset preserves the boundaries used by metadata and PES walkers.
pub(crate) fn find_start_code_prefix(data: &[u8], start: usize) -> Option<usize> {
    data.get(start..)?;
    find_annexb_start_code(data, start).map(|range| range.end - 3)
}

pub(crate) fn find_program_start_code(data: &[u8], start: usize) -> Option<usize> {
    let mut at = start;
    while let Some(found) = find_start_code_prefix(data, at) {
        if *data.get(found + 3)? >= 0xb9 {
            return Some(found);
        }
        at = found + 3;
    }
    None
}

pub(crate) fn find_syncword(data: &[u8], word: u32, start: usize) -> Option<usize> {
    let word = word.to_be_bytes();
    let mut at = start;
    while let Some(found) = find_byte_from(data, word[0], at) {
        let suffix = &data[found..];
        if suffix.len() < 4 {
            return None;
        }
        if suffix[..4] == word {
            return Some(found);
        }
        at = found + 1;
    }
    None
}

/// Collect several fixed signatures in one pass, validating only SIMD candidates.
pub(crate) fn syncword_presence<const N: usize>(data: &[u8], words: [u32; N]) -> [bool; N] {
    let patterns = words.map(u32::to_be_bytes);
    let needles = patterns.map(|word| word[0]);
    let mut found = [false; N];
    let mut at = 0;
    while let Some(candidate) = find_any_byte_from(data, &needles, at) {
        let suffix = &data[candidate..];
        if suffix.len() < 4 {
            break;
        }
        for (index, pattern) in patterns.iter().enumerate() {
            found[index] |= suffix[..4] == *pattern;
        }
        if found.iter().all(|found| *found) {
            break;
        }
        at = candidate + 1;
    }
    found
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AudioSyncKind {
    Adts,
    Latm,
    MpegAudio,
    Ac3,
    Dts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AudioSyncCandidate {
    pub(crate) offset: usize,
    pub(crate) kind: AudioSyncKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EbmlCandidate {
    pub(crate) offset: usize,
    pub(crate) id: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LengthPrefixedNalCandidate {
    pub(crate) offset: usize,
    pub(crate) nal_type: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Mp4BoxNameCandidate {
    pub(crate) offset: usize,
    pub(crate) name: [u8; 4],
}

#[allow(dead_code)]
pub(crate) fn find_audio_sync_candidate(data: &[u8], start: usize) -> Option<AudioSyncCandidate> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_audio_sync_candidate_avx2(data) }.map(|candidate| {
                AudioSyncCandidate {
                    offset: start + candidate.offset,
                    kind: candidate.kind,
                }
            });
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_audio_sync_candidate_neon(data) }.map(|candidate| {
                AudioSyncCandidate {
                    offset: start + candidate.offset,
                    kind: candidate.kind,
                }
            });
        }
    }

    scalar::find_audio_sync_candidate_from(data, 0).map(|candidate| AudioSyncCandidate {
        offset: start + candidate.offset,
        kind: candidate.kind,
    })
}

pub(crate) fn find_ebml_candidate(data: &[u8], start: usize, ids: &[u32]) -> Option<EbmlCandidate> {
    let data = data.get(start..)?;
    if ids.is_empty() {
        return None;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_ebml_candidate_avx2(data, ids) }.map(|candidate| {
                EbmlCandidate {
                    offset: start + candidate.offset,
                    id: candidate.id,
                }
            });
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_ebml_candidate_neon(data, ids) }.map(|candidate| {
                EbmlCandidate {
                    offset: start + candidate.offset,
                    id: candidate.id,
                }
            });
        }
    }

    scalar::find_ebml_candidate_from(data, 0, ids).map(|candidate| EbmlCandidate {
        offset: start + candidate.offset,
        id: candidate.id,
    })
}

pub(crate) fn find_hevc_sei_nal_header_candidate(
    data: &[u8],
    start: usize,
) -> Option<LengthPrefixedNalCandidate> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_hevc_sei_nal_header_candidate_avx2(data) }.map(
                |candidate| LengthPrefixedNalCandidate {
                    offset: start + candidate.offset,
                    nal_type: candidate.nal_type,
                },
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_hevc_sei_nal_header_candidate_neon(data) }.map(
                |candidate| LengthPrefixedNalCandidate {
                    offset: start + candidate.offset,
                    nal_type: candidate.nal_type,
                },
            );
        }
    }

    scalar::find_hevc_sei_nal_header_candidate_from(data, 0).map(|candidate| {
        LengthPrefixedNalCandidate {
            offset: start + candidate.offset,
            nal_type: candidate.nal_type,
        }
    })
}

pub(crate) fn find_hdr10plus_itu_t35_candidate(data: &[u8], start: usize) -> Option<usize> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_hdr10plus_itu_t35_candidate_avx2(data) }
                .map(|offset| start + offset);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_hdr10plus_itu_t35_candidate_neon(data) }
                .map(|offset| start + offset);
        }
    }

    scalar::find_hdr10plus_itu_t35_candidate_from(data, 0).map(|offset| start + offset)
}

pub(crate) fn find_mp4_box_name_candidate(
    data: &[u8],
    start: usize,
    names: &[[u8; 4]],
) -> Option<Mp4BoxNameCandidate> {
    if names.is_empty() || data.len() < 8 {
        return None;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_mp4_box_name_candidate_avx2(data, start, names) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_mp4_box_name_candidate_neon(data, start, names) };
        }
    }

    scalar::find_mp4_box_name_candidate_from(data, start, names)
}

#[allow(dead_code)]
pub(crate) fn find_avi_idx1_stream_prefix(
    data: &[u8],
    start: usize,
    stream_number: usize,
) -> Option<usize> {
    let prefix = avi_idx1_stream_prefix(stream_number)?;
    let mut cursor = start;
    while let Some(offset) = find_pair_from(data, cursor, prefix[0], 0xFF, prefix[1]) {
        if offset % 16 == 0 {
            return Some(offset);
        }
        cursor = offset + 1;
    }
    None
}

pub(crate) fn accumulate_avi_idx1_stream_sizes(data: &[u8], stream_sizes: &mut [u64]) {
    #[cfg(target_arch = "x86_64")]
    if data.len() >= 128 && accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
        // The kernel loads only complete index entries and retains scalar scatter semantics.
        unsafe { x86_64::accumulate_avi_idx1_avx2(data, stream_sizes) };
        return;
    }
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    if data.len() >= 64 && accel_runtime_enabled() && aarch64_neon_available() {
        unsafe { aarch64::accumulate_avi_idx1_neon(data, stream_sizes) };
        return;
    }
    scalar::accumulate_avi_idx1_stream_sizes(data, stream_sizes);
}

fn accumulate_avi_lanes<const N: usize>(ids: [u32; N], sizes: [u32; N], totals: &mut [u64]) {
    // Consecutive entries often belong to one stream. Reduce the batch before
    // touching its accumulator to avoid a serial load/store chain for every lane.
    if let Some((&id, rest)) = ids.split_first()
        && rest.iter().all(|&other| other == id)
    {
        if let Some(total) = totals.get_mut(id as usize) {
            let size = sizes.into_iter().map(u64::from).sum::<u64>();
            *total = total.saturating_add(size);
        }
        return;
    }
    for (id, size) in ids.into_iter().zip(sizes) {
        if let Some(total) = totals.get_mut(id as usize) {
            *total = total.saturating_add(u64::from(size));
        }
    }
}

pub(crate) fn score_ts_packet_layout(
    data: &[u8],
    raw_packet_size: usize,
    sync_offset: usize,
    sync_byte: u8,
) -> usize {
    if raw_packet_size == 0 || sync_offset >= raw_packet_size || data.len() < raw_packet_size {
        return 0;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe {
                x86_64::score_ts_packet_layout_avx2(data, raw_packet_size, sync_offset, sync_byte)
            };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe {
                aarch64::score_ts_packet_layout_neon(data, raw_packet_size, sync_offset, sync_byte)
            };
        }
    }

    scalar::score_ts_packet_layout(data, raw_packet_size, sync_offset, sync_byte)
}

pub(crate) fn find_h2645_emulation_prevention_byte(data: &[u8], start: usize) -> Option<usize> {
    if data.len() < 3 || start >= data.len() {
        return None;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_h2645_emulation_prevention_byte_avx2(data, start) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_h2645_emulation_prevention_byte_neon(data, start) };
        }
    }

    scalar::find_h2645_emulation_prevention_byte_from(data, start)
}

pub(crate) fn h2645_unescape_rbsp(data: &[u8]) -> Cow<'_, [u8]> {
    let Some(mut epb_offset) = find_h2645_emulation_prevention_byte(data, 2) else {
        return Cow::Borrowed(data);
    };

    let mut out = Vec::with_capacity(data.len());
    let mut cursor = 0usize;
    loop {
        out.extend_from_slice(&data[cursor..epb_offset]);
        cursor = epb_offset + 1;
        let Some(next_offset) = find_h2645_emulation_prevention_byte(data, cursor.max(2)) else {
            break;
        };
        epb_offset = next_offset;
    }
    out.extend_from_slice(&data[cursor..]);
    Cow::Owned(out)
}

fn audio_sync_kind_at(data: &[u8], offset: usize) -> Option<AudioSyncKind> {
    if offset + 2 <= data.len() {
        let first = data[offset];
        let second = data[offset + 1];
        if first == 0xFF && second & 0xF0 == 0xF0 {
            return Some(AudioSyncKind::Adts);
        }
        if first == 0x56 && second & 0xE0 == 0xE0 {
            return Some(AudioSyncKind::Latm);
        }
        if first == 0xFF && second & 0xE0 == 0xE0 {
            return Some(AudioSyncKind::MpegAudio);
        }
        if first == 0x0B && second == 0x77 {
            return Some(AudioSyncKind::Ac3);
        }
    }

    if offset + 4 <= data.len() {
        let marker = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?);
        if matches!(
            marker,
            0x7FFE_8001 | 0xFE7F_0180 | 0x1FFF_E800 | 0xFF1F_00E8
        ) {
            return Some(AudioSyncKind::Dts);
        }
    }

    None
}

fn ebml_id_bytes(id: u32) -> ([u8; 4], usize) {
    let bytes = id.to_be_bytes();
    let first = bytes.iter().position(|&byte| byte != 0).unwrap_or(3);
    let mut out = [0_u8; 4];
    let len = 4 - first;
    out[..len].copy_from_slice(&bytes[first..]);
    (out, len)
}

fn ebml_candidate_at(data: &[u8], offset: usize, ids: &[u32]) -> Option<EbmlCandidate> {
    for &id in ids {
        let (bytes, len) = ebml_id_bytes(id);
        if len > 0 && offset + len <= data.len() && data[offset..offset + len] == bytes[..len] {
            return Some(EbmlCandidate { offset, id });
        }
    }
    None
}

fn hevc_sei_nal_candidate_at(data: &[u8], offset: usize) -> Option<LengthPrefixedNalCandidate> {
    let byte = *data.get(offset)?;
    let nal_type = (byte >> 1) & 0x3F;
    matches!(nal_type, 39 | 40).then_some(LengthPrefixedNalCandidate { offset, nal_type })
}

fn audio_sync_candidate_at(data: &[u8], offset: usize) -> Option<AudioSyncCandidate> {
    Some(AudioSyncCandidate {
        offset,
        kind: audio_sync_kind_at(data, offset)?,
    })
}

fn mp4_box_name_candidate_at(
    data: &[u8],
    name_offset: usize,
    names: &[[u8; 4]],
) -> Option<Mp4BoxNameCandidate> {
    if name_offset < 4 || name_offset + 4 > data.len() {
        return None;
    }
    let name: [u8; 4] = data[name_offset..name_offset + 4].try_into().ok()?;
    names.contains(&name).then_some(Mp4BoxNameCandidate {
        offset: name_offset - 4,
        name,
    })
}

#[allow(dead_code)]
fn avi_idx1_stream_prefix(stream_number: usize) -> Option<[u8; 2]> {
    (stream_number < 100).then_some([
        b'0' + (stream_number / 10) as u8,
        b'0' + (stream_number % 10) as u8,
    ])
}

fn avi_idx1_decimal_stream_number(first: u8, second: u8) -> Option<usize> {
    (first.is_ascii_digit() && second.is_ascii_digit())
        .then(|| usize::from(first - b'0') * 10 + usize::from(second - b'0'))
}

fn avi_idx1_entry_size(entry: &[u8]) -> u32 {
    u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]])
}

#[allow(dead_code)]
pub(crate) fn find_adts_sync(data: &[u8], start: usize) -> Option<usize> {
    find_pair_from(data, start, 0xFF, 0xF0, 0xF0)
}

#[allow(dead_code)]
pub(crate) fn find_latm_sync(data: &[u8], start: usize) -> Option<usize> {
    find_pair_from(data, start, 0x56, 0xE0, 0xE0)
}

#[allow(dead_code)]
pub(crate) fn find_mpeg_audio_sync(data: &[u8], start: usize) -> Option<usize> {
    find_pair_from(data, start, 0xFF, 0xE0, 0xE0)
}

#[allow(dead_code)]
pub(crate) fn find_ac3_sync(data: &[u8], start: usize) -> Option<usize> {
    find_pair_from(data, start, 0x0B, 0xFF, 0x77)
}

#[allow(dead_code)]
pub(crate) fn find_dts_sync(data: &[u8], start: usize) -> Option<usize> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_dts_sync_avx2(data) }.map(|offset| start + offset);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_dts_sync_neon(data) }.map(|offset| start + offset);
        }
    }

    scalar::find_dts_sync_from(data, 0).map(|offset| start + offset)
}

#[allow(dead_code)]
pub(crate) fn find_pair_from(
    data: &[u8],
    start: usize,
    first: u8,
    second_mask: u8,
    second_value: u8,
) -> Option<usize> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_pair_avx2(data, first, second_mask, second_value) }
                .map(|offset| start + offset);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_pair_neon(data, first, second_mask, second_value) }
                .map(|offset| start + offset);
        }
    }

    scalar::find_pair_from(data, 0, first, second_mask, second_value).map(|offset| start + offset)
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn accel_runtime_enabled() -> bool {
    match ACCEL_MODE.load(Ordering::Relaxed) {
        ACCEL_AUTO => true,
        ACCEL_OFF => false,
        _ => {
            let disabled = std::env::var_os("SCRYER_MEDIAINFO_ACCEL")
                .map(|value| {
                    let value = value.to_string_lossy();
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "off" | "0" | "false" | "no"
                    )
                })
                .unwrap_or(false);
            let mode = if disabled { ACCEL_OFF } else { ACCEL_AUTO };
            ACCEL_MODE.store(mode, Ordering::Relaxed);
            !disabled
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
fn aarch64_neon_available() -> bool {
    std::arch::is_aarch64_feature_detected!("neon")
}

pub(crate) mod scalar {
    use super::{
        AudioSyncCandidate, EbmlCandidate, LengthPrefixedNalCandidate, Mp4BoxNameCandidate, Range,
    };

    pub(crate) fn find_byte_from(data: &[u8], needle: u8, start: usize) -> Option<usize> {
        data.get(start..)?
            .iter()
            .position(|&byte| byte == needle)
            .map(|offset| start + offset)
    }

    pub(crate) fn find_any_byte_from(data: &[u8], needles: &[u8], start: usize) -> Option<usize> {
        if needles.is_empty() {
            return None;
        }
        data.get(start..)?
            .iter()
            .position(|byte| needles.contains(byte))
            .map(|offset| start + offset)
    }

    #[allow(dead_code)]
    pub(crate) fn find_pair_from(
        data: &[u8],
        start: usize,
        first: u8,
        second_mask: u8,
        second_value: u8,
    ) -> Option<usize> {
        let mut pos = start;
        while pos + 2 <= data.len() {
            let i = find_byte_from(data, first, pos)?;
            if i + 2 > data.len() {
                return None;
            }
            if data[i + 1] & second_mask == second_value {
                return Some(i);
            }
            pos = i + 1;
        }
        None
    }

    pub(crate) fn accumulate_avi_idx1_stream_sizes(data: &[u8], stream_sizes: &mut [u64]) {
        for entry in data.chunks_exact(16) {
            let Some(stream_number) = super::avi_idx1_decimal_stream_number(entry[0], entry[1])
            else {
                continue;
            };
            if let Some(total) = stream_sizes.get_mut(stream_number) {
                *total = total.saturating_add(u64::from(super::avi_idx1_entry_size(entry)));
            }
        }
    }

    pub(crate) fn score_ts_packet_layout(
        data: &[u8],
        raw_packet_size: usize,
        sync_offset: usize,
        sync_byte: u8,
    ) -> usize {
        score_ts_packet_layout_from(data, raw_packet_size, sync_offset, sync_byte, 0)
    }

    pub(crate) fn score_ts_packet_layout_from(
        data: &[u8],
        raw_packet_size: usize,
        sync_offset: usize,
        sync_byte: u8,
        packet_start: usize,
    ) -> usize {
        if raw_packet_size == 0 || sync_offset >= raw_packet_size || data.len() < raw_packet_size {
            return 0;
        }

        let mut score = 0usize;
        let mut offset = packet_start;
        while offset + raw_packet_size <= data.len() {
            if data[offset + sync_offset] == sync_byte {
                score += 1;
            }
            offset += raw_packet_size;
        }
        score
    }

    pub(crate) fn find_h2645_emulation_prevention_byte_from(
        data: &[u8],
        start: usize,
    ) -> Option<usize> {
        let mut pos = start.max(2);
        while pos < data.len() {
            let offset = find_byte_from(data, 0x03, pos)?;
            if offset >= 2 && data[offset - 2] == 0 && data[offset - 1] == 0 {
                return Some(offset);
            }
            pos = offset + 1;
        }
        None
    }

    pub(crate) fn find_mpeg_start_code_from(data: &[u8], code: u8, start: usize) -> Option<usize> {
        find_mpeg_start_code_with(data, code, start, find_byte_from)
    }

    pub(crate) fn find_annexb_start_code_from(data: &[u8], start: usize) -> Option<Range<usize>> {
        find_annexb_start_code_with(data, start, find_byte_from)
    }

    #[allow(dead_code)]
    pub(crate) fn find_dts_sync_from(data: &[u8], start: usize) -> Option<usize> {
        let mut pos = start;
        while pos + 4 <= data.len() {
            let i = find_any_byte_from(data, &[0x7F, 0xFE, 0x1F, 0xFF], pos)?;
            if i + 4 > data.len() {
                return None;
            }
            let marker = u32::from_be_bytes(data[i..i + 4].try_into().ok()?);
            if matches!(
                marker,
                0x7FFE_8001 | 0xFE7F_0180 | 0x1FFF_E800 | 0xFF1F_00E8
            ) {
                return Some(i);
            }
            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_audio_sync_candidate_from(
        data: &[u8],
        start: usize,
    ) -> Option<AudioSyncCandidate> {
        let mut pos = start;
        while pos + 2 <= data.len() {
            let i = find_any_byte_from(data, &[0xFF, 0x56, 0x0B, 0x7F, 0xFE, 0x1F], pos)?;
            if let Some(candidate) = super::audio_sync_candidate_at(data, i) {
                return Some(candidate);
            }

            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_ebml_candidate_from(
        data: &[u8],
        start: usize,
        ids: &[u32],
    ) -> Option<EbmlCandidate> {
        if ids.is_empty() {
            return None;
        }
        let mut first_bytes = [0_u8; 8];
        let mut first_count = 0usize;
        for &id in ids {
            let (bytes, len) = super::ebml_id_bytes(id);
            if len == 0 || first_bytes[..first_count].contains(&bytes[0]) {
                continue;
            }
            if first_count < first_bytes.len() {
                first_bytes[first_count] = bytes[0];
                first_count += 1;
            }
        }

        let mut pos = start;
        while pos < data.len() {
            let i = find_any_byte_from(data, &first_bytes[..first_count], pos)?;
            if let Some(candidate) = super::ebml_candidate_at(data, i, ids) {
                return Some(candidate);
            }
            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_hevc_sei_nal_header_candidate_from(
        data: &[u8],
        start: usize,
    ) -> Option<LengthPrefixedNalCandidate> {
        let mut pos = start;
        while pos < data.len() {
            let i = find_any_byte_from(data, &[0x4E, 0x4F, 0x50, 0x51], pos)?;
            if let Some(candidate) = super::hevc_sei_nal_candidate_at(data, i) {
                return Some(candidate);
            }
            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_hdr10plus_itu_t35_candidate_from(
        data: &[u8],
        start: usize,
    ) -> Option<usize> {
        let pattern = [0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04];
        let mut pos = start;
        while pos + pattern.len() <= data.len() {
            let i = find_byte_from(data, pattern[0], pos)?;
            if i + pattern.len() > data.len() {
                return None;
            }
            if data[i..i + pattern.len()] == pattern {
                return Some(i);
            }
            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_mp4_box_name_candidate_from(
        data: &[u8],
        start: usize,
        names: &[[u8; 4]],
    ) -> Option<Mp4BoxNameCandidate> {
        if names.is_empty() || data.len() < 8 {
            return None;
        }
        let mut first_bytes = [0_u8; 16];
        let mut first_count = 0usize;
        for name in names {
            if !first_bytes[..first_count].contains(&name[0]) {
                if first_count == first_bytes.len() {
                    return find_mp4_box_name_candidate_by_name(data, start, names);
                }
                first_bytes[first_count] = name[0];
                first_count += 1;
            }
        }

        let mut pos = start.saturating_add(4);
        while pos + 4 <= data.len() {
            let i = find_any_byte_from(data, &first_bytes[..first_count], pos)?;
            if i + 4 > data.len() {
                return None;
            }
            if let Some(candidate) = super::mp4_box_name_candidate_at(data, i, names) {
                return Some(candidate);
            }
            pos = i + 1;
        }
        None
    }

    fn find_mp4_box_name_candidate_by_name(
        data: &[u8],
        start: usize,
        names: &[[u8; 4]],
    ) -> Option<Mp4BoxNameCandidate> {
        let mut best: Option<Mp4BoxNameCandidate> = None;
        for name in names {
            let mut pos = start.saturating_add(4);
            while pos + 4 <= data.len() {
                let i = find_byte_from(data, name[0], pos)?;
                if i + 4 > data.len() {
                    break;
                }
                if data[i..i + 4] == *name
                    && let Some(candidate) = super::mp4_box_name_candidate_at(data, i, names)
                    && best.is_none_or(|existing| candidate.offset < existing.offset)
                {
                    best = Some(candidate);
                    break;
                }
                pos = i + 1;
            }
        }
        best
    }

    fn find_mpeg_start_code_with(
        data: &[u8],
        code: u8,
        start: usize,
        mut find_zero: impl FnMut(&[u8], u8, usize) -> Option<usize>,
    ) -> Option<usize> {
        let mut pos = start;
        while pos + 4 <= data.len() {
            let i = find_zero(data, 0, pos)?;
            if i + 4 > data.len() {
                return None;
            }
            if data[i + 1] == 0 && data[i + 2] == 1 && data[i + 3] == code {
                return Some(i);
            }
            pos = i + 1;
        }
        None
    }

    fn find_annexb_start_code_with(
        data: &[u8],
        start: usize,
        mut find_zero: impl FnMut(&[u8], u8, usize) -> Option<usize>,
    ) -> Option<Range<usize>> {
        let mut pos = start;
        while pos + 3 <= data.len() {
            let i = find_zero(data, 0, pos)?;
            if i + 3 <= data.len() && data[i + 1] == 0 && data[i + 2] == 1 {
                return Some(Range {
                    start: i,
                    end: i + 3,
                });
            }
            if i + 4 <= data.len() && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
                return Some(Range {
                    start: i,
                    end: i + 4,
                });
            }
            pos = i + 1;
        }
        None
    }
}

#[cfg(target_arch = "x86_64")]
mod x86_64 {
    use super::{AudioSyncCandidate, Range};
    use std::arch::x86_64::*;

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn accumulate_avi_idx1_avx2(data: &[u8], totals: &mut [u64]) {
        let byte_mask = _mm256_set1_epi32(255);
        let ascii_zero = _mm256_set1_epi32(48);
        let nine = _mm256_set1_epi32(9);
        let minus_one = _mm256_set1_epi32(-1);
        let mut at = 0;
        while data.len() - at >= 128 {
            // Four contiguous loads cover eight complete entries. Transpose headers and
            // sizes into matching lanes; saturating addition does not depend on lane order.
            let (a, b, c, d) = unsafe {
                (
                    _mm256_loadu_si256(data.as_ptr().add(at).cast()),
                    _mm256_loadu_si256(data.as_ptr().add(at + 32).cast()),
                    _mm256_loadu_si256(data.as_ptr().add(at + 64).cast()),
                    _mm256_loadu_si256(data.as_ptr().add(at + 96).cast()),
                )
            };
            let headers =
                _mm256_unpacklo_epi64(_mm256_unpacklo_epi32(a, b), _mm256_unpacklo_epi32(c, d));
            let sizes =
                _mm256_unpackhi_epi64(_mm256_unpackhi_epi32(a, b), _mm256_unpackhi_epi32(c, d));
            let first = _mm256_sub_epi32(_mm256_and_si256(headers, byte_mask), ascii_zero);
            let second = _mm256_sub_epi32(
                _mm256_and_si256(_mm256_srli_epi32::<8>(headers), byte_mask),
                ascii_zero,
            );
            let invalid = _mm256_or_si256(
                _mm256_or_si256(
                    _mm256_cmpgt_epi32(first, nine),
                    _mm256_cmpgt_epi32(second, nine),
                ),
                _mm256_or_si256(
                    _mm256_cmpgt_epi32(_mm256_setzero_si256(), first),
                    _mm256_cmpgt_epi32(_mm256_setzero_si256(), second),
                ),
            );
            let ids = _mm256_add_epi32(_mm256_mullo_epi32(first, _mm256_set1_epi32(10)), second);
            let ids = _mm256_blendv_epi8(ids, minus_one, invalid);
            let mut id_lanes = [0_u32; 8];
            let mut size_lanes = [0_u32; 8];
            unsafe {
                _mm256_storeu_si256(id_lanes.as_mut_ptr().cast(), ids);
                _mm256_storeu_si256(size_lanes.as_mut_ptr().cast(), sizes);
            }
            super::accumulate_avi_lanes(id_lanes, size_lanes, totals);
            at += 128;
        }
        super::scalar::accumulate_avi_idx1_stream_sizes(&data[at..], totals);
    }

    #[target_feature(enable = "avx2")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_byte_avx2(data: &[u8], needle: u8) -> Option<usize> {
        let needle_vec = _mm256_set1_epi8(needle as i8);
        let mut offset = 0;
        while offset + 32 <= data.len() {
            let chunk = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, needle_vec)) as u32;
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }
        super::scalar::find_byte_from(data, needle, offset)
    }

    #[target_feature(enable = "avx2")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_any_byte_avx2(data: &[u8], needles: &[u8]) -> Option<usize> {
        if needles.len() == 1 {
            return unsafe { find_byte_avx2(data, needles[0]) };
        }

        let mut offset = 0;
        while offset + 32 <= data.len() {
            let chunk = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let mut mask = 0_u32;
            for &needle in needles {
                let needle = _mm256_set1_epi8(needle as i8);
                mask |= _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, needle)) as u32;
            }
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }
        super::scalar::find_any_byte_from(data, needles, offset)
    }

    #[target_feature(enable = "avx2")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_pair_avx2(
        data: &[u8],
        first: u8,
        second_mask: u8,
        second_value: u8,
    ) -> Option<usize> {
        let first_vec = _mm256_set1_epi8(first as i8);
        let second_mask_vec = _mm256_set1_epi8(second_mask as i8);
        let second_value_vec = _mm256_set1_epi8(second_value as i8);
        let mut offset = 0;
        while offset + 33 <= data.len() {
            let first_chunk = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let second_chunk = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let mask = _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_cmpeq_epi8(first_chunk, first_vec),
                _mm256_cmpeq_epi8(
                    _mm256_and_si256(second_chunk, second_mask_vec),
                    second_value_vec,
                ),
            )) as u32;
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }
        super::scalar::find_pair_from(data, offset, first, second_mask, second_value)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_mpeg_start_code_avx2(data: &[u8], code: u8) -> Option<usize> {
        let zero = _mm256_setzero_si256();
        let one = _mm256_set1_epi8(1);
        let code_vec = _mm256_set1_epi8(code as i8);
        let mut offset = 0;
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let mask = _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_and_si256(_mm256_cmpeq_epi8(b0, zero), _mm256_cmpeq_epi8(b1, zero)),
                _mm256_and_si256(_mm256_cmpeq_epi8(b2, one), _mm256_cmpeq_epi8(b3, code_vec)),
            )) as u32;
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }
        super::scalar::find_mpeg_start_code_from(data, code, offset)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_annexb_start_code_avx2(
        data: &[u8],
        start: usize,
    ) -> Option<Range<usize>> {
        let data = data.get(start..)?;
        let zero = _mm256_setzero_si256();
        let one = _mm256_set1_epi8(1);
        let mut offset = 0;
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let first_two =
                _mm256_and_si256(_mm256_cmpeq_epi8(b0, zero), _mm256_cmpeq_epi8(b1, zero));
            let three = _mm256_and_si256(first_two, _mm256_cmpeq_epi8(b2, one));
            let four = _mm256_and_si256(
                first_two,
                _mm256_and_si256(_mm256_cmpeq_epi8(b2, zero), _mm256_cmpeq_epi8(b3, one)),
            );
            let mask = _mm256_movemask_epi8(_mm256_or_si256(three, four)) as u32;
            if mask != 0 {
                let found = offset + mask.trailing_zeros() as usize;
                let len = if data[found + 2] == 1 { 3 } else { 4 };
                let range_start = start + found;
                return Some(Range {
                    start: range_start,
                    end: range_start + len,
                });
            }
            offset += 32;
        }
        super::scalar::find_annexb_start_code_from(data, offset).map(|range| Range {
            start: start + range.start,
            end: start + range.end,
        })
    }

    #[target_feature(enable = "avx2")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_dts_sync_avx2(data: &[u8]) -> Option<usize> {
        let mut offset = 0;
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let mask = unsafe { dts_word_mask(b0, b1, b2, b3) };
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }
        super::scalar::find_dts_sync_from(data, offset)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_audio_sync_candidate_avx2(data: &[u8]) -> Option<AudioSyncCandidate> {
        let ff = _mm256_set1_epi8(0xFF_u8 as i8);
        let f0_mask = _mm256_set1_epi8(0xF0_u8 as i8);
        let f0 = _mm256_set1_epi8(0xF0_u8 as i8);
        let e0_mask = _mm256_set1_epi8(0xE0_u8 as i8);
        let e0 = _mm256_set1_epi8(0xE0_u8 as i8);
        let latm_first = _mm256_set1_epi8(0x56);
        let ac3_first = _mm256_set1_epi8(0x0B);
        let ac3_second = _mm256_set1_epi8(0x77);

        let mut offset = 0;
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };

            let first_ff = _mm256_cmpeq_epi8(b0, ff);
            let adts = _mm256_and_si256(
                first_ff,
                _mm256_cmpeq_epi8(_mm256_and_si256(b1, f0_mask), f0),
            );
            let mpeg_audio = _mm256_and_si256(
                first_ff,
                _mm256_cmpeq_epi8(_mm256_and_si256(b1, e0_mask), e0),
            );
            let latm = _mm256_and_si256(
                _mm256_cmpeq_epi8(b0, latm_first),
                _mm256_cmpeq_epi8(_mm256_and_si256(b1, e0_mask), e0),
            );
            let ac3 = _mm256_and_si256(
                _mm256_cmpeq_epi8(b0, ac3_first),
                _mm256_cmpeq_epi8(b1, ac3_second),
            );
            let dts = unsafe { dts_word_mask(b0, b1, b2, b3) };

            let pair_mask = _mm256_movemask_epi8(_mm256_or_si256(
                _mm256_or_si256(adts, mpeg_audio),
                _mm256_or_si256(latm, ac3),
            )) as u32;
            let mask = pair_mask | dts;
            if mask != 0 {
                let found = offset + mask.trailing_zeros() as usize;
                return super::audio_sync_candidate_at(data, found);
            }
            offset += 32;
        }

        super::scalar::find_audio_sync_candidate_from(data, offset)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_ebml_candidate_avx2(
        data: &[u8],
        ids: &[u32],
    ) -> Option<super::EbmlCandidate> {
        if ids.len() > 8 {
            return super::scalar::find_ebml_candidate_from(data, 0, ids);
        }
        let mut specs = [([0_u8; 4], 0usize, 0_u32); 8];
        let mut spec_count = 0usize;
        for &id in ids.iter().take(specs.len()) {
            let (bytes, len) = super::ebml_id_bytes(id);
            specs[spec_count] = (bytes, len, id);
            spec_count += 1;
        }

        let mut offset = 0;
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let mut mask = 0_u32;
            for &(bytes, len, _) in &specs[..spec_count] {
                mask |= unsafe { byte_sequence_mask(b0, b1, b2, b3, bytes, len) };
            }
            if mask != 0 {
                let found = offset + mask.trailing_zeros() as usize;
                return super::ebml_candidate_at(data, found, ids);
            }
            offset += 32;
        }

        super::scalar::find_ebml_candidate_from(data, offset, ids)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_hevc_sei_nal_header_candidate_avx2(
        data: &[u8],
    ) -> Option<super::LengthPrefixedNalCandidate> {
        let candidates = [0x4E_u8, 0x4F, 0x50, 0x51];
        let mut offset = 0;
        while offset + 32 <= data.len() {
            let chunk = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let mut mask = 0_u32;
            for &candidate in &candidates {
                mask |= _mm256_movemask_epi8(_mm256_cmpeq_epi8(
                    chunk,
                    _mm256_set1_epi8(candidate as i8),
                )) as u32;
            }
            if mask != 0 {
                let found = offset + mask.trailing_zeros() as usize;
                return super::hevc_sei_nal_candidate_at(data, found);
            }
            offset += 32;
        }

        super::scalar::find_hevc_sei_nal_header_candidate_from(data, offset)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_hdr10plus_itu_t35_candidate_avx2(data: &[u8]) -> Option<usize> {
        let mut offset = 0;
        while offset + 37 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let b4 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 4).cast()) };
            let b5 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 5).cast()) };
            let first_four = unsafe { word_mask(b0, b1, b2, b3, [0xB5, 0x00, 0x3C, 0x00]) };
            let last_two = _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_cmpeq_epi8(b4, _mm256_set1_epi8(0x01)),
                _mm256_cmpeq_epi8(b5, _mm256_set1_epi8(0x04)),
            )) as u32;
            let mask = first_four & last_two;
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }

        super::scalar::find_hdr10plus_itu_t35_candidate_from(data, offset)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_mp4_box_name_candidate_avx2(
        data: &[u8],
        start: usize,
        names: &[[u8; 4]],
    ) -> Option<super::Mp4BoxNameCandidate> {
        let mut offset = start.saturating_add(4);
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let mut mask = 0_u32;
            for &name in names {
                mask |= unsafe { word_mask(b0, b1, b2, b3, name) };
            }
            if mask != 0 {
                let name_offset = offset + mask.trailing_zeros() as usize;
                return super::mp4_box_name_candidate_at(data, name_offset, names);
            }
            offset += 32;
        }

        super::scalar::find_mp4_box_name_candidate_from(data, offset.saturating_sub(4), names)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn score_ts_packet_layout_avx2(
        data: &[u8],
        raw_packet_size: usize,
        sync_offset: usize,
        sync_byte: u8,
    ) -> usize {
        if raw_packet_size == 0
            || sync_offset >= raw_packet_size
            || data.len() > i32::MAX as usize
            || raw_packet_size > i32::MAX as usize / 8
        {
            return super::scalar::score_ts_packet_layout(
                data,
                raw_packet_size,
                sync_offset,
                sync_byte,
            );
        }

        let stride = raw_packet_size as i32;
        let lane_offsets = _mm256_setr_epi32(
            0,
            stride,
            stride * 2,
            stride * 3,
            stride * 4,
            stride * 5,
            stride * 6,
            stride * 7,
        );
        let byte_mask = _mm256_set1_epi32(0xFF);
        let sync = _mm256_set1_epi32(i32::from(sync_byte));

        let mut score = 0usize;
        let mut packet_start = 0usize;
        while packet_start + raw_packet_size * 8 <= data.len() {
            let base = packet_start + sync_offset;
            let last_sync = packet_start + raw_packet_size * 7 + sync_offset;
            if last_sync + 4 > data.len() {
                break;
            }

            let offsets = _mm256_add_epi32(_mm256_set1_epi32(base as i32), lane_offsets);
            let gathered =
                unsafe { _mm256_i32gather_epi32::<1>(data.as_ptr().cast::<i32>(), offsets) };
            let cmp = _mm256_cmpeq_epi32(_mm256_and_si256(gathered, byte_mask), sync);
            let mask = _mm256_movemask_ps(_mm256_castsi256_ps(cmp)) as u32;
            score += mask.count_ones() as usize;
            packet_start += raw_packet_size * 8;
        }

        score
            + super::scalar::score_ts_packet_layout_from(
                data,
                raw_packet_size,
                sync_offset,
                sync_byte,
                packet_start,
            )
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_h2645_emulation_prevention_byte_avx2(
        data: &[u8],
        start: usize,
    ) -> Option<usize> {
        let zero = _mm256_setzero_si256();
        let three = _mm256_set1_epi8(3);
        let mut offset = start.saturating_sub(2);
        while offset + 34 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let mut mask = _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_and_si256(_mm256_cmpeq_epi8(b0, zero), _mm256_cmpeq_epi8(b1, zero)),
                _mm256_cmpeq_epi8(b2, three),
            )) as u32;
            while mask != 0 {
                let epb_offset = offset + mask.trailing_zeros() as usize + 2;
                if epb_offset >= start {
                    return Some(epb_offset);
                }
                mask &= mask - 1;
            }
            offset += 32;
        }

        super::scalar::find_h2645_emulation_prevention_byte_from(
            data,
            start.max(offset.saturating_sub(2)),
        )
    }

    #[target_feature(enable = "avx2")]
    unsafe fn dts_word_mask(b0: __m256i, b1: __m256i, b2: __m256i, b3: __m256i) -> u32 {
        let mut mask = unsafe { word_mask(b0, b1, b2, b3, [0x7F, 0xFE, 0x80, 0x01]) };
        mask |= unsafe { word_mask(b0, b1, b2, b3, [0xFE, 0x7F, 0x01, 0x80]) };
        mask |= unsafe { word_mask(b0, b1, b2, b3, [0x1F, 0xFF, 0xE8, 0x00]) };
        mask |= unsafe { word_mask(b0, b1, b2, b3, [0xFF, 0x1F, 0x00, 0xE8]) };
        mask
    }

    #[target_feature(enable = "avx2")]
    unsafe fn byte_sequence_mask(
        b0: __m256i,
        b1: __m256i,
        b2: __m256i,
        b3: __m256i,
        bytes: [u8; 4],
        len: usize,
    ) -> u32 {
        match len {
            1 => {
                _mm256_movemask_epi8(_mm256_cmpeq_epi8(b0, _mm256_set1_epi8(bytes[0] as i8))) as u32
            }
            2 => _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_cmpeq_epi8(b0, _mm256_set1_epi8(bytes[0] as i8)),
                _mm256_cmpeq_epi8(b1, _mm256_set1_epi8(bytes[1] as i8)),
            )) as u32,
            3 => _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_and_si256(
                    _mm256_cmpeq_epi8(b0, _mm256_set1_epi8(bytes[0] as i8)),
                    _mm256_cmpeq_epi8(b1, _mm256_set1_epi8(bytes[1] as i8)),
                ),
                _mm256_cmpeq_epi8(b2, _mm256_set1_epi8(bytes[2] as i8)),
            )) as u32,
            4 => unsafe { word_mask(b0, b1, b2, b3, bytes) },
            _ => 0,
        }
    }

    #[target_feature(enable = "avx2")]
    unsafe fn word_mask(b0: __m256i, b1: __m256i, b2: __m256i, b3: __m256i, word: [u8; 4]) -> u32 {
        let cmp = _mm256_and_si256(
            _mm256_and_si256(
                _mm256_cmpeq_epi8(b0, _mm256_set1_epi8(word[0] as i8)),
                _mm256_cmpeq_epi8(b1, _mm256_set1_epi8(word[1] as i8)),
            ),
            _mm256_and_si256(
                _mm256_cmpeq_epi8(b2, _mm256_set1_epi8(word[2] as i8)),
                _mm256_cmpeq_epi8(b3, _mm256_set1_epi8(word[3] as i8)),
            ),
        );
        _mm256_movemask_epi8(cmp) as u32
    }
}

#[cfg(target_arch = "aarch64")]
mod aarch64 {
    use super::{AudioSyncCandidate, Range};
    use std::arch::aarch64::*;

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn accumulate_avi_idx1_neon(data: &[u8], totals: &mut [u64]) {
        let mut at = 0;
        while data.len() - at >= 64 {
            // Deinterleave four complete entries: chunk ID, flags, offset, and size.
            let fields = unsafe { vld4q_u32(data.as_ptr().add(at).cast()) };
            let first = vsubq_u32(vandq_u32(fields.0, vdupq_n_u32(255)), vdupq_n_u32(48));
            let second = vsubq_u32(
                vandq_u32(vshrq_n_u32::<8>(fields.0), vdupq_n_u32(255)),
                vdupq_n_u32(48),
            );
            let valid = vandq_u32(
                vcleq_u32(first, vdupq_n_u32(9)),
                vcleq_u32(second, vdupq_n_u32(9)),
            );
            let ids = vaddq_u32(vmulq_n_u32(first, 10), second);
            let ids = vbslq_u32(valid, ids, vdupq_n_u32(u32::MAX));
            let mut id_lanes = [0_u32; 4];
            let mut size_lanes = [0_u32; 4];
            unsafe {
                vst1q_u32(id_lanes.as_mut_ptr(), ids);
                vst1q_u32(size_lanes.as_mut_ptr(), fields.3);
            }
            super::accumulate_avi_lanes(id_lanes, size_lanes, totals);
            at += 64;
        }
        super::scalar::accumulate_avi_idx1_stream_sizes(&data[at..], totals);
    }

    #[target_feature(enable = "neon")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_byte_neon(data: &[u8], needle: u8) -> Option<usize> {
        let needle_vec = vdupq_n_u8(needle);
        let mut offset = 0;
        while offset + 16 <= data.len() {
            let chunk = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let cmp = vceqq_u8(chunk, needle_vec);
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }
        super::scalar::find_byte_from(data, needle, offset)
    }

    #[target_feature(enable = "neon")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_any_byte_neon(data: &[u8], needles: &[u8]) -> Option<usize> {
        if needles.len() == 1 {
            return unsafe { find_byte_neon(data, needles[0]) };
        }

        let mut offset = 0;
        while offset + 16 <= data.len() {
            let chunk = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let mut cmp = vdupq_n_u8(0);
            for &needle in needles {
                cmp = vorrq_u8(cmp, vceqq_u8(chunk, vdupq_n_u8(needle)));
            }
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }
        super::scalar::find_any_byte_from(data, needles, offset)
    }

    #[target_feature(enable = "neon")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_pair_neon(
        data: &[u8],
        first: u8,
        second_mask: u8,
        second_value: u8,
    ) -> Option<usize> {
        let first_vec = vdupq_n_u8(first);
        let second_mask_vec = vdupq_n_u8(second_mask);
        let second_value_vec = vdupq_n_u8(second_value);
        let mut offset = 0;
        while offset + 17 <= data.len() {
            let first_chunk = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let second_chunk = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let cmp = vandq_u8(
                vceqq_u8(first_chunk, first_vec),
                vceqq_u8(vandq_u8(second_chunk, second_mask_vec), second_value_vec),
            );
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }
        super::scalar::find_pair_from(data, offset, first, second_mask, second_value)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_mpeg_start_code_neon(data: &[u8], code: u8) -> Option<usize> {
        let zero = vdupq_n_u8(0);
        let one = vdupq_n_u8(1);
        let code_vec = vdupq_n_u8(code);
        let mut offset = 0;
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let cmp = vandq_u8(
                vandq_u8(vceqq_u8(b0, zero), vceqq_u8(b1, zero)),
                vandq_u8(vceqq_u8(b2, one), vceqq_u8(b3, code_vec)),
            );
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }
        super::scalar::find_mpeg_start_code_from(data, code, offset)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_annexb_start_code_neon(
        data: &[u8],
        start: usize,
    ) -> Option<Range<usize>> {
        let data = data.get(start..)?;
        let zero = vdupq_n_u8(0);
        let one = vdupq_n_u8(1);
        let mut offset = 0;
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let first_two = vandq_u8(vceqq_u8(b0, zero), vceqq_u8(b1, zero));
            let three = vandq_u8(first_two, vceqq_u8(b2, one));
            let four = vandq_u8(first_two, vandq_u8(vceqq_u8(b2, zero), vceqq_u8(b3, one)));
            let cmp = vorrq_u8(three, four);
            if let Some(index) = first_neon_match(cmp) {
                let found = offset + index;
                let len = if data[found + 2] == 1 { 3 } else { 4 };
                let range_start = start + found;
                return Some(Range {
                    start: range_start,
                    end: range_start + len,
                });
            }
            offset += 16;
        }
        super::scalar::find_annexb_start_code_from(data, offset).map(|range| Range {
            start: start + range.start,
            end: start + range.end,
        })
    }

    #[target_feature(enable = "neon")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_dts_sync_neon(data: &[u8]) -> Option<usize> {
        let mut offset = 0;
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let cmp = unsafe { dts_word_match(b0, b1, b2, b3) };
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }
        super::scalar::find_dts_sync_from(data, offset)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_audio_sync_candidate_neon(data: &[u8]) -> Option<AudioSyncCandidate> {
        let mut offset = 0;
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };

            let first_ff = vceqq_u8(b0, vdupq_n_u8(0xFF));
            let adts = vandq_u8(
                first_ff,
                vceqq_u8(vandq_u8(b1, vdupq_n_u8(0xF0)), vdupq_n_u8(0xF0)),
            );
            let mpeg_audio = vandq_u8(
                first_ff,
                vceqq_u8(vandq_u8(b1, vdupq_n_u8(0xE0)), vdupq_n_u8(0xE0)),
            );
            let latm = vandq_u8(
                vceqq_u8(b0, vdupq_n_u8(0x56)),
                vceqq_u8(vandq_u8(b1, vdupq_n_u8(0xE0)), vdupq_n_u8(0xE0)),
            );
            let ac3 = vandq_u8(
                vceqq_u8(b0, vdupq_n_u8(0x0B)),
                vceqq_u8(b1, vdupq_n_u8(0x77)),
            );
            let dts = unsafe { dts_word_match(b0, b1, b2, b3) };
            let cmp = vorrq_u8(
                vorrq_u8(vorrq_u8(adts, mpeg_audio), vorrq_u8(latm, ac3)),
                dts,
            );

            if let Some(index) = first_neon_match(cmp) {
                return super::audio_sync_candidate_at(data, offset + index);
            }
            offset += 16;
        }

        super::scalar::find_audio_sync_candidate_from(data, offset)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_ebml_candidate_neon(
        data: &[u8],
        ids: &[u32],
    ) -> Option<super::EbmlCandidate> {
        if ids.len() > 8 {
            return super::scalar::find_ebml_candidate_from(data, 0, ids);
        }
        let mut specs = [([0_u8; 4], 0usize, 0_u32); 8];
        let mut spec_count = 0usize;
        for &id in ids.iter().take(specs.len()) {
            let (bytes, len) = super::ebml_id_bytes(id);
            specs[spec_count] = (bytes, len, id);
            spec_count += 1;
        }

        let mut offset = 0;
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let mut cmp = vdupq_n_u8(0);
            for &(bytes, len, _) in &specs[..spec_count] {
                cmp = vorrq_u8(cmp, unsafe {
                    byte_sequence_match(b0, b1, b2, b3, bytes, len)
                });
            }
            if let Some(index) = first_neon_match(cmp) {
                return super::ebml_candidate_at(data, offset + index, ids);
            }
            offset += 16;
        }

        super::scalar::find_ebml_candidate_from(data, offset, ids)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_hevc_sei_nal_header_candidate_neon(
        data: &[u8],
    ) -> Option<super::LengthPrefixedNalCandidate> {
        let mut offset = 0;
        while offset + 16 <= data.len() {
            let chunk = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let cmp = vorrq_u8(
                vorrq_u8(
                    vceqq_u8(chunk, vdupq_n_u8(0x4E)),
                    vceqq_u8(chunk, vdupq_n_u8(0x4F)),
                ),
                vorrq_u8(
                    vceqq_u8(chunk, vdupq_n_u8(0x50)),
                    vceqq_u8(chunk, vdupq_n_u8(0x51)),
                ),
            );
            if let Some(index) = first_neon_match(cmp) {
                return super::hevc_sei_nal_candidate_at(data, offset + index);
            }
            offset += 16;
        }

        super::scalar::find_hevc_sei_nal_header_candidate_from(data, offset)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_hdr10plus_itu_t35_candidate_neon(data: &[u8]) -> Option<usize> {
        let mut offset = 0;
        while offset + 21 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let b4 = unsafe { vld1q_u8(data.as_ptr().add(offset + 4)) };
            let b5 = unsafe { vld1q_u8(data.as_ptr().add(offset + 5)) };
            let cmp = vandq_u8(
                unsafe { word_match(b0, b1, b2, b3, [0xB5, 0x00, 0x3C, 0x00]) },
                vandq_u8(
                    vceqq_u8(b4, vdupq_n_u8(0x01)),
                    vceqq_u8(b5, vdupq_n_u8(0x04)),
                ),
            );
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }

        super::scalar::find_hdr10plus_itu_t35_candidate_from(data, offset)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_mp4_box_name_candidate_neon(
        data: &[u8],
        start: usize,
        names: &[[u8; 4]],
    ) -> Option<super::Mp4BoxNameCandidate> {
        let mut offset = start.saturating_add(4);
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let mut cmp = vdupq_n_u8(0);
            for &name in names {
                cmp = vorrq_u8(cmp, unsafe { word_match(b0, b1, b2, b3, name) });
            }
            if let Some(index) = first_neon_match(cmp) {
                return super::mp4_box_name_candidate_at(data, offset + index, names);
            }
            offset += 16;
        }

        super::scalar::find_mp4_box_name_candidate_from(data, offset.saturating_sub(4), names)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn score_ts_packet_layout_neon(
        data: &[u8],
        raw_packet_size: usize,
        sync_offset: usize,
        sync_byte: u8,
    ) -> usize {
        if raw_packet_size == 0
            || sync_offset >= raw_packet_size
            || raw_packet_size > usize::MAX / 16
        {
            return super::scalar::score_ts_packet_layout(
                data,
                raw_packet_size,
                sync_offset,
                sync_byte,
            );
        }

        let mut score = 0usize;
        let mut packet_start = 0usize;
        while packet_start + raw_packet_size * 16 <= data.len() {
            let mut lanes = [0_u8; 16];
            for (lane, byte) in lanes.iter_mut().enumerate() {
                *byte = data[packet_start + raw_packet_size * lane + sync_offset];
            }
            let chunk = unsafe { vld1q_u8(lanes.as_ptr()) };
            score += neon_match_count(vceqq_u8(chunk, vdupq_n_u8(sync_byte)));
            packet_start += raw_packet_size * 16;
        }

        score
            + super::scalar::score_ts_packet_layout_from(
                data,
                raw_packet_size,
                sync_offset,
                sync_byte,
                packet_start,
            )
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_h2645_emulation_prevention_byte_neon(
        data: &[u8],
        start: usize,
    ) -> Option<usize> {
        let zero = vdupq_n_u8(0);
        let three = vdupq_n_u8(3);
        let mut offset = start.saturating_sub(2);
        while offset + 18 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let cmp = vandq_u8(
                vandq_u8(vceqq_u8(b0, zero), vceqq_u8(b1, zero)),
                vceqq_u8(b2, three),
            );
            if let Some(index) = first_neon_match(cmp) {
                let epb_offset = offset + index + 2;
                if epb_offset >= start {
                    return Some(epb_offset);
                }
                offset += index + 1;
            } else {
                offset += 16;
            }
        }

        super::scalar::find_h2645_emulation_prevention_byte_from(
            data,
            start.max(offset.saturating_sub(2)),
        )
    }

    #[target_feature(enable = "neon")]
    unsafe fn dts_word_match(
        b0: uint8x16_t,
        b1: uint8x16_t,
        b2: uint8x16_t,
        b3: uint8x16_t,
    ) -> uint8x16_t {
        let mut cmp = unsafe { word_match(b0, b1, b2, b3, [0x7F, 0xFE, 0x80, 0x01]) };
        cmp = vorrq_u8(cmp, unsafe {
            word_match(b0, b1, b2, b3, [0xFE, 0x7F, 0x01, 0x80])
        });
        cmp = vorrq_u8(cmp, unsafe {
            word_match(b0, b1, b2, b3, [0x1F, 0xFF, 0xE8, 0x00])
        });
        vorrq_u8(cmp, unsafe {
            word_match(b0, b1, b2, b3, [0xFF, 0x1F, 0x00, 0xE8])
        })
    }

    #[target_feature(enable = "neon")]
    unsafe fn byte_sequence_match(
        b0: uint8x16_t,
        b1: uint8x16_t,
        b2: uint8x16_t,
        b3: uint8x16_t,
        bytes: [u8; 4],
        len: usize,
    ) -> uint8x16_t {
        match len {
            1 => vceqq_u8(b0, vdupq_n_u8(bytes[0])),
            2 => vandq_u8(
                vceqq_u8(b0, vdupq_n_u8(bytes[0])),
                vceqq_u8(b1, vdupq_n_u8(bytes[1])),
            ),
            3 => vandq_u8(
                vandq_u8(
                    vceqq_u8(b0, vdupq_n_u8(bytes[0])),
                    vceqq_u8(b1, vdupq_n_u8(bytes[1])),
                ),
                vceqq_u8(b2, vdupq_n_u8(bytes[2])),
            ),
            4 => unsafe { word_match(b0, b1, b2, b3, bytes) },
            _ => vdupq_n_u8(0),
        }
    }

    #[target_feature(enable = "neon")]
    unsafe fn word_match(
        b0: uint8x16_t,
        b1: uint8x16_t,
        b2: uint8x16_t,
        b3: uint8x16_t,
        word: [u8; 4],
    ) -> uint8x16_t {
        vandq_u8(
            vandq_u8(
                vceqq_u8(b0, vdupq_n_u8(word[0])),
                vceqq_u8(b1, vdupq_n_u8(word[1])),
            ),
            vandq_u8(
                vceqq_u8(b2, vdupq_n_u8(word[2])),
                vceqq_u8(b3, vdupq_n_u8(word[3])),
            ),
        )
    }

    #[inline]
    fn first_neon_match(mask: uint8x16_t) -> Option<usize> {
        if unsafe { vmaxvq_u8(mask) } == 0 {
            return None;
        }
        let mut lanes = [0_u8; 16];
        unsafe { vst1q_u8(lanes.as_mut_ptr(), mask) };
        lanes.iter().position(|&lane| lane != 0)
    }

    #[inline]
    fn neon_match_count(mask: uint8x16_t) -> usize {
        if unsafe { vmaxvq_u8(mask) } == 0 {
            return 0;
        }
        let mut lanes = [0_u8; 16];
        unsafe { vst1q_u8(lanes.as_mut_ptr(), mask) };
        lanes.iter().filter(|&&lane| lane != 0).count()
    }
}

#[cfg(test)]
mod optimization_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn avi_idx1_entry(id: &[u8; 4], size: u32) -> [u8; 16] {
        let mut entry = [0_u8; 16];
        entry[..4].copy_from_slice(id);
        entry[12..16].copy_from_slice(&size.to_le_bytes());
        entry
    }

    fn avi_idx1_payload_from_fixture(bytes: &[u8]) -> &[u8] {
        let idx1_offset = bytes
            .windows(4)
            .position(|window| window == b"idx1")
            .expect("fixture should contain idx1");
        let len_offset = idx1_offset + 4;
        let payload_offset = idx1_offset + 8;
        let len = u32::from_le_bytes(
            bytes[len_offset..payload_offset]
                .try_into()
                .expect("idx1 length should be complete"),
        ) as usize;
        bytes
            .get(payload_offset..payload_offset + len)
            .expect("idx1 payload should be complete")
    }

    fn simd_scan_fixture() -> &'static [u8] {
        static FIXTURE: std::sync::LazyLock<Vec<u8>> = std::sync::LazyLock::new(|| {
            let mut data = vec![0x55; 0x3060];
            let patterns: &[(usize, &[u8])] = &[
                (0x0400, &[0xA7]),
                (0x0601, &[0x47]),
                (0x0A02, &[0x00, 0x00, 0x01, 0xB3]),
                (0x0E06, &[0x00, 0x00, 0x00, 0x01, 0x67]),
                (0x100B, &[0xFF, 0xF1, 0x50, 0x80]),
                (0x120F, &[0x56, 0xE0, 0x00, 0x00]),
                (0x1413, &[0xFF, 0xE2, 0x00, 0x00]),
                (0x1617, &[0x0B, 0x77, 0x00, 0x00]),
                (0x181B, &[0x7F, 0xFE, 0x80, 0x01]),
                (0x1C1F, &[0x1F, 0x43, 0xB6, 0x75]),
                (0x1E23, &[0xA3]),
                (0x2024, &[0x75, 0xA1]),
                (0x2226, &[0xE7]),
                (0x2627, &[0x4E, 0x01, 0x50, 0x00]),
                (0x2A2B, &[0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04]),
                (0x2C31, &[0x00, 0x00, 0x03, 0x01]),
                (
                    0x3035,
                    &[0x00, 0x00, 0x00, 0x0C, b'm', b'o', b'o', b'v', 0, 0, 0, 0],
                ),
                (
                    0x3050,
                    &[
                        b'0', b'2', b'w', b'b', 0, 0, 0, 0, 0x80, 0, 0, 0, 0x40, 0, 0, 0,
                    ],
                ),
            ];

            for &(offset, pattern) in patterns {
                data[offset..offset + pattern.len()].copy_from_slice(pattern);
            }
            data
        });

        FIXTURE.as_slice()
    }

    fn slice_from_pattern<'a>(data: &'a [u8], pattern: &[u8]) -> &'a [u8] {
        let offset = data
            .windows(pattern.len())
            .position(|window| window == pattern)
            .expect("fixture should contain pattern");
        &data[offset..]
    }

    fn fixture_slice_from_pattern(pattern: &[u8]) -> &'static [u8] {
        slice_from_pattern(simd_scan_fixture(), pattern)
    }

    fn ts_layout_probe(raw_packet_size: usize, sync_offset: usize, packets: usize) -> Vec<u8> {
        let mut data = Vec::with_capacity(raw_packet_size * packets + raw_packet_size - 1);
        for packet_index in 0..packets {
            let mut packet = vec![0x21; raw_packet_size];
            packet[sync_offset] = 0x47;
            if sync_offset + 1 < raw_packet_size {
                packet[sync_offset + 1] = packet_index as u8;
            }
            data.extend_from_slice(&packet);
        }
        data.extend(std::iter::repeat_n(0x33, raw_packet_size.saturating_sub(1)));
        data
    }

    #[test]
    fn find_byte_matches_scalar() {
        for len in 0..96 {
            let mut data = vec![0x55; len];
            for needle_at in 0..=len {
                if needle_at < len {
                    data[needle_at] = 0xA7;
                }
                assert_eq!(
                    find_byte_from(&data, 0xA7, 0),
                    scalar::find_byte_from(&data, 0xA7, 0)
                );
                assert_eq!(
                    find_byte_from(&data, 0xA7, len.min(needle_at.saturating_add(1))),
                    scalar::find_byte_from(&data, 0xA7, len.min(needle_at.saturating_add(1)))
                );
                if needle_at < len {
                    data[needle_at] = 0x55;
                }
            }
        }
    }

    #[test]
    fn find_any_byte_matches_scalar() {
        let needles = [0x00, 0x47, 0xFF, 0x0B];
        for len in 0..128 {
            let mut data = vec![0x22; len];
            for needle_at in 0..=len {
                if needle_at < len {
                    data[needle_at] = needles[needle_at % needles.len()];
                }
                assert_eq!(
                    find_any_byte_from(&data, &needles, 0),
                    scalar::find_any_byte_from(&data, &needles, 0)
                );
                if needle_at < len {
                    data[needle_at] = 0x22;
                }
            }
        }
    }

    #[test]
    fn finds_mpeg_start_code_boundaries() {
        assert_eq!(find_mpeg_start_code(&[], 0xB3), None);
        assert_eq!(find_mpeg_start_code(&[0, 0, 1, 0xB3], 0xB3), Some(0));
        assert_eq!(find_mpeg_start_code(&[9, 0, 0, 1, 0xB3, 1], 0xB3), Some(1));
        assert_eq!(find_mpeg_start_code(&[0, 0, 1, 0xB8], 0xB3), None);
        assert_eq!(find_mpeg_start_code(&[0, 0, 1], 0xB3), None);
    }

    #[test]
    fn finds_annexb_start_code_boundaries() {
        assert_eq!(find_annexb_start_code(&[], 0), None);
        assert_eq!(
            find_annexb_start_code(&[0, 0, 1, 7], 0),
            Some(Range { start: 0, end: 3 })
        );
        assert_eq!(
            find_annexb_start_code(&[9, 0, 0, 0, 1, 7], 0),
            Some(Range { start: 1, end: 5 })
        );
        assert_eq!(find_annexb_start_code(&[0, 0, 2, 1], 0), None);
        assert_eq!(find_annexb_start_code(&[0, 0], 0), None);
    }

    #[test]
    fn finds_audio_sync_boundaries() {
        assert_eq!(find_adts_sync(&[0, 0xFF, 0xF1, 0], 0), Some(1));
        assert_eq!(find_latm_sync(&[0, 0x56, 0xE0, 0], 0), Some(1));
        assert_eq!(find_mpeg_audio_sync(&[0, 0xFF, 0xE2, 0], 0), Some(1));
        assert_eq!(find_ac3_sync(&[0, 0x0B, 0x77, 0], 0), Some(1));
        assert_eq!(find_dts_sync(&[0, 0x7F, 0xFE, 0x80, 0x01, 0], 0), Some(1));
        assert_eq!(
            find_audio_sync_candidate(&[0, 0x56, 0xE0, 0xFF, 0xF1], 0),
            Some(AudioSyncCandidate {
                offset: 1,
                kind: AudioSyncKind::Latm,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(&[0, 0xFF, 0xF1, 0], 0),
            Some(AudioSyncCandidate {
                offset: 1,
                kind: AudioSyncKind::Adts,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(&[0, 0xFF, 0xE2, 0], 0),
            Some(AudioSyncCandidate {
                offset: 1,
                kind: AudioSyncKind::MpegAudio,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(&[0, 0x0B, 0x77, 0], 0),
            Some(AudioSyncCandidate {
                offset: 1,
                kind: AudioSyncKind::Ac3,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(&[0, 0x7F, 0xFE, 0x80, 0x01, 0], 0),
            Some(AudioSyncCandidate {
                offset: 1,
                kind: AudioSyncKind::Dts,
            })
        );
        assert_eq!(find_adts_sync(&[0xFF, 0xE1], 0), None);
        assert_eq!(find_dts_sync(&[0x7F, 0xFE, 0x80], 0), None);
    }

    #[test]
    fn finds_ebml_and_payload_candidates() {
        let ids = [0x1F43_B675, 0xA3, 0xA0, 0xA1, 0x75A1, 0xA6, 0xA5, 0xE7];
        let data = [0x55, 0x75, 0x00, 0xAA, 0x1F, 0x43, 0xB6, 0x75, 0xE7];
        assert_eq!(
            find_ebml_candidate(&data, 0, &ids),
            Some(EbmlCandidate {
                offset: 4,
                id: 0x1F43_B675,
            })
        );
        assert_eq!(
            find_ebml_candidate(&data, 5, &ids),
            Some(EbmlCandidate {
                offset: 8,
                id: 0xE7,
            })
        );

        let nal_data = [0x00, 0x4D, 0x4E, 0x01, 0x50];
        assert_eq!(
            find_hevc_sei_nal_header_candidate(&nal_data, 0),
            Some(LengthPrefixedNalCandidate {
                offset: 2,
                nal_type: 39,
            })
        );
        assert_eq!(
            find_hdr10plus_itu_t35_candidate(&[0, 0xB5, 0, 0x3C, 0, 1, 4], 0),
            Some(1)
        );

        let mp4 = [0, 0, 0, 12, b'm', b'o', b'o', b'v', 0, 0, 0, 0];
        assert_eq!(
            find_mp4_box_name_candidate(&mp4, 0, &[*b"moov", *b"trak"]),
            Some(Mp4BoxNameCandidate {
                offset: 0,
                name: *b"moov",
            })
        );
        assert_eq!(
            find_avi_idx1_stream_prefix(&[b'0', b'2', b'w', b'b', 0, 0, 0, 0], 0, 2),
            Some(0)
        );
    }

    #[test]
    fn accumulates_avi_idx1_stream_sizes() {
        let mut data = Vec::new();
        data.extend_from_slice(&avi_idx1_entry(b"00db", 7));
        data.extend_from_slice(&avi_idx1_entry(b"01wb", 11));
        data.extend_from_slice(&avi_idx1_entry(b"99dc", 13));
        data.extend_from_slice(&avi_idx1_entry(b"AAwb", 17));
        data.extend_from_slice(b"02d");

        let mut totals = [0_u64; 100];
        accumulate_avi_idx1_stream_sizes(&data, &mut totals);
        assert_eq!(totals[0], 7);
        assert_eq!(totals[1], 11);
        assert_eq!(totals[99], 13);
        assert_eq!(totals[2], 0);

        let mut short_totals = [0_u64; 2];
        accumulate_avi_idx1_stream_sizes(&data, &mut short_totals);
        assert_eq!(short_totals, [7, 11]);
    }

    #[test]
    fn scores_ts_packet_layouts_against_scalar() {
        for (raw_packet_size, sync_offset) in [(188, 0), (192, 4), (204, 0)] {
            let mut data = ts_layout_probe(raw_packet_size, sync_offset, 97);
            data[raw_packet_size * 13 + sync_offset] = 0x00;

            assert_eq!(
                score_ts_packet_layout(&data, raw_packet_size, sync_offset, 0x47),
                scalar::score_ts_packet_layout(&data, raw_packet_size, sync_offset, 0x47)
            );
            assert_eq!(
                scalar::score_ts_packet_layout(&data, raw_packet_size, sync_offset, 0x47),
                96
            );
            assert_eq!(
                score_ts_packet_layout(&data, raw_packet_size, sync_offset, 0x00),
                scalar::score_ts_packet_layout(&data, raw_packet_size, sync_offset, 0x00)
            );
        }

        assert_eq!(score_ts_packet_layout(&[0x47; 16], 188, 0, 0x47), 0);
        assert_eq!(score_ts_packet_layout(&[0x47; 204], 0, 0, 0x47), 0);
        assert_eq!(score_ts_packet_layout(&[0x47; 204], 188, 188, 0x47), 0);
    }

    #[test]
    fn unescapes_h2645_rbsp_and_finds_emulation_prevention_bytes() {
        let escaped = [0x67, 0x00, 0x00, 0x03, 0x01, 0xAA, 0x00, 0x00, 0x03, 0x00];
        assert_eq!(find_h2645_emulation_prevention_byte(&escaped, 0), Some(3));
        assert_eq!(find_h2645_emulation_prevention_byte(&escaped, 4), Some(8));
        assert_eq!(find_h2645_emulation_prevention_byte(&escaped, 9), None);
        assert_eq!(
            h2645_unescape_rbsp(&escaped).as_ref(),
            &[0x67, 0x00, 0x00, 0x01, 0xAA, 0x00, 0x00, 0x00]
        );

        let clean = [0x67, 0x00, 0x00, 0x02, 0xAA];
        assert!(matches!(
            h2645_unescape_rbsp(&clean),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(find_h2645_emulation_prevention_byte(&[0x00, 0x00], 0), None);
    }

    #[test]
    fn dense_avi_fixture_exercises_idx1_accumulator() {
        let payload = avi_idx1_payload_from_fixture(include_bytes!(
            "../tests/media/avi_idx1_dense_mpeg4.avi"
        ));
        assert!(
            payload.len() >= 960 * 16,
            "dense fixture should have at least 960 idx1 entries"
        );

        let mut dispatched_totals = [0_u64; 100];
        let mut scalar_totals = [0_u64; 100];
        accumulate_avi_idx1_stream_sizes(payload, &mut dispatched_totals);
        scalar::accumulate_avi_idx1_stream_sizes(payload, &mut scalar_totals);

        assert_eq!(dispatched_totals, scalar_totals);
        assert!(dispatched_totals[0] > 0);
        assert!(dispatched_totals[1..].iter().all(|&total| total == 0));
    }

    #[test]
    fn dense_scan_fixture_exercises_all_simd_dispatchers() {
        let data = simd_scan_fixture();
        assert!(data.len() > 8 * 1024);

        assert_eq!(
            find_byte_from(data, 0xA7, 0),
            scalar::find_byte_from(data, 0xA7, 0)
        );
        assert_eq!(
            find_any_byte_from(data, &[0x47, 0xA7, 0x0B], 0),
            scalar::find_any_byte_from(data, &[0x47, 0xA7, 0x0B], 0)
        );
        assert_eq!(
            find_mpeg_start_code(data, 0xB3),
            scalar::find_mpeg_start_code_from(data, 0xB3, 0)
        );
        assert_eq!(
            find_annexb_start_code(data, 0),
            scalar::find_annexb_start_code_from(data, 0)
        );
        assert_eq!(
            find_h2645_emulation_prevention_byte(data, 0),
            scalar::find_h2645_emulation_prevention_byte_from(data, 0)
        );
        assert!(matches!(
            h2645_unescape_rbsp(fixture_slice_from_pattern(&[0x00, 0x00, 0x03, 0x01])),
            std::borrow::Cow::Owned(_)
        ));
        assert_eq!(
            find_audio_sync_candidate(data, 0),
            scalar::find_audio_sync_candidate_from(data, 0)
        );
        assert_eq!(
            find_ebml_candidate(data, 0, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7]),
            scalar::find_ebml_candidate_from(data, 0, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7])
        );
        assert_eq!(
            find_hevc_sei_nal_header_candidate(data, 0),
            scalar::find_hevc_sei_nal_header_candidate_from(data, 0)
        );
        assert_eq!(
            find_hdr10plus_itu_t35_candidate(data, 0),
            scalar::find_hdr10plus_itu_t35_candidate_from(data, 0)
        );
        assert_eq!(
            find_mp4_box_name_candidate(data, 0, &[*b"moov", *b"trak", *b"mdat"]),
            scalar::find_mp4_box_name_candidate_from(data, 0, &[*b"moov", *b"trak", *b"mdat"])
        );
        assert_eq!(
            find_avi_idx1_stream_prefix(data, 0, 2),
            scalar::find_pair_from(data, 0, b'0', 0xFF, b'2').filter(|offset| offset % 16 == 0)
        );

        let adts = fixture_slice_from_pattern(&[0xFF, 0xF1, 0x50, 0x80]);
        let latm = fixture_slice_from_pattern(&[0x56, 0xE0, 0x00, 0x00]);
        let mpeg_audio = fixture_slice_from_pattern(&[0xFF, 0xE2, 0x00, 0x00]);
        let ac3 = fixture_slice_from_pattern(&[0x0B, 0x77, 0x00, 0x00]);
        let dts = fixture_slice_from_pattern(&[0x7F, 0xFE, 0x80, 0x01]);

        assert_eq!(find_adts_sync(adts, 0), Some(0));
        assert_eq!(find_latm_sync(latm, 0), Some(0));
        assert_eq!(find_mpeg_audio_sync(mpeg_audio, 0), Some(0));
        assert_eq!(find_ac3_sync(ac3, 0), Some(0));
        assert_eq!(find_dts_sync(dts, 0), Some(0));
        assert_eq!(
            find_audio_sync_candidate(latm, 0),
            Some(AudioSyncCandidate {
                offset: 0,
                kind: AudioSyncKind::Latm,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(ac3, 0),
            Some(AudioSyncCandidate {
                offset: 0,
                kind: AudioSyncKind::Ac3,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(dts, 0),
            Some(AudioSyncCandidate {
                offset: 0,
                kind: AudioSyncKind::Dts,
            })
        );
    }

    #[test]
    fn dense_media_fixtures_exercise_simd_dispatchers() {
        let h264_aac_ts = include_bytes!("../tests/media/simd_dense_h264_aac.ts");
        let h264_aac_m2ts = include_bytes!("../tests/media/simd_dense_h264_aac_192.m2ts");
        let h264_aac_fec_ts = include_bytes!("../tests/media/simd_dense_h264_aac_204.ts");
        let mpeg2_mp2_ts = include_bytes!("../tests/media/simd_dense_mpeg2_mp2.ts");
        let h264_ac3_ts = include_bytes!("../tests/media/simd_dense_h264_ac3.ts");
        let h264_dts_ts = include_bytes!("../tests/media/simd_dense_h264_dts.ts");
        let webm = include_bytes!("../tests/media/simd_dense_vp9_opus.webm");
        let mp4 = include_bytes!("../tests/media/simd_dense_h264_aac.mp4");
        let avi_idx1 = avi_idx1_payload_from_fixture(include_bytes!(
            "../tests/media/avi_idx1_dense_mpeg4.avi"
        ));

        assert!(h264_aac_ts.len() > 200 * 1024);
        assert!(h264_aac_m2ts.len() > 200 * 1024);
        assert!(h264_aac_fec_ts.len() > 200 * 1024);
        assert!(mpeg2_mp2_ts.len() > 300 * 1024);
        assert!(h264_ac3_ts.len() > 300 * 1024);
        assert!(h264_dts_ts.len() > 1024 * 1024);
        assert!(webm.len() > 30 * 1024);
        assert!(mp4.len() > 30 * 1024);

        assert_eq!(
            score_ts_packet_layout(h264_aac_ts, 188, 0, 0x47),
            scalar::score_ts_packet_layout(h264_aac_ts, 188, 0, 0x47)
        );
        assert_eq!(
            score_ts_packet_layout(h264_aac_m2ts, 192, 4, 0x47),
            scalar::score_ts_packet_layout(h264_aac_m2ts, 192, 4, 0x47)
        );
        assert_eq!(
            score_ts_packet_layout(h264_aac_fec_ts, 204, 0, 0x47),
            scalar::score_ts_packet_layout(h264_aac_fec_ts, 204, 0, 0x47)
        );
        assert_eq!(
            find_annexb_start_code(h264_aac_ts, 0),
            scalar::find_annexb_start_code_from(h264_aac_ts, 0)
        );
        assert_eq!(
            find_mpeg_start_code(mpeg2_mp2_ts, 0xB3),
            scalar::find_mpeg_start_code_from(mpeg2_mp2_ts, 0xB3, 0)
        );
        assert_eq!(
            find_ebml_candidate(webm, 0, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7]),
            scalar::find_ebml_candidate_from(webm, 0, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7])
        );
        assert_eq!(
            find_mp4_box_name_candidate(mp4, 0, &[*b"moov", *b"trak", *b"mdat"]),
            scalar::find_mp4_box_name_candidate_from(mp4, 0, &[*b"moov", *b"trak", *b"mdat"])
        );
        assert_eq!(find_avi_idx1_stream_prefix(avi_idx1, 0, 0), Some(0));

        let adts = slice_from_pattern(h264_aac_ts, &[0xFF, 0xF1]);
        let mpeg_audio = slice_from_pattern(mpeg2_mp2_ts, &[0xFF, 0xFC]);
        let ac3 = slice_from_pattern(h264_ac3_ts, &[0x0B, 0x77]);
        let dts = slice_from_pattern(h264_dts_ts, &[0x7F, 0xFE, 0x80, 0x01]);

        assert_eq!(
            find_adts_sync(adts, 0),
            scalar::find_pair_from(adts, 0, 0xFF, 0xF0, 0xF0)
        );
        assert_eq!(
            find_mpeg_audio_sync(mpeg_audio, 0),
            scalar::find_pair_from(mpeg_audio, 0, 0xFF, 0xE0, 0xE0)
        );
        assert_eq!(
            find_ac3_sync(ac3, 0),
            scalar::find_pair_from(ac3, 0, 0x0B, 0xFF, 0x77)
        );
        assert_eq!(find_dts_sync(dts, 0), scalar::find_dts_sync_from(dts, 0));
        assert_eq!(
            find_audio_sync_candidate(adts, 0),
            Some(AudioSyncCandidate {
                offset: 0,
                kind: AudioSyncKind::Adts,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(ac3, 0),
            Some(AudioSyncCandidate {
                offset: 0,
                kind: AudioSyncKind::Ac3,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(dts, 0),
            Some(AudioSyncCandidate {
                offset: 0,
                kind: AudioSyncKind::Dts,
            })
        );
    }

    #[test]
    fn random_buffers_match_scalar() {
        let mut state = 0x1234_5678_9ABC_DEF0_u64;
        for len in [0, 1, 2, 3, 4, 7, 15, 16, 31, 32, 33, 63, 64, 127, 255] {
            let mut data = vec![0_u8; len];
            for byte in &mut data {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }

            for start in 0..=len {
                assert_eq!(
                    find_byte_from(&data, 0x00, start),
                    scalar::find_byte_from(&data, 0x00, start)
                );
                assert_eq!(
                    find_any_byte_from(&data, &[0, 1, 0x47, 0xFF], start),
                    scalar::find_any_byte_from(&data, &[0, 1, 0x47, 0xFF], start)
                );
                assert_eq!(
                    find_h2645_emulation_prevention_byte(&data, start),
                    scalar::find_h2645_emulation_prevention_byte_from(&data, start)
                );
                assert_eq!(
                    find_adts_sync(&data, start),
                    scalar::find_pair_from(&data, start, 0xFF, 0xF0, 0xF0)
                );
                assert_eq!(
                    find_latm_sync(&data, start),
                    scalar::find_pair_from(&data, start, 0x56, 0xE0, 0xE0)
                );
                assert_eq!(
                    find_mpeg_audio_sync(&data, start),
                    scalar::find_pair_from(&data, start, 0xFF, 0xE0, 0xE0)
                );
                assert_eq!(
                    find_ac3_sync(&data, start),
                    scalar::find_pair_from(&data, start, 0x0B, 0xFF, 0x77)
                );
                assert_eq!(
                    find_dts_sync(&data, start),
                    scalar::find_dts_sync_from(&data, start)
                );
                assert_eq!(
                    find_audio_sync_candidate(&data, start),
                    scalar::find_audio_sync_candidate_from(&data, start)
                );
                assert_eq!(
                    find_ebml_candidate(&data, start, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7]),
                    scalar::find_ebml_candidate_from(
                        &data,
                        start,
                        &[0x1F43_B675, 0xA3, 0x75A1, 0xE7]
                    )
                );
                assert_eq!(
                    find_hevc_sei_nal_header_candidate(&data, start),
                    scalar::find_hevc_sei_nal_header_candidate_from(&data, start)
                );
                assert_eq!(
                    find_hdr10plus_itu_t35_candidate(&data, start),
                    scalar::find_hdr10plus_itu_t35_candidate_from(&data, start)
                );
                assert_eq!(
                    find_mp4_box_name_candidate(&data, start, &[*b"moov", *b"trak", *b"stsd"]),
                    scalar::find_mp4_box_name_candidate_from(
                        &data,
                        start,
                        &[*b"moov", *b"trak", *b"stsd"]
                    )
                );
                assert_eq!(find_avi_idx1_stream_prefix(&data, start, 2), {
                    let mut cursor = start;
                    let mut found = None;
                    while let Some(offset) = scalar::find_pair_from(&data, cursor, b'0', 0xFF, b'2')
                    {
                        if offset % 16 == 0 {
                            found = Some(offset);
                            break;
                        }
                        cursor = offset + 1;
                    }
                    found
                });

                let mut dispatched_totals = [0_u64; 100];
                let mut scalar_totals = [0_u64; 100];
                accumulate_avi_idx1_stream_sizes(&data[start..], &mut dispatched_totals);
                scalar::accumulate_avi_idx1_stream_sizes(&data[start..], &mut scalar_totals);
                assert_eq!(dispatched_totals, scalar_totals);
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_backend_matches_scalar_when_supported() {
        let mut data = vec![0x31; 257];
        data[211] = 0xA7;
        data[233] = 0x47;
        data[241..245].copy_from_slice(&[0x7F, 0xFE, 0x80, 0x01]);
        let needles = [0x00, 0x47, 0xFF];

        if std::arch::is_x86_feature_detected!("avx2") {
            assert_eq!(
                unsafe { super::x86_64::find_byte_avx2(&data, 0xA7) },
                scalar::find_byte_from(&data, 0xA7, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_any_byte_avx2(&data, &needles) },
                scalar::find_any_byte_from(&data, &needles, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_pair_avx2(&data, 0x7F, 0xFF, 0xFE) },
                scalar::find_pair_from(&data, 0, 0x7F, 0xFF, 0xFE)
            );
            assert_eq!(
                unsafe { super::x86_64::find_dts_sync_avx2(&data) },
                scalar::find_dts_sync_from(&data, 0)
            );
            data[13..16].copy_from_slice(&[0x00, 0x00, 0x03]);
            assert_eq!(
                unsafe { super::x86_64::find_h2645_emulation_prevention_byte_avx2(&data, 0) },
                scalar::find_h2645_emulation_prevention_byte_from(&data, 0)
            );
            let ts_probe = ts_layout_probe(192, 4, 64);
            assert_eq!(
                unsafe { super::x86_64::score_ts_packet_layout_avx2(&ts_probe, 192, 4, 0x47) },
                scalar::score_ts_packet_layout(&ts_probe, 192, 4, 0x47)
            );
            assert_eq!(
                unsafe { super::x86_64::find_audio_sync_candidate_avx2(&data) },
                scalar::find_audio_sync_candidate_from(&data, 0)
            );
            data[17..21].copy_from_slice(&[0x1F, 0x43, 0xB6, 0x75]);
            data[41] = 0x4E;
            data[73..79].copy_from_slice(&[0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04]);
            assert_eq!(
                unsafe {
                    super::x86_64::find_ebml_candidate_avx2(
                        &data,
                        &[0x1F43_B675, 0xA3, 0x75A1, 0xE7],
                    )
                },
                scalar::find_ebml_candidate_from(&data, 0, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7])
            );
            assert_eq!(
                unsafe { super::x86_64::find_hevc_sei_nal_header_candidate_avx2(&data) },
                scalar::find_hevc_sei_nal_header_candidate_from(&data, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_hdr10plus_itu_t35_candidate_avx2(&data) },
                scalar::find_hdr10plus_itu_t35_candidate_from(&data, 0)
            );
            data[105..109].copy_from_slice(b"moov");
            assert_eq!(
                unsafe {
                    super::x86_64::find_mp4_box_name_candidate_avx2(&data, 0, &[*b"moov", *b"trak"])
                },
                scalar::find_mp4_box_name_candidate_from(&data, 0, &[*b"moov", *b"trak"])
            );

            let fixture = simd_scan_fixture();
            assert_eq!(
                unsafe { super::x86_64::find_byte_avx2(fixture, 0xA7) },
                scalar::find_byte_from(fixture, 0xA7, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_any_byte_avx2(fixture, &[0x47, 0xA7, 0x0B]) },
                scalar::find_any_byte_from(fixture, &[0x47, 0xA7, 0x0B], 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_pair_avx2(fixture, 0xFF, 0xF0, 0xF0) },
                scalar::find_pair_from(fixture, 0, 0xFF, 0xF0, 0xF0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_mpeg_start_code_avx2(fixture, 0xB3) },
                scalar::find_mpeg_start_code_from(fixture, 0xB3, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_annexb_start_code_avx2(fixture, 0) },
                scalar::find_annexb_start_code_from(fixture, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_h2645_emulation_prevention_byte_avx2(fixture, 0) },
                scalar::find_h2645_emulation_prevention_byte_from(fixture, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_dts_sync_avx2(fixture) },
                scalar::find_dts_sync_from(fixture, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_audio_sync_candidate_avx2(fixture) },
                scalar::find_audio_sync_candidate_from(fixture, 0)
            );
            assert_eq!(
                unsafe {
                    super::x86_64::find_ebml_candidate_avx2(
                        fixture,
                        &[0x1F43_B675, 0xA3, 0x75A1, 0xE7],
                    )
                },
                scalar::find_ebml_candidate_from(fixture, 0, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7])
            );
            assert_eq!(
                unsafe { super::x86_64::find_hevc_sei_nal_header_candidate_avx2(fixture) },
                scalar::find_hevc_sei_nal_header_candidate_from(fixture, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_hdr10plus_itu_t35_candidate_avx2(fixture) },
                scalar::find_hdr10plus_itu_t35_candidate_from(fixture, 0)
            );
            assert_eq!(
                unsafe {
                    super::x86_64::find_mp4_box_name_candidate_avx2(
                        fixture,
                        0,
                        &[*b"moov", *b"trak", *b"mdat"],
                    )
                },
                scalar::find_mp4_box_name_candidate_from(
                    fixture,
                    0,
                    &[*b"moov", *b"trak", *b"mdat"],
                )
            );
        } else {
            assert_eq!(
                find_byte_from(&data, 0xA7, 0),
                scalar::find_byte_from(&data, 0xA7, 0)
            );
            assert_eq!(
                find_any_byte_from(&data, &needles, 0),
                scalar::find_any_byte_from(&data, &needles, 0)
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn neon_backend_matches_scalar_when_supported() {
        let mut data = vec![0x31; 257];
        data[211] = 0xA7;
        data[233] = 0x47;
        data[241..245].copy_from_slice(&[0x7F, 0xFE, 0x80, 0x01]);
        let needles = [0x00, 0x47, 0xFF];

        if super::aarch64_neon_available() {
            assert_eq!(
                unsafe { super::aarch64::find_byte_neon(&data, 0xA7) },
                scalar::find_byte_from(&data, 0xA7, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_any_byte_neon(&data, &needles) },
                scalar::find_any_byte_from(&data, &needles, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_pair_neon(&data, 0x7F, 0xFF, 0xFE) },
                scalar::find_pair_from(&data, 0, 0x7F, 0xFF, 0xFE)
            );
            assert_eq!(
                unsafe { super::aarch64::find_dts_sync_neon(&data) },
                scalar::find_dts_sync_from(&data, 0)
            );
            data[13..16].copy_from_slice(&[0x00, 0x00, 0x03]);
            assert_eq!(
                unsafe { super::aarch64::find_h2645_emulation_prevention_byte_neon(&data, 0) },
                scalar::find_h2645_emulation_prevention_byte_from(&data, 0)
            );
            let ts_probe = ts_layout_probe(192, 4, 64);
            assert_eq!(
                unsafe { super::aarch64::score_ts_packet_layout_neon(&ts_probe, 192, 4, 0x47) },
                scalar::score_ts_packet_layout(&ts_probe, 192, 4, 0x47)
            );
            assert_eq!(
                unsafe { super::aarch64::find_audio_sync_candidate_neon(&data) },
                scalar::find_audio_sync_candidate_from(&data, 0)
            );
            data[17..21].copy_from_slice(&[0x1F, 0x43, 0xB6, 0x75]);
            data[41] = 0x4E;
            data[73..79].copy_from_slice(&[0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04]);
            assert_eq!(
                unsafe {
                    super::aarch64::find_ebml_candidate_neon(
                        &data,
                        &[0x1F43_B675, 0xA3, 0x75A1, 0xE7],
                    )
                },
                scalar::find_ebml_candidate_from(&data, 0, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7])
            );
            assert_eq!(
                unsafe { super::aarch64::find_hevc_sei_nal_header_candidate_neon(&data) },
                scalar::find_hevc_sei_nal_header_candidate_from(&data, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_hdr10plus_itu_t35_candidate_neon(&data) },
                scalar::find_hdr10plus_itu_t35_candidate_from(&data, 0)
            );
            data[105..109].copy_from_slice(b"moov");
            assert_eq!(
                unsafe {
                    super::aarch64::find_mp4_box_name_candidate_neon(
                        &data,
                        0,
                        &[*b"moov", *b"trak"],
                    )
                },
                scalar::find_mp4_box_name_candidate_from(&data, 0, &[*b"moov", *b"trak"])
            );

            let fixture = simd_scan_fixture();
            assert_eq!(
                unsafe { super::aarch64::find_byte_neon(fixture, 0xA7) },
                scalar::find_byte_from(fixture, 0xA7, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_any_byte_neon(fixture, &[0x47, 0xA7, 0x0B]) },
                scalar::find_any_byte_from(fixture, &[0x47, 0xA7, 0x0B], 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_pair_neon(fixture, 0xFF, 0xF0, 0xF0) },
                scalar::find_pair_from(fixture, 0, 0xFF, 0xF0, 0xF0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_mpeg_start_code_neon(fixture, 0xB3) },
                scalar::find_mpeg_start_code_from(fixture, 0xB3, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_annexb_start_code_neon(fixture, 0) },
                scalar::find_annexb_start_code_from(fixture, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_h2645_emulation_prevention_byte_neon(fixture, 0) },
                scalar::find_h2645_emulation_prevention_byte_from(fixture, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_dts_sync_neon(fixture) },
                scalar::find_dts_sync_from(fixture, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_audio_sync_candidate_neon(fixture) },
                scalar::find_audio_sync_candidate_from(fixture, 0)
            );
            assert_eq!(
                unsafe {
                    super::aarch64::find_ebml_candidate_neon(
                        fixture,
                        &[0x1F43_B675, 0xA3, 0x75A1, 0xE7],
                    )
                },
                scalar::find_ebml_candidate_from(fixture, 0, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7])
            );
            assert_eq!(
                unsafe { super::aarch64::find_hevc_sei_nal_header_candidate_neon(fixture) },
                scalar::find_hevc_sei_nal_header_candidate_from(fixture, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_hdr10plus_itu_t35_candidate_neon(fixture) },
                scalar::find_hdr10plus_itu_t35_candidate_from(fixture, 0)
            );
            assert_eq!(
                unsafe {
                    super::aarch64::find_mp4_box_name_candidate_neon(
                        fixture,
                        0,
                        &[*b"moov", *b"trak", *b"mdat"],
                    )
                },
                scalar::find_mp4_box_name_candidate_from(
                    fixture,
                    0,
                    &[*b"moov", *b"trak", *b"mdat"],
                )
            );
        } else {
            assert_eq!(
                find_byte_from(&data, 0xA7, 0),
                scalar::find_byte_from(&data, 0xA7, 0)
            );
            assert_eq!(
                find_any_byte_from(&data, &needles, 0),
                scalar::find_any_byte_from(&data, &needles, 0)
            );
        }
    }
}
