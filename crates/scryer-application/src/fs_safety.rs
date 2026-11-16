use std::io;
use std::path::{Path, PathBuf};

use crate::disk::{Disk, EntryKind, Link, LinkOutcome, Removal, Remove};
use crate::{AppError, AppResult};

pub(crate) fn most_specific_containing_root(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|root| crate::recycle_bin::path_is_under_configured_root(path, root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

pub(crate) fn resolve_available_root_for_path(path: &Path, roots: &[PathBuf]) -> AppResult<()> {
    let root = containing_root_for_path(path, roots)?;
    ensure_root_available(&root)?;
    Ok(())
}

/// The configured root `path` belongs to, refusing a path outside every root.
/// Pure path arithmetic: it touches no filesystem.
pub(crate) fn containing_root_for_path(path: &Path, roots: &[PathBuf]) -> AppResult<PathBuf> {
    most_specific_containing_root(path, roots).ok_or_else(|| {
        AppError::validation_refused(
            crate::delete_refusal::PATH_OUTSIDE_ROOTS,
            format!(
                "refusing filesystem operation for {} because it is outside configured media roots",
                path.display()
            ),
        )
    })
}

pub(crate) fn ensure_root_available(root: &Path) -> AppResult<()> {
    use crate::error_reasons::fs_safety as reasons;
    let disk = Disk::process();
    let reading = disk.uncached_blocking(root);
    let (code, problem) = match reading.kind() {
        Some(EntryKind::Directory) => match disk.has_entries_blocking(root) {
            Ok(true) => return Ok(()),
            Ok(false) => (reasons::CONFIGURED_MEDIA_ROOT_EMPTY, "is empty".to_string()),
            Err(error) => (
                reasons::CONFIGURED_MEDIA_ROOT_UNREADABLE,
                format!("is unreadable: {}", io::Error::from(error)),
            ),
        },
        Some(EntryKind::Symlink) => (
            reasons::CONFIGURED_MEDIA_ROOT_SYMLINK,
            "is a symlink".into(),
        ),
        Some(_) => (
            reasons::CONFIGURED_MEDIA_ROOT_NOT_DIRECTORY,
            "is not a directory".into(),
        ),
        None => (
            reasons::CONFIGURED_MEDIA_ROOT_UNAVAILABLE,
            format!("is unavailable: {}", reading.describe_absence()),
        ),
    };
    Err(AppError::validation_refused(
        code,
        format!("configured media root {} {problem}", root.display()),
    ))
}

// The removals below clear a Windows read-only attribute that refuses them.

pub(crate) async fn remove_file_safely(path: &Path) -> AppResult<()> {
    remove_safely(path, Remove::FileEvenReadOnly, false).await
}

pub(crate) async fn remove_file_safely_if_exists(path: &Path) -> AppResult<()> {
    remove_safely(path, Remove::FileEvenReadOnly, true).await
}

pub(crate) async fn remove_dir_safely(path: &Path) -> AppResult<()> {
    remove_safely(path, Remove::EmptyDirEvenReadOnly, false).await
}

pub(crate) async fn remove_dir_all_safely(path: &Path) -> AppResult<()> {
    remove_safely(path, Remove::TreeEvenReadOnly, false).await
}

pub(crate) async fn remove_dir_all_safely_if_exists(path: &Path) -> AppResult<()> {
    remove_safely(path, Remove::TreeEvenReadOnly, true).await
}

async fn remove_safely(path: &Path, what: Remove, if_exists: bool) -> AppResult<()> {
    let noun = match what {
        Remove::EmptyDirEvenReadOnly => "directory",
        Remove::TreeEvenReadOnly => "directory tree",
        _ => "file",
    };
    let error = match Disk::process().remove(path, what).await {
        Ok(Removal::Removed) => return Ok(()),
        Ok(Removal::AlreadyAbsent) if if_exists => return Ok(()),
        Ok(Removal::AlreadyAbsent) => io::Error::from(io::ErrorKind::NotFound),
        Err(error) => io::Error::from(error),
    };
    Err(AppError::Repository(format!(
        "failed to remove {noun} {}: {error}",
        path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_availability_requires_content_for_user_disk_delete_policy() {
        let tempdir = tempfile::tempdir().expect("create temp dir");
        let target = tempdir.path().join("missing.mkv");
        let roots = vec![tempdir.path().to_path_buf()];

        let empty_result = resolve_available_root_for_path(&target, &roots);
        assert!(
            matches!(
                empty_result,
                Err(AppError::Validation(_)
                    | AppError::ValidationRefused { .. }
                    | AppError::ValidationRefusedWithCount { .. })
            ),
            "empty roots should fail closed for destructive user disk deletes"
        );

        std::fs::write(tempdir.path().join(".mounted"), b"mounted").expect("write mount marker");
        resolve_available_root_for_path(&target, &roots)
            .expect("non-empty roots should prove availability even when target is missing");
    }

    #[test]
    fn root_availability_rejects_empty_roots_for_housekeeping_policy() {
        let tempdir = tempfile::tempdir().expect("create temp dir");
        let target = tempdir.path().join("missing.mkv");
        let roots = vec![tempdir.path().to_path_buf()];

        let result = resolve_available_root_for_path(&target, &roots);
        assert!(
            matches!(
                result,
                Err(AppError::Validation(_)
                    | AppError::ValidationRefused { .. }
                    | AppError::ValidationRefusedWithCount { .. })
            ),
            "DB-only housekeeping must also fail closed for empty roots"
        );
    }

    #[test]
    fn root_availability_rejects_out_of_root_targets() {
        let tempdir = tempfile::tempdir().expect("create temp dir");
        let outside = tempdir
            .path()
            .parent()
            .expect("temp dir has parent")
            .join("outside.mkv");
        let roots = vec![tempdir.path().join("media")];

        let error = resolve_available_root_for_path(&outside, &roots)
            .expect_err("targets outside configured roots must fail closed");
        assert_eq!(
            error.validation_reason(),
            Some(crate::delete_refusal::PATH_OUTSIDE_ROOTS),
            "{error}"
        );
    }

    #[test]
    fn root_availability_names_why_a_root_is_unusable() {
        use crate::error_reasons::fs_safety as reasons;
        let tempdir = tempfile::tempdir().expect("create temp dir");
        let missing = tempdir.path().join("gone");
        let file = tempdir.path().join("plain-file");
        std::fs::write(&file, b"x").expect("write file");
        assert_eq!(
            ensure_root_available(&missing)
                .expect_err("missing root")
                .validation_reason(),
            Some(reasons::CONFIGURED_MEDIA_ROOT_UNAVAILABLE)
        );
        assert_eq!(
            ensure_root_available(&file)
                .expect_err("file root")
                .validation_reason(),
            Some(reasons::CONFIGURED_MEDIA_ROOT_NOT_DIRECTORY)
        );
        #[cfg(unix)]
        {
            let link = tempdir.path().join("linked-root");
            std::os::unix::fs::symlink(tempdir.path(), &link).expect("symlink");
            assert_eq!(
                ensure_root_available(&link)
                    .expect_err("linked root")
                    .validation_reason(),
                Some(reasons::CONFIGURED_MEDIA_ROOT_SYMLINK)
            );
        }
    }

    #[tokio::test]
    async fn removals_take_only_their_target_and_tolerate_absence_only_when_asked() {
        let tempdir = tempfile::tempdir().expect("create temp dir");
        let tree = tempdir.path().join("Invented Show");
        let neighbour = tempdir.path().join("Invented Neighbour");
        std::fs::create_dir_all(tree.join("Season 01")).expect("tree");
        std::fs::create_dir_all(&neighbour).expect("neighbour");
        std::fs::write(tree.join("Season 01").join("episode.mkv"), b"a").expect("file");
        std::fs::write(neighbour.join("keep.mkv"), b"b").expect("neighbour file");
        let lone = tempdir.path().join("lone.mkv");
        std::fs::write(&lone, b"c").expect("lone");
        let empty = tempdir.path().join("empty");
        std::fs::create_dir(&empty).expect("empty dir");

        remove_dir_all_safely(&tree).await.expect("tree removed");
        remove_file_safely(&lone).await.expect("file removed");
        remove_dir_safely(&empty).await.expect("empty dir removed");
        assert!(!tree.exists() && !lone.exists() && !empty.exists());
        assert_eq!(
            std::fs::read(neighbour.join("keep.mkv")).expect("neighbour kept"),
            b"b"
        );

        remove_dir_all_safely_if_exists(&tree)
            .await
            .expect("absent tree is fine when asked");
        remove_file_safely_if_exists(&lone)
            .await
            .expect("absent file is fine when asked");
        assert!(remove_dir_all_safely(&tree).await.is_err());
        assert!(remove_file_safely(&lone).await.is_err());
        assert!(remove_dir_safely(&neighbour).await.is_err());
        assert!(neighbour.join("keep.mkv").exists());
    }
}

/// Place a journaled recovery copy without the claim-and-replace fallback.
/// On volumes without exclusive rename, retain the staging link as well.
/// If neither operation is supported, preserve everything and refuse.
pub(crate) async fn promote_recovery_staged_file(staged: &Path, dest: &Path) -> io::Result<()> {
    let disk = Disk::process();
    if let Some(result) = disk.exclusive_rename(staged, dest).await {
        return result.map_err(Into::into);
    }
    match disk.link(staged, dest, Link::Hard).await? {
        LinkOutcome::Linked => Ok(()),
        LinkOutcome::NotPossible => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the volume offers neither an exclusive rename nor a hard link",
        )),
    }
}
