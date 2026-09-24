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
//! `crc` is the 1.1.0 catalog function: any named `crc-fast` algorithm,
//! started fresh or resumed from a previous finalized result.
//!
//! `archive_component_host` is the only consumer: it wires these functions
//! straight into the generated WIT `Host` implementation.

use aws_lc_rs::{
    cipher::{AES_128, AES_256, DecryptingKey, DecryptionContext, UnboundCipherKey},
    iv::{FixedLength, IV_LEN_128_BIT},
};
use crc_fast::{CrcAlgorithm, Digest};

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
    let mut hasher =
        Digest::new_with_init_state(CrcAlgorithm::Crc32IsoHdlc, u64::from(seed ^ u32::MAX));
    hasher.update(buf);
    hasher.finalize() as u32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrcError {
    /// A custom-parameter variant; only named catalog algorithms are served.
    UnsupportedAlgorithm,
    /// The resume seed has bits set above the algorithm's width.
    SeedOutOfRange,
}

/// Register width in bits of a named `crc-fast` catalog algorithm.
#[allow(deprecated)]
const fn crc_width(algorithm: CrcAlgorithm) -> Option<u32> {
    use CrcAlgorithm::*;
    match algorithm {
        Crc16Arc | Crc16Cdma2000 | Crc16Cms | Crc16Dds110 | Crc16DectR | Crc16DectX | Crc16Dnp
        | Crc16En13757 | Crc16Genibus | Crc16Gsm | Crc16Ibm3740 | Crc16IbmSdlc
        | Crc16IsoIec144433A | Crc16Kermit | Crc16Lj1200 | Crc16M17 | Crc16MaximDow
        | Crc16Mcrf4xx | Crc16Modbus | Crc16Nrsc5 | Crc16OpensafetyA | Crc16OpensafetyB
        | Crc16Profibus | Crc16Riello | Crc16SpiFujitsu | Crc16T10Dif | Crc16Teledisk
        | Crc16Tms37157 | Crc16Umts | Crc16Usb | Crc16Xmodem => Some(16),
        Crc32Aixm | Crc32Autosar | Crc32Base91D | Crc32Bzip2 | Crc32CdRomEdc | Crc32Cksum
        | Crc32Iscsi | Crc32IsoHdlc | Crc32Jamcrc | Crc32Mef | Crc32Mpeg2 | Crc32Xfer => Some(32),
        Crc64Ecma182 | Crc64GoIso | Crc64Ms | Crc64Nvme | Crc64Redis | Crc64We | Crc64Xz => {
            Some(64)
        }
        // Custom variants carry no catalog parameters; crc-fast panics on them.
        CrcCustom | Crc32Custom | Crc64Custom => None,
    }
}

/// `algorithm` over `buf`, finalized and zero-extended to 64 bits.
///
/// `None` starts from the algorithm's initial value. `Some(previous)` resumes
/// from a finalized result of an earlier call: crc-fast finalizes as
/// `state ^ xorout`, so the running state is `previous ^ xorout` exactly, and
/// the resumed result equals the one-shot checksum of the concatenated input.
pub(crate) fn crc(algorithm: CrcAlgorithm, seed: Option<u64>, buf: &[u8]) -> Result<u64, CrcError> {
    let width = crc_width(algorithm).ok_or(CrcError::UnsupportedAlgorithm)?;
    let mut digest = match seed {
        None => Digest::new(algorithm),
        Some(previous) => {
            if width < u64::BITS && previous >> width != 0 {
                return Err(CrcError::SeedOutOfRange);
            }
            // A zero-state digest finalizes to the algorithm's xorout.
            let xorout = Digest::new_with_init_state(algorithm, 0).finalize();
            Digest::new_with_init_state(algorithm, previous ^ xorout)
        }
    };
    digest.update(buf);
    Ok(digest.finalize())
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

    /// Every named catalog algorithm the 1.1.0 `crc-algorithm` enum maps to.
    const CATALOG: [CrcAlgorithm; 50] = {
        use CrcAlgorithm::*;
        [
            Crc16Arc,
            Crc16Cdma2000,
            Crc16Cms,
            Crc16Dds110,
            Crc16DectR,
            Crc16DectX,
            Crc16Dnp,
            Crc16En13757,
            Crc16Genibus,
            Crc16Gsm,
            Crc16Ibm3740,
            Crc16IbmSdlc,
            Crc16IsoIec144433A,
            Crc16Kermit,
            Crc16Lj1200,
            Crc16M17,
            Crc16MaximDow,
            Crc16Mcrf4xx,
            Crc16Modbus,
            Crc16Nrsc5,
            Crc16OpensafetyA,
            Crc16OpensafetyB,
            Crc16Profibus,
            Crc16Riello,
            Crc16SpiFujitsu,
            Crc16T10Dif,
            Crc16Teledisk,
            Crc16Tms37157,
            Crc16Umts,
            Crc16Usb,
            Crc16Xmodem,
            Crc32Aixm,
            Crc32Autosar,
            Crc32Base91D,
            Crc32Bzip2,
            Crc32CdRomEdc,
            Crc32Cksum,
            Crc32Iscsi,
            Crc32IsoHdlc,
            Crc32Jamcrc,
            Crc32Mef,
            Crc32Mpeg2,
            Crc32Xfer,
            Crc64Ecma182,
            Crc64GoIso,
            Crc64Ms,
            Crc64Nvme,
            Crc64Redis,
            Crc64We,
            Crc64Xz,
        ]
    };

    /// Long enough to cross crc-fast's SIMD folding thresholds, with a
    /// non-repeating byte pattern.
    fn crc_payload() -> Vec<u8> {
        (0..600u32)
            .map(|i| (i.wrapping_mul(131) >> 3) as u8)
            .collect()
    }

    #[test]
    fn crc_from_none_matches_the_one_shot_checksum() {
        for algorithm in CATALOG {
            for input in [&b""[..], b"123456789", &crc_payload()] {
                assert_eq!(
                    crc(algorithm, None, input),
                    Ok(crc_fast::checksum(algorithm, input)),
                    "{algorithm:?}"
                );
            }
        }
    }

    #[test]
    fn crc_results_fit_the_algorithm_width() {
        for algorithm in CATALOG {
            let width = crc_width(algorithm).expect("catalog algorithm has a width");
            let result = crc(algorithm, None, &crc_payload()).unwrap();
            assert!(width == 64 || result >> width == 0, "{algorithm:?}");
        }
    }

    /// Resuming from any split point reproduces the one-shot checksum, which
    /// is the whole streaming contract.
    #[test]
    fn crc_resumed_at_every_split_matches_the_one_shot_checksum() {
        let payload = crc_payload();
        for algorithm in CATALOG {
            let whole = crc_fast::checksum(algorithm, &payload);
            for split in [0, 1, 7, 16, 63, 64, 65, 255, 256, 300, 599, 600] {
                let (head, tail) = payload.split_at(split);
                let first = crc(algorithm, None, head).unwrap();
                assert_eq!(
                    crc(algorithm, Some(first), tail),
                    Ok(whole),
                    "{algorithm:?} split at {split}"
                );
            }
        }
    }

    #[test]
    fn crc_of_an_empty_buffer_returns_the_seed() {
        for algorithm in CATALOG {
            let seed = crc(algorithm, None, b"123456789").unwrap();
            assert_eq!(crc(algorithm, Some(seed), b""), Ok(seed), "{algorithm:?}");
        }
    }

    #[test]
    fn crc_iso_hdlc_matches_the_frozen_crc32() {
        let payload = crc_payload();
        let seed = crc32(0, b"archive ");
        assert_eq!(
            crc(CrcAlgorithm::Crc32IsoHdlc, Some(u64::from(seed)), &payload),
            Ok(u64::from(crc32(seed, &payload)))
        );
        assert_eq!(
            crc(CrcAlgorithm::Crc32IsoHdlc, Some(0), &payload),
            crc(CrcAlgorithm::Crc32IsoHdlc, None, &payload)
        );
    }

    #[test]
    fn crc64_xz_matches_the_catalog_check_value() {
        assert_eq!(
            crc(CrcAlgorithm::Crc64Xz, None, b"123456789"),
            Ok(0x995d_c9bb_df19_39fa)
        );
    }

    #[test]
    fn crc_rejects_a_seed_wider_than_the_algorithm() {
        assert_eq!(
            crc(CrcAlgorithm::Crc16Arc, Some(0x1_0000), b"x"),
            Err(CrcError::SeedOutOfRange)
        );
        assert_eq!(
            crc(CrcAlgorithm::Crc32IsoHdlc, Some(1 << 32), b"x"),
            Err(CrcError::SeedOutOfRange)
        );
        assert!(crc(CrcAlgorithm::Crc16Arc, Some(0xffff), b"x").is_ok());
        assert!(crc(CrcAlgorithm::Crc64Xz, Some(u64::MAX), b"x").is_ok());
    }

    #[test]
    fn crc_rejects_custom_variants_instead_of_panicking() {
        assert_eq!(
            crc(CrcAlgorithm::CrcCustom, None, b"x"),
            Err(CrcError::UnsupportedAlgorithm)
        );
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
