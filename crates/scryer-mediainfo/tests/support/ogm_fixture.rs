#![allow(dead_code)]

use ogg::{PacketWriteEndInfo, PacketWriter, reading::PacketReader};
use std::io::Cursor;

/// Synthetic declarations and timed packets, not a playable encoded movie.
pub fn modern_header(kind: &[u8; 8], subtype: &[u8; 4], time_unit: u64, samples: u64) -> Vec<u8> {
    let mut header = vec![1];
    header.extend_from_slice(kind);
    header.extend_from_slice(subtype);
    header.extend_from_slice(&52_u32.to_le_bytes());
    header.extend_from_slice(&time_unit.to_le_bytes());
    header.extend_from_slice(&samples.to_le_bytes());
    header.extend_from_slice(&1_u32.to_le_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());
    header.extend_from_slice(&16_u16.to_le_bytes());
    header.extend_from_slice(&0_u16.to_le_bytes());
    match kind {
        b"video\0\0\0" => {
            header.extend_from_slice(&1280_u32.to_le_bytes());
            header.extend_from_slice(&720_u32.to_le_bytes());
        }
        b"audio\0\0\0" => {
            header.extend_from_slice(&6_u16.to_le_bytes());
            header.extend_from_slice(&0_u16.to_le_bytes());
            header.extend_from_slice(&48_000_u32.to_le_bytes());
        }
        _ => header.extend_from_slice(&[0; 8]),
    }
    header
}

pub fn comments(entries: &[&str]) -> Vec<u8> {
    let mut packet = b"\x03vorbis".to_vec();
    packet.extend_from_slice(&7_u32.to_le_bytes());
    packet.extend_from_slice(b"fixture");
    packet.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        packet.extend_from_slice(&(entry.len() as u32).to_le_bytes());
        packet.extend_from_slice(entry.as_bytes());
    }
    packet.push(1);
    packet
}

pub fn vorbis_headers() -> [Vec<u8>; 3] {
    let bytes = include_bytes!("../media/ogv_theora_vorbis.ogv");
    let mut reader = PacketReader::new(Cursor::new(bytes.as_slice()));
    let mut serial = None;
    let mut packets = Vec::new();
    while let Some(packet) = reader.read_packet().unwrap() {
        if packet.data.starts_with(b"\x01vorbis") {
            serial = Some(packet.stream_serial());
        }
        if serial == Some(packet.stream_serial()) {
            packets.push(packet.data);
            if packets.len() == 3 {
                break;
            }
        }
    }
    packets.try_into().unwrap()
}

pub fn video_audio_text() -> Vec<u8> {
    let [vorbis, _, setup] = vorbis_headers();
    let rate = u32::from_le_bytes(vorbis[12..16].try_into().unwrap());
    let mut output = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new(&mut output);
    for (serial, header) in [
        (1, modern_header(b"video\0\0\0", b"XVID", 400_000, 1)),
        (2, vorbis),
        (3, modern_header(b"text\0\0\0\0", &[0; 4], 10_000, 1)),
    ] {
        writer
            .write_packet(header, serial, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
    }
    for (serial, packet) in [
        (1, comments(&["TITLE=Main video"])),
        (2, comments(&["LANGUAGE=eng", "TITLE=Stereo"])),
        (2, setup),
        (3, comments(&["Language=jpn", "NAME=Captions"])),
    ] {
        writer
            .write_packet(packet, serial, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
    }
    writer
        .write_packet(
            &[0x80, 0xf4, 1, b'H', b'i'],
            3,
            PacketWriteEndInfo::EndStream,
            2_000,
        )
        .unwrap();
    writer
        .write_packet(&[0x48, 1, 0], 1, PacketWriteEndInfo::EndStream, 74)
        .unwrap();
    writer
        .write_packet(&[0], 2, PacketWriteEndInfo::EndStream, u64::from(rate) * 3)
        .unwrap();
    drop(writer);
    output.into_inner()
}

pub fn audio_only() -> Vec<u8> {
    single_stream(
        &modern_header(b"audio\0\0\0", b"2000", 10_000_000, 48_000),
        Some(&comments(&["LANGUAGE=deu", "TITLE=Surround"])),
        &[0x80, 0, 6, 0],
        142_464,
        true,
    )
}

pub fn single_stream(
    header: &[u8],
    comment: Option<&[u8]>,
    data: &[u8],
    granule: u64,
    eos: bool,
) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new(&mut output);
    writer
        .write_packet(header, 7, PacketWriteEndInfo::EndPage, 0)
        .unwrap();
    if let Some(comment) = comment {
        writer
            .write_packet(comment, 7, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
    }
    writer
        .write_packet(
            data,
            7,
            if eos {
                PacketWriteEndInfo::EndStream
            } else {
                PacketWriteEndInfo::EndPage
            },
            granule,
        )
        .unwrap();
    drop(writer);
    output.into_inner()
}

pub fn legacy_video() -> Vec<u8> {
    let mut header = vec![0; 184];
    let magic = b"\x01Direct Show Samples embedded in Ogg";
    header[..magic.len()].copy_from_slice(magic);
    header[68..72].copy_from_slice(b"XVID");
    header[96..100].copy_from_slice(&0x0558_9f80_u32.to_le_bytes());
    header[164..172].copy_from_slice(&400_000_u64.to_le_bytes());
    header[176..180].copy_from_slice(&1280_u32.to_le_bytes());
    header[180..184].copy_from_slice(&720_u32.to_le_bytes());
    header
}

pub fn legacy_audio() -> Vec<u8> {
    let mut header = vec![0; 136];
    let magic = b"\x01Direct Show Samples embedded in Ogg";
    header[..magic.len()].copy_from_slice(magic);
    header[96..100].copy_from_slice(&0x0558_9f81_u32.to_le_bytes());
    header[124..126].copy_from_slice(&0x2000_u16.to_le_bytes());
    header[126..128].copy_from_slice(&6_u16.to_le_bytes());
    header[128..132].copy_from_slice(&48_000_u32.to_le_bytes());
    header[132..136].copy_from_slice(&48_000_u32.to_le_bytes());
    header
}

pub fn nil_eos_page(serial: u32, granule: u64, sequence: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    PacketWriter::new(&mut bytes)
        .write_packet(&[], serial, PacketWriteEndInfo::EndStream, granule)
        .unwrap();
    let mut page = bytes.into_inner();
    page.truncate(27);
    page[5] = 4;
    page[18..22].copy_from_slice(&sequence.to_le_bytes());
    page[22..26].fill(0);
    page[26] = 0;
    let mut checksum = 0_u32;
    for byte in &page {
        checksum ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            checksum = if checksum & 0x8000_0000 != 0 {
                (checksum << 1) ^ 0x04c1_1db7
            } else {
                checksum << 1
            };
        }
    }
    page[22..26].copy_from_slice(&checksum.to_le_bytes());
    page
}
