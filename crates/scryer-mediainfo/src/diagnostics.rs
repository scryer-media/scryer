//! Opt-in bounded structural inspection. No picture or audio decoding is performed.

use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Instant;

use scryer_media_types::{
    DiagnosticCoverage, DiscSelection, ProbeReport, ProbeStatus, ProbeWarning,
    StructuralDiagnostics,
};

use crate::source::{BoundedSource, FileSource, MediaSource};
use crate::{AnalysisProfile, AnalyzeOptions, MediaInfoError};

const MAX_RANGES: usize = 4096;
const MAX_WARNINGS: usize = 256;
const MAX_BUDGET: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct DiagnosticsOptions {
    pub read_budget_bytes: u64,
    pub sample_windows: usize,
    pub disc_selection: DiscSelection,
}

impl Default for DiagnosticsOptions {
    fn default() -> Self {
        Self {
            read_budget_bytes: 8 * 1024 * 1024,
            sample_windows: 4,
            disc_selection: DiscSelection::default(),
        }
    }
}

pub fn diagnose_file(
    path: &Path,
    options: DiagnosticsOptions,
) -> Result<StructuralDiagnostics, MediaInfoError> {
    let mut source = FileSource::open(path)?;
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let report = diagnose_source(&mut source, extension, options);
    if !source.unchanged(path)? {
        return Err(MediaInfoError::Io(
            "source changed during diagnostics".into(),
        ));
    }
    Ok(report)
}

pub fn diagnose_source(
    source: &mut dyn MediaSource,
    extension: &str,
    options: DiagnosticsOptions,
) -> StructuralDiagnostics {
    let started = Instant::now();
    let budget = options.read_budget_bytes.min(MAX_BUDGET);
    let length = source.len();
    let mut trace = TraceSource::new(BoundedSource::new(source, budget));
    let mut result = StructuralDiagnostics {
        source_length: length,
        report: ProbeReport {
            status: ProbeStatus::Incomplete,
            ..Default::default()
        },
        checks: vec!["container_and_reference_headers".into()],
        ..Default::default()
    };
    let mut codecs = BTreeMap::new();
    // Reserve independent payload sampling capacity even when container enrichment is large.
    let mut headers = BoundedSource::new(&mut trace, budget / 2);
    let header_report = if extension.eq_ignore_ascii_case("iso") {
        result
            .checks
            .push("disc_filesystem_and_navigation_references".into());
        crate::disc::diagnose_source(&mut headers, options.disc_selection)
    } else {
        match crate::analyze_source(
            &mut headers,
            extension,
            AnalyzeOptions {
                profile: AnalysisProfile::DefaultRich,
            },
        ) {
            Ok(analysis) => {
                for stream in &analysis.details.streams {
                    if let Some(pid) = stream
                        .metadata
                        .id
                        .as_deref()
                        .and_then(|id| id.parse::<u16>().ok())
                    {
                        if let Some(codec) = &stream.codec {
                            codecs.insert(pid, codec.clone());
                        }
                    }
                }
                analysis.details.report
            }
            Err(error) => ProbeReport {
                status: match error {
                    MediaInfoError::UnsupportedFormat(_) => ProbeStatus::Unsupported,
                    MediaInfoError::Parse(_) => ProbeStatus::Malformed,
                    MediaInfoError::Io(_) => ProbeStatus::Incomplete,
                },
                warnings: vec![ProbeWarning {
                    code: "container_inspection".into(),
                    message: error.to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        }
    };
    let header_budget_exhausted = headers.exhausted || header_report.budget_exhausted;
    drop(headers);
    // A parser failure after a read limit is inconclusive, even if wrapped as a parse error.
    if !header_budget_exhausted
        && matches!(
            header_report.status,
            ProbeStatus::Malformed | ProbeStatus::Encrypted | ProbeStatus::Unsupported
        )
    {
        result.report.status = header_report.status;
    }
    for warning in header_report.warnings {
        push_warning(&mut result, warning);
    }
    result.report.budget_exhausted = header_budget_exhausted;
    if !extension.eq_ignore_ascii_case("iso") {
        if let Err(error) = sample_transport(
            &mut trace,
            budget,
            options.sample_windows.clamp(1, 8),
            &codecs,
            &mut result,
        ) {
            warn(
                &mut result,
                "sampling_incomplete",
                &error.to_string(),
                None,
                None,
            );
        }
    }
    result.report.bytes_read = trace.inner.bytes_read;
    result.report.seeks = trace.inner.seeks;
    result.report.elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    result.report.budget_exhausted |= trace.inner.exhausted;
    result.coverage = trace.ranges;
    result.coverage_truncated = trace.truncated;
    warn(
        &mut result,
        "sampled_headers_only",
        "Only bounded container and payload headers were inspected; unsampled media and decoded content were not validated",
        None,
        None,
    );
    result
}

struct TraceSource<R> {
    inner: R,
    position: u64,
    ranges: Vec<DiagnosticCoverage>,
    truncated: bool,
}
impl<R> TraceSource<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            position: 0,
            ranges: Vec::new(),
            truncated: false,
        }
    }
    fn record(&mut self, offset: u64, length: u64) {
        if length == 0 {
            return;
        }
        let first = self
            .ranges
            .partition_point(|r| r.offset + r.length < offset);
        let mut start = offset;
        let mut end = offset + length;
        let mut last = first;
        while let Some(range) = self.ranges.get(last).filter(|r| r.offset <= end) {
            start = start.min(range.offset);
            end = end.max(range.offset + range.length);
            last += 1;
        }
        if first == last && self.ranges.len() >= MAX_RANGES {
            self.truncated = true;
            return;
        }
        self.ranges.splice(
            first..last,
            [DiagnosticCoverage {
                offset: start,
                length: end - start,
            }],
        );
    }
}
impl<R: Read> Read for TraceSource<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(bytes)?;
        self.record(self.position, count as u64);
        self.position += count as u64;
        Ok(count)
    }
}
impl<R: Seek> Seek for TraceSource<R> {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        self.position = self.inner.seek(offset)?;
        Ok(self.position)
    }
}
impl<R: MediaSource> MediaSource for TraceSource<R> {
    fn len(&self) -> u64 {
        self.inner.len()
    }
}

fn push_warning(result: &mut StructuralDiagnostics, warning: ProbeWarning) {
    if result.report.warnings.len() < MAX_WARNINGS {
        result.report.warnings.push(warning);
    } else {
        result.warnings_truncated = true;
    }
}
fn warn(
    result: &mut StructuralDiagnostics,
    code: &str,
    message: &str,
    pid: Option<u16>,
    offset: Option<u64>,
) {
    push_warning(
        result,
        ProbeWarning {
            code: code.into(),
            message: message.into(),
            stream_id: pid.map(|v| v.to_string()),
            offset,
        },
    );
}

fn sample_transport<R: MediaSource>(
    source: &mut TraceSource<BoundedSource<R>>,
    budget: u64,
    windows: usize,
    codecs: &BTreeMap<u16, String>,
    result: &mut StructuralDiagnostics,
) -> io::Result<()> {
    source.seek(SeekFrom::Start(0))?;
    let mut prefix = [0; 1024];
    let count = source
        .len()
        .min(prefix.len() as u64)
        .min(budget.saturating_sub(source.inner.bytes_read)) as usize;
    if count == 0 {
        return Ok(());
    }
    source.read_exact(&mut prefix[..count])?;
    let layout = [(188_usize, 0_usize), (192, 4), (204, 0)]
        .into_iter()
        .find(|(size, sync)| {
            (0..3).all(|n| {
                prefix
                    .get(sync + n * size)
                    .filter(|_| sync + n * size < count)
                    == Some(&0x47)
            })
        });
    let Some((size, sync)) = layout else {
        if prefix[..count].starts_with(&[0, 0, 1, 0xba]) {
            return sample_program(source, budget, windows, result);
        }
        return Ok(());
    };
    result.checks.extend(
        [
            "transport_packet_headers",
            "transport_continuity",
            "presentation_timestamp_continuity",
            "elementary_frame_headers",
        ]
        .map(str::to_string),
    );
    let total_packets = source.len() / size as u64;
    let per_window =
        (budget.saturating_sub(source.inner.bytes_read) / windows as u64 / size as u64)
            .min(1024 * 1024 / size as u64);
    if per_window == 0 {
        result.report.budget_exhausted = true;
        return Ok(());
    }
    let per_window = per_window.min(total_packets);
    let mut previous_end = 0;
    for index in 0..windows {
        let first = if windows == 1 {
            0
        } else {
            total_packets.saturating_sub(per_window) * index as u64 / (windows - 1) as u64
        };
        let first = first.max(previous_end);
        let count = per_window.min(total_packets.saturating_sub(first));
        if count == 0 {
            continue;
        }
        let offset = first * size as u64;
        source.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; count as usize * size];
        source.read_exact(&mut bytes)?;
        inspect_packets(&bytes, size, sync, offset, codecs, result);
        previous_end = first + count;
    }
    if source.len() % size as u64 != 0 {
        warn(
            result,
            "transport_truncation",
            "Source ends with an incomplete transport packet",
            None,
            Some(total_packets * size as u64),
        );
    }
    Ok(())
}

fn sample_program<R: MediaSource>(
    source: &mut TraceSource<BoundedSource<R>>,
    budget: u64,
    windows: usize,
    result: &mut StructuralDiagnostics,
) -> io::Result<()> {
    result.checks.extend(
        [
            "program_stream_packet_headers",
            "presentation_timestamp_continuity",
            "elementary_frame_headers",
        ]
        .map(str::to_owned),
    );
    let count = (budget.saturating_sub(source.inner.bytes_read) / windows as u64)
        .min(1024 * 1024)
        .min(source.len());
    if count == 0 {
        result.report.budget_exhausted = true;
        return Ok(());
    }
    let mut previous_end = 0;
    for index in 0..windows {
        let first = if windows == 1 {
            0
        } else {
            (u128::from(source.len().saturating_sub(count)) * index as u128 / (windows - 1) as u128)
                as u64
        }
        .max(previous_end);
        let count = count.min(source.len().saturating_sub(first));
        if count == 0 {
            continue;
        }
        source.seek(SeekFrom::Start(first))?;
        let mut bytes = vec![0; count as usize];
        source.read_exact(&mut bytes)?;
        inspect_program_packets(&bytes, first, source.len(), result);
        previous_end = first + count;
    }
    Ok(())
}

fn program_warning(
    result: &mut StructuralDiagnostics,
    code: &str,
    message: &str,
    id: u16,
    offset: u64,
) {
    push_warning(
        result,
        ProbeWarning {
            code: code.into(),
            message: message.into(),
            stream_id: Some(format!("{id:04x}")),
            offset: Some(offset),
        },
    );
}

fn inspect_program_packets(
    bytes: &[u8],
    offset: u64,
    source_length: u64,
    result: &mut StructuralDiagnostics,
) {
    let mut pos = 0;
    let mut timestamps = BTreeMap::new();
    while let Some(found) = crate::scan::find_program_start_code(bytes, pos) {
        pos = found;
        let stream = bytes[pos + 3];
        let at = offset + pos as u64;
        result.packets_sampled += 1;
        if stream == 0xb9 {
            pos += 4;
            continue;
        }
        let header = if stream == 0xba {
            match bytes.get(pos + 4) {
                Some(value) if value & 0xc0 == 0x40 => {
                    bytes.get(pos + 13).map(|value| 14 + usize::from(value & 7))
                }
                Some(value) if value & 0xf0 == 0x20 => Some(12),
                Some(_) => {
                    program_warning(
                        result,
                        "program_pack_header",
                        "Unrecognized pack header version",
                        u16::from(stream),
                        at,
                    );
                    pos += 4;
                    continue;
                }
                None => None,
            }
        } else {
            bytes
                .get(pos + 4..pos + 6)
                .map(|length| 6 + usize::from(u16::from_be_bytes([length[0], length[1]])))
        };
        let Some(length) = header else {
            if offset + bytes.len() as u64 == source_length {
                program_warning(
                    result,
                    "program_truncation",
                    "Source ends inside a packet header",
                    u16::from(stream),
                    at,
                );
            }
            break;
        };
        if length as u64 > source_length.saturating_sub(at) {
            program_warning(
                result,
                "program_truncation",
                "Declared packet extends beyond the source",
                u16::from(stream),
                at,
            );
            break;
        }
        if length > bytes.len() - pos {
            // The packet can continue outside this sampled window. Its source
            // extent was checked above; a sample boundary is not truncation.
            break;
        }
        let packet = &bytes[pos..pos + length];
        pos += length;
        if !matches!(stream, 0xbd | 0xc0..=0xef) {
            continue;
        }
        if length == 6 {
            program_warning(
                result,
                "program_unbounded_packet",
                "A zero-length PES packet cannot be fully delimited in this sample",
                u16::from(stream),
                at,
            );
            continue;
        }
        let body = &packet[6..];
        let mpeg2 = body[0] & 0xc0 == 0x80;
        if mpeg2 && body[0] & 0x30 != 0 {
            result.report.status = ProbeStatus::Encrypted;
            program_warning(
                result,
                "program_scrambled",
                "PES packet declares scrambled payload",
                u16::from(stream),
                at,
            );
            continue;
        }
        let Some(crate::ps::PesPayload {
            mut payload,
            timestamp,
            timestamps_valid,
        }) = crate::ps::pes_payload(body)
        else {
            program_warning(
                result,
                "program_pes_header",
                "PES header does not fit its declared packet",
                u16::from(stream),
                at,
            );
            continue;
        };
        let mut id = u16::from(stream);
        if stream == 0xbd {
            let Some(substream) = payload.first().copied() else {
                program_warning(
                    result,
                    "program_private_header",
                    "Private packet has no substream identifier",
                    id,
                    at,
                );
                continue;
            };
            id = 0xbd00 | u16::from(substream);
            let skip = if substream >= 0x80 { 4 } else { 1 };
            let Some(rest) = payload.get(skip..) else {
                program_warning(
                    result,
                    "program_private_header",
                    "Private stream header is truncated",
                    id,
                    at,
                );
                continue;
            };
            payload = rest;
        }
        if !timestamps_valid {
            program_warning(
                result,
                "presentation_timestamp_markers",
                "PES timestamp flags, length, or marker bits are invalid",
                id,
                at,
            );
        } else if let Some(timestamp) = timestamp {
            if let Some(old) = timestamps.insert(id, timestamp) {
                let backward = (old + (1_u64 << 33) - timestamp) % (1_u64 << 33);
                if backward > 90_000 && backward < (1_u64 << 32) {
                    program_warning(
                        result,
                        "presentation_timestamp_regression",
                        "Presentation timestamp moves backward by over one second within a sampled window",
                        id,
                        at,
                    );
                }
            }
        }
        if (0xe0..=0xef).contains(&stream) {
            let mut cursor = 0;
            while let Some(found) = crate::scan::find_mpeg_start_code(&payload[cursor..], 0) {
                let at = cursor + found;
                let Some(header) = payload.get(at..at + 6) else {
                    break;
                };
                result.frame_headers_sampled +=
                    u64::from((1..=4).contains(&((header[5] >> 3) & 7)));
                cursor = at + 3;
            }
        }
    }
}

fn inspect_packets(
    bytes: &[u8],
    size: usize,
    sync: usize,
    offset: u64,
    codecs: &BTreeMap<u16, String>,
    result: &mut StructuralDiagnostics,
) {
    let mut continuity: BTreeMap<u16, (u8, [u8; 188])> = BTreeMap::new();
    let mut timestamps = BTreeMap::new();
    for (index, raw) in bytes.chunks_exact(size).enumerate() {
        let packet: &[u8; 188] = raw[sync..sync + 188]
            .try_into()
            .expect("validated transport packet layout");
        let at = offset + (index * size + sync) as u64;
        result.packets_sampled += 1;
        if packet[0] != 0x47 {
            warn(
                result,
                "transport_sync",
                "Transport sync byte is missing",
                None,
                Some(at),
            );
            continue;
        }
        let pid = u16::from(packet[1] & 0x1f) * 256 + u16::from(packet[2]);
        if packet[1] & 0x80 != 0 {
            warn(
                result,
                "transport_error_indicator",
                "Packet declares a transport error",
                Some(pid),
                Some(at),
            );
        }
        if packet[3] & 0xc0 != 0 {
            result.report.status = ProbeStatus::Encrypted;
            warn(
                result,
                "transport_scrambled",
                "Packet declares scrambled payload",
                Some(pid),
                Some(at),
            );
            continue;
        }
        let mode = (packet[3] >> 4) & 3;
        if mode == 0 {
            warn(
                result,
                "transport_adaptation_control",
                "Reserved adaptation-field control",
                Some(pid),
                Some(at),
            );
            continue;
        }
        let payload_start = if mode & 2 != 0 {
            5 + packet[4] as usize
        } else {
            4
        };
        if payload_start > 188
            || (mode == 2 && payload_start != 188)
            || (mode == 3 && payload_start >= 188)
        {
            warn(
                result,
                "transport_adaptation_length",
                "Adaptation field lies outside its packet or leaves no declared payload",
                Some(pid),
                Some(at),
            );
            continue;
        }
        let discontinuity = mode & 2 != 0 && packet[4] > 0 && packet[5] & 0x80 != 0;
        if discontinuity {
            continuity.remove(&pid);
            timestamps.remove(&pid);
        }
        if pid == 0x1fff {
            continue;
        }
        let counter = packet[3] & 15;
        let has_payload = mode & 1 != 0;
        if let Some((old, previous)) = continuity.get(&pid) {
            let expected = if has_payload { (old + 1) & 15 } else { *old };
            if counter != expected && !(has_payload && counter == *old && packet == previous) {
                warn(
                    result,
                    "transport_continuity_gap",
                    "Unexpected continuity counter within a sampled window",
                    Some(pid),
                    Some(at),
                );
            }
        }
        continuity.insert(pid, (counter, *packet));
        if !has_payload {
            continue;
        }
        let payload = &packet[payload_start..];
        let mut elementary = payload;
        if packet[1] & 0x40 != 0 && payload.starts_with(&[0, 0, 1]) && payload.len() >= 9 {
            let header_end = 9 + usize::from(payload[8]);
            if payload[6] & 0xc0 == 0x80 && header_end <= payload.len() {
                elementary = &payload[header_end..];
                if payload[7] & 0x80 != 0 && payload[8] >= 5 {
                    if let Some(pts) = pts(&payload[9..14]) {
                        if let Some(old) = timestamps.insert(pid, pts) {
                            let backward = (old + (1_u64 << 33) - pts) % (1_u64 << 33);
                            if backward > 90_000 && backward < (1_u64 << 32) {
                                warn(
                                    result,
                                    "presentation_timestamp_regression",
                                    "Presentation timestamp moves backward by over one second within a sampled window",
                                    Some(pid),
                                    Some(at),
                                );
                            }
                        }
                    } else {
                        warn(
                            result,
                            "presentation_timestamp_markers",
                            "Invalid presentation timestamp marker bits",
                            Some(pid),
                            Some(at),
                        );
                    }
                }
            }
        }
        if let Some(codec) = codecs.get(&pid) {
            let mut cursor = 0;
            while let Some(at) = crate::scan::find_start_code_prefix(elementary, cursor) {
                let Some(start) = elementary.get(at..at + 5) else {
                    break;
                };
                cursor = at + 3;
                let valid = match codec.as_str() {
                    "h264" => start[3] & 0x80 == 0 && (1..=5).contains(&(start[3] & 31)),
                    "hevc" => {
                        start[3] & 0x80 == 0 && (start[3] >> 1) & 63 <= 31 && start[4] & 7 != 0
                    }
                    "mpeg1video" | "mpeg2video" => start[3] == 0,
                    _ => false,
                };
                result.frame_headers_sampled += u64::from(valid);
            }
        }
    }
}

fn pts(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 5 || bytes[0] & 1 == 0 || bytes[2] & 1 == 0 || bytes[4] & 1 == 0 {
        return None;
    }
    Some(
        (u64::from(bytes[0] & 14) << 29)
            | (u64::from(bytes[1]) << 22)
            | (u64::from(bytes[2] & 254) << 14)
            | (u64::from(bytes[3]) << 7)
            | u64::from(bytes[4] >> 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(counter: u8, timestamp: u64) -> [u8; 188] {
        let mut packet = [0xff; 188];
        packet[..13].copy_from_slice(&[
            0x47,
            0x41,
            0,
            0x10 | counter,
            0,
            0,
            1,
            0xe0,
            0,
            0,
            0x80,
            0x80,
            5,
        ]);
        packet[13..18].copy_from_slice(&[
            0x21 | (((timestamp >> 30) as u8 & 7) << 1),
            (timestamp >> 22) as u8,
            ((timestamp >> 14) as u8 & 0xfe) | 1,
            (timestamp >> 7) as u8,
            ((timestamp as u8 & 0x7f) << 1) | 1,
        ]);
        packet
    }

    #[test]
    fn program_sampling_checks_timestamps_scrambling_and_true_truncation() {
        fn pes(timestamp: u64) -> Vec<u8> {
            let mut bytes = packet(0, timestamp)[4..18].to_vec();
            bytes[4..6].copy_from_slice(&14_u16.to_be_bytes());
            bytes.extend([0, 0, 1, 0, 0, 8]);
            bytes
        }
        let bytes = [pes(900_000), pes(903_600), pes(90_000)].concat();
        let mut report = StructuralDiagnostics::default();
        inspect_program_packets(&bytes, 0, bytes.len() as u64, &mut report);
        assert_eq!(report.packets_sampled, 3);
        assert_eq!(report.frame_headers_sampled, 3);
        assert!(report.report.warnings.iter().any(|warning| warning.code
            == "presentation_timestamp_regression"
            && warning.stream_id.as_deref() == Some("00e0")));
        let bytes = [pes((1 << 33) - 1800), pes(1800)].concat();
        let mut report = StructuralDiagnostics::default();
        inspect_program_packets(&bytes, 0, bytes.len() as u64, &mut report);
        assert!(report.report.warnings.is_empty());
        let full = pes(0);
        for (source_length, truncated) in [
            (full.len() as u64 + 100, false),
            (full.len() as u64 - 1, true),
        ] {
            let mut report = StructuralDiagnostics::default();
            inspect_program_packets(&full[..full.len() - 1], 0, source_length, &mut report);
            assert_eq!(
                report
                    .report
                    .warnings
                    .iter()
                    .any(|warning| warning.code == "program_truncation"),
                truncated
            );
        }
        for (index, value, warning) in [
            (6, 0x90, "program_scrambled"),
            (8, 255, "program_pes_header"),
            (9, 0x20, "presentation_timestamp_markers"),
        ] {
            let mut bytes = full.clone();
            bytes[index] = value;
            let mut report = StructuralDiagnostics::default();
            inspect_program_packets(&bytes, 0, bytes.len() as u64, &mut report);
            assert!(
                report
                    .report
                    .warnings
                    .iter()
                    .any(|item| item.code == warning),
                "{warning}"
            );
            if index == 6 {
                assert_eq!(report.report.status, ProbeStatus::Encrypted);
            }
        }
    }

    #[test]
    fn program_diagnostics_share_the_aggregate_budget_and_report_partial_coverage() {
        let mut unit = vec![0, 0, 1, 0xba, 0x44, 0, 4, 0, 4, 1, 0, 0, 3, 0xf8];
        let mut pes = packet(0, 0)[4..18].to_vec();
        pes[4..6].copy_from_slice(&14_u16.to_be_bytes());
        pes.extend([0, 0, 1, 0, 0, 8]);
        unit.extend(pes);
        let mut source = io::Cursor::new(unit.repeat(5000));
        let report = diagnose_source(
            &mut source,
            "vob",
            DiagnosticsOptions {
                read_budget_bytes: 8192,
                sample_windows: 4,
                ..Default::default()
            },
        );
        assert!(
            report
                .checks
                .iter()
                .any(|check| check == "program_stream_packet_headers")
        );
        assert!(report.packets_sampled > 0);
        assert!(report.frame_headers_sampled > 0);
        assert!(report.report.bytes_read <= 8192);
        assert_eq!(report.report.status, ProbeStatus::Incomplete);
        assert!(
            report
                .coverage
                .iter()
                .map(|range| range.length)
                .sum::<u64>()
                < report.source_length
        );
        assert!(
            !report
                .report
                .warnings
                .iter()
                .any(|warning| warning.code == "program_truncation")
        );
    }

    #[test]
    fn transport_sampling_checks_gaps_markers_scrambling_and_discontinuities() {
        let mut result = StructuralDiagnostics::default();
        let packets = [packet(0, 900_000), packet(1, 903_600), packet(4, 90_000)];
        inspect_packets(&packets.concat(), 188, 0, 0, &BTreeMap::new(), &mut result);
        assert!(
            result
                .report
                .warnings
                .iter()
                .any(|w| w.code == "transport_continuity_gap")
        );
        assert!(
            result
                .report
                .warnings
                .iter()
                .any(|w| w.code == "presentation_timestamp_regression")
        );
        let mut clean = StructuralDiagnostics::default();
        inspect_packets(
            &[
                packet(0, (1 << 33) - 1800),
                packet(1, 1800),
                packet(1, 1800),
            ]
            .concat(),
            188,
            0,
            0,
            &BTreeMap::new(),
            &mut clean,
        );
        assert!(
            clean.report.warnings.is_empty(),
            "wrap and an identical duplicate are valid"
        );
        let mut scrambled = packet(2, 3600);
        scrambled[3] |= 0x80;
        inspect_packets(&scrambled, 188, 0, 0, &BTreeMap::new(), &mut clean);
        assert_eq!(clean.report.status, ProbeStatus::Encrypted);
    }

    #[test]
    fn diagnostics_enforce_aggregate_reads_and_report_partial_coverage() {
        let bytes = (0..100_000)
            .flat_map(|n| packet((n % 16) as u8, n * 3600))
            .collect::<Vec<_>>();
        let mut source = io::Cursor::new(bytes);
        let report = diagnose_source(
            &mut source,
            "ts",
            DiagnosticsOptions {
                read_budget_bytes: 128 * 1024,
                sample_windows: 3,
                ..Default::default()
            },
        );
        assert!(report.report.bytes_read <= 128 * 1024);
        assert!(report.packets_sampled > 0);
        assert!(report.coverage.iter().map(|r| r.length).sum::<u64>() < report.source_length);
        assert!(
            report
                .coverage
                .iter()
                .any(|r| r.offset > report.source_length / 2)
        );
        assert_eq!(report.report.status, ProbeStatus::Incomplete);
        assert!(
            report
                .report
                .warnings
                .iter()
                .any(|w| w.code == "sampled_headers_only")
        );
    }

    #[test]
    fn coverage_merges_rereads_and_warning_inventory_is_bounded() {
        let mut source = TraceSource::new(io::Cursor::new(vec![0; 32]));
        source.read_exact(&mut [0; 10]).unwrap();
        source.seek(SeekFrom::Start(5)).unwrap();
        source.read_exact(&mut [0; 10]).unwrap();
        assert_eq!(
            source.ranges,
            vec![DiagnosticCoverage {
                offset: 0,
                length: 15
            }]
        );
        let mut result = StructuralDiagnostics::default();
        for _ in 0..1000 {
            warn(&mut result, "test", "test", None, None);
        }
        assert_eq!(result.report.warnings.len(), MAX_WARNINGS);
        assert!(result.warnings_truncated);
    }
}
