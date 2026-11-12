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

/// The zstd level for text payloads stored with [`compress_text`].
pub const TEXT_COMPRESSION_LEVEL: i32 = 3;

/// The first four bytes of every zstd frame. A stored text payload that starts
/// with them is compressed; any other value is the text's plain UTF-8 bytes.
pub const ZSTD_FRAME_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Largest location-operation plan a write accepts and a read decodes:
/// SQLite's default maximum string length, so every plan the former text
/// column could hold still reads. A cross-library transfer of about 10,000
/// titles is around 14 MB.
pub const LOCATION_PLAN_MAX_BYTES: usize = 1_000_000_000;

/// Largest media analysis a write accepts and a read decodes. A typical
/// analysis is a few kilobytes; a file with hundreds of streams stays far
/// below this.
pub const MEDIA_ANALYSIS_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Whether a stored text payload is a zstd frame rather than plain bytes.
pub fn is_compressed_text(stored: &[u8]) -> bool {
    stored.starts_with(&ZSTD_FRAME_MAGIC)
}

/// Compresses one text payload into a zstd frame, refusing text longer than
/// `max_bytes` so nothing is written that [`decompress_text`] would refuse.
pub fn compress_text(text: &str, max_bytes: usize) -> AppResult<Vec<u8>> {
    if text.len() > max_bytes {
        return Err(AppError::Repository(format!(
            "text payload is {} bytes, exceeding the {max_bytes}-byte limit",
            text.len()
        )));
    }
    compress_blob(text.as_bytes(), TEXT_COMPRESSION_LEVEL)
}

/// Restores one stored text payload. A value that is not a zstd frame is the
/// text's own UTF-8 bytes, as a migration copied it before compression; valid
/// UTF-8 never starts with the frame magic, so the two forms cannot be
/// confused.
pub fn decompress_text(stored: &[u8], max_bytes: usize) -> AppResult<String> {
    let bytes = if is_compressed_text(stored) {
        decompress_blob(stored, max_bytes)?
    } else {
        stored.to_vec()
    };
    String::from_utf8(bytes).map_err(repo_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_text_round_trips_and_plain_bytes_read_as_themselves() {
        let text = "{\"synthetic\":\"payload\"}".repeat(500);
        let encoded = compress_text(&text, text.len()).expect("compress");
        assert!(encoded.starts_with(&ZSTD_FRAME_MAGIC));
        assert!(encoded.len() < text.len() / 4);
        assert_eq!(decompress_text(&encoded, text.len()).expect("decode"), text);
        assert!(decompress_text(&encoded, text.len() - 1).is_err());
        assert!(compress_text(&text, text.len() - 1).is_err());

        assert_eq!(
            decompress_text(b"{\"legacy\":true}", 4).expect("plain bytes"),
            "{\"legacy\":true}"
        );
        assert_eq!(decompress_text(b"", 4).expect("empty"), "");
        assert!(decompress_text(&[0xff, 0xfe], 4).is_err());
        assert!(decompress_text(&encoded[..encoded.len() / 2], text.len()).is_err());
    }

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
