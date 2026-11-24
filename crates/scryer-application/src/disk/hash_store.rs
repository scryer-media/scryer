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

    /// A table of one row, as the media file rows would hold it.
    struct OneRow(std::sync::Mutex<Option<PersistedContentHashes>>);

    impl HashStore for OneRow {
        fn stored<'a>(&'a self, _: &'a HashRow) -> StoreFuture<'a, Option<StoredHash>> {
            let stored = self.0.lock().unwrap().as_ref().map(loaded);
            Box::pin(async move { stored })
        }
        fn record<'a>(&'a self, _: &'a HashRow, hash: &'a KnownHash) -> StoreFuture<'a, ()> {
            *self.0.lock().unwrap() = Some(PersistedContentHashes::from_known(hash, Utc::now()));
            Box::pin(async {})
        }
    }

    /// A quick copy damaged in its middle: the head and tail match, so the
    /// quick proof passes, and the streamed hash is the source's. Stored
    /// for the destination at quick depth, it carries no mark, so a later
    /// full ask reads the destination whole and tells it apart; at full
    /// depth the destination was read back, and its mark stands.
    #[tokio::test]
    async fn a_quick_copys_hash_is_stored_without_its_mark_and_read_before_it_answers() {
        use crate::disk::{Check, CheckSide};
        use scryer_domain::{AppliedVerificationDepth, VerificationDepth};

        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("synthetic-source.mkv");
        let dest = temp.path().join("Synthetic Title.mkv");
        let bytes = vec![7u8; 3 * crate::disk::CONTENT_SAMPLE_BYTES];
        std::fs::write(&source, &bytes).expect("write source");
        let mut damaged = bytes.clone();
        damaged[bytes.len() / 2] ^= 1;
        std::fs::write(&dest, &damaged).expect("write damaged copy");
        let streamed = crate::location::verify::hash_existing_file(&source)
            .await
            .expect("source hashes");
        let row = HashRow::Id("synthetic-row".into());

        let mark = Mark::of(&std::fs::metadata(&dest).expect("dest metadata"));
        let quick = PersistedContentHashes::from_verified(
            &streamed,
            &AppliedVerificationDepth::exact(VerificationDepth::Quick),
            Some(&mark),
            Utc::now(),
        );
        assert!(quick.mark.is_none(), "nothing read the destination whole");
        let disk = crate::disk::test_support::standalone();
        disk.count_whole_reads();
        disk.install_hash_store(Arc::new(OneRow(std::sync::Mutex::new(Some(quick)))));
        assert_eq!(
            disk.check_against(
                CheckSide::new(&dest).of_row(row.clone()),
                Some(&streamed.full_blake3)
            )
            .await,
            Check::Differs
        );
        assert_eq!(disk.whole_reads_of(&dest), 1);

        let full = PersistedContentHashes::from_verified(
            &streamed,
            &AppliedVerificationDepth::exact(VerificationDepth::Full),
            Some(&mark),
            Utc::now(),
        );
        assert_eq!(full.mark, Some(mark.to_stored()));
        let fell_back = PersistedContentHashes::from_verified(
            &streamed,
            &AppliedVerificationDepth::quick_fallback(),
            Some(&mark),
            Utc::now(),
        );
        assert!(fell_back.mark.is_none());
    }
}
