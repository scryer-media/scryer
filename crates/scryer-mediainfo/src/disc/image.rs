//! Read-only mastered-image filesystem inventory (ECMA-119 and ECMA-167).
//! Payloads remain in place; only bounded filesystem metadata is read here.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, SeekFrom};

use crate::source::{Extent, ExtentSource, MediaSource};

const BLOCK: u64 = 2048;
const METADATA_BUDGET: u64 = 32 * 1024 * 1024;
const DIRECTORY_LIMIT: u64 = 4 * 1024 * 1024;
const ENTRY_LIMIT: usize = 65_536;

#[derive(Debug)]
pub(super) enum ImageError {
    Io(io::Error),
    Malformed(&'static str),
    Unsupported(&'static str),
    Budget,
}
impl From<io::Error> for ImageError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Malformed(message) | Self::Unsupported(message) => f.write_str(message),
            Self::Budget => f.write_str("disc filesystem metadata budget exhausted"),
        }
    }
}
type Result<T> = std::result::Result<T, ImageError>;

#[derive(Debug, Clone)]
pub(super) struct ImageFile {
    pub length: u64,
    pub extents: Vec<Extent>,
}
#[derive(Debug)]
pub(super) struct Image {
    pub filesystem: String,
    pub volume_label: Option<String>,
    pub files: BTreeMap<String, ImageFile>,
    pub bytes_read: u64,
}

struct Reader<'a> {
    source: &'a mut dyn MediaSource,
    remaining: u64,
    entries: usize,
}
impl Reader<'_> {
    fn read(&mut self, offset: u64, length: u64) -> Result<Vec<u8>> {
        if length > self.remaining {
            return Err(ImageError::Budget);
        }
        if offset
            .checked_add(length)
            .is_none_or(|end| end > self.source.len())
        {
            return Err(ImageError::Malformed("filesystem extent outside image"));
        }
        self.remaining -= length;
        let mut bytes = vec![0; length as usize];
        self.source.seek(SeekFrom::Start(offset))?;
        self.source.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    fn file(&mut self, file: &ImageFile) -> Result<Vec<u8>> {
        if file.length > DIRECTORY_LIMIT || file.length > self.remaining {
            return Err(ImageError::Budget);
        }
        self.remaining -= file.length;
        let mut bytes = vec![0; file.length as usize];
        ExtentSource::new(self.source, file.extents.clone())?.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    fn entry(&mut self, depth: usize) -> Result<()> {
        self.entries += 1;
        if depth > 32 || self.entries > ENTRY_LIMIT {
            return Err(ImageError::Budget);
        }
        Ok(())
    }
}

pub(super) fn open(source: &mut dyn MediaSource) -> Result<Image> {
    let mut reader = Reader {
        source,
        remaining: METADATA_BUDGET,
        entries: 0,
    };
    let sectors = reader.source.len() / BLOCK;
    // UDF takes precedence over the limited ISO bridge directory tree.
    let mut anchor = None;
    let mut anchor_error = None;
    for sector in [Some(256), sectors.checked_sub(257), sectors.checked_sub(1)]
        .into_iter()
        .flatten()
    {
        if sector >= sectors {
            continue;
        }
        let bytes = reader.read(sector * BLOCK, BLOCK)?;
        if le16(&bytes, 0)? == 2 {
            let location = u32::try_from(sector).map_err(|_| {
                ImageError::Unsupported("UDF sector address exceeds descriptor range")
            })?;
            if let Err(error) = validate_tag(&bytes, Some(location)) {
                anchor_error = Some(error);
                continue;
            }
            anchor = Some(bytes);
            break;
        }
    }
    let mut image = if let Some(anchor) = anchor {
        udf(&mut reader, &anchor)?
    } else if let Some(error) = anchor_error {
        return Err(error);
    } else {
        iso9660(&mut reader)?
    };
    image.bytes_read = METADATA_BUDGET - reader.remaining;
    Ok(image)
}

fn le16(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or(ImageError::Malformed("short filesystem field"))?
            .try_into()
            .unwrap(),
    ))
}
fn le32(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or(ImageError::Malformed("short filesystem field"))?
            .try_into()
            .unwrap(),
    ))
}
fn le64(bytes: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .get(at..at + 8)
            .ok_or(ImageError::Malformed("short filesystem field"))?
            .try_into()
            .unwrap(),
    ))
}
fn both32(bytes: &[u8], at: usize) -> Result<u32> {
    let value = le32(bytes, at)?;
    let other = u32::from_be_bytes(
        bytes
            .get(at + 4..at + 8)
            .ok_or(ImageError::Malformed("short ISO field"))?
            .try_into()
            .unwrap(),
    );
    if value != other {
        return Err(ImageError::Malformed("ISO byte-order copies disagree"));
    }
    Ok(value)
}
fn filename(name: &str) -> Result<String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(ImageError::Malformed("invalid image filename"));
    }
    Ok(name.to_ascii_uppercase())
}

fn iso9660(reader: &mut Reader<'_>) -> Result<Image> {
    let mut primary = None;
    for sector in 16..80 {
        let bytes = reader.read(sector * BLOCK, BLOCK)?;
        if bytes.get(1..6) != Some(b"CD001") {
            break;
        }
        if bytes[6] != 1 {
            return Err(ImageError::Unsupported("ISO descriptor version"));
        }
        if bytes[0] == 1 {
            primary = Some(bytes);
            break;
        }
        if bytes[0] == 255 {
            break;
        }
    }
    let pvd = primary.ok_or(ImageError::Unsupported(
        "no supported ISO9660 or UDF filesystem",
    ))?;
    if le16(&pvd, 128)? != 2048 || u16::from_be_bytes([pvd[130], pvd[131]]) != 2048 {
        return Err(ImageError::Unsupported("ISO logical block size"));
    }
    let root = iso_record(&pvd[156..])?;
    let mut files = BTreeMap::new();
    iso_directory(reader, root.0, "", 0, &mut BTreeSet::new(), &mut files)?;
    let label = String::from_utf8_lossy(&pvd[40..72]).trim().to_string();
    Ok(Image {
        filesystem: "ISO9660".into(),
        volume_label: (!label.is_empty()).then_some(label),
        files,
        bytes_read: 0,
    })
}
fn iso_record(bytes: &[u8]) -> Result<(ImageFile, u8, String)> {
    let length = usize::from(
        *bytes
            .first()
            .ok_or(ImageError::Malformed("missing ISO record"))?,
    );
    if length < 34 || length > bytes.len() {
        return Err(ImageError::Malformed("short ISO directory record"));
    }
    if bytes[1] != 0 || bytes[26] != 0 || bytes[27] != 0 {
        return Err(ImageError::Unsupported(
            "ISO extended attributes or interleaved files",
        ));
    }
    let size = u64::from(both32(bytes, 10)?);
    let offset = u64::from(both32(bytes, 2)?) * BLOCK;
    let name = bytes
        .get(33..33 + usize::from(bytes[32]))
        .filter(|_| 33 + usize::from(bytes[32]) <= length)
        .ok_or(ImageError::Malformed("short ISO filename"))?;
    let name = if name == [0] || name == [1] {
        String::new()
    } else {
        let text = std::str::from_utf8(name)
            .map_err(|_| ImageError::Unsupported("ISO filename encoding"))?;
        filename(text.split(';').next().unwrap_or(text).trim_end_matches('.'))?
    };
    Ok((
        ImageFile {
            length: size,
            extents: if size == 0 {
                vec![]
            } else {
                vec![Extent {
                    offset,
                    length: size,
                }]
            },
        },
        bytes[25],
        name,
    ))
}
fn iso_directory(
    reader: &mut Reader<'_>,
    directory: ImageFile,
    path: &str,
    depth: usize,
    visited: &mut BTreeSet<u64>,
    files: &mut BTreeMap<String, ImageFile>,
) -> Result<()> {
    reader.entry(depth)?;
    let start = directory
        .extents
        .first()
        .ok_or(ImageError::Malformed("empty ISO directory"))?
        .offset;
    if !visited.insert(start) {
        return Err(ImageError::Malformed("cyclic ISO directory"));
    }
    let bytes = reader.file(&directory)?;
    let mut at = 0;
    let mut pending: Option<String> = None;
    while at < bytes.len() {
        if bytes[at] == 0 {
            at = (at / BLOCK as usize + 1) * BLOCK as usize;
            continue;
        }
        reader.entry(depth)?;
        let (file, flags, name) = iso_record(&bytes[at..])?;
        if at % BLOCK as usize + usize::from(bytes[at]) > BLOCK as usize {
            return Err(ImageError::Malformed("ISO directory record crosses block"));
        }
        at += usize::from(bytes[at]);
        if name.is_empty() {
            continue;
        }
        let key = if path.is_empty() {
            name
        } else {
            format!("{path}/{name}")
        };
        if pending.as_ref().is_some_and(|name| name != &key) {
            return Err(ImageError::Malformed("interrupted ISO multi-extent file"));
        }
        if flags & 2 != 0 {
            if flags & 128 != 0 {
                return Err(ImageError::Unsupported("multi-extent ISO directory"));
            }
            iso_directory(reader, file, &key, depth + 1, visited, files)?;
        } else {
            ExtentSource::new(reader.source, file.extents.clone())?;
            if let Some(existing) = files.get_mut(&key) {
                if pending.as_ref() != Some(&key) {
                    return Err(ImageError::Malformed("duplicate ISO filename"));
                }
                existing.length = existing
                    .length
                    .checked_add(file.length)
                    .ok_or(ImageError::Malformed("ISO file size overflow"))?;
                existing.extents.extend(file.extents);
                if existing.extents.len() > ENTRY_LIMIT {
                    return Err(ImageError::Budget);
                }
            } else {
                files.insert(key.clone(), file);
            }
            pending = (flags & 128 != 0).then_some(key);
        }
    }
    if pending.is_some() {
        return Err(ImageError::Malformed("unfinished ISO multi-extent file"));
    }
    Ok(())
}

// UDF uses the non-reflected 0x1021 polynomial with an initial value of zero.
const UDF_CRC_TABLE: [[u16; 256]; 4] = {
    let mut table = [[0; 256]; 4];
    let mut byte = 0;
    while byte < 256 {
        let mut crc = (byte as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
            bit += 1;
        }
        table[0][byte] = crc;
        byte += 1;
    }
    let mut row = 1;
    while row < 4 {
        let mut byte = 0;
        while byte < 256 {
            let crc = table[row - 1][byte];
            table[row][byte] = (crc << 8) ^ table[0][(crc >> 8) as usize];
            byte += 1;
        }
        row += 1;
    }
    table
};

fn udf_crc(body: &[u8]) -> u16 {
    let mut crc = 0_u16;
    let mut chunks = body.chunks_exact(4);
    for bytes in &mut chunks {
        crc = UDF_CRC_TABLE[3][usize::from((crc >> 8) as u8 ^ bytes[0])]
            ^ UDF_CRC_TABLE[2][usize::from(crc as u8 ^ bytes[1])]
            ^ UDF_CRC_TABLE[1][usize::from(bytes[2])]
            ^ UDF_CRC_TABLE[0][usize::from(bytes[3])];
    }
    for &byte in chunks.remainder() {
        crc = (crc << 8) ^ UDF_CRC_TABLE[0][usize::from((crc >> 8) as u8 ^ byte)];
    }
    crc
}

fn validate_tag(bytes: &[u8], location: Option<u32>) -> Result<u16> {
    if bytes.len() < 16 {
        return Err(ImageError::Malformed("short UDF descriptor tag"));
    }
    let sum = bytes[..16]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0_u8, |sum, (_, byte)| sum.wrapping_add(*byte));
    if sum != bytes[4] {
        return Err(ImageError::Malformed("UDF descriptor checksum mismatch"));
    }
    if !matches!(le16(bytes, 2)?, 2 | 3) {
        return Err(ImageError::Unsupported("UDF descriptor version"));
    }
    if location.is_some_and(|location| le32(bytes, 12).ok() != Some(location)) {
        return Err(ImageError::Malformed("UDF descriptor location mismatch"));
    }
    let length = usize::from(le16(bytes, 10)?);
    let kind = le16(bytes, 0)?;
    let minimum = match kind {
        2 | 3 | 5 | 8 | 256 => 496_u64,
        6 => 424 + u64::from(le32(bytes, 264)?),
        257 => bytes.len().saturating_sub(16) as u64,
        258 => 8 + u64::from(le32(bytes, 20)?),
        261 => 160 + u64::from(le32(bytes, 168)?) + u64::from(le32(bytes, 172)?),
        266 => 200 + u64::from(le32(bytes, 208)?) + u64::from(le32(bytes, 212)?),
        _ => 0,
    };
    if (length as u64) < minimum {
        return Err(ImageError::Malformed(
            "UDF CRC does not cover descriptor fields",
        ));
    }
    let body = bytes
        .get(16..16 + length)
        .ok_or(ImageError::Malformed("short UDF descriptor body"))?;
    if udf_crc(body) != le16(bytes, 8)? {
        return Err(ImageError::Malformed("UDF descriptor CRC mismatch"));
    }
    Ok(le16(bytes, 0)?)
}

#[derive(Clone)]
enum Partition {
    Physical(Extent),
    Metadata(Vec<Extent>),
}
pub(super) fn range(extents: &[Extent], offset: u64, length: u64) -> Result<Vec<Extent>> {
    let mut skip = offset;
    let mut remaining = length;
    let mut result = Vec::new();
    for extent in extents {
        if skip >= extent.length {
            skip -= extent.length;
            continue;
        }
        let take = remaining.min(extent.length - skip);
        if take > 0 {
            result.push(Extent {
                offset: extent
                    .offset
                    .checked_add(skip)
                    .ok_or(ImageError::Malformed("extent offset overflow"))?,
                length: take,
            });
        }
        remaining -= take;
        skip = 0;
        if remaining == 0 {
            return Ok(result);
        }
    }
    if remaining == 0 {
        Ok(result)
    } else {
        Err(ImageError::Malformed("UDF range outside partition"))
    }
}
fn mapped(maps: &[Partition], partition: u16, block: u32, length: u64) -> Result<Vec<Extent>> {
    let offset = u64::from(block) * BLOCK;
    match maps
        .get(usize::from(partition))
        .ok_or(ImageError::Malformed("invalid UDF partition reference"))?
    {
        Partition::Physical(extent) => range(&[*extent], offset, length),
        Partition::Metadata(extents) => range(extents, offset, length),
    }
}
fn mapped_read(
    reader: &mut Reader<'_>,
    maps: &[Partition],
    partition: u16,
    block: u32,
    length: u64,
) -> Result<Vec<u8>> {
    reader.file(&ImageFile {
        length,
        extents: mapped(maps, partition, block, length)?,
    })
}
fn udf_domain(bytes: &[u8], at: usize) -> Result<()> {
    let id = bytes
        .get(at..at + 32)
        .ok_or(ImageError::Malformed("short UDF domain identifier"))?;
    if id[0] != 0 || &id[1..24] != b"*OSTA UDF Compliant\0\0\0\0" {
        return Err(ImageError::Unsupported(
            "unrecognized or dirty UDF domain identifier",
        ));
    }
    if !matches!(
        le16(id, 24)?,
        0x0102 | 0x0150 | 0x0200 | 0x0201 | 0x0250 | 0x0260
    ) {
        return Err(ImageError::Unsupported("UDF domain revision"));
    }
    // Hard/soft write protection does not restrict this read-only reader.
    if id[26] & !3 != 0 || id[27..].iter().any(|byte| *byte != 0) {
        return Err(ImageError::Unsupported("UDF domain flags or extensions"));
    }
    Ok(())
}

fn long_ad(bytes: &[u8], at: usize) -> Result<(u32, u16, u32)> {
    let length = le32(bytes, at)?;
    if length >> 30 != 0 {
        return Err(ImageError::Unsupported("unrecorded UDF ICB extent"));
    }
    if le16(bytes, at + 10)? != 0 {
        return Err(ImageError::Unsupported(
            "erased or extended UDF ICB allocation",
        ));
    }
    Ok((le32(bytes, at + 4)?, le16(bytes, at + 8)?, length))
}
fn udf(reader: &mut Reader<'_>, anchor: &[u8]) -> Result<Image> {
    let length = le32(anchor, 16)?;
    let start = le32(anchor, 20)?;
    match udf_volume(reader, start, length) {
        Ok(image) => Ok(image),
        Err(error) => {
            let reserve_length = le32(anchor, 24)?;
            let reserve_start = le32(anchor, 28)?;
            if reserve_length == 0 || matches!(error, ImageError::Budget) {
                return Err(error);
            }
            udf_volume(reader, reserve_start, reserve_length).map_err(|_| error)
        }
    }
}
fn udf_volume(reader: &mut Reader<'_>, mut start: u32, mut length: u32) -> Result<Image> {
    let mut physical = BTreeMap::new();
    let mut logical: Option<(u32, Vec<u8>)> = None;
    let mut visited = BTreeSet::new();
    let mut descriptor_bytes = 0_u64;
    loop {
        if length == 0 || length % BLOCK as u32 != 0 {
            return Err(ImageError::Malformed(
                "invalid UDF descriptor sequence extent",
            ));
        }
        descriptor_bytes += u64::from(length);
        if descriptor_bytes > 1024 * 1024 || visited.len() >= 64 {
            return Err(ImageError::Budget);
        }
        if u64::from(start) * BLOCK + u64::from(length) > reader.source.len() {
            return Err(ImageError::Malformed(
                "UDF descriptor sequence outside image",
            ));
        }
        if !visited.insert(start) {
            return Err(ImageError::Malformed(
                "cyclic UDF volume descriptor sequence",
            ));
        }
        let mut continuation = None;
        for index in 0..length / BLOCK as u32 {
            let block = start
                .checked_add(index)
                .ok_or(ImageError::Malformed("UDF descriptor sequence overflow"))?;
            let bytes = reader.read(u64::from(block) * BLOCK, BLOCK)?;
            let sequence = le32(&bytes, 16)?;
            match validate_tag(&bytes, Some(block))? {
                5 => {
                    let extent = Extent {
                        offset: u64::from(le32(&bytes, 188)?) * BLOCK,
                        length: u64::from(le32(&bytes, 192)?) * BLOCK,
                    };
                    ExtentSource::new(reader.source, vec![extent])?;
                    let number = le16(&bytes, 22)?;
                    if let Some((previous_sequence, previous_extent)) = physical.get(&number) {
                        if sequence < *previous_sequence {
                            continue;
                        }
                        if sequence == *previous_sequence && extent != *previous_extent {
                            return Err(ImageError::Malformed(
                                "conflicting UDF partition descriptors",
                            ));
                        }
                    }
                    physical.insert(number, (sequence, extent));
                }
                6 => {
                    if let Some((previous_sequence, previous)) = &logical {
                        if bytes[84..212] != previous[84..212] {
                            return Err(ImageError::Unsupported("multiple UDF logical volumes"));
                        }
                        if sequence < *previous_sequence {
                            continue;
                        }
                        if sequence == *previous_sequence && bytes[16..] != previous[16..] {
                            return Err(ImageError::Malformed(
                                "conflicting UDF logical volume descriptors",
                            ));
                        }
                    }
                    logical = Some((sequence, bytes));
                }
                8 => break,
                3 => {
                    let next_length = le32(&bytes, 20)?;
                    if next_length != 0 {
                        continuation = Some((le32(&bytes, 24)?, next_length));
                    }
                    break;
                }
                _ => {}
            }
        }
        let Some((next_start, next_length)) = continuation else {
            break;
        };
        start = next_start;
        length = next_length;
    }
    let (_, logical) = logical.ok_or(ImageError::Malformed(
        "missing UDF logical volume descriptor",
    ))?;
    udf_domain(&logical, 216)?;
    if le32(&logical, 212)? != BLOCK as u32 {
        return Err(ImageError::Unsupported("UDF logical block size"));
    }
    let table_end = 440_usize
        .checked_add(le32(&logical, 264)? as usize)
        .filter(|end| *end <= logical.len())
        .ok_or(ImageError::Malformed("UDF partition map length"))?;
    let count = le32(&logical, 268)?;
    if count == 0 || count > 64 {
        return Err(ImageError::Unsupported("UDF partition map count"));
    }
    let mut at = 440;
    let mut maps = Vec::new();
    let mut metadata_maps = Vec::new();
    for _ in 0..count {
        let header = logical
            .get(at..at + 2)
            .filter(|_| at + 2 <= table_end)
            .ok_or(ImageError::Malformed("short UDF partition map"))?;
        let size = usize::from(header[1]);
        if size < 2 || at + size > table_end {
            return Err(ImageError::Malformed("invalid UDF partition map size"));
        }
        let map = &logical[at..at + size];
        let number = match (map[0], size) {
            (1, 6) => {
                if le16(map, 2)? != 1 {
                    return Err(ImageError::Unsupported(
                        "multi-volume UDF partition reference",
                    ));
                }
                le16(map, 4)?
            }
            (2, 64) if map.get(5..28) == Some(b"*UDF Metadata Partition") => {
                if le16(map, 36)? != 1 {
                    return Err(ImageError::Unsupported(
                        "multi-volume UDF metadata partition",
                    ));
                }
                if map[4] != 0
                    || !matches!(le16(map, 28)?, 0x0250 | 0x0260)
                    || map[58] & !1 != 0
                    || map[59..].iter().any(|byte| *byte != 0)
                {
                    return Err(ImageError::Unsupported(
                        "UDF metadata partition revision or flags",
                    ));
                }
                metadata_maps.push((maps.len(), le32(map, 40)?, le32(map, 44)?));
                le16(map, 38)?
            }
            _ => {
                return Err(ImageError::Unsupported(
                    "UDF virtual, sparable, or unknown partition map",
                ));
            }
        };
        maps.push(Partition::Physical(
            physical
                .get(&number)
                .ok_or(ImageError::Malformed("missing UDF physical partition"))?
                .1,
        ));
        at += size;
    }
    if at != table_end {
        return Err(ImageError::Malformed("trailing UDF partition map data"));
    }
    for (index, location, mirror) in metadata_maps {
        let primary =
            udf_file(reader, &maps, index as u16, location).and_then(|(file, kind, _)| {
                if kind == 250 {
                    Ok(file)
                } else {
                    Err(ImageError::Malformed("invalid UDF metadata file type"))
                }
            });
        let file = match primary {
            Ok(file) => file,
            Err(error) if mirror != u32::MAX && !matches!(error, ImageError::Budget) => {
                let (file, kind, _) = udf_file(reader, &maps, index as u16, mirror)?;
                if kind != 251 {
                    return Err(ImageError::Malformed(
                        "invalid UDF metadata mirror file type",
                    ));
                }
                file
            }
            Err(error) => return Err(error),
        };
        maps[index] = Partition::Metadata(file.extents);
    }
    let (block, partition, length) = long_ad(&logical, 248)?;
    if length < BLOCK as u32 {
        return Err(ImageError::Malformed("short UDF file set descriptor"));
    }
    let fsd = mapped_read(reader, &maps, partition, block, BLOCK)?;
    if validate_tag(&fsd, Some(block))? != 256 {
        return Err(ImageError::Malformed("missing UDF file set descriptor"));
    }
    udf_domain(&fsd, 416)?;
    if le32(&fsd, 448)? != 0 {
        return Err(ImageError::Unsupported("chained UDF file sets"));
    }
    let (root, partition, _) = long_ad(&fsd, 400)?;
    let mut files = BTreeMap::new();
    udf_directory(
        reader,
        &maps,
        partition,
        root,
        "",
        0,
        &mut BTreeSet::new(),
        &mut files,
    )?;
    Ok(Image {
        filesystem: "UDF".into(),
        volume_label: None,
        files,
        bytes_read: 0,
    })
}

fn udf_file(
    reader: &mut Reader<'_>,
    maps: &[Partition],
    partition: u16,
    block: u32,
) -> Result<(ImageFile, u8, Vec<Extent>)> {
    let bytes = mapped_read(reader, maps, partition, block, BLOCK)?;
    let tag = validate_tag(&bytes, Some(block))?;
    let (ea_at, ad_at, data_at) = match tag {
        261 => (168, 172, 176),
        266 => (208, 212, 216),
        _ => {
            return Err(ImageError::Unsupported(
                "UDF indirect or unsupported file entry",
            ));
        }
    };
    if le16(&bytes, 20)? != 4 || le16(&bytes, 22)? != 0 || le16(&bytes, 24)? != 1 {
        return Err(ImageError::Unsupported("UDF file entry strategy"));
    }
    if le16(&bytes, 34)? & 0xf800 != 0 {
        return Err(ImageError::Unsupported(
            "transformed, versioned, or extended UDF file entry",
        ));
    }
    if bytes[50] != 0 {
        return Err(ImageError::Unsupported("UDF record-oriented file"));
    }
    let kind = bytes[27];
    if !matches!(kind, 4 | 5 | 250 | 251) {
        return Err(ImageError::Unsupported("UDF non-regular file type"));
    }
    let length = le64(&bytes, 56)?;
    let start = (data_at as usize)
        .checked_add(le32(&bytes, ea_at)? as usize)
        .ok_or(ImageError::Malformed(
            "UDF extended attribute size overflow",
        ))?;
    let end = start
        .checked_add(le32(&bytes, ad_at)? as usize)
        .filter(|end| *end <= bytes.len())
        .ok_or(ImageError::Malformed("UDF allocation descriptor bounds"))?;
    let mode = le16(&bytes, 34)? & 7;
    // Keep partition-relative byte positions until directory tags are checked.
    // Physical image addresses cannot recover these after metadata mapping.
    let mut logical_extents = Vec::new();
    let extents = if mode == 3 {
        if length > (end - start) as u64 {
            return Err(ImageError::Malformed("short inline UDF file"));
        }
        logical_extents.push(Extent {
            offset: u64::from(block) * BLOCK + start as u64,
            length,
        });
        range(
            &mapped(maps, partition, block, BLOCK)?,
            start as u64,
            length,
        )?
    } else {
        let mut extents = Vec::new();
        allocation_descriptors(
            reader,
            maps,
            partition,
            mode,
            &bytes[start..end],
            0,
            &mut BTreeSet::new(),
            &mut extents,
            &mut logical_extents,
        )?;
        range(&extents, 0, length)?
    };
    Ok((
        ImageFile { length, extents },
        kind,
        range(&logical_extents, 0, length)?,
    ))
}
fn allocation_descriptors(
    reader: &mut Reader<'_>,
    maps: &[Partition],
    partition: u16,
    mode: u16,
    bytes: &[u8],
    depth: usize,
    visited: &mut BTreeSet<(u16, u32)>,
    extents: &mut Vec<Extent>,
    logical_extents: &mut Vec<Extent>,
) -> Result<()> {
    if depth > 16 {
        return Err(ImageError::Budget);
    }
    let size = match mode {
        0 => 8,
        1 => 16,
        _ => {
            return Err(ImageError::Unsupported(
                "UDF extended allocation descriptors",
            ));
        }
    };
    if bytes.len() % size != 0 {
        return Err(ImageError::Malformed("partial UDF allocation descriptor"));
    }
    for ad in bytes.chunks_exact(size) {
        let raw = le32(ad, 0)?;
        let length = u64::from(raw & 0x3fff_ffff);
        if length == 0 {
            continue;
        }
        let block = le32(ad, 4)?;
        let part = if mode == 1 { le16(ad, 8)? } else { partition };
        match raw >> 30 {
            0 => {
                extents.extend(mapped(maps, part, block, length)?);
                logical_extents.push(Extent {
                    offset: u64::from(block) * BLOCK,
                    length,
                });
            }
            3 => {
                if !visited.insert((part, block)) {
                    return Err(ImageError::Malformed("cyclic UDF allocation extent"));
                }
                let data = mapped_read(reader, maps, part, block, length)?;
                if validate_tag(&data, Some(block))? != 258 {
                    return Err(ImageError::Malformed("invalid UDF allocation extent"));
                }
                let end = 24_usize
                    .checked_add(le32(&data, 20)? as usize)
                    .filter(|end| *end <= data.len())
                    .ok_or(ImageError::Malformed("UDF allocation extent bounds"))?;
                allocation_descriptors(
                    reader,
                    maps,
                    part,
                    mode,
                    &data[24..end],
                    depth + 1,
                    visited,
                    extents,
                    logical_extents,
                )?;
            }
            _ => return Err(ImageError::Unsupported("sparse or unrecorded UDF data")),
        }
        if extents.len() > ENTRY_LIMIT {
            return Err(ImageError::Budget);
        }
    }
    Ok(())
}
fn udf_directory(
    reader: &mut Reader<'_>,
    maps: &[Partition],
    partition: u16,
    block: u32,
    path: &str,
    depth: usize,
    visited: &mut BTreeSet<(u16, u32)>,
    files: &mut BTreeMap<String, ImageFile>,
) -> Result<()> {
    reader.entry(depth)?;
    if !visited.insert((partition, block)) {
        return Err(ImageError::Malformed("cyclic UDF directory"));
    }
    let (directory, kind, logical_extents) = udf_file(reader, maps, partition, block)?;
    if kind != 4 {
        return Err(ImageError::Malformed(
            "UDF directory points to ordinary file",
        ));
    }
    let bytes = reader.file(&directory)?;
    let mut at = 0;
    while at < bytes.len() {
        reader.entry(depth)?;
        let fid = bytes
            .get(at..)
            .filter(|bytes| bytes.len() >= 38)
            .ok_or(ImageError::Malformed("short UDF file identifier"))?;
        let name_length = usize::from(fid[19]);
        let implementation_length = usize::from(le16(fid, 36)?);
        if implementation_length % 4 != 0 {
            return Err(ImageError::Malformed(
                "unaligned UDF file identifier implementation data",
            ));
        }
        let name_at = 38 + implementation_length;
        let record_length = (name_at + name_length + 3) & !3;
        let fid = fid
            .get(..record_length)
            .ok_or(ImageError::Malformed("UDF file identifier bounds"))?;
        let position = range(&logical_extents, at as u64, 1)?[0].offset / BLOCK;
        let location = u32::try_from(position).map_err(|_| {
            ImageError::Unsupported("UDF directory tag address exceeds descriptor range")
        })?;
        if validate_tag(fid, Some(location))? != 257 {
            return Err(ImageError::Malformed("invalid UDF file identifier"));
        }
        at += record_length;
        if fid[18] & 12 != 0 {
            continue;
        }
        let encoded = &fid[name_at..name_at + name_length];
        let name = match encoded.first() {
            Some(8) => encoded[1..]
                .iter()
                .map(|byte| char::from(*byte))
                .collect::<String>(),
            Some(16) if (encoded.len() - 1) % 2 == 0 => String::from_utf16(
                &encoded[1..]
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>(),
            )
            .map_err(|_| ImageError::Malformed("invalid UDF Unicode name"))?,
            _ => return Err(ImageError::Unsupported("UDF filename compression")),
        };
        let name = filename(&name)?;
        let key = if path.is_empty() {
            name
        } else {
            format!("{path}/{name}")
        };
        let (child, part, _) = long_ad(fid, 20)?;
        if fid[18] & 2 != 0 {
            udf_directory(reader, maps, part, child, &key, depth + 1, visited, files)?;
        } else {
            let (file, kind, _) = udf_file(reader, maps, part, child)?;
            if kind != 5 {
                return Err(ImageError::Malformed("UDF regular file has invalid type"));
            }
            if files.insert(key, file).is_some() {
                return Err(ImageError::Malformed("duplicate UDF filename"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_ranges_across_split_extents_and_rejects_overflow() {
        let extents = [
            Extent {
                offset: 100,
                length: 10,
            },
            Extent {
                offset: 500,
                length: 20,
            },
        ];
        assert_eq!(
            range(&extents, 8, 8).unwrap(),
            [
                Extent {
                    offset: 108,
                    length: 2
                },
                Extent {
                    offset: 500,
                    length: 6
                }
            ]
        );
        assert!(range(&extents, u64::MAX, 2).is_err());
        assert!(range(&extents, 29, 2).is_err());
    }
    fn reference_crc(bytes: &[u8]) -> u16 {
        let mut crc = 0_u16;
        for &byte in bytes {
            crc ^= u16::from(byte) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 {
                    (crc << 1) ^ 0x1021
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    #[test]
    fn table_crc_matches_bitwise_reference_and_known_vector() {
        assert_eq!(udf_crc(b"123456789"), 0x31c3);
        assert_eq!(udf_crc(b""), 0);
        let data: Vec<u8> = (0..65535).map(|i| ((i * 37) ^ (i >> 8)) as u8).collect();
        for offset in 0..16 {
            for size in [0, 1, 2, 15, 16, 255, 496, 2048, data.len() - offset] {
                let data = &data[offset..offset + size];
                assert_eq!(udf_crc(data), reference_crc(data));
            }
        }
    }

    #[test]
    #[ignore = "optimized-build microbenchmark; run explicitly with --release --no-capture"]
    fn benchmark_udf_crc() {
        use std::hint::black_box;
        for size in [496, 2048, 65535] {
            let data: Vec<u8> = (0..size).map(|i| (i * 37) as u8).collect();
            for (name, crc) in [
                ("bitwise", reference_crc as fn(&[u8]) -> u16),
                ("table", udf_crc),
            ] {
                let mut samples = [0.0_f64; 7];
                for sample in &mut samples {
                    let start = std::time::Instant::now();
                    for _ in 0..200 {
                        black_box(crc(black_box(&data)));
                    }
                    *sample = start.elapsed().as_secs_f64() * 1e6 / 200.0;
                }
                samples.sort_by(f64::total_cmp);
                eprintln!("UDF CRC {name} bytes={size}: {:.3}us", samples[3]);
            }
        }
    }

    #[test]
    fn invalid_udf_checksums_are_structural_errors() {
        let mut tag = [0_u8; 512];
        tag[0] = 2;
        tag[2] = 2;
        tag[4] = 4;
        assert!(matches!(
            validate_tag(&tag, Some(0)),
            Err(ImageError::Malformed(
                "UDF CRC does not cover descriptor fields"
            ))
        ));
        tag[10..12].copy_from_slice(&496_u16.to_le_bytes());
        tag[4] = 245;
        assert_eq!(validate_tag(&tag, Some(0)).unwrap(), 2);
        tag[16] = 1;
        assert!(matches!(
            validate_tag(&tag, Some(0)),
            Err(ImageError::Malformed("UDF descriptor CRC mismatch"))
        ));
    }
}
