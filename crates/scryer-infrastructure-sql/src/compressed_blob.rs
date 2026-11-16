//! zstd framing for binary column values that are written whole and read
//! whole.

use std::cell::RefCell;
use std::io::{self, Read};

use scryer_application::{AppError, AppResult};

use super::runtime::repo_err;

/// How many compression setups (level and checksum flag) one thread keeps a
/// context for. The callers use a handful of constant setups; any beyond this
/// get a context of their own for that call.
const MAX_CACHED_COMPRESSORS: usize = 4;

thread_local! {
    /// zstd compression contexts, one per setup, reused across calls on this
    /// thread. A context is only borrowed for the duration of one synchronous
    /// call, so nothing is held across an await point.
    static COMPRESSORS: RefCell<Vec<(i32, bool, zstd::bulk::Compressor<'static>)>> =
        const { RefCell::new(Vec::new()) };
    /// One zstd decompression context reused across calls on this thread.
    static DECOMPRESSOR: RefCell<Option<zstd::bulk::Decompressor<'static>>> =
        const { RefCell::new(None) };
}

fn new_compressor(level: i32, checksum: bool) -> io::Result<zstd::bulk::Compressor<'static>> {
    let mut compressor = zstd::bulk::Compressor::new(level)?;
    compressor.include_checksum(checksum)?;
    Ok(compressor)
}

/// Compresses `bytes` into one zstd frame with a context reused on this
/// thread. The frame is byte-for-byte what a fresh
/// `zstd::bulk::Compressor` with the same level and checksum flag produces.
pub(crate) fn compress_with_thread_context(
    bytes: &[u8],
    level: i32,
    checksum: bool,
) -> io::Result<Vec<u8>> {
    let cached = COMPRESSORS.try_with(|cell| {
        let Ok(mut cache) = cell.try_borrow_mut() else {
            return None;
        };
        if let Some((_, _, compressor)) =
            cache.iter_mut().find(|(cached_level, cached_checksum, _)| {
                *cached_level == level && *cached_checksum == checksum
            })
        {
            return Some(compressor.compress(bytes));
        }
        let mut compressor = match new_compressor(level, checksum) {
            Ok(compressor) => compressor,
            Err(error) => return Some(Err(error)),
        };
        let compressed = compressor.compress(bytes);
        if cache.len() < MAX_CACHED_COMPRESSORS {
            cache.push((level, checksum, compressor));
        }
        Some(compressed)
    });
    match cached {
        Ok(Some(result)) => result,
        _ => new_compressor(level, checksum)?.compress(bytes),
    }
}

/// Decompresses `encoded` with zstd's bulk decoder through a context reused
/// on this thread, allowing at most `capacity` bytes of output. The output
/// buffer is sized from the frames' recorded content size when they carry
/// one, and to `capacity` otherwise, exactly as `zstd::bulk::decompress`.
pub(crate) fn decompress_with_thread_context(
    encoded: &[u8],
    capacity: usize,
) -> io::Result<Vec<u8>> {
    let cached = DECOMPRESSOR.try_with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else {
            return None;
        };
        let decompressor = match slot.as_mut() {
            Some(decompressor) => decompressor,
            None => match zstd::bulk::Decompressor::new() {
                Ok(decompressor) => slot.insert(decompressor),
                Err(error) => return Some(Err(error)),
            },
        };
        Some(decompressor.decompress(encoded, capacity))
    });
    match cached {
        Ok(Some(result)) => result,
        _ => zstd::bulk::Decompressor::new()?.decompress(encoded, capacity),
    }
}

/// The decompressed size `encoded` records, when it is exactly one zstd frame
/// whose header carries its content size.
fn single_frame_content_size(encoded: &[u8]) -> Option<u64> {
    let frame_len = zstd::zstd_safe::find_frame_compressed_size(encoded).ok()?;
    if frame_len != encoded.len() {
        return None;
    }
    zstd::zstd_safe::get_frame_content_size(encoded).ok()?
}

/// Compresses `bytes` into one zstd frame at `level`, with the frame's
/// content checksum on so a damaged value is refused rather than misread.
pub fn compress_blob(bytes: &[u8], level: i32) -> AppResult<Vec<u8>> {
    compress_with_thread_context(bytes, level, true).map_err(repo_err)
}

/// Decompresses one zstd frame, refusing output larger than `max_bytes`.
/// A single frame that records a content size within `max_bytes` decodes into
/// a buffer of exactly that size through a reused context. Any other input
/// streams into a buffer that grows with the data rather than being sized to
/// `max_bytes` up front.
pub fn decompress_blob(encoded: &[u8], max_bytes: usize) -> AppResult<Vec<u8>> {
    if let Some(size) = single_frame_content_size(encoded)
        && size <= max_bytes as u64
    {
        return decompress_with_thread_context(encoded, size as usize).map_err(repo_err);
    }
    decompress_blob_streaming(encoded, max_bytes)
}

fn decompress_blob_streaming(encoded: &[u8], max_bytes: usize) -> AppResult<Vec<u8>> {
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

    fn fresh_compress(bytes: &[u8], level: i32, checksum: bool) -> Vec<u8> {
        let mut compressor = zstd::bulk::Compressor::new(level).expect("compressor");
        compressor
            .include_checksum(checksum)
            .expect("checksum flag");
        compressor.compress(bytes).expect("compress")
    }

    fn synthetic_inputs() -> Vec<Vec<u8>> {
        vec![
            Vec::new(),
            b"x".to_vec(),
            b"synthetic payload ".repeat(1_000),
            (0..200_000u32)
                .flat_map(|value| value.wrapping_mul(2_654_435_761).to_le_bytes())
                .collect(),
            b"{\"synthetic\":\"row\"}".repeat(20_000),
        ]
    }

    #[test]
    fn reused_compression_context_writes_the_frames_a_fresh_one_does() {
        // Interleave setups and repeat inputs so every cached context is
        // reused after compressing something else.
        for _round in 0..3 {
            for input in synthetic_inputs() {
                for (level, checksum) in [(3, true), (3, false), (1, true), (19, true)] {
                    let reused = compress_with_thread_context(&input, level, checksum)
                        .expect("compress through the thread context");
                    assert_eq!(reused, fresh_compress(&input, level, checksum));
                }
                assert_eq!(
                    compress_blob(&input, 3).expect("compress_blob"),
                    fresh_compress(&input, 3, true)
                );
            }
        }
        // More setups than the cache holds still compress correctly.
        for level in 1..=8 {
            let input = b"overflow the cache".repeat(64);
            assert_eq!(
                compress_with_thread_context(&input, level, true).expect("compress"),
                fresh_compress(&input, level, true)
            );
        }
    }

    fn outcome(result: AppResult<Vec<u8>>) -> Result<Vec<u8>, String> {
        result.map_err(|error| match error {
            AppError::Repository(_) => "repository".to_string(),
            other => format!("unexpected error variant: {other:?}"),
        })
    }

    #[test]
    fn sized_decode_matches_the_streaming_decode_for_every_input_shape() {
        let mut cases: Vec<(Vec<u8>, usize)> = Vec::new();
        for input in synthetic_inputs() {
            let encoded = fresh_compress(&input, 3, true);
            cases.push((encoded.clone(), input.len()));
            cases.push((encoded.clone(), input.len() + 10));
            if !input.is_empty() {
                // Over the cap: the frame records more than `max_bytes`.
                cases.push((encoded.clone(), input.len() - 1));
                // Truncated and damaged frames.
                cases.push((encoded[..encoded.len() / 2].to_vec(), input.len()));
                let mut damaged = encoded.clone();
                let at = damaged.len() - 2;
                damaged[at] ^= 0x55;
                cases.push((damaged, input.len()));
            }
            // A streamed frame records no content size.
            let streamed = {
                let mut encoder =
                    zstd::stream::write::Encoder::new(Vec::new(), 3).expect("encoder");
                encoder.include_contentsize(false).expect("no content size");
                std::io::Write::write_all(&mut encoder, &input).expect("write");
                encoder.finish().expect("finish")
            };
            cases.push((streamed, input.len()));
            // Two concatenated frames.
            let mut two = encoded.clone();
            two.extend_from_slice(&encoded);
            cases.push((two, input.len() * 2));
        }
        cases.push((Vec::new(), 16));
        cases.push((b"not a zstd frame".to_vec(), 1024));

        for (encoded, max_bytes) in cases {
            let sized = outcome(decompress_blob(&encoded, max_bytes));
            let streamed = outcome(decompress_blob_streaming(&encoded, max_bytes));
            assert_eq!(
                sized,
                streamed,
                "decode differs for {} encoded bytes with a {max_bytes}-byte cap",
                encoded.len()
            );
        }

        // The oversized refusal keeps its wording.
        let input = b"synthetic payload ".repeat(100);
        let encoded = compress_blob(&input, 3).expect("compress");
        let error = decompress_blob(&encoded, input.len() - 1).expect_err("over the cap");
        assert_eq!(
            error.to_string(),
            decompress_blob_streaming(&encoded, input.len() - 1)
                .expect_err("over the cap")
                .to_string()
        );
    }

    #[test]
    fn compressed_json_through_thread_contexts_matches_the_one_shot_calls() {
        use crate::json::{decode_compressed_json, encode_compressed_json};

        for round in 0..3 {
            let value = serde_json::json!({
                "synthetic": "row".repeat(round * 500 + 1),
                "numbers": (0..round * 100).collect::<Vec<_>>(),
            });
            let encoded = encode_compressed_json(&value).expect("encode");
            let json = serde_json::to_vec(&value).expect("json");
            assert_eq!(
                encoded,
                zstd::bulk::compress(&json, 3).expect("one-shot compress")
            );
            let decoded: serde_json::Value = decode_compressed_json(&encoded).expect("decode");
            assert_eq!(decoded, value);
        }
        // A frame over the 1 MiB cap is refused by both paths.
        let big = zstd::bulk::compress(&vec![b'a'; 1024 * 1024 + 1], 3).expect("compress");
        assert!(zstd::bulk::decompress(&big, 1024 * 1024).is_err());
        assert!(decode_compressed_json::<serde_json::Value>(&big).is_err());
        assert!(decode_compressed_json::<serde_json::Value>(b"not a zstd frame").is_err());
    }
}
