//! The frozen crypto/CRC cores served to archive-extractor guests.
//!
//! The AES-CBC (aws-lc, stateless per call) and CRC-32 (`crc-fast`, seeded)
//! cores are unchanged from the core-module host ABI (`host_aes_cbc_decrypt` /
//! `host_crc32`); only the binding layer moved. Archive extractors are WASI
//! Preview 2 components now, so buffers cross the boundary as `list<u8>`
//! values through the `scryer:archive/crypto@1.0.0` interface instead of as
//! guest pointers into an exported linear memory. The canonical ABI owns the
//! bounds checking the old `checked_range` helper performed, so the `-3` /
//! `-1` out-of-bounds statuses have no counterpart; every other status and
//! every numeric result is bit-for-bit what the core ABI produced.
//!
//! `archive_component_host` is the only consumer: it wires these functions
//! straight into the generated WIT `Host` implementation.

use aws_lc_rs::{
    cipher::{AES_128, AES_256, DecryptingKey, DecryptionContext, UnboundCipherKey},
    iv::{FixedLength, IV_LEN_128_BIT},
};

pub(crate) const AES_BLOCK_LEN: usize = 16;
const AES_128_KEY_LEN: usize = 16;
const AES_256_KEY_LEN: usize = 32;

/// Reflected IEEE CRC-32 resumed from `seed`.
///
/// `buf.is_empty()` returns `seed` unchanged, preserving the streaming
/// verification contract the guest relies on.
pub(crate) fn crc32(seed: u32, buf: &[u8]) -> u32 {
    // `new_with_init_state` accepts the unfinalized state; invert the finalized
    // IEEE CRC seed to preserve the guest ABI's streaming verification contract.
    let mut hasher = crc_fast::Digest::new_with_init_state(
        crc_fast::CrcAlgorithm::Crc32IsoHdlc,
        u64::from(seed ^ u32::MAX),
    );
    hasher.update(buf);
    hasher.finalize() as u32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AesDecryptError {
    KeyLength,
    BlockAlignment,
    IvLength,
}

/// AES-CBC decrypt `data` under `key`/`iv`, returning the plaintext.
///
/// Validation order is the frozen one: key length, then block alignment, then
/// (new, because the IV is now a value rather than a fixed-size read) IV
/// length. The decrypt itself is still performed in place — on the host's own
/// copy of the guest's bytes.
pub(crate) fn aes_cbc_decrypt(
    key: &[u8],
    iv: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, AesDecryptError> {
    if !matches!(key.len(), AES_128_KEY_LEN | AES_256_KEY_LEN) {
        return Err(AesDecryptError::KeyLength);
    }
    if !data.len().is_multiple_of(AES_BLOCK_LEN) {
        return Err(AesDecryptError::BlockAlignment);
    }
    let iv: &[u8; AES_BLOCK_LEN] = iv.try_into().map_err(|_| AesDecryptError::IvLength)?;

    let mut buf = data.to_vec();
    aes_cbc_decrypt_in_place(key, iv, &mut buf)?;
    Ok(buf)
}

fn aes_cbc_decrypt_in_place(
    key: &[u8],
    iv: &[u8; AES_BLOCK_LEN],
    buf: &mut [u8],
) -> Result<(), AesDecryptError> {
    if !matches!(key.len(), AES_128_KEY_LEN | AES_256_KEY_LEN) {
        return Err(AesDecryptError::KeyLength);
    }
    if !buf.len().is_multiple_of(AES_BLOCK_LEN) {
        return Err(AesDecryptError::BlockAlignment);
    }
    if buf.is_empty() {
        return Ok(());
    }

    let algorithm = match key.len() {
        AES_128_KEY_LEN => &AES_128,
        AES_256_KEY_LEN => &AES_256,
        _ => return Err(AesDecryptError::KeyLength),
    };
    let key = UnboundCipherKey::new(algorithm, key).map_err(|_| AesDecryptError::KeyLength)?;
    let decrypting_key = DecryptingKey::cbc(key).map_err(|_| AesDecryptError::KeyLength)?;
    let context = DecryptionContext::Iv128(FixedLength::<IV_LEN_128_BIT>::from(iv));
    decrypting_key
        .decrypt(buf, context)
        .map_err(|_| AesDecryptError::BlockAlignment)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_cbc_decrypts_nist_aes128_vector() {
        let key = hex_bytes("2b7e151628aed2a6abf7158809cf4f3c");
        let iv = hex_bytes("000102030405060708090a0b0c0d0e0f");
        let ciphertext = hex_bytes(
            "7649abac8119b246cee98e9b12e9197d\
             5086cb9b507219ee95db113a917678b2",
        );
        let expected = hex_bytes(
            "6bc1bee22e409f96e93d7e117393172a\
             ae2d8a571e03ac9c9eb76fac45af8e51",
        );

        assert_eq!(aes_cbc_decrypt(&key, &iv, &ciphertext).unwrap(), expected);
    }

    #[test]
    fn aes_cbc_decrypts_nist_aes256_vector() {
        let key = hex_bytes(
            "603deb1015ca71be2b73aef0857d77811\
             f352c073b6108d72d9810a30914dff4",
        );
        let iv: [u8; AES_BLOCK_LEN] = hex_bytes("000102030405060708090a0b0c0d0e0f")
            .try_into()
            .unwrap();
        let mut buf = hex_bytes(
            "f58c4c04d6e5f1ba779eabfb5f7bfbd6\
             9cfc4e967edb808d679f777bc6702c7d",
        );
        let expected = hex_bytes(
            "6bc1bee22e409f96e93d7e117393172a\
             ae2d8a571e03ac9c9eb76fac45af8e51",
        );

        aes_cbc_decrypt_in_place(&key, &iv, &mut buf).unwrap();

        assert_eq!(buf, expected);
    }

    #[test]
    fn aes_cbc_decrypt_accepts_empty_buffer() {
        let key = [0u8; AES_128_KEY_LEN];
        let iv = [0u8; AES_BLOCK_LEN];

        assert_eq!(aes_cbc_decrypt(&key, &iv, &[]).unwrap(), Vec::<u8>::new());
    }

    /// The frozen validation order: key length is rejected before block
    /// alignment, and both before the IV length.
    #[test]
    fn aes_cbc_decrypt_rejects_invalid_lengths_in_order() {
        let iv = [0u8; AES_BLOCK_LEN];
        assert_eq!(
            aes_cbc_decrypt(&[0u8; 15], &iv, &[0u8; AES_BLOCK_LEN]),
            Err(AesDecryptError::KeyLength)
        );
        assert_eq!(
            aes_cbc_decrypt(&[0u8; AES_128_KEY_LEN], &iv, &[0u8; 15]),
            Err(AesDecryptError::BlockAlignment)
        );
        assert_eq!(
            aes_cbc_decrypt(&[0u8; AES_128_KEY_LEN], &[0u8; 15], &[0u8; AES_BLOCK_LEN]),
            Err(AesDecryptError::IvLength)
        );
        // Key length still wins over a simultaneously bad block length.
        assert_eq!(
            aes_cbc_decrypt(&[0u8; 15], &iv, &[0u8; 15]),
            Err(AesDecryptError::KeyLength)
        );
    }

    #[test]
    fn crc32_matches_ieee_check_value() {
        assert_eq!(crc32(0, b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn crc32_chains_from_running_seed() {
        let first = crc32(0, b"archive ");
        let chained = crc32(first, b"payload");
        let combined = crc32(0, b"archive payload");

        assert_eq!(chained, combined);
    }

    #[test]
    fn crc32_of_an_empty_buffer_returns_the_seed() {
        assert_eq!(crc32(0x1234_5678, b""), 0x1234_5678);
    }

    fn hex_bytes(input: &str) -> Vec<u8> {
        let compact = input
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert_eq!(compact.len() % 2, 0);
        (0..compact.len())
            .step_by(2)
            .map(|idx| u8::from_str_radix(&compact[idx..idx + 2], 16).unwrap())
            .collect()
    }
}
