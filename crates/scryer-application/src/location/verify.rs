//! Verified copy machinery: one streaming pass, two hashers (D2).
//!
//! Every byte a location operation copies is read once at the source and fed to
//! both a streaming CRC (the move-corruption check, FR-040) and a full-file
//! BLAKE3 (the dedup identity, FR-041/D4). The CRC is what a full-depth
//! destination read-back is compared against (FR-042); the BLAKE3 is persisted
//! with the media file so dedup never has to re-read anything (D4).
//!
//! # Chosen CRC algorithm: CRC-64/NVME
//!
//! [`MOVE_CRC_ALGORITHM`] is `crc_fast::CrcAlgorithm::Crc64Nvme`, persisted under
//! the tag [`MoveCrcAlgorithm::Crc64Nvme`] so a future default change can never
//! be mistaken for corruption.
//!
//! Measured with a throwaway harness (256 MiB in-memory buffer, 1 MiB update
//! chunks, best of 7 passes, `crc-fast` and `blake3` at `opt-level = 3`) on an
//! Apple M5 Max, native `aarch64-apple-darwin`. `crc-fast` reported the
//! `aarch64-neon-pmull-sha3` tier for every candidate, i.e. the hardware-
//! accelerated path was live throughout. The host was under heavy concurrent
//! build load, so these are contended lower bounds — the ranges are the spread
//! across four repeats of the whole sweep, and only the *relative* ordering is
//! load-independent:
//!
//! | Candidate | Throughput |
//! |---|---|
//! | CRC-64/NVME | 0.36 – 0.58 GiB/s |
//! | CRC-64/GO-ISO | 0.40 – 0.59 GiB/s |
//! | CRC-64/XZ (ECMA-182) | 0.33 – 0.54 GiB/s |
//! | CRC-32/ISCSI (Castagnoli) | 0.88 – 1.09 GiB/s |
//! | CRC-32/ISO-HDLC | 0.52 – 1.08 GiB/s |
//! | BLAKE3 (single-threaded, reference) | 1.27 – 2.31 GiB/s |
//! | Combined pass (CRC-64/NVME + BLAKE3) | 0.26 – 0.46 GiB/s |
//!
//! Why CRC-64/NVME:
//!
//! - It is the fastest CRC-64 variant the crate offers on this hardware.
//!   CRC-64/GO-ISO tied with it inside the measurement noise; CRC-64/XZ was
//!   consistently last. NVME wins the tie as the crate's own headline reflected
//!   CRC-64 (it has a dedicated `crc64_nvme()` entry point) and as a current
//!   standard rather than a legacy one.
//! - The CRC-32 variants are roughly twice as fast but 32 bits is too narrow a
//!   check for multi-gigabyte media files, which is exactly the size class this
//!   subsystem exists to move safely (C4: evidence scales with irreversibility).
//! - The extra CRC-64 cost is not on the critical path. A copy is bounded by
//!   device and network throughput, and the combined pass is bounded by BLAKE3,
//!   which D2 requires regardless. Even contended, the combined pass exceeds any
//!   spinning disk or network share this runs over.
//!
//! Re-measure if the default ever moves: `MoveCrcAlgorithm` exists so stored
//! CRCs stay interpretable across such a change.
//!
//! The copy loop, fsync, depth-governed read-back (`F_NOCACHE` on macOS,
//! `O_DIRECT`/`posix_fadvise` on Linux), and persistence land in T014.

use crc_fast::CrcAlgorithm;

use crate::location::model::{MoveCrcAlgorithm, StreamedContentHashes};

/// The `crc-fast` algorithm used for the streaming move-corruption check.
pub const MOVE_CRC_ALGORITHM: CrcAlgorithm = CrcAlgorithm::Crc64Nvme;

/// The persisted tag written alongside every CRC produced by
/// [`MOVE_CRC_ALGORITHM`] (FR-041 "algorithm-tagged").
pub const MOVE_CRC_ALGORITHM_TAG: MoveCrcAlgorithm = MoveCrcAlgorithm::Crc64Nvme;

/// Feeds every copied buffer to both hashers so the source is read exactly once
/// (D2). The CRC is the move-corruption check compared against the destination
/// read-back (FR-040); the BLAKE3 is the dedup identity persisted with the media
/// file (FR-041, D4).
pub struct StreamedContentHasher {
    crc: crc_fast::Digest,
    blake3: blake3::Hasher,
    size_bytes: u64,
}

impl StreamedContentHasher {
    pub fn new() -> Self {
        Self {
            crc: crc_fast::Digest::new(MOVE_CRC_ALGORITHM),
            blake3: blake3::Hasher::new(),
            size_bytes: 0,
        }
    }

    /// Absorb one buffer of copied bytes.
    pub fn update(&mut self, buffer: &[u8]) {
        self.crc.update(buffer);
        self.blake3.update(buffer);
        self.size_bytes = self.size_bytes.saturating_add(buffer.len() as u64);
    }

    /// Bytes absorbed so far.
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Finish both hashers and produce the pair persisted with the media file.
    pub fn finalize(self) -> StreamedContentHashes {
        StreamedContentHashes {
            size_bytes: self.size_bytes,
            crc_algorithm: MOVE_CRC_ALGORITHM_TAG,
            move_crc: self.crc.finalize(),
            full_blake3: self.blake3.finalize().to_hex().to_string(),
        }
    }
}

impl Default for StreamedContentHasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The streamed pair must match the one-shot values for the same bytes, and
    /// the CRC must be the algorithm the persisted tag claims.
    #[test]
    fn streamed_hashes_match_one_shot_values() {
        let data: Vec<u8> = (0..(3 * 1024 * 1024u32 + 7))
            .map(|index| (index % 251) as u8)
            .collect();

        let mut hasher = StreamedContentHasher::new();
        for chunk in data.chunks(64 * 1024) {
            hasher.update(chunk);
        }
        let hashes = hasher.finalize();

        assert_eq!(hashes.size_bytes, data.len() as u64);
        assert_eq!(hashes.crc_algorithm, MoveCrcAlgorithm::Crc64Nvme);
        assert_eq!(
            hashes.move_crc,
            crc_fast::checksum(MOVE_CRC_ALGORITHM, &data)
        );
        assert_eq!(hashes.full_blake3, blake3::hash(&data).to_hex().to_string());
    }

    /// A single flipped bit must change both the CRC and the BLAKE3 — the whole
    /// point of the streamed pair (FR-040, SC-006).
    #[test]
    fn streamed_hashes_detect_a_flipped_bit() {
        let clean: Vec<u8> = (0..1024 * 1024u32)
            .map(|index| (index % 251) as u8)
            .collect();
        let mut corrupted = clean.clone();
        corrupted[512 * 1024] ^= 0b0000_0001;

        let mut clean_hasher = StreamedContentHasher::new();
        clean_hasher.update(&clean);
        let clean_hashes = clean_hasher.finalize();

        let mut corrupted_hasher = StreamedContentHasher::new();
        corrupted_hasher.update(&corrupted);
        let corrupted_hashes = corrupted_hasher.finalize();

        assert_eq!(clean_hashes.size_bytes, corrupted_hashes.size_bytes);
        assert_ne!(clean_hashes.move_crc, corrupted_hashes.move_crc);
        assert_ne!(clean_hashes.full_blake3, corrupted_hashes.full_blake3);
    }

    /// The persisted algorithm tag round-trips through its setting form.
    #[test]
    fn move_crc_algorithm_tag_round_trips() {
        assert_eq!(MOVE_CRC_ALGORITHM_TAG.as_str(), "crc64_nvme");
        assert_eq!(
            MoveCrcAlgorithm::from_setting("crc64_nvme"),
            Ok(MOVE_CRC_ALGORITHM_TAG)
        );
        assert!(MoveCrcAlgorithm::from_setting("crc32_iscsi").is_err());
    }
}
