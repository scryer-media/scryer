//! Small mastered UDF images with real media payloads for production-path tests.
use std::collections::BTreeMap;

const BLOCK: usize = 2048;
fn word(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn dword(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn tag(bytes: &mut [u8], kind: u16, block: u32, used: usize, version: u16) {
    word(bytes, 0, kind);
    word(bytes, 2, version);
    word(bytes, 6, 1);
    dword(bytes, 12, block);
    let mut crc = 0_u16;
    for byte in &bytes[16..used] {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    word(bytes, 8, crc);
    word(bytes, 10, (used - 16) as u16);
    bytes[4] = bytes[..16]
        .iter()
        .enumerate()
        .filter(|(at, _)| *at != 4)
        .fold(0_u8, |sum, (_, byte)| sum.wrapping_add(*byte));
}
fn allocation(bytes: &mut [u8], at: usize, block: u32, partition: u16, size: usize) {
    dword(bytes, at, size as u32);
    dword(bytes, at + 4, block);
    word(bytes, at + 8, partition);
}
fn entry(kind: u8, block: u32, size: usize, inline: bool, payload: &[u8], version: u16) -> Vec<u8> {
    assert!(payload.len() <= BLOCK - 176);
    let mut bytes = vec![0; BLOCK];
    bytes[27] = kind;
    word(
        &mut bytes,
        34,
        if inline {
            3
        } else if kind == 250 {
            0
        } else {
            1
        },
    );
    bytes[56..64].copy_from_slice(&(size as u64).to_le_bytes());
    dword(&mut bytes, 172, payload.len() as u32);
    bytes[176..176 + payload.len()].copy_from_slice(payload);
    tag(&mut bytes, 261, block, 176 + payload.len(), version);
    bytes
}
fn identifier(
    name: &str,
    child: u32,
    directory: bool,
    partition: u16,
    location: u32,
    version: u16,
) -> Vec<u8> {
    assert!(name.is_ascii() && name.len() < 254);
    let mut bytes = vec![0; (39 + name.len() + 3) & !3];
    word(&mut bytes, 16, 1);
    bytes[18] = if directory { 2 } else { 0 };
    bytes[19] = (name.len() + 1) as u8;
    allocation(&mut bytes, 20, child, partition, BLOCK);
    bytes[38] = 8;
    bytes[39..39 + name.len()].copy_from_slice(name.as_bytes());
    let size = bytes.len();
    tag(&mut bytes, 257, location, size, version);
    bytes
}
fn put(image: &mut [u8], block: usize, bytes: &[u8]) {
    image[block * BLOCK..block * BLOCK + bytes.len()].copy_from_slice(bytes);
}

/// Metadata mode splits the FSD and directory/file entries into separate
/// physical extents. File payloads remain in the ordinary physical partition.
pub fn udf(files: &[(&str, Vec<u8>)], metadata: bool) -> Vec<u8> {
    let mut directories = BTreeMap::from([(String::new(), 0_u32)]);
    for (path, _) in files {
        let components = path.split('/').collect::<Vec<_>>();
        for end in 1..components.len() {
            directories.insert(components[..end].join("/"), 0);
        }
    }
    let first_entry = if metadata { 17 } else { 1 };
    for (index, block) in directories.values_mut().enumerate() {
        *block = index as u32 + first_entry;
    }
    assert!(directories.len() + files.len() < 32);
    let version = if metadata { 3 } else { 2 };
    let part = u16::from(metadata);
    let mapped = |block: u32| {
        if metadata {
            if block < 16 {
                308 + block as usize
            } else {
                324 + block as usize
            }
        } else {
            300 + block as usize
        }
    };
    let mut next = 200;
    let allocations = files
        .iter()
        .enumerate()
        .map(|(index, (path, bytes))| {
            let block = next;
            next += bytes.len().div_ceil(BLOCK) as u32;
            (
                *path,
                directories.len() as u32 + first_entry + index as u32,
                block,
                bytes,
            )
        })
        .collect::<Vec<_>>();
    let mut image = vec![0; (300 + next as usize + 1).max(512) * BLOCK];
    let last = image.len() / BLOCK - 1;
    let mut anchor = vec![0; BLOCK];
    dword(&mut anchor, 16, (3 * BLOCK) as u32);
    dword(&mut anchor, 20, 20);
    tag(&mut anchor, 2, 256, 512, version);
    put(&mut image, 256, &anchor);
    tag(&mut anchor, 2, last as u32, 512, version);
    put(&mut image, last, &anchor);
    let mut partition = vec![0; BLOCK];
    dword(&mut partition, 16, 1);
    dword(&mut partition, 188, 300);
    dword(&mut partition, 192, next);
    tag(&mut partition, 5, 20, 512, version);
    put(&mut image, 20, &partition);
    let mut logical = vec![0; BLOCK];
    dword(&mut logical, 16, 2);
    dword(&mut logical, 212, BLOCK as u32);
    allocation(&mut logical, 248, 0, part, BLOCK);
    let map_size = if metadata { 70 } else { 6 };
    dword(&mut logical, 264, map_size);
    dword(&mut logical, 268, u32::from(part) + 1);
    logical[440..446].copy_from_slice(&[1, 6, 1, 0, 0, 0]);
    if metadata {
        let map = &mut logical[446..510];
        map[0] = 2;
        map[1] = 64;
        map[5..28].copy_from_slice(b"*UDF Metadata Partition");
        word(map, 36, 1);
        dword(map, 44, u32::MAX);
        dword(map, 48, u32::MAX);
        dword(map, 52, 16);
        word(map, 56, 1);
        let mut extents = vec![0; 16];
        dword(&mut extents, 0, (16 * BLOCK) as u32);
        dword(&mut extents, 4, 8);
        dword(&mut extents, 8, (32 * BLOCK) as u32);
        dword(&mut extents, 12, 40);
        put(
            &mut image,
            300,
            &entry(250, 0, 48 * BLOCK, false, &extents, version),
        );
    }
    tag(&mut logical, 6, 21, 440 + map_size as usize, version);
    put(&mut image, 21, &logical);
    let mut end = vec![0; BLOCK];
    tag(&mut end, 8, 22, 512, version);
    put(&mut image, 22, &end);
    let mut fsd = vec![0; BLOCK];
    allocation(&mut fsd, 400, directories[""], part, BLOCK);
    tag(&mut fsd, 256, 0, 512, version);
    put(&mut image, mapped(0), &fsd);
    for (path, block) in &directories {
        let mut children = Vec::new();
        for (child, child_block) in &directories {
            if !child.is_empty() && child.rsplit_once('/').map_or("", |(parent, _)| parent) == path
            {
                children.extend(identifier(
                    child.rsplit('/').next().unwrap(),
                    *child_block,
                    true,
                    part,
                    *block,
                    version,
                ));
            }
        }
        for (child, child_block, _, _) in &allocations {
            if child.rsplit_once('/').map_or("", |(parent, _)| parent) == path {
                children.extend(identifier(
                    child.rsplit('/').next().unwrap(),
                    *child_block,
                    false,
                    part,
                    *block,
                    version,
                ));
            }
        }
        put(
            &mut image,
            mapped(*block),
            &entry(4, *block, children.len(), true, &children, version),
        );
    }
    for (_, block, payload, bytes) in allocations {
        let mut extent = vec![0; 16];
        allocation(&mut extent, 0, payload, 0, bytes.len());
        put(
            &mut image,
            mapped(block),
            &entry(5, block, bytes.len(), false, &extent, version),
        );
        put(&mut image, 300 + payload as usize, bytes);
    }
    image
}
