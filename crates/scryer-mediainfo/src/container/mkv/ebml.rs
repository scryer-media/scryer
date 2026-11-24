//! EBML element ids, and the primitives that read variable-size integers
//! and elements from byte slices and readers.

use std::io::Read;

use crate::ProbeError;

pub(super) const EBML_ID_EBML: u32 = 0x1A45_DFA3;
pub(super) const EBML_ID_DOC_TYPE: u32 = 0x4282;
pub(super) const EBML_ID_SEGMENT: u32 = 0x1853_8067;
pub(super) const EBML_ID_SEEK_HEAD: u32 = 0x114D_9B74;
pub(super) const EBML_ID_SEEK: u32 = 0x4DBB;
pub(super) const EBML_ID_SEEK_ID: u32 = 0x53AB;
pub(super) const EBML_ID_SEEK_POSITION: u32 = 0x53AC;
pub(super) const EBML_ID_INFO: u32 = 0x1549_A966;
pub(super) const EBML_ID_TIMESTAMP_SCALE: u32 = 0x002A_D7B1;
pub(super) const EBML_ID_DURATION: u32 = 0x4489;
pub(super) const EBML_ID_TRACKS: u32 = 0x1654_AE6B;
pub(super) const EBML_ID_TRACK_ENTRY: u32 = 0xAE;
pub(super) const EBML_ID_TRACK_NUMBER: u32 = 0xD7;
pub(super) const EBML_ID_TRACK_TYPE: u32 = 0x83;
pub(super) const EBML_ID_FLAG_DEFAULT: u32 = 0x88;
pub(super) const EBML_ID_FLAG_FORCED: u32 = 0x55AA;
pub(super) const EBML_ID_DEFAULT_DURATION: u32 = 0x0023_E383;
pub(super) const EBML_ID_NAME: u32 = 0x536E;
pub(super) const EBML_ID_LANGUAGE: u32 = 0x0022_B59C;
pub(super) const EBML_ID_LANGUAGE_BCP47: u32 = 0x0022_B59D;
pub(super) const EBML_ID_CODEC_ID: u32 = 0x86;
pub(super) const EBML_ID_CODEC_PRIVATE: u32 = 0x63A2;
pub(super) const EBML_ID_VIDEO: u32 = 0xE0;
pub(super) const EBML_ID_PIXEL_WIDTH: u32 = 0xB0;
pub(super) const EBML_ID_PIXEL_HEIGHT: u32 = 0xBA;
pub(super) const EBML_ID_COLOUR: u32 = 0x55B0;
pub(super) const EBML_ID_TRANSFER_CHARACTERISTICS: u32 = 0x55BA;
pub(super) const EBML_ID_AUDIO: u32 = 0xE1;
pub(super) const EBML_ID_CHANNELS: u32 = 0x9F;
pub(super) const EBML_ID_BIT_DEPTH: u32 = 0x6264;
pub(super) const EBML_ID_TRACK_CONTENT_ENCODINGS: u32 = 0x6D80;
pub(super) const EBML_ID_TRACK_CONTENT_ENCODING: u32 = 0x6240;
pub(super) const EBML_ID_ENCODING_ORDER: u32 = 0x5031;
pub(super) const EBML_ID_ENCODING_SCOPE: u32 = 0x5032;
pub(super) const EBML_ID_ENCODING_TYPE: u32 = 0x5033;
pub(super) const EBML_ID_ENCODING_COMPRESSION: u32 = 0x5034;
pub(super) const EBML_ID_ENCODING_COMP_ALGO: u32 = 0x4254;
pub(super) const EBML_ID_ENCODING_COMP_SETTINGS: u32 = 0x4255;
pub(super) const EBML_ID_CLUSTER: u32 = 0x1F43_B675;
pub(super) const EBML_ID_TIMESTAMP: u32 = 0xE7;
pub(super) const EBML_ID_SIMPLE_BLOCK: u32 = 0xA3;
pub(super) const EBML_ID_BLOCK_GROUP: u32 = 0xA0;
pub(super) const EBML_ID_BLOCK: u32 = 0xA1;
pub(super) const EBML_ID_BLOCK_ADDITIONS: u32 = 0x75A1;
pub(super) const EBML_ID_BLOCK_MORE: u32 = 0xA6;
pub(super) const EBML_ID_BLOCK_ADDITIONAL: u32 = 0xA5;
pub(super) const EBML_ID_CHAPTERS: u32 = 0x1043_A770;
pub(super) const EBML_ID_EDITION_ENTRY: u32 = 0x45B9;
pub(super) const EBML_ID_CHAPTER_ATOM: u32 = 0xB6;
pub(super) const EBML_ID_CHAPTER_TIME_START: u32 = 0x91;
pub(super) const EBML_ID_BLOCK_ADDITION_MAPPING: u32 = 0x41E4;
pub(super) const EBML_ID_BLOCK_ADD_ID_TYPE: u32 = 0x41E7;
pub(super) const EBML_ID_BLOCK_ADD_ID_EXTRA_DATA: u32 = 0x41ED;

#[cfg(test)]
pub(super) fn find_ebml_element_payload(mut data: &[u8], target_id: u32) -> Option<&[u8]> {
    while !data.is_empty() {
        let (id, payload, consumed) = next_ebml_element(data)?;
        if id == target_id {
            return Some(payload);
        }
        data = &data[consumed..];
    }
    None
}

pub(super) fn find_first_direct_ebml_child(data: &[u8], target_id: u32) -> Option<&[u8]> {
    let mut current = data;
    while !current.is_empty() {
        let (id, payload, consumed) = next_ebml_element(current)?;
        if id == target_id {
            return Some(payload);
        }
        current = &current[consumed..];
    }
    None
}

pub(super) fn next_ebml_element(data: &[u8]) -> Option<(u32, &[u8], usize)> {
    let (id, id_len) = parse_ebml_id(data)?;
    let (size, size_len) = parse_ebml_vint(&data[id_len..])?;
    let payload_start = id_len + size_len;
    let payload_end = payload_start.checked_add(size)?;
    if payload_end > data.len() {
        return None;
    }
    Some((id, &data[payload_start..payload_end], payload_end))
}

pub(super) fn parse_ebml_id(data: &[u8]) -> Option<(u32, usize)> {
    if data.is_empty() {
        return None;
    }
    let first = data[0];
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 4 || len > data.len() {
        return None;
    }

    let mut value = 0_u32;
    for &byte in &data[..len] {
        value = (value << 8) | u32::from(byte);
    }
    Some((value, len))
}

pub(super) fn parse_ebml_uint(data: &[u8]) -> Option<u64> {
    if data.is_empty() || data.len() > 8 {
        return None;
    }

    let mut value = 0_u64;
    for &byte in data {
        value = (value << 8) | u64::from(byte);
    }
    Some(value)
}

pub(super) fn parse_ebml_float(data: &[u8]) -> Option<f64> {
    match data.len() {
        4 => Some(f32::from_be_bytes(data.try_into().ok()?) as f64),
        8 => Some(f64::from_be_bytes(data.try_into().ok()?)),
        _ => None,
    }
}

pub(super) fn parse_ebml_string(data: &[u8]) -> Result<String, ProbeError> {
    let value = std::str::from_utf8(data)
        .map_err(|e| ProbeError::Malformed(format!("invalid utf-8 string: {e}")))?;
    Ok(value.trim_end_matches('\0').to_owned())
}

#[derive(Debug, Clone, Copy)]
pub(super) struct EbmlElementHeader {
    pub(super) id: u32,
    pub(super) data_offset: u64,
    pub(super) size: Option<u64>,
}

impl EbmlElementHeader {
    pub(super) fn end(self, parent_end: u64, file_len: u64) -> u64 {
        self.size
            .and_then(|size| self.data_offset.checked_add(size))
            .map(|end| end.min(parent_end).min(file_len))
            .unwrap_or_else(|| parent_end.min(file_len))
    }
}

pub(super) fn parse_ebml_vint_value(data: &[u8]) -> Option<(u64, usize)> {
    if data.is_empty() {
        return None;
    }
    let first = data[0];
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 8 || len > data.len() {
        return None;
    }

    let value_mask = if len == 8 { 0 } else { 0xFF >> len };
    let mut value = u64::from(first & value_mask);
    for &byte in &data[1..len] {
        value = (value << 8) | u64::from(byte);
    }
    Some((value, len))
}

pub(super) fn parse_ebml_sint_value(data: &[u8]) -> Option<(i64, usize)> {
    let (unsigned, len) = parse_ebml_vint_value(data)?;
    let value_bits = len.checked_mul(7)?;
    let bias = (1_i64.checked_shl((value_bits.checked_sub(1)?) as u32)?).checked_sub(1)?;
    let unsigned = i64::try_from(unsigned).ok()?;
    Some((unsigned - bias, len))
}

pub(super) fn read_u8_from_reader<R: Read>(reader: &mut R) -> Result<u8, ProbeError> {
    let mut buf = [0_u8; 1];
    reader.read_exact(&mut buf).map_err(ProbeError::from)?;
    Ok(buf[0])
}

pub(super) fn read_ebml_vint_value_from_reader<R: Read>(
    reader: &mut R,
) -> Result<(u64, usize), ProbeError> {
    let first = read_u8_from_reader(reader)?;
    if first == 0 {
        return Err(ProbeError::Malformed("invalid ebml vint".into()));
    }

    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 8 {
        return Err(ProbeError::Malformed("invalid ebml vint".into()));
    }

    let mut buf = [0_u8; 8];
    buf[0] = first;
    if len > 1 {
        reader
            .read_exact(&mut buf[1..len])
            .map_err(ProbeError::from)?;
    }

    parse_ebml_vint_value(&buf[..len])
        .ok_or_else(|| ProbeError::Malformed("invalid ebml vint".into()))
}

pub(super) fn read_ebml_sint_value_from_reader<R: Read>(
    reader: &mut R,
) -> Result<(i64, usize), ProbeError> {
    let first = read_u8_from_reader(reader)?;
    if first == 0 {
        return Err(ProbeError::Malformed("invalid ebml signed vint".into()));
    }

    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 8 {
        return Err(ProbeError::Malformed("invalid ebml signed vint".into()));
    }

    let mut buf = [0_u8; 8];
    buf[0] = first;
    if len > 1 {
        reader
            .read_exact(&mut buf[1..len])
            .map_err(ProbeError::from)?;
    }

    parse_ebml_sint_value(&buf[..len])
        .ok_or_else(|| ProbeError::Malformed("invalid ebml signed vint".into()))
}

pub(super) fn read_ebml_id_from_reader<R: Read>(
    reader: &mut R,
) -> Result<Option<(u32, usize)>, ProbeError> {
    let mut first = [0_u8; 1];
    let bytes_read = reader.read(&mut first).map_err(ProbeError::from)?;
    if bytes_read == 0 {
        return Ok(None);
    }

    let len = first[0].leading_zeros() as usize + 1;
    if len == 0 || len > 4 {
        return Err(ProbeError::Malformed("invalid ebml id".into()));
    }

    let mut value = u32::from(first[0]);
    let mut rest = [0_u8; 3];
    if len > 1 {
        reader
            .read_exact(&mut rest[..len - 1])
            .map_err(ProbeError::from)?;
        for &byte in &rest[..len - 1] {
            value = (value << 8) | u32::from(byte);
        }
    }

    Ok(Some((value, len)))
}

pub(super) fn read_ebml_size_from_reader<R: Read>(
    reader: &mut R,
) -> Result<(Option<u64>, usize), ProbeError> {
    let mut first = [0_u8; 1];
    reader.read_exact(&mut first).map_err(ProbeError::from)?;

    if first[0] == 0 {
        return Err(ProbeError::Malformed("invalid ebml size".into()));
    }

    let len = first[0].leading_zeros() as usize + 1;
    if len > 8 {
        return Err(ProbeError::Malformed("invalid ebml size".into()));
    }

    let mask = if len == 8 { 0 } else { 0xFF >> len };
    let mut value = u64::from(first[0] & mask);
    let mut unknown = value == u64::from(mask);

    let mut rest = [0_u8; 7];
    if len > 1 {
        reader
            .read_exact(&mut rest[..len - 1])
            .map_err(ProbeError::from)?;
        for &byte in &rest[..len - 1] {
            value = (value << 8) | u64::from(byte);
            unknown &= byte == 0xFF;
        }
    }

    Ok(((!unknown).then_some(value), len))
}

/// Parse an EBML variable-size integer (VINT). Returns (value, bytes_consumed).
pub(super) fn parse_ebml_vint(data: &[u8]) -> Option<(usize, usize)> {
    if data.is_empty() {
        return None;
    }
    let first = data[0];
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || len > data.len() {
        return None;
    }
    let value_mask = if len == 8 { 0 } else { 0xFF >> len };
    let mut value = (first & value_mask) as usize;
    for &b in &data[1..len] {
        value = (value << 8) | b as usize;
    }
    Some((value, len))
}
