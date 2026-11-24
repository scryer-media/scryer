//! Box headers and the bounded box walk.

use super::*;

pub(super) fn for_each_mp4_box(mut data: &[u8], mut f: impl FnMut(Mp4BoxHeader, &[u8])) {
    while let Some(header) = read_box_header_from_bytes(data) {
        let size = header.size as usize;
        if size < header.header_size || size > data.len() {
            break;
        }
        let payload = &data[header.header_size..size];
        f(header, payload);
        if size == data.len() {
            break;
        }
        data = &data[size..];
    }
}

pub(super) fn walk_mp4_boxes<'a, FDescend, FVisit>(
    data: &'a [u8],
    max_depth: usize,
    mut should_descend: FDescend,
    mut visit: FVisit,
) where
    FDescend: FnMut(Mp4BoxHeader, &'a [u8], usize) -> Option<&'a [u8]>,
    FVisit: FnMut(Mp4BoxHeader, &'a [u8], usize),
{
    let mut stack = vec![(data, 0_usize)];

    while let Some((mut current, depth)) = stack.pop() {
        let mut children = Vec::new();

        while let Some(header) = read_box_header_from_bytes(current) {
            let size = header.size as usize;
            if size < header.header_size || size > current.len() {
                break;
            }

            let payload = &current[header.header_size..size];
            visit(header, payload, depth);

            if depth < max_depth
                && let Some(child_payload) = should_descend(header, payload, depth)
                && !child_payload.is_empty()
            {
                children.push((child_payload, depth + 1));
            }

            if size == current.len() {
                break;
            }
            current = &current[size..];
        }

        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
}

pub(super) fn read_top_level_box_header<R: Read>(
    reader: &mut R,
) -> Result<Option<(Mp4BoxHeader, Vec<u8>)>, ProbeError> {
    let mut header = [0_u8; 8];
    let mut read = 0;
    while read < header.len() {
        let bytes_read = reader.read(&mut header[read..]).map_err(ProbeError::from)?;
        if bytes_read == 0 {
            if read == 0 {
                return Ok(None);
            }
            return Err(ProbeError::Malformed("truncated MP4 box header".into()));
        }
        read += bytes_read;
    }

    let size32 = u32::from_be_bytes(header[0..4].try_into().unwrap()) as u64;
    let name: [u8; 4] = header[4..8].try_into().unwrap();
    let mut size = size32;
    let mut header_size = 8;
    let mut raw = header.to_vec();

    if size32 == 1 {
        let mut extended = [0_u8; 8];
        reader.read_exact(&mut extended).map_err(ProbeError::from)?;
        size = u64::from_be_bytes(extended);
        header_size = 16;
        raw.extend_from_slice(&extended);
    }

    if size != 0 && size < header_size as u64 {
        return Err(ProbeError::Malformed(format!(
            "invalid MP4 box size {} for {}",
            size,
            fourcc_to_string(name)
        )));
    }

    Ok(Some((
        Mp4BoxHeader {
            name,
            size,
            header_size,
        },
        raw,
    )))
}

pub(super) fn read_zero_sized_top_level_box<R: Read>(
    reader: &mut R,
    output: &mut Vec<u8>,
    pos: &mut u64,
) -> Result<(), ProbeError> {
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buf).map_err(ProbeError::from)?;
        if read == 0 {
            return Ok(());
        }
        if output.len().saturating_add(read) > MP4_METADATA_OUTPUT_MAX_BYTES {
            return Err(ProbeError::Malformed(
                "MP4 metadata output exceeds parser budget".into(),
            ));
        }
        output.extend_from_slice(&buf[..read]);
        *pos = (*pos).saturating_add(read as u64);
    }
}

pub(super) fn read_box_header_from_bytes(data: &[u8]) -> Option<Mp4BoxHeader> {
    if data.len() < 8 {
        return None;
    }
    let size32 = u32::from_be_bytes(data[0..4].try_into().ok()?) as u64;
    let name: [u8; 4] = data[4..8].try_into().ok()?;
    let (size, header_size) = if size32 == 1 {
        if data.len() < 16 {
            return None;
        }
        (u64::from_be_bytes(data[8..16].try_into().ok()?), 16)
    } else if size32 == 0 {
        (data.len() as u64, 8)
    } else {
        (size32, 8)
    };
    if size < header_size as u64 {
        return None;
    }
    Some(Mp4BoxHeader {
        name,
        size,
        header_size,
    })
}

pub(super) fn read_be_u16(data: &[u8]) -> Option<u16> {
    let bytes: [u8; 2] = data.get(0..2)?.try_into().ok()?;
    Some(u16::from_be_bytes(bytes))
}

pub(super) fn read_be_u32(data: &[u8]) -> Option<u32> {
    let bytes: [u8; 4] = data.get(0..4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

pub(super) fn fourcc_to_string(fourcc: [u8; 4]) -> String {
    String::from_utf8_lossy(&fourcc).into_owned()
}

pub(super) fn mp4_box_name_present(data: &[u8], names: &[[u8; 4]]) -> bool {
    scan::find_mp4_box_name_candidate(data, 0, names).is_some()
}
