//! Bounded CLPI stream declarations and authored clock-sequence ranges.
use super::image::ImageError;
use scryer_media_types::{DiscSegment, Provenance, Rational, StreamDetail, StreamKind};

type Result<T> = std::result::Result<T, ImageError>;
#[derive(Clone)]
pub(super) struct ClipInfo {
    pub source_packets: u32,
    sequences: Vec<Sequence>,
    programs: Vec<Program>,
    entry_maps: Vec<super::cpi::EntryMap>,
}
impl ClipInfo {
    pub(super) fn stream_count(&self) -> usize {
        self.programs
            .iter()
            .map(|program| program.streams.len())
            .sum()
    }
    pub(super) fn point_count(&self) -> usize {
        self.entry_maps
            .iter()
            .map(super::cpi::EntryMap::point_count)
            .sum()
    }
}
#[derive(Clone)]
struct Sequence {
    id: u8,
    start_packet: u32,
    input: u32,
    output: u32,
}
#[derive(Clone)]
struct Program {
    start_packet: u32,
    streams: Vec<StreamDetail>,
}

struct Fields<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Fields<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(len)
            .ok_or(ImageError::Malformed("CLPI field overflow"))?;
        let data = self
            .data
            .get(self.at..end)
            .ok_or(ImageError::Malformed("truncated CLPI field"))?;
        self.at = end;
        Ok(data)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn word(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn dword(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
}
fn section(data: &[u8], at: u32) -> Result<Fields<'_>> {
    if at < 40 {
        return Err(ImageError::Malformed("CLPI section overlaps header"));
    }
    let mut cursor = Fields {
        data,
        at: at as usize,
    };
    let size = cursor.dword()? as usize;
    Ok(Fields {
        data: cursor.take(size)?,
        at: 0,
    })
}

pub(super) fn parse(data: &[u8]) -> Result<ClipInfo> {
    if data.len() > 4 * 1024 * 1024 {
        return Err(ImageError::Budget);
    }
    let mut header = Fields { data, at: 0 };
    if header.take(4)? != b"HDMV" || !matches!(header.take(4)?, b"0100" | b"0200" | b"0300") {
        return Err(ImageError::Unsupported("CLPI signature or version"));
    }
    let sequence_at = header.dword()?;
    let program_at = header.dword()?;
    let cpi_at = header.dword()?;
    let mut info = section(data, 40)?;
    info.take(12)?;
    let source_packets = info.dword()?;
    if (sequence_at as usize) < 44 + info.data.len() {
        return Err(ImageError::Malformed(
            "overlapping CLPI clip and sequence sections",
        ));
    }
    let mut timing = section(data, sequence_at)?;
    if (program_at as usize) < sequence_at as usize + 4 + timing.data.len() {
        return Err(ImageError::Malformed(
            "overlapping CLPI sequence and program sections",
        ));
    }
    timing.byte()?;
    let count = timing.byte()?;
    let mut sequences = Vec::new();
    for _ in 0..count {
        let atc_start = timing.dword()?;
        let count = timing.byte()?;
        let offset = timing.byte()?;
        if atc_start >= source_packets {
            return Err(ImageError::Malformed("CLPI ATC packet outside clip"));
        }
        for index in 0..count {
            let id = offset
                .checked_add(index)
                .ok_or(ImageError::Malformed("CLPI clock identity overflow"))?;
            let pcr_pid = timing.word()?;
            let start_packet = timing.dword()?;
            let input = timing.dword()?;
            let output = timing.dword()?;
            if pcr_pid >= 8192
                || start_packet < atc_start
                || start_packet >= source_packets
                || output <= input
                || sequences
                    .iter()
                    .any(|sequence: &Sequence| sequence.id == id)
            {
                return Err(ImageError::Malformed("invalid CLPI clock sequence"));
            }
            sequences.push(Sequence {
                id,
                start_packet,
                input,
                output,
            });
            if sequences.len() > 256 {
                return Err(ImageError::Budget);
            }
        }
    }
    let mut inventory = section(data, program_at)?;
    inventory.byte()?;
    let count = inventory.byte()?;
    let mut programs = Vec::new();
    let mut stream_count = 0_usize;
    for _ in 0..count {
        let start_packet = inventory.dword()?;
        let map_pid = inventory.word()?;
        let count = inventory.byte()?;
        inventory.byte()?;
        if map_pid >= 8192
            || start_packet >= source_packets
            || programs
                .last()
                .is_some_and(|previous: &Program| previous.start_packet >= start_packet)
        {
            return Err(ImageError::Malformed("invalid CLPI program sequence"));
        }
        let mut streams = Vec::new();
        for _ in 0..count {
            let pid = inventory.word()?;
            let length = inventory.byte()? as usize;
            let attributes = inventory.take(length)?;
            if pid >= 8192
                || streams.iter().any(|stream: &StreamDetail| {
                    stream.metadata.id.as_deref() == Some(&pid.to_string())
                })
            {
                return Err(ImageError::Malformed("invalid CLPI stream PID"));
            }
            streams.push(stream(pid, attributes)?);
            stream_count += 1;
            if stream_count > 4096 {
                return Err(ImageError::Budget);
            }
        }
        programs.push(Program {
            start_packet,
            streams,
        });
    }
    if sequences.is_empty() || programs.is_empty() {
        return Err(ImageError::Malformed(
            "CLPI has no clock or program sequence",
        ));
    }
    let entry_maps = if cpi_at == 0 {
        Vec::new()
    } else {
        if (cpi_at as usize) < program_at as usize + 4 + inventory.data.len() {
            return Err(ImageError::Malformed(
                "overlapping CLPI program and CPI sections",
            ));
        }
        super::cpi::parse(section(data, cpi_at)?.data, source_packets)?
    };
    Ok(ClipInfo {
        source_packets,
        sequences,
        programs,
        entry_maps,
    })
}

impl ClipInfo {
    pub(super) fn streams_for(&self, segment: &DiscSegment) -> Result<Vec<StreamDetail>> {
        let range = self.packet_range(segment)?;
        let first_index = self
            .programs
            .iter()
            .rposition(|program| program.start_packet <= range.start)
            .ok_or(ImageError::Malformed("CLPI playback range has no program"))?;
        let first = &self.programs[first_index].streams;
        if self.programs[first_index..]
            .iter()
            .take_while(|program| program.start_packet < range.end)
            .any(|program| program.streams != *first)
        {
            return Err(ImageError::Unsupported(
                "CLPI stream formats change within selected playback range",
            ));
        }
        Ok(first.clone())
    }

    pub(super) fn packet_range(&self, segment: &DiscSegment) -> Result<std::ops::Range<u32>> {
        let sequence = self
            .sequences
            .iter()
            .find(|sequence| Some(sequence.id) == segment.sequence_id)
            .ok_or(ImageError::Malformed(
                "playlist references missing CLPI clock sequence",
            ))?;
        if !segment.in_seconds.is_finite()
            || !segment.out_seconds.is_finite()
            || segment.out_seconds <= segment.in_seconds
            || segment.in_seconds < f64::from(sequence.input) / 45_000.0
            || segment.out_seconds > f64::from(sequence.output) / 45_000.0
        {
            return Err(ImageError::Malformed(
                "playlist trim lies outside CLPI presentation range",
            ));
        }
        let end_packet = self
            .sequences
            .iter()
            .filter(|other| other.start_packet > sequence.start_packet)
            .map(|other| other.start_packet)
            .min()
            .unwrap_or(self.source_packets);
        let program = self
            .programs
            .iter()
            .rfind(|program| program.start_packet <= sequence.start_packet)
            .ok_or(ImageError::Malformed("CLPI clock sequence has no program"))?;
        let pid = program
            .streams
            .iter()
            .find(|stream| stream.kind == StreamKind::Video)
            .and_then(|stream| stream.metadata.id.as_deref()?.parse::<u16>().ok());
        let range = sequence.start_packet..end_packet;
        let input = (segment.in_seconds * 45_000.0).round() as u32;
        let output = (segment.out_seconds * 45_000.0).round() as u32;
        if let Some(map) = self.entry_maps.iter().find(|map| Some(map.pid) == pid) {
            map.range(range, input, output)
        } else if input == sequence.input && output == sequence.output {
            Ok(range)
        } else {
            Err(ImageError::Unsupported(
                "trimmed CLPI sequence lacks a video access-point map",
            ))
        }
    }
}

fn stream(pid: u16, data: &[u8]) -> Result<StreamDetail> {
    let mut fields = Fields { data, at: 0 };
    let coding = fields.byte()?;
    let (kind, codec) = match coding {
        1 => (StreamKind::Video, "mpeg1video"),
        2 => (StreamKind::Video, "mpeg2video"),
        0x1b | 0x20 => (StreamKind::Video, "h264"),
        0x24 => (StreamKind::Video, "hevc"),
        0xea => (StreamKind::Video, "vc1"),
        3 | 4 => (StreamKind::Audio, "mp2"),
        0x80 => (StreamKind::Audio, "pcm_bluray"),
        0x81 => (StreamKind::Audio, "ac3"),
        0x82 | 0x85 | 0x86 | 0xa2 => (StreamKind::Audio, "dts"),
        0x83 => (StreamKind::Audio, "truehd"),
        0x84 | 0xa1 => (StreamKind::Audio, "eac3"),
        0x90 => (StreamKind::Subtitle, "hdmv_pgs_subtitle"),
        0x91 => (StreamKind::Subtitle, "hdmv_interactive_graphics"),
        0x92 => (StreamKind::Subtitle, "hdmv_text_subtitle"),
        _ => return Err(ImageError::Unsupported("unrecognized CLPI stream coding")),
    };
    let mut stream = StreamDetail {
        kind,
        codec: Some(codec.into()),
        ..Default::default()
    };
    stream.metadata.id = Some(pid.to_string());
    match kind {
        StreamKind::Video => {
            let format_rate = fields.byte()?;
            stream.height = match format_rate >> 4 {
                1 | 3 => Some(480),
                2 | 7 => Some(576),
                4 | 6 => Some(1080),
                5 => Some(720),
                8 => Some(2160),
                _ => None,
            };
            stream.metadata.declared_frame_rate = match format_rate & 15 {
                1 => Rational::new(24_000, 1001),
                2 => Rational::new(24, 1),
                3 => Rational::new(25, 1),
                4 => Rational::new(30_000, 1001),
                6 => Rational::new(50, 1),
                7 => Rational::new(60_000, 1001),
                _ => None,
            };
            stream.metadata.display_aspect_ratio = match fields.byte()? >> 4 {
                2 => Rational::new(4, 3),
                3 => Rational::new(16, 9),
                _ => None,
            };
            if coding == 0x24 {
                let dynamic_color = fields.byte()?;
                stream.metadata.color.primaries = match dynamic_color & 15 {
                    1 => Some(1),
                    2 => Some(9),
                    _ => None,
                };
                stream.metadata.color.provenance = Provenance::Container;
                match dynamic_color >> 4 {
                    1 => stream.metadata.hdr.hdr10 = Some(true),
                    2 => stream.metadata.hdr.dolby_vision = Some(true),
                    _ => {}
                }
                if fields.byte()? & 0x80 != 0 {
                    stream.metadata.hdr.hdr10plus = Some(true);
                }
            }
        }
        StreamKind::Audio => {
            let format_rate = fields.byte()?;
            stream.channels = match format_rate >> 4 {
                1 => Some(1),
                3 => Some(2),
                _ => None,
            };
            stream.metadata.channel_layout = stream
                .channels
                .map(|channels| if channels == 1 { "mono" } else { "stereo" }.into());
            stream.metadata.sample_rate = match format_rate & 15 {
                1 => Some(48_000),
                4 => Some(96_000),
                5 => Some(192_000),
                _ => None,
            };
            stream.metadata.profile = match coding {
                0x85 | 0xa2 => Some("DTS-HD HRA".into()),
                0x86 => Some("DTS-HD MA".into()),
                _ => None,
            };
            language(&mut stream, fields.take(3)?)?;
        }
        StreamKind::Subtitle => {
            if coding == 0x92 {
                fields.byte()?;
            }
            language(&mut stream, fields.take(3)?)?;
        }
    }
    Ok(stream)
}
fn language(stream: &mut StreamDetail, language: &[u8]) -> Result<()> {
    if language.iter().all(|byte| *byte == 0) {
        return Ok(());
    }
    if !language.iter().all(u8::is_ascii_alphabetic) {
        return Err(ImageError::Malformed("invalid CLPI language"));
    }
    let value = std::str::from_utf8(language).unwrap().to_ascii_lowercase();
    stream.metadata.original_language = Some(value.clone());
    stream.metadata.language_provenance = Provenance::Container;
    if value != "und" {
        stream.language = Some(value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hevc_capabilities_remain_independent() {
        let video = stream(0x1011, &[0x24, 0x81, 0x30, 0x22, 0x80]).unwrap();
        assert_eq!(video.height, Some(2160));
        assert_eq!(
            video.metadata.declared_frame_rate,
            Rational::new(24_000, 1001)
        );
        assert_eq!(video.metadata.hdr.dolby_vision, Some(true));
        assert_eq!(video.metadata.hdr.hdr10plus, Some(true));
        assert_eq!(video.metadata.color.primaries, Some(9));
    }

    #[test]
    fn multichannel_declaration_does_not_invent_a_layout() {
        let audio = stream(0x1100, &[0x81, 0x61, b'e', b'n', b'g']).unwrap();
        assert_eq!(audio.channels, None);
        assert_eq!(audio.metadata.channel_layout, None);
        assert_eq!(audio.metadata.sample_rate, Some(48_000));
    }
}
