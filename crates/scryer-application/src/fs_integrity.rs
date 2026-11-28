//! Filesystem integrity helpers shared across file move/copy/delete surfaces.
//!
//! Whenever a file is copied (e.g. a cross-device move, or moving into/out of the
//! recycle bin) and the source is subsequently removed, we compare the same
//! sampled content proof used by import cleanup: file size plus a BLAKE3 digest of
//! the first and last sample windows. This is intentionally not a full-file hash.

use std::io::{Read, Seek};
use std::path::Path;

use crate::{AppError, AppResult};
use scryer_domain::ImportContentProof;

pub const IMPORT_CONTENT_PROOF_SAMPLE_BYTES: usize = crate::disk::CONTENT_SAMPLE_BYTES;
pub const IMPORT_CONTENT_PROOF_SCHEME: &str = "import_content_proof_v1";

/// Retain an import filesystem failure's native kind beside its diagnostic.
pub fn import_io_failure(error: &std::io::Error, message: String) -> AppError {
    use crate::error_reasons::import as reason;
    let reason = match error.kind() {
        std::io::ErrorKind::NotFound => reason::FILE_NOT_FOUND,
        std::io::ErrorKind::PermissionDenied => reason::PERMISSION_DENIED,
        std::io::ErrorKind::StorageFull => reason::DISK_FULL,
        _ => reason::IO_FAILED,
    };
    AppError::RepositoryFailed { reason, message }
}

/// Compute the import content proof for `path`: file size plus a BLAKE3 digest
/// over the first and last `IMPORT_CONTENT_PROOF_SAMPLE_BYTES` bytes.
pub fn import_content_proof(path: &Path) -> AppResult<ImportContentProof> {
    let opened = crate::disk::Disk::process().open_blocking(path, crate::disk::Open::read());
    let mut file = opened.map_err(std::io::Error::from).map_err(|error| {
        import_io_failure(
            &error,
            format!(
                "failed to open import content proof path: {}: {error}",
                path.display()
            ),
        )
    })?;
    let size_bytes = file
        .metadata()
        .map_err(|error| {
            import_io_failure(
                &error,
                format!(
                    "failed to stat import content proof path: {}: {error}",
                    path.display()
                ),
            )
        })?
        .len();

    sampled_content_proof_from_reader(&mut file, &path.display().to_string(), size_bytes)
}

/// Async wrapper for sampled proof computation on the blocking pool.
pub async fn import_content_proof_async(path: &Path) -> AppResult<ImportContentProof> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || import_content_proof(&path))
        .await
        .map_err(|error| {
            AppError::Repository(format!("import content proof task failed to join: {error}"))
        })?
}

/// Compute the sampled proof from an already-open seekable reader. This is used
/// by both import verification and media-file scan signatures so they share the
/// same first/last-window semantics.
pub fn sampled_content_proof_from_reader<R: Read + Seek>(
    reader: &mut R,
    label: &str,
    size_bytes: u64,
) -> AppResult<ImportContentProof> {
    let sample = crate::disk::sample_reader(reader, size_bytes).map_err(|(step, error)| {
        let step = match step {
            crate::disk::SampleStep::Seek => "seek",
            crate::disk::SampleStep::Read => "read",
        };
        import_io_failure(
            &error,
            format!("failed to {step} import content proof path: {label}: {error}"),
        )
    })?;
    Ok(ImportContentProof {
        size_bytes: sample.size_bytes,
        sample_bytes: sample.sample_bytes,
        sample_blake3: sample.sample_blake3,
    })
}

pub fn verify_content_proofs(
    source_label: &str,
    source: &ImportContentProof,
    dest_label: &str,
    dest: &ImportContentProof,
) -> AppResult<()> {
    if source != dest {
        return Err(AppError::Repository(format!(
            "copy verification failed: {source_label} (size={} sample_bytes={} sample_blake3={}) is not identical to {dest_label} (size={} sample_bytes={} sample_blake3={})",
            source.size_bytes,
            source.sample_bytes,
            source.sample_blake3,
            dest.size_bytes,
            dest.sample_bytes,
            dest.sample_blake3
        )));
    }
    Ok(())
}

/// Verify that `dest` has the same sampled import content proof as `source`.
pub fn verify_same_file(source: &Path, dest: &Path) -> AppResult<()> {
    let source_proof = import_content_proof(source)?;
    let dest_proof = import_content_proof(dest)?;
    verify_content_proofs(
        &source.display().to_string(),
        &source_proof,
        &dest.display().to_string(),
        &dest_proof,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, SeekFrom};

    struct RecordingReader {
        bytes: Vec<u8>,
        position: u64,
        seeks: Vec<u64>,
        reads: Vec<usize>,
    }

    impl RecordingReader {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                position: 0,
                seeks: Vec::new(),
                reads: Vec::new(),
            }
        }
    }

    impl Read for RecordingReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let start = usize::try_from(self.position).unwrap_or(usize::MAX);
            if start >= self.bytes.len() {
                return Ok(0);
            }
            let len = buf.len().min(self.bytes.len() - start);
            buf[..len].copy_from_slice(&self.bytes[start..start + len]);
            self.position = self.position.saturating_add(len as u64);
            self.reads.push(len);
            Ok(len)
        }
    }

    impl Seek for RecordingReader {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            let next = match pos {
                SeekFrom::Start(offset) => offset,
                SeekFrom::End(offset) => {
                    let base = i128::try_from(self.bytes.len()).unwrap_or(i128::MAX);
                    u64::try_from(base + i128::from(offset))
                        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"))?
                }
                SeekFrom::Current(offset) => {
                    let base = i128::from(self.position);
                    u64::try_from(base + i128::from(offset))
                        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"))?
                }
            };
            self.position = next;
            self.seeks.push(next);
            Ok(next)
        }
    }

    #[test]
    fn small_file_uses_one_non_overlapping_window() {
        let bytes = b"quick identity";
        let mut reader = RecordingReader::new(bytes.to_vec());
        let proof =
            sampled_content_proof_from_reader(&mut reader, "small", bytes.len() as u64).unwrap();
        assert_eq!(proof.size_bytes, bytes.len() as u64);
        assert_eq!(proof.sample_bytes, bytes.len() as u64);
        assert!(!reader.seeks.is_empty());
        assert_eq!(reader.reads, vec![bytes.len()]);
    }

    #[test]
    fn large_file_samples_head_and_tail_only() {
        let size = IMPORT_CONTENT_PROOF_SAMPLE_BYTES * 2 + 17;
        let mut bytes = vec![0u8; size];
        bytes[..32].fill(7);
        bytes[size - 32..].fill(9);
        let mut reader = RecordingReader::new(bytes);
        let proof = sampled_content_proof_from_reader(&mut reader, "large", size as u64).unwrap();
        assert_eq!(proof.size_bytes, size as u64);
        assert_eq!(
            proof.sample_bytes,
            (IMPORT_CONTENT_PROOF_SAMPLE_BYTES * 2) as u64
        );
        assert_eq!(reader.seeks.len(), 2);
        assert_eq!(
            reader.reads,
            vec![
                IMPORT_CONTENT_PROOF_SAMPLE_BYTES,
                IMPORT_CONTENT_PROOF_SAMPLE_BYTES
            ]
        );
    }

    #[test]
    fn identical_files_verify_ok() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, b"the quick brown fox").unwrap();
        std::fs::write(&b, b"the quick brown fox").unwrap();
        verify_same_file(&a, &b).expect("identical files should verify");
    }

    #[test]
    fn changed_tail_sample_fails() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        let size = IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 4096;
        let mut left = vec![b'a'; size];
        let mut right = left.clone();
        left[size - 1] = b'b';
        right[size - 1] = b'c';
        std::fs::write(&a, left).unwrap();
        std::fs::write(&b, right).unwrap();
        assert!(
            verify_same_file(&a, &b).is_err(),
            "same-size content with changed tail sample must fail verification"
        );
    }

    #[test]
    fn sampled_proof_seeks_to_tail_without_reading_middle() {
        let size = IMPORT_CONTENT_PROOF_SAMPLE_BYTES * 3;
        let mut reader = RecordingReader::new(vec![b'x'; size]);

        let proof = sampled_content_proof_from_reader(&mut reader, "recording", size as u64)
            .expect("sampled proof");

        assert_eq!(proof.size_bytes, size as u64);
        assert_eq!(
            proof.sample_bytes,
            (IMPORT_CONTENT_PROOF_SAMPLE_BYTES * 2) as u64
        );
        assert_eq!(
            reader.seeks,
            vec![0, (size - IMPORT_CONTENT_PROOF_SAMPLE_BYTES) as u64]
        );
        assert_eq!(
            reader.reads,
            vec![
                IMPORT_CONTENT_PROOF_SAMPLE_BYTES,
                IMPORT_CONTENT_PROOF_SAMPLE_BYTES
            ]
        );
    }

    #[test]
    fn sampled_proof_hashes_small_file_once() {
        let size = IMPORT_CONTENT_PROOF_SAMPLE_BYTES / 2;
        let mut reader = RecordingReader::new(vec![b'x'; size]);

        let proof = sampled_content_proof_from_reader(&mut reader, "recording", size as u64)
            .expect("sampled proof");

        assert_eq!(proof.size_bytes, size as u64);
        assert_eq!(proof.sample_bytes, size as u64);
        assert_eq!(reader.seeks, vec![0]);
        assert_eq!(reader.reads, vec![size]);
    }

    #[test]
    fn sampled_proof_hashes_between_one_and_two_windows_without_overlap() {
        let size = IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 123;
        let mut reader = RecordingReader::new(vec![b'x'; size]);

        let proof = sampled_content_proof_from_reader(&mut reader, "recording", size as u64)
            .expect("sampled proof");

        assert_eq!(proof.size_bytes, size as u64);
        assert_eq!(proof.sample_bytes, size as u64);
        assert_eq!(
            reader.seeks,
            vec![0, IMPORT_CONTENT_PROOF_SAMPLE_BYTES as u64]
        );
        assert_eq!(reader.reads, vec![IMPORT_CONTENT_PROOF_SAMPLE_BYTES, 123]);
    }

    #[test]
    fn same_sample_blake3_different_size_fails() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        let first_sample = vec![b'f'; IMPORT_CONTENT_PROOF_SAMPLE_BYTES];
        let last_sample = vec![b'l'; IMPORT_CONTENT_PROOF_SAMPLE_BYTES];
        let mut left = Vec::new();
        left.extend_from_slice(&first_sample);
        left.extend_from_slice(&[b'a'; 128]);
        left.extend_from_slice(&last_sample);
        let mut right = Vec::new();
        right.extend_from_slice(&first_sample);
        right.extend_from_slice(&[b'b'; 256]);
        right.extend_from_slice(&last_sample);
        std::fs::write(&a, left).unwrap();
        std::fs::write(&b, right).unwrap();

        let left_proof = import_content_proof(&a).unwrap();
        let right_proof = import_content_proof(&b).unwrap();
        assert_eq!(left_proof.sample_blake3, right_proof.sample_blake3);
        assert_eq!(left_proof.sample_bytes, right_proof.sample_bytes);
        assert_ne!(left_proof.size_bytes, right_proof.size_bytes);
        assert!(
            verify_same_file(&a, &b).is_err(),
            "size_bytes must be part of sampled proof verification"
        );
    }

    #[test]
    fn same_size_unsampled_middle_change_matches_proof() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        let size = IMPORT_CONTENT_PROOF_SAMPLE_BYTES * 3;
        let mut left = vec![b'a'; size];
        let mut right = left.clone();
        left[IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 17] = b'b';
        right[IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 17] = b'c';
        std::fs::write(&a, left).unwrap();
        std::fs::write(&b, right).unwrap();

        verify_same_file(&a, &b)
            .expect("the sampled proof intentionally ignores unsampled middle bytes");
    }

    #[test]
    fn truncated_copy_fails() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, vec![7u8; 4096]).unwrap();
        std::fs::write(&b, vec![7u8; 2048]).unwrap();
        assert!(
            verify_same_file(&a, &b).is_err(),
            "truncated copy must fail verification"
        );
    }
}
