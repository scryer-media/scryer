//! The media file rows as the disk owner's table of whole-file hashes.
//!
//! A row stores its hash beside the mark of the file it was read of. A hash
//! stored before hashes carried a mark is judged by the row's size and the
//! modification time its source signature records; volume, change and birth
//! times, owner and link count never decide it.

use std::sync::Arc;

use chrono::Utc;

use super::{HashRow, HashStore, KnownHash, Mark, StoreFuture};
use crate::location::model::{MoveCrcAlgorithm, PersistedContentHashes};
use crate::{MediaFileRepository, StoredMediaFileHash};

pub(crate) struct MediaFileHashStore {
    media_files: Arc<dyn MediaFileRepository>,
}

impl MediaFileHashStore {
    pub(crate) fn new(media_files: Arc<dyn MediaFileRepository>) -> Self {
        Self { media_files }
    }
}

impl HashStore for MediaFileHashStore {
    fn stored<'a>(&'a self, row: &'a HashRow) -> StoreFuture<'a, Option<KnownHash>> {
        Box::pin(async move {
            match self.media_files.stored_media_file_full_hash(row).await {
                Ok(stored) => stored.as_ref().and_then(known_from_stored),
                Err(error) => {
                    tracing::warn!(error = %error, "could not read a stored full hash; the file is read instead");
                    None
                }
            }
        })
    }

    fn record<'a>(&'a self, row: &'a HashRow, hash: &'a KnownHash) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let Ok(size_bytes) = i64::try_from(hash.size_bytes) else {
                return;
            };
            let hashes = PersistedContentHashes::from_known(hash, Utc::now());
            if let Err(error) = self
                .media_files
                .record_media_file_full_hash(row, size_bytes, &hashes)
                .await
            {
                tracing::warn!(error = %error, "could not store a full hash; it is read again when next needed");
            }
        })
    }
}

/// The hash a row stores, held to the file it describes. `None` for a hash
/// whose vintage the row cannot attest, a mark this build did not write, or
/// a hash stored without a mark whose signature cannot be read back.
pub(crate) fn known_from_stored(stored: &StoredMediaFileHash) -> Option<KnownHash> {
    let hashes = &stored.hashes;
    hashes.hash_computed_at?;
    let size_bytes = u64::try_from(stored.size_bytes).ok()?;
    let mark = match hashes.mark.as_deref() {
        Some(text) => Mark::from_stored(text)?,
        None => Mark::of_length_and_modified(
            size_bytes,
            crate::file_source_signature::modified_from_signature(
                stored.source_signature_scheme.as_deref()?,
                stored.source_signature_value.as_deref()?,
            )?,
        ),
    };
    let crc = hashes
        .move_crc
        .filter(|_| hashes.crc_algorithm == Some(MoveCrcAlgorithm::Crc64Nvme));
    Some(KnownHash {
        size_bytes,
        blake3: hashes.full_blake3.clone(),
        crc,
        mark,
    })
}

impl crate::AppUseCase {
    /// Install the media file rows as the disk owner's table of whole-file
    /// hashes. Run once at startup; only the first install has an effect.
    pub fn install_disk_hash_store(&self) {
        super::Disk::process().install_hash_store(Arc::new(MediaFileHashStore::new(
            self.services.library.media_files.clone(),
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(mark: Option<String>, signature: Option<(&str, &str)>) -> StoredMediaFileHash {
        StoredMediaFileHash {
            size_bytes: 4096,
            source_signature_scheme: signature.map(|(scheme, _)| scheme.to_string()),
            source_signature_value: signature.map(|(_, value)| value.to_string()),
            hashes: PersistedContentHashes {
                full_blake3: "ab".repeat(32),
                move_crc: Some(7),
                crc_algorithm: Some(MoveCrcAlgorithm::Crc64Nvme),
                hash_computed_at: Some(Utc::now()),
                mark,
            },
        }
    }

    #[test]
    fn a_marked_row_answers_for_the_file_its_mark_describes() {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), vec![1u8; 4096]).expect("write");
        let now = Mark::of_length_and_modified(
            4096,
            std::fs::metadata(file.path())
                .expect("metadata")
                .modified()
                .expect("modified"),
        );
        let known = known_from_stored(&stored(Some(now.to_stored()), None)).expect("known");

        assert!(known.describes(&now));
        assert_eq!(known.crc, Some(7));
        let grown = Mark::of_length_and_modified(
            4097,
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(1),
        );
        assert!(!known.describes(&grown));
    }

    #[test]
    fn an_unmarked_row_is_judged_by_its_size_and_signature() {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), vec![1u8; 4096]).expect("write");
        let metadata = std::fs::metadata(file.path()).expect("metadata");
        let signature =
            crate::file_source_signature::file_source_signature_from_metadata(&metadata)
                .expect("signature");
        let modified = metadata.modified().expect("modified");

        let known = known_from_stored(&stored(
            None,
            Some((signature.scheme.as_str(), signature.value.as_str())),
        ))
        .expect("known");

        assert!(known.describes(&Mark::of_length_and_modified(4096, modified)));
        let later = modified + std::time::Duration::from_secs(1);
        assert!(!known.describes(&Mark::of_length_and_modified(4096, later)));
    }

    #[test]
    fn a_row_that_cannot_be_judged_answers_nothing() {
        let mut unattested = stored(None, Some(("unix_mtime_nsec_v1", "1:0")));
        unattested.hashes.hash_computed_at = None;
        assert!(known_from_stored(&unattested).is_none());
        assert!(known_from_stored(&stored(Some("not a mark".to_string()), None)).is_none());
        assert!(known_from_stored(&stored(None, None)).is_none());
        assert!(known_from_stored(&stored(None, Some(("another_scheme_v1", "1:0")))).is_none());
    }
}
