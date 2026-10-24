//! Stable refusal and outcome reasons for title deletion.

mod recycle;
pub use recycle::RecyclePurgeOutcome;

/// Execution-only exclusion. Durable bindings govern observations after it ends.
pub(crate) struct TitleDeletionGuard {
    pub(crate) _lock: tokio::sync::OwnedMutexGuard<()>,
    pub(crate) title_id: String,
    pub(crate) active: std::sync::Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
}

impl TitleDeletionGuard {
    /// Block new imports after permission and any retirement preflight, before
    /// capturing and validating the file manifest.
    pub(crate) fn activate(&self) {
        self.active
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(self.title_id.clone());
    }
}

impl Drop for TitleDeletionGuard {
    fn drop(&mut self) {
        self.active
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.title_id);
    }
}

impl crate::AppUseCase {
    pub(crate) fn title_is_being_deleted(&self, id: &str) -> bool {
        self.runtime
            .acquisition
            .titles_in_deletion
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(id)
    }
}

/// No titles were supplied.
pub const EMPTY_REQUEST: &str = "TITLE_DELETE_EMPTY_REQUEST";
/// Another bulk title deletion is running.
pub const JOB_ALREADY_RUNNING: &str = "TITLE_DELETE_JOB_ALREADY_RUNNING";
/// The request repeats a title.
pub const DUPLICATE_TITLE: &str = "TITLE_DELETE_DUPLICATE_TITLE";
/// Disk deletion needs a preview confirmation.
pub const CONFIRMATION_REQUIRED: &str = "TITLE_DELETE_CONFIRMATION_REQUIRED";
/// The confirmed file set has changed.
pub const PREVIEW_STALE: &str = "TITLE_DELETE_PREVIEW_STALE";
/// The selection requires typed confirmation.
pub const TYPED_CONFIRMATION_REQUIRED: &str = "TITLE_DELETE_TYPED_CONFIRMATION_REQUIRED";
/// The selected folder equals a library root.
pub const FOLDER_IS_ROOT: &str = "TITLE_DELETE_FOLDER_IS_ROOT";
/// The selected folder is outside its library roots.
pub const FOLDER_OUTSIDE_ROOTS: &str = "TITLE_DELETE_FOLDER_OUTSIDE_ROOTS";
/// Another title owns an overlapping folder.
pub const FOLDER_OVERLAPS_TITLE: &str = "TITLE_DELETE_FOLDER_OVERLAPS_TITLE";
/// The title folder is a symbolic link.
pub const FOLDER_IS_SYMLINK: &str = "TITLE_DELETE_FOLDER_IS_SYMLINK";
/// The title folder is not a directory.
pub const FOLDER_NOT_DIRECTORY: &str = "TITLE_DELETE_FOLDER_NOT_DIRECTORY";
/// No recorded media verifies folder ownership.
pub const NO_VERIFIED_MEDIA: &str = "TITLE_DELETE_NO_VERIFIED_MEDIA";
/// The folder contains another title's recorded media.
pub const FOREIGN_MEDIA: &str = "TITLE_DELETE_FOREIGN_MEDIA";
/// The folder contains a mixed or organizing layout.
pub const MIXED_LAYOUT: &str = "TITLE_DELETE_MIXED_LAYOUT";
/// The library root is unavailable.
pub const ROOT_UNAVAILABLE: &str = "TITLE_DELETE_ROOT_UNAVAILABLE";
/// Another deletion holds the title.
pub const IN_PROGRESS: &str = "TITLE_DELETE_IN_PROGRESS";
/// Another operation changed the inferred folder claim.
pub const FOLDER_CLAIM_LOST: &str = "TITLE_DELETE_FOLDER_CLAIM_LOST";
/// Some recycle entries could not be purged.
pub const RECYCLE_PURGE_INCOMPLETE: &str = "TITLE_DELETE_RECYCLE_PURGE_INCOMPLETE";
/// A recycle entry is held by another operation.
pub const RECYCLE_ENTRY_BUSY: &str = "TITLE_DELETE_RECYCLE_ENTRY_BUSY";
/// Catalog cleanup did not complete.
pub const CATALOG_CLEANUP_FAILED: &str = "TITLE_DELETE_CATALOG_CLEANUP_FAILED";

/// All title deletion reasons.
pub const ALL: &[&str] = &[
    EMPTY_REQUEST,
    JOB_ALREADY_RUNNING,
    DUPLICATE_TITLE,
    CONFIRMATION_REQUIRED,
    PREVIEW_STALE,
    TYPED_CONFIRMATION_REQUIRED,
    FOLDER_IS_ROOT,
    FOLDER_OUTSIDE_ROOTS,
    FOLDER_OVERLAPS_TITLE,
    FOLDER_IS_SYMLINK,
    FOLDER_NOT_DIRECTORY,
    NO_VERIFIED_MEDIA,
    FOREIGN_MEDIA,
    MIXED_LAYOUT,
    ROOT_UNAVAILABLE,
    IN_PROGRESS,
    FOLDER_CLAIM_LOST,
    RECYCLE_PURGE_INCOMPLETE,
    RECYCLE_ENTRY_BUSY,
    CATALOG_CLEANUP_FAILED,
];

#[cfg(test)]
mod tests {
    #[test]
    fn title_delete_reasons_are_distinct_and_stable() {
        let mut seen = std::collections::HashSet::new();
        for reason in super::ALL {
            assert!(reason.starts_with("TITLE_DELETE_"));
            assert!(
                reason
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            );
            assert!(seen.insert(*reason));
        }
    }
}
