//! Authored disc builders shared by native and application regression tests.
use std::collections::BTreeMap;

#[path = "udf_image.rs"]
pub mod udf_image;

pub fn both32(target: &mut [u8], at: usize, value: u32) {
    target[at..at + 4].copy_from_slice(&value.to_le_bytes());
    target[at + 4..at + 8].copy_from_slice(&value.to_be_bytes());
}
fn record(name: &[u8], sector: u32, length: usize, directory: bool) -> Vec<u8> {
    let size = (33 + name.len() + 1) & !1;
    let mut bytes = vec![0; size];
    bytes[0] = size as u8;
    both32(&mut bytes, 2, sector);
    both32(&mut bytes, 10, length as u32);
    bytes[25] = if directory { 2 } else { 0 };
    bytes[28..32].copy_from_slice(&[1, 0, 0, 1]);
    bytes[32] = name.len() as u8;
    bytes[33..33 + name.len()].copy_from_slice(name);
    bytes
}
pub fn iso(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut directories: BTreeMap<String, u32> = BTreeMap::from([(String::new(), 0)]);
    for (path, _) in files {
        let mut prefix = String::new();
        let mut components = path.split('/').peekable();
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                break;
            }
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            directories.insert(prefix.clone(), 0);
        }
    }
    for (index, sector) in directories.values_mut().enumerate() {
        *sector = 20 + index as u32;
    }
    let mut next = 20 + directories.len() as u32;
    let mut allocations = Vec::new();
    for (path, bytes) in files {
        allocations.push((*path, next, bytes));
        next += bytes.len().div_ceil(2048) as u32;
    }
    let mut image = vec![0; (next as usize + 1) * 2048];
    let pvd = &mut image[16 * 2048..17 * 2048];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 1;
    pvd[128..132].copy_from_slice(&[0, 8, 8, 0]);
    pvd[156..190].copy_from_slice(&record(&[0], directories[""], 2048, true));
    for (path, sector) in &directories {
        let mut contents = record(&[0], *sector, 2048, true);
        for (child, child_sector) in &directories {
            if !child.is_empty() && child.rsplit_once('/').map_or("", |(parent, _)| parent) == path
            {
                let name = child.rsplit('/').next().unwrap();
                contents.extend(record(name.as_bytes(), *child_sector, 2048, true));
            }
        }
        for (child, child_sector, data) in &allocations {
            if child.rsplit_once('/').map_or("", |(parent, _)| parent) == path {
                contents.extend(record(
                    format!("{};1", child.rsplit('/').next().unwrap()).as_bytes(),
                    *child_sector,
                    data.len(),
                    false,
                ));
            }
        }
        assert!(contents.len() <= 2048);
        let at = *sector as usize * 2048;
        image[at..at + contents.len()].copy_from_slice(&contents);
    }
    for (_, sector, bytes) in allocations {
        let at = sector as usize * 2048;
        image[at..at + bytes.len()].copy_from_slice(bytes);
    }
    image
}
pub fn mpls(items: &[(&str, u32, u32)]) -> Vec<u8> {
    let mut list = vec![0, 0];
    list.extend_from_slice(&(items.len() as u16).to_be_bytes());
    list.extend_from_slice(&0_u16.to_be_bytes());
    for (clip, input, output) in items {
        list.extend_from_slice(&32_u16.to_be_bytes());
        let mut item = vec![0; 32];
        item[..5].copy_from_slice(clip.as_bytes());
        item[5..9].copy_from_slice(b"M2TS");
        item[10] = 1;
        item[12..16].copy_from_slice(&input.to_be_bytes());
        item[16..20].copy_from_slice(&output.to_be_bytes());
        list.extend(item);
    }
    let mut bytes = b"MPLS0200".to_vec();
    bytes.extend_from_slice(&40_u32.to_be_bytes());
    bytes.resize(40, 0);
    bytes.extend_from_slice(&(list.len() as u32).to_be_bytes());
    bytes.extend(list);
    bytes
}

/// Author a single clock sequence and an explicit video access-point map.
pub fn clpi(source_packets: u32, output: u32, points: &[(u32, u32)]) -> Vec<u8> {
    let mut bytes = b"HDMV0300".to_vec();
    bytes.resize(40, 0);
    bytes.extend(16_u32.to_be_bytes());
    let mut clip = [0_u8; 16];
    clip[2] = 1;
    clip[3] = 1;
    clip[12..16].copy_from_slice(&source_packets.to_be_bytes());
    bytes.extend(clip);
    let sequence_at = bytes.len() as u32;
    let mut sequence = vec![0, 1];
    sequence.extend(0_u32.to_be_bytes());
    sequence.extend([1, 0]);
    sequence.extend(0x1011_u16.to_be_bytes());
    sequence.extend(0_u32.to_be_bytes());
    sequence.extend(0_u32.to_be_bytes());
    sequence.extend(output.to_be_bytes());
    bytes.extend((sequence.len() as u32).to_be_bytes());
    bytes.extend(sequence);
    let program_at = bytes.len() as u32;
    let mut program = vec![0, 1];
    program.extend(0_u32.to_be_bytes());
    program.extend(0x100_u16.to_be_bytes());
    program.extend([2, 0]);
    program.extend(0x1011_u16.to_be_bytes());
    program.extend([3, 0x1b, 0x62, 0x30]);
    program.extend(0x1100_u16.to_be_bytes());
    program.extend([5, 3, 0x31, b'f', b'r', b'a']);
    bytes.extend((program.len() as u32).to_be_bytes());
    bytes.extend(program);
    let cpi_at = bytes.len() as u32;
    assert!(!points.is_empty() && points.len() <= u16::MAX as usize);
    let count = points.len() as u32;
    let mut cpi = vec![0, 1, 0, 1];
    cpi.extend(0x1011_u16.to_be_bytes());
    let packed = (1_u64 << 34) | (u64::from(count) << 18) | u64::from(count);
    cpi.extend_from_slice(&packed.to_be_bytes()[2..]);
    cpi.extend(14_u32.to_be_bytes());
    cpi.extend((4 + count * 8).to_be_bytes());
    for (index, (packet, time)) in points.iter().enumerate() {
        cpi.extend((((index as u32) << 14) | ((time >> 18) & 0x3fff)).to_be_bytes());
        cpi.extend(packet.to_be_bytes());
    }
    for (packet, time) in points {
        cpi.extend((((time >> 8) & 0x7ff) << 17 | (packet & 0x1ffff)).to_be_bytes());
    }
    bytes.extend((cpi.len() as u32).to_be_bytes());
    bytes.extend(cpi);
    bytes[8..12].copy_from_slice(&sequence_at.to_be_bytes());
    bytes[12..16].copy_from_slice(&program_at.to_be_bytes());
    bytes[16..20].copy_from_slice(&cpi_at.to_be_bytes());
    bytes
}
