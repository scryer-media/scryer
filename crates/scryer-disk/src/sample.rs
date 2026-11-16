//! The sampled content proof: a file's size plus a BLAKE3 digest of its first
//! and last windows. Deliberately not a full-file hash. Scan signatures, import
//! cleanup and copy verification all compare this one proof, so they can never
//! disagree about what "the same file" means.

use std::io::{self, Read, Seek, SeekFrom};

/// The length of each sampled window.
pub const CONTENT_SAMPLE_BYTES: usize = 1024 * 1024;

/// What the sampled proof says about one file's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentSample {
    pub size_bytes: u64,
    pub sample_bytes: u64,
    pub sample_blake3: String,
}

/// Which step of taking a sample failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleStep {
    Seek,
    Read,
}

/// Sample an open file of `size_bytes`: its first window, then its last when
/// anything is left after the first.
pub fn sample_reader<R: Read + Seek>(
    reader: &mut R,
    size_bytes: u64,
) -> Result<ContentSample, (SampleStep, io::Error)> {
    let first_len = size_bytes.min(CONTENT_SAMPLE_BYTES as u64) as usize;
    let mut sample = Vec::with_capacity(first_len.saturating_mul(2));
    read_window(reader, 0, first_len, &mut sample)?;

    let remaining_after_first = size_bytes.saturating_sub(first_len as u64);
    let last_len = remaining_after_first.min(CONTENT_SAMPLE_BYTES as u64) as usize;
    if last_len > 0 {
        read_window(reader, size_bytes - last_len as u64, last_len, &mut sample)?;
    }

    Ok(ContentSample {
        size_bytes,
        sample_bytes: sample.len() as u64,
        sample_blake3: blake3::hash(&sample).to_hex().to_string(),
    })
}

fn read_window<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    len: usize,
    sample: &mut Vec<u8>,
) -> Result<(), (SampleStep, io::Error)> {
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|error| (SampleStep::Seek, error))?;
    let start = sample.len();
    sample.resize(start + len, 0);
    reader
        .read_exact(&mut sample[start..])
        .map_err(|error| (SampleStep::Read, error))
}
