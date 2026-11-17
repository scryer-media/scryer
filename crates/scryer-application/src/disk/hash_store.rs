//! The media file rows as the disk owner's table of whole-file hashes.
//!
//! A row stores its hash beside the mark of the file it was read of. This
//! adapter only loads and saves: whether a stored hash still describes the
//! file is the disk owner's call.

use std::sync::Arc;

use chrono::Utc;

use super::{HashRow, HashStore, KnownHash, Mark, StoreFuture, StoredHash};
use crate::MediaFileRepository;
use crate::location::model::{MoveCrcAlgorithm, PersistedContentHashes};

pub(crate) struct MediaFileHashStore {
    media_files: Arc<dyn MediaFileRepository>,
}

impl MediaFileHashStore {
    pub(crate) fn new(media_files: Arc<dyn MediaFileRepository>) -> Self {
        Self { media_files }
    }
}

impl HashStore for MediaFileHashStore {
    fn stored<'a>(&'a self, row: &'a HashRow) -> StoreFuture<'a, Option<StoredHash>> {
        Box::pin(async move {
            match self.media_files.stored_media_file_full_hash(row).await {
                Ok(stored) => stored.as_ref().map(loaded),
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

/// The row's hash as stored. A mark this build did not write loads as none.
fn loaded(hashes: &PersistedContentHashes) -> StoredHash {
    StoredHash {
        blake3: hashes.full_blake3.clone(),
        crc: hashes
            .move_crc
            .filter(|_| hashes.crc_algorithm == Some(MoveCrcAlgorithm::Crc64Nvme)),
        mark: hashes.mark.as_deref().and_then(Mark::from_stored),
    }
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

    fn stored(mark: Option<String>) -> PersistedContentHashes {
        PersistedContentHashes {
            full_blake3: "ab".repeat(32),
            move_crc: Some(7),
            crc_algorithm: Some(MoveCrcAlgorithm::Crc64Nvme),
            hash_computed_at: Some(Utc::now()),
            mark,
        }
    }

    #[test]
    fn a_row_loads_with_the_mark_it_was_stored_with() {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), vec![1u8; 4096]).expect("write");
        let mark = Mark::of(&std::fs::metadata(file.path()).expect("metadata"));

        let loaded = loaded(&stored(Some(mark.to_stored())));
        assert_eq!(loaded.blake3, "ab".repeat(32));
        assert_eq!(loaded.crc, Some(7));
        let read_back = loaded.mark.expect("the stored mark loads");
        assert_eq!(read_back.to_stored(), mark.to_stored());
    }

    #[test]
    fn a_row_without_a_readable_mark_loads_with_none() {
        assert!(loaded(&stored(None)).mark.is_none());
        assert!(
            loaded(&stored(Some("not a mark".to_string())))
                .mark
                .is_none()
        );
    }
}
