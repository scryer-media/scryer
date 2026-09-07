//! Storage codec for post-processing script output tails.
//!
//! `post_processing_script_runs.stdout_tail` and `stderr_tail` hold the last
//! 32 KiB of a captured stream as a zstd frame (level 3). Rows written before
//! migration 0210 were plain text; the migrator converts them, and the decoder
//! still accepts raw UTF-8 so a text value that reaches it anyway (a SQLite
//! column with BLOB affinity stores either) renders instead of erroring.

use std::fmt;

const COMPRESSION_LEVEL: i32 = 3;
const ZSTD_FRAME_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

#[derive(Debug)]
pub struct ScriptOutputCodecError(String);

impl fmt::Display for ScriptOutputCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ScriptOutputCodecError {}

/// Compress one captured output tail for storage.
pub fn encode_script_output_tail(text: &str) -> Result<Vec<u8>, ScriptOutputCodecError> {
    zstd::stream::encode_all(text.as_bytes(), COMPRESSION_LEVEL).map_err(|error| {
        ScriptOutputCodecError(format!("failed to compress script output: {error}"))
    })
}

/// Restore the text of one stored output tail. Bytes that are not a zstd
/// frame are treated as legacy raw text.
pub fn decode_script_output_tail(bytes: &[u8]) -> Result<String, ScriptOutputCodecError> {
    if !bytes.starts_with(&ZSTD_FRAME_MAGIC) {
        return Ok(String::from_utf8_lossy(bytes).into_owned());
    }
    let decoded = zstd::stream::decode_all(bytes).map_err(|error| {
        ScriptOutputCodecError(format!("failed to decompress script output: {error}"))
    })?;
    Ok(String::from_utf8_lossy(&decoded).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_text_through_a_zstd_frame() {
        let text = "line one\nline two\n".repeat(2_000);
        let encoded = encode_script_output_tail(&text).unwrap();
        assert!(encoded.starts_with(&ZSTD_FRAME_MAGIC));
        assert!(encoded.len() < text.len() / 4);
        assert_eq!(decode_script_output_tail(&encoded).unwrap(), text);
    }

    #[test]
    fn round_trips_empty_text() {
        let encoded = encode_script_output_tail("").unwrap();
        assert_eq!(decode_script_output_tail(&encoded).unwrap(), "");
    }

    #[test]
    fn legacy_raw_text_decodes_as_itself() {
        assert_eq!(
            decode_script_output_tail(b"plain legacy output").unwrap(),
            "plain legacy output"
        );
    }

    #[test]
    fn truncated_frame_is_an_error() {
        let encoded = encode_script_output_tail("some output that compresses").unwrap();
        assert!(decode_script_output_tail(&encoded[..encoded.len() / 2]).is_err());
    }
}
