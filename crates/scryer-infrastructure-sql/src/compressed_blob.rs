//! zstd framing for binary column values that are written whole and read
//! whole.

use std::io::Read;

use scryer_application::{AppError, AppResult};

use super::runtime::repo_err;

/// Compresses `bytes` into one zstd frame at `level`, with the frame's
/// content checksum on so a damaged value is refused rather than misread.
pub fn compress_blob(bytes: &[u8], level: i32) -> AppResult<Vec<u8>> {
    let mut compressor = zstd::bulk::Compressor::new(level).map_err(repo_err)?;
    compressor.include_checksum(true).map_err(repo_err)?;
    compressor.compress(bytes).map_err(repo_err)
}

/// Decompresses one zstd frame, refusing output larger than `max_bytes`.
/// The output buffer grows with the data rather than being sized to
/// `max_bytes` up front.
pub fn decompress_blob(encoded: &[u8], max_bytes: usize) -> AppResult<Vec<u8>> {
    let decoder = zstd::stream::read::Decoder::new(encoded).map_err(repo_err)?;
    let mut decoded = Vec::new();
    decoder
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut decoded)
        .map_err(repo_err)?;
    if decoded.len() > max_bytes {
        return Err(AppError::Repository(format!(
            "compressed value decodes to more than {max_bytes} bytes"
        )));
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_blob_round_trips_and_refuses_oversized_output() {
        let bytes = b"synthetic payload ".repeat(1_000);
        let encoded = compress_blob(&bytes, 3).expect("compress");
        assert!(encoded.len() < bytes.len());
        assert_eq!(
            decompress_blob(&encoded, bytes.len()).expect("decompress"),
            bytes
        );
        assert!(decompress_blob(&encoded, bytes.len() - 1).is_err());
        assert!(decompress_blob(b"not a zstd frame", 1024).is_err());
        let mut damaged = encoded.clone();
        let last = damaged.len() - 8;
        damaged[last] ^= 0x55;
        assert!(decompress_blob(&damaged, bytes.len()).is_err());
    }
}
