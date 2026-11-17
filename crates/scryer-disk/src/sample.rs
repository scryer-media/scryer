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

/// The sample of a stream, taken from the bytes as they pass: the first
/// window kept as it arrives, and the latest window's worth held in a ring,
/// so [`Self::finish`] gives what [`sample_reader`] would give for a file
/// holding exactly those bytes. Memory only, at most two windows; nothing is
/// read from any file.
#[derive(Debug, Default)]
pub struct StreamedSample {
    head: Vec<u8>,
    ring: Vec<u8>,
    /// Where the next byte goes in `ring`, once it is a full window long.
    next: usize,
    size_bytes: u64,
}

impl StreamedSample {
    pub fn update(&mut self, buffer: &[u8]) {
        self.size_bytes = self.size_bytes.saturating_add(buffer.len() as u64);
        let room = CONTENT_SAMPLE_BYTES - self.head.len();
        self.head
            .extend_from_slice(&buffer[..buffer.len().min(room)]);
        let kept = &buffer[buffer.len().saturating_sub(CONTENT_SAMPLE_BYTES)..];
        let filling = (CONTENT_SAMPLE_BYTES - self.ring.len()).min(kept.len());
        self.ring.extend_from_slice(&kept[..filling]);
        // Bytes past a full ring overwrite the oldest, in order, at most one
        // window of them, in at most two copies.
        let mut rest = &kept[filling..];
        while !rest.is_empty() {
            let run = (CONTENT_SAMPLE_BYTES - self.next).min(rest.len());
            self.ring[self.next..self.next + run].copy_from_slice(&rest[..run]);
            self.next = (self.next + run) % CONTENT_SAMPLE_BYTES;
            rest = &rest[run..];
        }
    }

    /// The last `len` bytes the stream passed, oldest first.
    fn last(&self, len: usize) -> Vec<u8> {
        if self.ring.len() < CONTENT_SAMPLE_BYTES {
            return self.ring[self.ring.len() - len..].to_vec();
        }
        let start = (self.next + CONTENT_SAMPLE_BYTES - len) % CONTENT_SAMPLE_BYTES;
        let mut last = Vec::with_capacity(len);
        if start + len <= CONTENT_SAMPLE_BYTES {
            last.extend_from_slice(&self.ring[start..start + len]);
        } else {
            last.extend_from_slice(&self.ring[start..]);
            last.extend_from_slice(&self.ring[..start + len - CONTENT_SAMPLE_BYTES]);
        }
        last
    }

    pub fn finish(self) -> ContentSample {
        let size_bytes = self.size_bytes;
        let first_len = size_bytes.min(CONTENT_SAMPLE_BYTES as u64) as usize;
        let last_len = size_bytes
            .saturating_sub(first_len as u64)
            .min(CONTENT_SAMPLE_BYTES as u64) as usize;
        let mut sample = self.head[..first_len].to_vec();
        sample.extend_from_slice(&self.last(last_len));
        ContentSample {
            size_bytes,
            sample_bytes: sample.len() as u64,
            sample_blake3: blake3::hash(&sample).to_hex().to_string(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The sample taken from a stream, in pieces of any size, is the sample
    /// read from a file of those bytes: empty, shorter than a window, between
    /// one and two windows, and past two.
    #[test]
    fn a_streamed_sample_is_the_sample_of_a_file_of_the_same_bytes() {
        let window = CONTENT_SAMPLE_BYTES;
        for len in [
            0,
            7,
            window - 1,
            window,
            window + 3,
            2 * window,
            3 * window + 5,
        ] {
            let bytes: Vec<u8> = (0..len).map(|index| (index % 253) as u8).collect();
            let read = sample_reader(&mut io::Cursor::new(&bytes), len as u64).unwrap();
            for piece in [1usize << 12, window / 3 + 1, window, 2 * window + 1] {
                let mut streamed = StreamedSample::default();
                for chunk in bytes.chunks(piece) {
                    streamed.update(chunk);
                }
                assert_eq!(streamed.finish(), read, "len {len} piece {piece}");
            }
        }
    }
}
