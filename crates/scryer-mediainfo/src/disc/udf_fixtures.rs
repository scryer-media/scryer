//! Authored UDF descriptor fixtures; payload inspection uses extent readers.
use super::image::{self, ImageError};
use crate::source::ExtentSource;
use std::io::{Cursor, Read};

const BLOCK: usize = 2048;
fn u16_at(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn tag(bytes: &mut [u8], kind: u16, location: u32, used: usize) {
    u16_at(bytes, 0, kind);
    u16_at(bytes, 2, 3);
    u16_at(bytes, 6, 1);
    u32_at(bytes, 12, location);
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
    u16_at(bytes, 8, crc);
    u16_at(bytes, 10, (used - 16) as u16);
    bytes[4] = bytes[..16]
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 4)
        .fold(0_u8, |sum, (_, byte)| sum.wrapping_add(*byte));
}
fn ad(bytes: &mut [u8], at: usize, block: u32, part: u16, length: u32) {
    u32_at(bytes, at, length);
    u32_at(bytes, at + 4, block);
    u16_at(bytes, at + 8, part);
}
fn fid(name: &str, child: u32, part: u16, directory: bool, location: u32) -> Vec<u8> {
    let name_len = 1 + name.len();
    let mut bytes = vec![0; (38 + name_len + 3) & !3];
    u16_at(&mut bytes, 16, 1);
    bytes[18] = if directory { 2 } else { 0 };
    bytes[19] = name_len as u8;
    ad(&mut bytes, 20, child, part, BLOCK as u32);
    bytes[38] = 8;
    bytes[39..39 + name.len()].copy_from_slice(name.as_bytes());
    let len = bytes.len();
    tag(&mut bytes, 257, location, len);
    bytes
}
fn fe(kind: u8, block: u32, length: u64, mode: u16, data: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; BLOCK];
    bytes[27] = kind;
    u16_at(&mut bytes, 34, mode);
    u64_at(&mut bytes, 56, length);
    u32_at(&mut bytes, 172, data.len() as u32);
    bytes[176..176 + data.len()].copy_from_slice(data);
    tag(&mut bytes, 261, block, 176 + data.len());
    bytes
}
fn put(image: &mut [u8], block: usize, bytes: &[u8]) {
    image[block * BLOCK..block * BLOCK + bytes.len()].copy_from_slice(bytes);
}

/// Metadata partition maps two disjoint extents; ordinary-file extents live
/// in the physical partition and are never folded into a giant read buffer.
fn authored_udf(metadata_partition: bool) -> Vec<u8> {
    let mut image = vec![0; 512 * BLOCK];
    let mut anchor = vec![0; BLOCK];
    u32_at(&mut anchor, 16, (3 * BLOCK) as u32);
    u32_at(&mut anchor, 20, 20);
    tag(&mut anchor, 2, 256, 512);
    put(&mut image, 256, &anchor);
    tag(&mut anchor, 2, 511, 512);
    put(&mut image, 511, &anchor);
    let mut partition = vec![0; BLOCK];
    u32_at(&mut partition, 16, 1);
    u16_at(&mut partition, 22, 0);
    u32_at(&mut partition, 188, 300);
    u32_at(&mut partition, 192, 128);
    tag(&mut partition, 5, 20, 512);
    put(&mut image, 20, &partition);
    let mut logical = vec![0; BLOCK];
    u32_at(&mut logical, 16, 2);
    u32_at(&mut logical, 212, BLOCK as u32);
    let part = u16::from(metadata_partition);
    ad(&mut logical, 248, 0, part, BLOCK as u32);
    let map_len = if metadata_partition { 70 } else { 6 };
    u32_at(&mut logical, 264, map_len);
    u32_at(&mut logical, 268, u32::from(part) + 1);
    logical[440..446].copy_from_slice(&[1, 6, 1, 0, 0, 0]);
    if metadata_partition {
        let map = &mut logical[446..510];
        map[0] = 2;
        map[1] = 64;
        map[5..28].copy_from_slice(b"*UDF Metadata Partition");
        u16_at(map, 36, 1);
        u32_at(map, 40, 0);
        u32_at(map, 44, u32::MAX);
        u32_at(map, 48, u32::MAX);
        u32_at(map, 52, 16);
        u16_at(map, 56, 1);
        let mut allocations = [0; 16];
        u32_at(&mut allocations, 0, (16 * BLOCK) as u32);
        u32_at(&mut allocations, 4, 8);
        u32_at(&mut allocations, 8, (16 * BLOCK) as u32);
        u32_at(&mut allocations, 12, 40);
        put(
            &mut image,
            300,
            &fe(250, 0, (32 * BLOCK) as u64, 0, &allocations),
        );
    }
    tag(&mut logical, 6, 21, 440 + map_len as usize);
    put(&mut image, 21, &logical);
    let mut end = vec![0; BLOCK];
    tag(&mut end, 8, 22, 512);
    put(&mut image, 22, &end);
    let physical = |logical: usize| {
        if metadata_partition {
            if logical < 16 {
                308 + logical
            } else {
                340 + logical - 16
            }
        } else {
            300 + logical
        }
    };
    let mut fsd = vec![0; BLOCK];
    ad(&mut fsd, 400, 1, part, BLOCK as u32);
    tag(&mut fsd, 256, 0, 512);
    put(&mut image, physical(0), &fsd);
    let root = fid("BDMV", 2, part, true, 1);
    put(
        &mut image,
        physical(1),
        &fe(4, 1, root.len() as u64, 3, &root),
    );
    let directory = fid("STREAM", 17, part, true, 2);
    put(
        &mut image,
        physical(2),
        &fe(4, 2, directory.len() as u64, 3, &directory),
    );
    let files = fid("00001.M2TS", 18, part, false, 17);
    put(
        &mut image,
        physical(17),
        &fe(4, 17, files.len() as u64, 3, &files),
    );
    let mut extents = [0; 32];
    ad(&mut extents, 0, 70, 0, BLOCK as u32);
    ad(&mut extents, 16, 90, 0, BLOCK as u32);
    put(
        &mut image,
        physical(18),
        &fe(5, 18, (2 * BLOCK) as u64, 1, &extents),
    );
    put(&mut image, 370, &vec![0x47; BLOCK]);
    put(&mut image, 390, &vec![0x31; BLOCK]);
    image
}

#[test]
fn udf_physical_and_metadata_partitions_read_fragmented_files() {
    for metadata in [false, true] {
        let mut source = Cursor::new(authored_udf(metadata));
        let image = image::open(&mut source).unwrap();
        assert_eq!(image.filesystem, "UDF");
        assert!(image.bytes_read < 32 * 2048);
        let file = &image.files["BDMV/STREAM/00001.M2TS"];
        assert_eq!(file.extents.len(), 2);
        let mut data = Vec::new();
        ExtentSource::new(&mut source, file.extents.clone())
            .unwrap()
            .read_to_end(&mut data)
            .unwrap();
        assert_eq!(&data[..BLOCK], &[0x47; BLOCK]);
        assert_eq!(&data[BLOCK..], &[0x31; BLOCK]);
    }
}

#[test]
fn udf_anchor_backup_and_descriptor_crc_are_enforced() {
    let mut bytes = authored_udf(true);
    bytes[256 * BLOCK + 4] ^= 1;
    assert!(image::open(&mut Cursor::new(bytes.clone())).is_ok());
    bytes[21 * BLOCK + 212] ^= 1;
    assert!(matches!(
        image::open(&mut Cursor::new(bytes)),
        Err(ImageError::Malformed(_))
    ));
}

#[test]
fn udf_continuation_uses_prevailing_descriptors_and_rejects_cycles() {
    let mut bytes = authored_udf(true);
    let mut older_partition = bytes[20 * BLOCK..21 * BLOCK].to_vec();
    u32_at(&mut older_partition, 16, 0);
    u32_at(&mut older_partition, 188, 301);
    tag(&mut older_partition, 5, 24, 512);
    put(&mut bytes, 24, &older_partition);
    let mut logical = bytes[21 * BLOCK..22 * BLOCK].to_vec();
    tag(&mut logical, 6, 25, 510);
    put(&mut bytes, 25, &logical);
    u32_at(&mut logical, 16, 1);
    u32_at(&mut logical, 212, 1024);
    tag(&mut logical, 6, 26, 510);
    put(&mut bytes, 26, &logical);
    let mut end = vec![0; BLOCK];
    tag(&mut end, 8, 27, 512);
    put(&mut bytes, 27, &end);
    let mut pointer = vec![0; BLOCK];
    u32_at(&mut pointer, 20, (4 * BLOCK) as u32);
    u32_at(&mut pointer, 24, 24);
    tag(&mut pointer, 3, 21, 512);
    put(&mut bytes, 21, &pointer);
    let image = image::open(&mut Cursor::new(bytes.clone())).unwrap();
    assert!(image.files.contains_key("BDMV/STREAM/00001.M2TS"));
    assert!(image.bytes_read < 40 * BLOCK as u64);

    let mut cycle = bytes.clone();
    u32_at(&mut pointer, 20, (3 * BLOCK) as u32);
    u32_at(&mut pointer, 24, 20);
    tag(&mut pointer, 3, 27, 512);
    put(&mut cycle, 27, &pointer);
    assert!(matches!(
        image::open(&mut Cursor::new(cycle)),
        Err(ImageError::Malformed(
            "cyclic UDF volume descriptor sequence"
        ))
    ));

    let duplicate = &mut bytes[26 * BLOCK..27 * BLOCK];
    u32_at(duplicate, 16, 2);
    tag(duplicate, 6, 26, 510);
    assert!(matches!(
        image::open(&mut Cursor::new(bytes)),
        Err(ImageError::Malformed(
            "conflicting UDF logical volume descriptors"
        ))
    ));
}

#[test]
fn udf_continuation_bounds_and_crc_coverage_are_enforced() {
    for (next, length, expected_budget) in [
        (510, (4 * BLOCK) as u32, false),
        (24, 2 * 1024 * 1024, true),
    ] {
        let mut bytes = authored_udf(false);
        let mut pointer = vec![0; BLOCK];
        u32_at(&mut pointer, 20, length);
        u32_at(&mut pointer, 24, next);
        tag(&mut pointer, 3, 21, 512);
        put(&mut bytes, 21, &pointer);
        let error = image::open(&mut Cursor::new(bytes)).unwrap_err();
        assert_eq!(matches!(error, ImageError::Budget), expected_budget);
        if !expected_budget {
            assert!(matches!(
                error,
                ImageError::Malformed("UDF descriptor sequence outside image")
            ));
        }
    }
    let mut bytes = authored_udf(false);
    let mut pointer = vec![0; BLOCK];
    u32_at(&mut pointer, 20, BLOCK as u32);
    u32_at(&mut pointer, 24, 24);
    tag(&mut pointer, 3, 21, 16);
    put(&mut bytes, 21, &pointer);
    assert!(matches!(
        image::open(&mut Cursor::new(bytes)),
        Err(ImageError::Malformed(
            "UDF CRC does not cover descriptor fields"
        ))
    ));
}

#[test]
fn udf_unknown_partition_map_is_explicitly_unsupported() {
    let mut bytes = authored_udf(true);
    let descriptor = &mut bytes[21 * BLOCK..22 * BLOCK];
    descriptor[451] = b'!';
    tag(descriptor, 6, 21, 510);
    assert!(matches!(
        image::open(&mut Cursor::new(bytes)),
        Err(ImageError::Unsupported(_))
    ));
}

#[test]
fn udf_metadata_mirror_and_reserve_volume_sequence_recover_damage() {
    let mut bytes = authored_udf(true);
    let mut mirror = bytes[300 * BLOCK..301 * BLOCK].to_vec();
    mirror[27] = 251;
    tag(&mut mirror, 261, 1, 192);
    put(&mut bytes, 301, &mirror);
    bytes[300 * BLOCK + 4] ^= 1;
    let descriptor = &mut bytes[21 * BLOCK..22 * BLOCK];
    u32_at(descriptor, 446 + 44, 1);
    tag(descriptor, 6, 21, 510);
    assert!(image::open(&mut Cursor::new(bytes.clone())).is_ok());
    for (from, to, kind, used) in [(20, 24, 5, 512), (21, 25, 6, 510), (22, 26, 8, 512)] {
        let mut descriptor = bytes[from * BLOCK..(from + 1) * BLOCK].to_vec();
        tag(&mut descriptor, kind, to as u32, used);
        put(&mut bytes, to, &descriptor);
    }
    let anchor = &mut bytes[256 * BLOCK..257 * BLOCK];
    u32_at(anchor, 24, (3 * BLOCK) as u32);
    u32_at(anchor, 28, 24);
    tag(anchor, 2, 256, 512);
    bytes[20 * BLOCK + 4] ^= 1;
    assert!(image::open(&mut Cursor::new(bytes)).is_ok());
}

struct SparseImage {
    cursor: Cursor<Vec<u8>>,
    length: u64,
}
impl Read for SparseImage {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let position = self.cursor.position();
        let length = (self.length.saturating_sub(position)).min(out.len() as u64) as usize;
        out[..length].fill(0);
        if position < self.cursor.get_ref().len() as u64 {
            let present = length.min(self.cursor.get_ref().len() - position as usize);
            out[..present].copy_from_slice(
                &self.cursor.get_ref()[position as usize..position as usize + present],
            );
        }
        self.cursor.set_position(position + length as u64);
        Ok(length)
    }
}
impl std::io::Seek for SparseImage {
    fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
        use std::io::{Error, ErrorKind, SeekFrom};
        let next = match from {
            SeekFrom::Start(value) => Some(value),
            SeekFrom::Current(delta) => self.cursor.position().checked_add_signed(delta),
            SeekFrom::End(delta) => self.length.checked_add_signed(delta),
        }
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "seek overflow"))?;
        self.cursor.set_position(next);
        Ok(next)
    }
}
impl crate::source::MediaSource for SparseImage {
    fn len(&self) -> u64 {
        self.length
    }
}

#[test]
fn udf_large_file_inventory_does_not_read_payload_or_allocate_image_size() {
    let mut bytes = authored_udf(false);
    let partition = &mut bytes[20 * BLOCK..21 * BLOCK];
    u32_at(partition, 192, 3_000_000);
    tag(partition, 5, 20, 512);
    let piece = 0x3fff_f800_u32;
    let mut extents = vec![0; 5 * 16];
    for index in 0..5 {
        ad(
            &mut extents,
            index * 16,
            70 + index as u32 * (piece / BLOCK as u32),
            0,
            piece,
        );
    }
    put(
        &mut bytes,
        318,
        &fe(5, 18, u64::from(piece) * 5, 1, &extents),
    );
    let mut source = SparseImage {
        cursor: Cursor::new(bytes),
        length: 6 * 1024 * 1024 * 1024,
    };
    let image = image::open(&mut source).unwrap();
    assert!(image.files["BDMV/STREAM/00001.M2TS"].length > u64::from(u32::MAX));
    assert_eq!(image.files["BDMV/STREAM/00001.M2TS"].extents.len(), 5);
    assert!(image.bytes_read < 32 * 2048);
}
