//! The frozen crypto/CRC host ABI, served natively on wasmtime.
//!
//! The AES-CBC (aws-lc, stateless per call) and CRC-32 (`crc-fast`, seeded)
//! cores are moved verbatim from the former `archive_crypto_host.rs` — only the
//! guest-memory plumbing changes. The Extism version addressed guest offsets as
//! kernel `MemoryHandle`s (the SIGBUS defect); here we slice the guest's
//! exported linear memory directly (`Memory::data_mut`) for true zero-copy, with
//! the `[ptr, ptr+len)` range validated against the real memory size using
//! overflow-checked arithmetic (§5).

use std::ops::Range;

use aws_lc_rs::{
    cipher::{AES_128, AES_256, DecryptingKey, DecryptionContext, UnboundCipherKey},
    iv::{FixedLength, IV_LEN_128_BIT},
};
use wasmtime::{Caller, Extern, Linker};

/// Import module string both functions live under. The unrar-rs guest's
/// default namespace is the embedder-neutral `host`; Scryer's plugin artifacts
/// opt into `extism:host/user` via unrar-rs's `host-abi-extism` feature, so
/// the host serves that namespace and both sides agree. No extism
/// dependency is involved — the string is just the agreed module name.
const CRYPTO_HOST_NAMESPACE: &str = "extism:host/user";

const AES_BLOCK_LEN: usize = 16;
const AES_128_KEY_LEN: usize = 16;
const AES_256_KEY_LEN: usize = 32;

const AES_STATUS_OK: i64 = 0;
const AES_STATUS_BAD_KEY_LEN: i64 = -1;
const AES_STATUS_BAD_BLOCK_LEN: i64 = -2;
const AES_STATUS_OUT_OF_BOUNDS: i64 = -3;
const CRC_STATUS_OUT_OF_BOUNDS: i64 = -1;

/// Register both §5 host functions under `extism:host/user` on `linker`.
///
/// The canonical import names are `host_aes_cbc_decrypt` / `host_crc32` (the
/// embedder-neutral unrar-rs ABI). The pre-rename `scryer_*` names are also
/// registered as transitional aliases so artifacts built against unrar-rs
/// ≤0.2.0 (the checked-in `fixtures/archive-extraction` blob) still instantiate
/// against this strict linker; drop them once every artifact imports `host_*`.
///
/// Generic over the store data `T`: the functions touch only the guest's
/// exported memory, never the host context, so they compose with any store
/// (the archive host's `HostCtx`, or a bare `()` store in tests).
pub(crate) fn add_to_linker<T: 'static>(linker: &mut Linker<T>) -> wasmtime::Result<()> {
    linker.func_wrap(
        CRYPTO_HOST_NAMESPACE,
        "host_aes_cbc_decrypt",
        host_aes_cbc_decrypt::<T>,
    )?;
    linker.func_wrap(CRYPTO_HOST_NAMESPACE, "host_crc32", host_crc32::<T>)?;
    // Installed artifacts built against the legacy crate <=0.2.x still import the
    // pre-rename `scryer_*` names. Keep these aliases through the plugin upgrade
    // compatibility window even though current artifacts use `host_*`.
    linker.func_wrap(
        CRYPTO_HOST_NAMESPACE,
        "scryer_aes_cbc_decrypt",
        host_aes_cbc_decrypt::<T>,
    )?;
    linker.func_wrap(CRYPTO_HOST_NAMESPACE, "scryer_crc32", host_crc32::<T>)?;
    Ok(())
}

/// `host_aes_cbc_decrypt(key_ptr, key_len, iv_ptr, buf_ptr, buf_len) -> i64`
///
/// AES-CBC decrypt in place over `[buf_ptr, buf_ptr+buf_len)`, stateless per
/// call. Validation order per §5: key length, then block alignment, then
/// memory/bounds. Codes: `0` ok · `-1` bad key length · `-2` buffer not
/// block-aligned · `-3` missing `"memory"` export or out-of-bounds range.
fn host_aes_cbc_decrypt<T: 'static>(
    mut caller: Caller<'_, T>,
    key_ptr: i64,
    key_len: i64,
    iv_ptr: i64,
    buf_ptr: i64,
    buf_len: i64,
) -> i64 {
    if key_len != AES_128_KEY_LEN as i64 && key_len != AES_256_KEY_LEN as i64 {
        return AES_STATUS_BAD_KEY_LEN;
    }
    if buf_len < 0 || buf_len % AES_BLOCK_LEN as i64 != 0 {
        return AES_STATUS_BAD_BLOCK_LEN;
    }

    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(memory)) => memory,
        _ => return AES_STATUS_OUT_OF_BOUNDS,
    };
    let data = memory.data_mut(&mut caller);
    let mem_len = data.len();

    let key_len = key_len as usize;
    let Some(key_range) = checked_range(key_ptr, key_len, mem_len) else {
        return AES_STATUS_OUT_OF_BOUNDS;
    };
    let Some(iv_range) = checked_range(iv_ptr, AES_BLOCK_LEN, mem_len) else {
        return AES_STATUS_OUT_OF_BOUNDS;
    };
    let Some(buf_range) = checked_range(buf_ptr, buf_len as u64 as usize, mem_len) else {
        return AES_STATUS_OUT_OF_BOUNDS;
    };

    // key and iv are small read-only inputs: copy them onto the stack so a
    // single mutable borrow of the bulk buffer drives true in-place decryption.
    let mut key = [0u8; AES_256_KEY_LEN];
    key[..key_len].copy_from_slice(&data[key_range]);
    let mut iv = [0u8; AES_BLOCK_LEN];
    iv.copy_from_slice(&data[iv_range]);

    match aes_cbc_decrypt_in_place(&key[..key_len], &iv, &mut data[buf_range]) {
        Ok(()) => AES_STATUS_OK,
        Err(AesDecryptError::BadKeyLen) => AES_STATUS_BAD_KEY_LEN,
        Err(AesDecryptError::BadBlockLen) => AES_STATUS_BAD_BLOCK_LEN,
    }
}

/// `host_crc32(seed, buf_ptr, buf_len) -> i64`
///
/// IEEE reflected CRC-32 resumed from `seed` (low 32 bits) over the read-only
/// `[buf_ptr, buf_ptr+buf_len)`. `buf_len == 0` returns `seed`. Result in the
/// low 32 bits of a non-negative i64; `-1` on missing `"memory"` export or
/// out-of-bounds range.
fn host_crc32<T: 'static>(mut caller: Caller<'_, T>, seed: i64, buf_ptr: i64, buf_len: i64) -> i64 {
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(memory)) => memory,
        _ => return CRC_STATUS_OUT_OF_BOUNDS,
    };

    let seed = seed as u64 as u32;
    let buf_len = buf_len as u64 as usize;
    if buf_len == 0 {
        // Empty update returns the seed unchanged (§5); no buffer is touched.
        return i64::from(seed);
    }

    let data = memory.data(&caller);
    let Some(range) = checked_range(buf_ptr, buf_len, data.len()) else {
        return CRC_STATUS_OUT_OF_BOUNDS;
    };
    i64::from(crc32(seed, &data[range]))
}

/// Reinterpret an `i64` offset as unsigned and return the checked byte range
/// `[ptr, ptr+len)` iff it fits within `mem_len` (overflow-checked). A negative
/// `i64` becomes a huge `usize` and is rejected here (§5).
fn checked_range(ptr: i64, len: usize, mem_len: usize) -> Option<Range<usize>> {
    let ptr = ptr as u64 as usize;
    let end = ptr.checked_add(len)?;
    (end <= mem_len).then_some(ptr..end)
}

// ── Cores moved verbatim from archive_crypto_host.rs ──────────

fn crc32(seed: u32, buf: &[u8]) -> u32 {
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
enum AesDecryptError {
    BadKeyLen,
    BadBlockLen,
}

fn aes_cbc_decrypt_in_place(
    key: &[u8],
    iv: &[u8; AES_BLOCK_LEN],
    buf: &mut [u8],
) -> Result<(), AesDecryptError> {
    if !matches!(key.len(), AES_128_KEY_LEN | AES_256_KEY_LEN) {
        return Err(AesDecryptError::BadKeyLen);
    }
    if !buf.len().is_multiple_of(AES_BLOCK_LEN) {
        return Err(AesDecryptError::BadBlockLen);
    }
    if buf.is_empty() {
        return Ok(());
    }

    let algorithm = match key.len() {
        AES_128_KEY_LEN => &AES_128,
        AES_256_KEY_LEN => &AES_256,
        _ => return Err(AesDecryptError::BadKeyLen),
    };
    let key = UnboundCipherKey::new(algorithm, key).map_err(|_| AesDecryptError::BadKeyLen)?;
    let decrypting_key = DecryptingKey::cbc(key).map_err(|_| AesDecryptError::BadKeyLen)?;
    let context = DecryptionContext::Iv128(FixedLength::<IV_LEN_128_BIT>::from(iv));
    decrypting_key
        .decrypt(buf, context)
        .map_err(|_| AesDecryptError::BadBlockLen)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Engine, Module, Store};

    // ── Core vector tests (moved verbatim) ────────────────────────────────

    #[test]
    fn aes_cbc_decrypts_nist_aes128_vector() {
        let key = hex_bytes("2b7e151628aed2a6abf7158809cf4f3c");
        let iv: [u8; AES_BLOCK_LEN] = hex_bytes("000102030405060708090a0b0c0d0e0f")
            .try_into()
            .unwrap();
        let mut buf = hex_bytes(
            "7649abac8119b246cee98e9b12e9197d\
             5086cb9b507219ee95db113a917678b2",
        );
        let expected = hex_bytes(
            "6bc1bee22e409f96e93d7e117393172a\
             ae2d8a571e03ac9c9eb76fac45af8e51",
        );

        aes_cbc_decrypt_in_place(&key, &iv, &mut buf).unwrap();

        assert_eq!(buf, expected);
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
        let mut buf = [];

        aes_cbc_decrypt_in_place(&key, &iv, &mut buf).unwrap();
    }

    #[test]
    fn aes_cbc_decrypt_rejects_invalid_lengths() {
        let iv = [0u8; AES_BLOCK_LEN];
        assert_eq!(
            aes_cbc_decrypt_in_place(&[0u8; 15], &iv, &mut [0u8; AES_BLOCK_LEN]),
            Err(AesDecryptError::BadKeyLen)
        );
        assert_eq!(
            aes_cbc_decrypt_in_place(&[0u8; AES_128_KEY_LEN], &iv, &mut [0u8; 15]),
            Err(AesDecryptError::BadBlockLen)
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

    // ── Host-fn tests THROUGH A REAL WASM GUEST (real linear memory) ───────
    //
    // This is the class of test that would have caught the Extism MemoryHandle
    // defect: the guest imports both functions and calls them with offsets into
    // its own exported linear memory; the host slices that same memory.

    /// A guest that imports both host fns, exports memory, and forwards calls.
    const CRYPTO_GUEST_WAT: &str = r#"
        (module
          (import "extism:host/user" "host_aes_cbc_decrypt"
            (func $aes (param i64 i64 i64 i64 i64) (result i64)))
          (import "extism:host/user" "host_crc32"
            (func $crc (param i64 i64 i64) (result i64)))
          (memory (export "memory") 1)
          (func (export "call_aes") (param i64 i64 i64 i64 i64) (result i64)
            (call $aes (local.get 0) (local.get 1) (local.get 2) (local.get 3) (local.get 4)))
          (func (export "call_crc") (param i64 i64 i64) (result i64)
            (call $crc (local.get 0) (local.get 1) (local.get 2))))
    "#;

    /// A guest that imports crc but exports NO memory — exercises the missing
    /// `"memory"` export branch.
    const NO_MEMORY_GUEST_WAT: &str = r#"
        (module
          (import "extism:host/user" "host_crc32"
            (func $crc (param i64 i64 i64) (result i64)))
          (func (export "call_crc") (param i64 i64 i64) (result i64)
            (call $crc (local.get 0) (local.get 1) (local.get 2))))
    "#;

    const LEGACY_CRYPTO_GUEST_WAT: &str = r#"
        (module
          (import "extism:host/user" "scryer_aes_cbc_decrypt"
            (func (param i64 i64 i64 i64 i64) (result i64)))
          (import "extism:host/user" "scryer_crc32"
            (func (param i64 i64 i64) (result i64)))
          (memory (export "memory") 1))
    "#;

    fn module_from_wat(engine: &Engine, wat: &str, context: &str) -> Module {
        let wasm = wat::parse_str(wat).unwrap_or_else(|error| panic!("{context}: {error}"));
        Module::new(engine, wasm).unwrap_or_else(|error| panic!("{context}: {error}"))
    }

    fn crypto_guest() -> (Store<()>, wasmtime::Instance) {
        let engine = Engine::default();
        let module = module_from_wat(&engine, CRYPTO_GUEST_WAT, "compile crypto guest");
        let mut linker: Linker<()> = Linker::new(&engine);
        add_to_linker(&mut linker).expect("register crypto host fns");
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate(&mut store, &module)
            .expect("instantiate crypto guest");
        (store, instance)
    }

    #[test]
    fn legacy_crypto_import_aliases_still_instantiate() {
        let engine = Engine::default();
        let module = module_from_wat(&engine, LEGACY_CRYPTO_GUEST_WAT, "compile legacy guest");
        let mut linker: Linker<()> = Linker::new(&engine);
        add_to_linker(&mut linker).expect("register crypto host fns");
        let mut store = Store::new(&engine, ());

        linker
            .instantiate(&mut store, &module)
            .expect("legacy archive plugin imports remain compatible");
    }

    fn memory_of(store: &mut Store<()>, instance: &wasmtime::Instance) -> wasmtime::Memory {
        instance
            .get_memory(&mut *store, "memory")
            .expect("guest exports memory")
    }

    #[test]
    fn host_aes_round_trips_through_guest_memory() {
        let (mut store, instance) = crypto_guest();
        let memory = memory_of(&mut store, &instance);

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

        // Layout in the guest's linear memory.
        let (key_ptr, iv_ptr, buf_ptr) = (0usize, 64usize, 128usize);
        memory.write(&mut store, key_ptr, &key).unwrap();
        memory.write(&mut store, iv_ptr, &iv).unwrap();
        memory.write(&mut store, buf_ptr, &ciphertext).unwrap();

        let call_aes = instance
            .get_typed_func::<(i64, i64, i64, i64, i64), i64>(&mut store, "call_aes")
            .unwrap();
        let rc = call_aes
            .call(
                &mut store,
                (
                    key_ptr as i64,
                    key.len() as i64,
                    iv_ptr as i64,
                    buf_ptr as i64,
                    ciphertext.len() as i64,
                ),
            )
            .unwrap();
        assert_eq!(rc, AES_STATUS_OK);

        let mut got = vec![0u8; expected.len()];
        memory.read(&store, buf_ptr, &mut got).unwrap();
        assert_eq!(got, expected, "in-place decrypt must match the NIST vector");
    }

    #[test]
    fn host_crc_matches_and_chains_through_guest_memory() {
        let (mut store, instance) = crypto_guest();
        let memory = memory_of(&mut store, &instance);
        let call_crc = instance
            .get_typed_func::<(i64, i64, i64), i64>(&mut store, "call_crc")
            .unwrap();

        let a = b"archive ";
        let b = b"payload";
        let a_ptr = 0usize;
        let b_ptr = 32usize;
        memory.write(&mut store, a_ptr, a).unwrap();
        memory.write(&mut store, b_ptr, b).unwrap();

        // Known IEEE check value for "123456789".
        let check = b"123456789";
        let check_ptr = 64usize;
        memory.write(&mut store, check_ptr, check).unwrap();
        let crc_check = call_crc
            .call(&mut store, (0, check_ptr as i64, check.len() as i64))
            .unwrap();
        assert_eq!(crc_check, 0xcbf4_3926);

        // Chaining: crc(crc(0, a), b) == crc(0, a ++ b).
        let crc_a = call_crc
            .call(&mut store, (0, a_ptr as i64, a.len() as i64))
            .unwrap();
        let chained = call_crc
            .call(&mut store, (crc_a, b_ptr as i64, b.len() as i64))
            .unwrap();
        let combined = crc32(0, b"archive payload");
        assert_eq!(chained, i64::from(combined));

        // Empty update returns the seed unchanged.
        let seed = 0x1234_5678i64;
        let empty = call_crc.call(&mut store, (seed, 0, 0)).unwrap();
        assert_eq!(empty, seed);
    }

    #[test]
    fn host_rejects_out_of_bounds_and_overflow() {
        let (mut store, instance) = crypto_guest();
        let _ = memory_of(&mut store, &instance);
        let call_aes = instance
            .get_typed_func::<(i64, i64, i64, i64, i64), i64>(&mut store, "call_aes")
            .unwrap();
        let call_crc = instance
            .get_typed_func::<(i64, i64, i64), i64>(&mut store, "call_crc")
            .unwrap();

        // Buffer starts inside memory but runs off the end -> OOB.
        let one_page = 65_536i64;
        let rc = call_aes
            .call(&mut store, (0, 16, 64, one_page - 16, 32))
            .unwrap();
        assert_eq!(rc, AES_STATUS_OUT_OF_BOUNDS);

        // Negative pointer reinterpreted as a huge usize -> overflow-rejected.
        let rc = call_aes.call(&mut store, (0, 16, 64, -16, 16)).unwrap();
        assert_eq!(rc, AES_STATUS_OUT_OF_BOUNDS);

        // Bad key length / block alignment precede the bounds check.
        assert_eq!(
            call_aes.call(&mut store, (0, 15, 64, 128, 16)).unwrap(),
            AES_STATUS_BAD_KEY_LEN
        );
        assert_eq!(
            call_aes.call(&mut store, (0, 16, 64, 128, 20)).unwrap(),
            AES_STATUS_BAD_BLOCK_LEN
        );

        // CRC past the end of memory -> OOB.
        let rc = call_crc.call(&mut store, (0, one_page - 4, 32)).unwrap();
        assert_eq!(rc, CRC_STATUS_OUT_OF_BOUNDS);
        // Huge length via negative i64 -> overflow-rejected.
        let rc = call_crc.call(&mut store, (0, 0, -1)).unwrap();
        assert_eq!(rc, CRC_STATUS_OUT_OF_BOUNDS);
    }

    #[test]
    fn host_reports_missing_memory_export() {
        let engine = Engine::default();
        let module = module_from_wat(&engine, NO_MEMORY_GUEST_WAT, "compile no-memory guest");
        let mut linker: Linker<()> = Linker::new(&engine);
        add_to_linker(&mut linker).unwrap();
        let mut store = Store::new(&engine, ());
        let instance = linker.instantiate(&mut store, &module).unwrap();
        let call_crc = instance
            .get_typed_func::<(i64, i64, i64), i64>(&mut store, "call_crc")
            .unwrap();
        // Non-empty buffer so the memory export is actually consulted.
        let rc = call_crc.call(&mut store, (0, 0, 4)).unwrap();
        assert_eq!(rc, CRC_STATUS_OUT_OF_BOUNDS);
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
