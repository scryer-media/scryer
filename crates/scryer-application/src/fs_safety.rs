use std::path::{Path, PathBuf};

use crate::{AppError, AppResult};

pub(crate) fn most_specific_containing_root(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|root| crate::recycle_bin::path_is_under_configured_root(path, root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

pub(crate) fn resolve_available_root_for_path(path: &Path, roots: &[PathBuf]) -> AppResult<()> {
    let root = most_specific_containing_root(path, roots).ok_or_else(|| {
        AppError::Validation(format!(
            "refusing filesystem operation for {} because it is outside configured media roots",
            path.display()
        ))
    })?;
    ensure_root_available(&root)?;
    Ok(())
}

pub(crate) fn ensure_root_available(root: &Path) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(root).map_err(|error| {
        AppError::Validation(format!(
            "configured media root {} is unavailable: {}",
            root.display(),
            error
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::Validation(format!(
            "configured media root {} is a symlink",
            root.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(AppError::Validation(format!(
            "configured media root {} is not a directory",
            root.display()
        )));
    }

    let mut entries = std::fs::read_dir(root).map_err(|error| {
        AppError::Validation(format!(
            "configured media root {} is unreadable: {}",
            root.display(),
            error
        ))
    })?;
    match entries.next() {
        Some(Ok(_)) => {}
        Some(Err(error)) => {
            return Err(AppError::Validation(format!(
                "configured media root {} is unreadable: {}",
                root.display(),
                error
            )));
        }
        None => {
            return Err(AppError::Validation(format!(
                "configured media root {} is empty",
                root.display()
            )));
        }
    }
    Ok(())
}

#[cfg(windows)]
async fn clear_readonly_for_remove(path: &Path) -> AppResult<()> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to inspect {} before delete: {}",
                path.display(),
                error
            )));
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        tokio::fs::set_permissions(path, permissions)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to clear read-only attribute on {}: {}",
                    path.display(),
                    error
                ))
            })?;
    }
    Ok(())
}

#[cfg(not(windows))]
async fn clear_readonly_for_remove(_path: &Path) -> AppResult<()> {
    Ok(())
}

pub(crate) async fn remove_file_safely(path: &Path) -> AppResult<()> {
    clear_readonly_for_remove(path).await?;
    tokio::fs::remove_file(path).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to remove file {}: {}",
            path.display(),
            error
        ))
    })
}

pub(crate) async fn remove_file_safely_if_exists(path: &Path) -> AppResult<()> {
    clear_readonly_for_remove(path).await?;
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to remove file {}: {}",
            path.display(),
            error
        ))),
    }
}

pub(crate) async fn remove_dir_safely(path: &Path) -> AppResult<()> {
    clear_readonly_for_remove(path).await?;
    tokio::fs::remove_dir(path).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to remove directory {}: {}",
            path.display(),
            error
        ))
    })
}

#[cfg(windows)]
async fn clear_readonly_tree_for_remove(path: &Path) -> AppResult<()> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to inspect {} before recursive delete: {}",
                path.display(),
                error
            )));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return clear_readonly_for_remove(path).await;
    }

    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        clear_readonly_for_remove(&dir).await?;
        let mut entries = tokio::fs::read_dir(&dir).await.map_err(|error| {
            AppError::Repository(format!(
                "failed to read directory {} before recursive delete: {}",
                dir.display(),
                error
            ))
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            AppError::Repository(format!(
                "failed to read directory entry in {} before recursive delete: {}",
                dir.display(),
                error
            ))
        })? {
            let child = entry.path();
            let child_metadata = tokio::fs::symlink_metadata(&child).await.map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect {} before recursive delete: {}",
                    child.display(),
                    error
                ))
            })?;
            if child_metadata.is_dir() && !child_metadata.file_type().is_symlink() {
                stack.push(child);
            } else {
                clear_readonly_for_remove(&child).await?;
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
async fn clear_readonly_tree_for_remove(_path: &Path) -> AppResult<()> {
    Ok(())
}

pub(crate) async fn remove_dir_all_safely(path: &Path) -> AppResult<()> {
    clear_readonly_tree_for_remove(path).await?;
    tokio::fs::remove_dir_all(path).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to remove directory tree {}: {}",
            path.display(),
            error
        ))
    })
}

pub(crate) async fn remove_dir_all_safely_if_exists(path: &Path) -> AppResult<()> {
    clear_readonly_tree_for_remove(path).await?;
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to remove directory tree {}: {}",
            path.display(),
            error
        ))),
    }
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
            matches!(empty_result, Err(AppError::Validation(_))),
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
            matches!(result, Err(AppError::Validation(_))),
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

        let result = resolve_available_root_for_path(&outside, &roots);
        assert!(
            matches!(result, Err(AppError::Validation(_))),
            "targets outside configured roots must fail closed"
        );
    }
}

// ── Moving files ─────────────────────────────────────────────────────────────
//
// Every path that relocates media goes through `move_file_exclusive`. Before it
// existed each caller grew its own move, and they disagreed about the one thing
// that matters: `rename(2)` replaces the destination silently, so a mover that
// does not claim the destination can destroy a file nobody asked it to touch.

/// How long to wait before re-checking a copy a network filesystem may not have
/// settled yet.
const COPY_VERIFY_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(3);

/// How a move should treat the destination and the copy it may have to make.
#[derive(Debug, Clone, Copy)]
pub struct MoveOptions {
    /// Replace an existing destination. Off by default: replacing is a
    /// deliberate act, never a fallback.
    pub overwrite: bool,
    /// Prove a cross-device copy matches the source before the source is
    /// unlinked. Only turn this off when the caller verifies for itself.
    pub verify_cross_device: bool,
}

impl Default for MoveOptions {
    fn default() -> Self {
        Self {
            overwrite: false,
            verify_cross_device: true,
        }
    }
}

/// Cross-device rename.
///
/// The bare numbers differ per platform: 18 is `EXDEV` on Unix, while 17 is
/// `ERROR_NOT_SAME_DEVICE` on Windows and `EEXIST` on Unix. Matching both
/// everywhere sent existing-destination failures down copy-and-delete paths.
pub fn is_cross_device_error(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::CrossesDevices {
        return true;
    }
    #[cfg(unix)]
    {
        matches!(error.raw_os_error(), Some(libc::EXDEV))
    }
    #[cfg(windows)]
    {
        matches!(error.raw_os_error(), Some(17))
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Moves `source` onto `dest` without ever replacing a file the caller did not
/// ask to replace.
///
/// Same-device moves use an exclusive rename where the kernel offers one and a
/// claim-then-rename otherwise. Cross-device moves claim the destination, copy
/// into the claimed handle, verify, and only then unlink the source. A symlink
/// is re-created rather than having the file it points at copied.
pub async fn move_file_exclusive(
    source: &Path,
    dest: &Path,
    options: MoveOptions,
) -> std::io::Result<()> {
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }

    // On a case-insensitive volume the destination name resolves to the source
    // file itself, so this is a rename to perform, not a collision to reject.
    if source != dest && paths_are_same_file(source, dest) {
        return move_via_intermediate_name(source, dest).await;
    }

    if options.overwrite {
        return match tokio::fs::rename(source, dest).await {
            Ok(()) => Ok(()),
            Err(error) if is_cross_device_error(&error) => {
                transfer_across_devices(source, dest, options).await
            }
            Err(error) => Err(error),
        };
    }

    match exclusive_rename(source, dest).await {
        Some(Ok(())) => return Ok(()),
        Some(Err(error)) if is_cross_device_error(&error) => {
            return transfer_across_devices(source, dest, options).await;
        }
        Some(Err(error)) => return Err(error),
        // The platform or filesystem has no exclusive rename; claim instead.
        None => {}
    }

    // Claiming fails closed: if the destination cannot be reserved, the move
    // does not happen. Falling back to a plain rename here would reintroduce
    // the silent replace on exactly the network mounts that reject the claim.
    //
    // The claim is taken beside the destination rather than on it, so a media
    // scanner never sees an empty file under the real name.
    let staged = staging_path_for(dest);
    claim_destination(&staged).await?;
    let outcome = match tokio::fs::rename(source, &staged).await {
        Ok(()) => promote_staged_file(&staged, dest, options).await,
        Err(error) if is_cross_device_error(&error) => {
            match copy_into_destination(source, &staged, options).await {
                Ok(()) => promote_staged_file(&staged, dest, options).await,
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    if outcome.is_err() {
        roll_back_staged_file(source, &staged).await;
    }
    outcome
}

/// Undoes a failed move without destroying the file.
///
/// Once the source has been renamed or copied into the staging name, that file
/// is the only copy, so deleting it on failure loses it. Sonarr draws the same
/// line: `RollbackPartialMove` deletes the target only after confirming the
/// source still exists, and `RollbackMove` moves the file back when it does
/// not. Leaving a stray file behind is always preferable to losing one.
async fn roll_back_staged_file(source: &Path, staged: &Path) {
    if tokio::fs::symlink_metadata(staged).await.is_err() {
        return;
    }

    if tokio::fs::symlink_metadata(source).await.is_ok() {
        // The source survived, so the staged file is a partial copy.
        let _ = tokio::fs::remove_file(staged).await;
        return;
    }

    // The staged file is the only copy: put it back where it came from.
    if tokio::fs::rename(staged, source).await.is_err() {
        tracing::error!(
            staged = %staged.display(),
            source = %source.display(),
            "failed to restore a staged file after a failed move; the file is left at the staging path rather than removed"
        );
    }
}

/// A sibling of `dest` that no scanner will mistake for media.
fn staging_path_for(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    // A short digest rather than the destination's own name: media filenames
    // already run long, and a staging name that grew with them could push the
    // component past what the filesystem accepts. This stays 25 bytes whatever
    // the file is called.
    let mut hasher = blake3::Hasher::new();
    hasher.update(dest.as_os_str().as_encoded_bytes());
    hasher.update(crate::Id::new().0.as_bytes());
    let digest = hasher.finalize().to_hex();
    parent.join(format!(".scryer-{}.partial", &digest[..9]))
}

/// Moves the finished file from its staging name onto the destination.
///
/// The destination was never claimed, so this is the moment another writer
/// could have taken it; the exclusive rename is tried first for that reason.
///
/// Public because the verified copier stages its own partial the same way: it
/// writes to a sibling name and promotes it here, so a crash mid-copy can never
/// leave a half-written file under the destination's real name
/// ([`crate::location::verify`]).
pub async fn promote_staged_file(
    staged: &Path,
    dest: &Path,
    options: MoveOptions,
) -> std::io::Result<()> {
    if options.overwrite {
        return tokio::fs::rename(staged, dest).await;
    }

    if let Some(result) = exclusive_rename(staged, dest).await {
        return result;
    }

    // No exclusive rename here, so link the staged file into place instead:
    // `link(2)` fails when the destination exists, which is the same refusal
    // without a window between checking and writing.
    match tokio::fs::hard_link(staged, dest).await {
        Ok(()) => {
            // The destination now holds the file. Failing to unlink the staging
            // name leaves a stray link, not a lost file, so the move is done.
            if tokio::fs::remove_file(staged).await.is_err() {
                tracing::warn!(
                    staged = %staged.display(),
                    dest = %dest.display(),
                    "moved the file but could not remove its staging link"
                );
            }
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Err(error),
        Err(_) => {}
    }

    // The volume has neither, so claim the name and rename over the claim. The
    // empty file exists only for the length of a rename, not a copy.
    claim_destination(dest).await?;
    match tokio::fs::rename(staged, dest).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = tokio::fs::remove_file(dest).await;
            Err(error)
        }
    }
}

/// Claims `dest` so no other writer can take it, failing if it is already held.
async fn claim_destination(dest: &Path) -> std::io::Result<()> {
    tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .await
        .map(|_| ())
}

async fn transfer_across_devices(
    source: &Path,
    dest: &Path,
    options: MoveOptions,
) -> std::io::Result<()> {
    // A symlinked media file is a pointer, not the payload: copying it would
    // silently duplicate the whole file and drop the link.
    if tokio::fs::symlink_metadata(source).await?.is_symlink() {
        let link_target = tokio::fs::read_link(source).await?;
        if options.overwrite {
            let _ = tokio::fs::remove_file(dest).await;
        }
        create_symlink(&link_target, dest).await?;
        tokio::fs::remove_file(source).await?;
        return Ok(());
    }

    if !options.overwrite {
        claim_destination(dest).await?;
    }
    let result = copy_into_destination(source, dest, options).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

/// Copies into a destination the caller already holds, proves it, then unlinks
/// the source. Never widens the destination: the handle is opened for writing
/// without creating, so the claim is what decided the file may be written.
async fn copy_into_destination(
    source: &Path,
    dest: &Path,
    options: MoveOptions,
) -> std::io::Result<()> {
    let mut source_file = tokio::fs::File::open(source).await?;
    let mut dest_file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(dest)
        .await?;
    tokio::io::copy(&mut source_file, &mut dest_file).await?;
    dest_file.sync_all().await?;
    drop(dest_file);

    // Carry the source's mode. Configured permissions are applied afterwards by
    // the caller and take precedence; this only stops a move from silently
    // changing access when nothing is configured.
    copy_permissions_best_effort(source, dest).await;

    if options.verify_cross_device {
        verify_copy_with_retry(source, dest).await?;
    }

    tokio::fs::remove_file(source).await
}

/// Verifies a copy, retrying once after a pause.
///
/// A network filesystem can report a stale size immediately after a write, so
/// failing on the first look turns a good copy into a failed move. Sonarr does
/// the same thing for the same reason, calling it out as needed for remote NAS
/// devices.
async fn verify_copy_with_retry(source: &Path, dest: &Path) -> std::io::Result<()> {
    let first = crate::fs_integrity::verify_same_file_async(source, dest).await;
    if first.is_ok() {
        return Ok(());
    }

    tokio::time::sleep(COPY_VERIFY_RETRY_DELAY).await;
    match crate::fs_integrity::verify_same_file_async(source, dest).await {
        Ok(()) => Ok(()),
        Err(error) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    }
}

#[cfg(unix)]
async fn copy_permissions_best_effort(source: &Path, dest: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = tokio::fs::metadata(source).await else {
        return;
    };
    let mode = metadata.permissions().mode();
    let _ = tokio::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode)).await;
}

#[cfg(not(unix))]
async fn copy_permissions_best_effort(_source: &Path, _dest: &Path) {}

#[cfg(unix)]
async fn create_symlink(link_target: &Path, at: &Path) -> std::io::Result<()> {
    tokio::fs::symlink(link_target, at).await
}

#[cfg(windows)]
async fn create_symlink(link_target: &Path, at: &Path) -> std::io::Result<()> {
    tokio::fs::symlink_file(link_target, at).await
}

#[cfg(target_os = "linux")]
fn renameat2_no_replace(source: *const libc::c_char, dest: *const libc::c_char) -> libc::c_int {
    // SAFETY: the caller provides valid, NUL-terminated paths. Calling the syscall directly
    // avoids depending on a libc `renameat2` wrapper, which musl does not export.
    unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source,
            libc::AT_FDCWD,
            dest,
            libc::RENAME_NOREPLACE,
        ) as libc::c_int
    }
}

/// `Some` when the platform answered with an exclusive rename, `None` when the
/// caller should claim the destination instead.
#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn exclusive_rename(source: &Path, dest: &Path) -> Option<std::io::Result<()>> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source_c = CString::new(source.as_os_str().as_bytes()).ok()?;
    let dest_c = CString::new(dest.as_os_str().as_bytes()).ok()?;
    let result = tokio::task::spawn_blocking(move || {
        let code = {
            #[cfg(target_os = "linux")]
            {
                renameat2_no_replace(source_c.as_ptr(), dest_c.as_ptr())
            }
            #[cfg(target_os = "macos")]
            {
                // SAFETY: both paths are NUL-terminated and outlive the call.
                unsafe { libc::renamex_np(source_c.as_ptr(), dest_c.as_ptr(), libc::RENAME_EXCL) }
            }
        };
        if code == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    })
    .await
    .ok()?;

    match result {
        Err(ref error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOSYS) | Some(libc::ENOTSUP) | Some(libc::EINVAL)
            ) =>
        {
            // Old kernel, or a filesystem without exclusive rename.
            None
        }
        other => Some(other),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn exclusive_rename(_source: &Path, _dest: &Path) -> Option<std::io::Result<()>> {
    None
}

/// Whether `dest` is available to receive `source`.
///
/// A destination that *is* the source is a case-only rename on a
/// case-insensitive volume, not a collision, so it counts as free.
pub async fn destination_is_free_for(source: &Path, dest: &Path) -> bool {
    if tokio::fs::symlink_metadata(dest).await.is_err() {
        return true;
    }
    paths_are_same_file(source, dest)
}

/// True when two paths differ only in case or Unicode form, which is how a
/// case-only rename looks on a case-insensitive volume.
///
/// Deliberately compares paths rather than device and inode numbers: CIFS
/// without `serverino`, mergerfs and rclone all synthesize inodes, and a
/// collision there would be misread as a case-only rename and overwrite the
/// destination. Sonarr compares paths for the same reason.
fn paths_are_same_file(source: &Path, dest: &Path) -> bool {
    let source_key = crate::stored_paths::path_to_stored_string(source);
    let dest_key = crate::stored_paths::path_to_stored_string(dest);
    crate::stored_paths::paths_match_ignoring_case(&source_key, &dest_key)
}

/// Renames through a uniquely named sibling so a case-only change lands even
/// where the volume treats both names as the same file.
async fn move_via_intermediate_name(source: &Path, dest: &Path) -> std::io::Result<()> {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let mut last_error = None;
    for _ in 0..10 {
        let id = crate::Id::new().0;
        let intermediate = parent.join(format!(".scryer-move-{}", &id[..8]));
        if claim_destination(&intermediate).await.is_err() {
            continue;
        }
        // The claim only reserved the name; the rename replaces it.
        match tokio::fs::rename(source, &intermediate).await {
            Ok(()) => {
                return match tokio::fs::rename(&intermediate, dest).await {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        // Put the file back rather than stranding it.
                        let _ = tokio::fs::rename(&intermediate, source).await;
                        Err(error)
                    }
                };
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&intermediate).await;
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        std::io::Error::other("could not claim an intermediate name for a case-only rename")
    }))
}

/// Blocking twin of the claim `move_file_exclusive` makes, for importers that
/// already run on a blocking thread.
///
/// Renames `staged` onto `dest` only after reserving the destination name, so a
/// file that appeared since the caller last looked is never replaced.
pub fn rename_into_claimed_destination_blocking(staged: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)?;
    match std::fs::rename(staged, dest) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Do not leave the empty reservation behind.
            let _ = std::fs::remove_file(dest);
            Err(error)
        }
    }
}

#[cfg(test)]
mod move_tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).expect("write fixture");
    }

    /// `rename(2)` replaces the destination silently. This is the guarantee that
    /// stops any mover from destroying a file it was not asked to touch.
    #[tokio::test]
    async fn refuses_to_replace_an_existing_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("dest.mkv");
        write(&source, b"source-payload");
        write(&dest, b"dest-payload");

        let error = move_file_exclusive(&source, &dest, MoveOptions::default())
            .await
            .expect_err("an occupied destination must not be replaced");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);

        assert_eq!(std::fs::read(&dest).expect("dest"), b"dest-payload");
        assert_eq!(std::fs::read(&source).expect("source"), b"source-payload");
    }

    #[tokio::test]
    async fn replaces_only_when_the_caller_asks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("dest.mkv");
        write(&source, b"source-payload");
        write(&dest, b"dest-payload");

        move_file_exclusive(
            &source,
            &dest,
            MoveOptions {
                overwrite: true,
                ..MoveOptions::default()
            },
        )
        .await
        .expect("an explicit overwrite should succeed");

        assert_eq!(std::fs::read(&dest).expect("dest"), b"source-payload");
        assert!(!source.exists());
    }

    /// On a case-insensitive volume the destination resolves to the source, so
    /// this is a rename to perform rather than a collision to reject.
    #[tokio::test]
    async fn performs_a_case_only_rename() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("one piece.mkv");
        let dest = dir.path().join("ONE PIECE.mkv");
        write(&source, b"payload");

        move_file_exclusive(&source, &dest, MoveOptions::default())
            .await
            .expect("case-only rename should succeed");

        assert_eq!(std::fs::read(&dest).expect("renamed"), b"payload");
        let names = std::fs::read_dir(dir.path())
            .expect("list")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["ONE PIECE.mkv".to_string()]);
    }

    #[tokio::test]
    async fn moves_a_file_that_has_no_destination_yet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("nested").join("dest.mkv");
        write(&source, b"payload");

        move_file_exclusive(&source, &dest, MoveOptions::default())
            .await
            .expect("a free destination should accept the move");

        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
        assert!(!source.exists());
    }

    /// 17 is `EEXIST` on Unix and `ERROR_NOT_SAME_DEVICE` on Windows. Reading it
    /// as cross-device on Unix sent existing-destination failures into
    /// copy-and-delete, which is how an import could overwrite a file.
    #[test]
    fn cross_device_detection_does_not_confuse_eexist() {
        #[cfg(unix)]
        {
            assert!(is_cross_device_error(&std::io::Error::from_raw_os_error(
                libc::EXDEV
            )));
            assert!(!is_cross_device_error(&std::io::Error::from_raw_os_error(
                libc::EEXIST
            )));
        }
        assert!(!is_cross_device_error(&std::io::Error::from_raw_os_error(
            5
        )));
    }

    /// A name written precomposed comes back decomposed from an SMB share, so
    /// the two spellings have to be recognized as one file.
    #[test]
    fn paths_differing_only_by_unicode_form_are_the_same_file() {
        let nfc = Path::new("/media/Pok\u{e9}mon/ep.mkv");
        let nfd = Path::new("/media/Poke\u{301}mon/ep.mkv");
        assert_ne!(nfc, nfd, "the two spellings differ as byte strings");
        assert!(paths_are_same_file(nfc, nfd));
    }

    #[test]
    fn paths_differing_by_more_than_form_are_not_the_same_file() {
        assert!(!paths_are_same_file(
            Path::new("/media/one.mkv"),
            Path::new("/media/two.mkv")
        ));
    }

    /// The destination name must never exist as an empty file: a media scanner
    /// that sees one records a broken item.
    #[tokio::test]
    async fn never_leaves_an_empty_file_under_the_destination_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("dest.mkv");
        write(&source, b"payload");

        move_file_exclusive(&source, &dest, MoveOptions::default())
            .await
            .expect("move should succeed");

        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
        let leftovers = std::fs::read_dir(dir.path())
            .expect("list")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.contains("scryer-move"))
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "staging files left behind: {leftovers:?}"
        );
    }

    /// A failed move must never consume the file. Sonarr's RollbackPartialMove
    /// only deletes the target after confirming the source survived, and
    /// RollbackMove puts the file back when it did not.
    #[tokio::test]
    async fn a_failed_move_leaves_the_file_where_it_started() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("dest.mkv");
        write(&source, b"irreplaceable");
        write(&dest, b"occupied");

        // Force the staging path the way a filesystem without an exclusive
        // rename would, then fail promotion because the destination is taken.
        let staged = staging_path_for(&dest);
        claim_destination(&staged).await.expect("claim staging");
        tokio::fs::rename(&source, &staged)
            .await
            .expect("stage the source");
        assert!(!source.exists(), "the source has moved to the staging name");

        roll_back_staged_file(&source, &staged).await;

        assert_eq!(
            std::fs::read(&source).expect("the source must come back"),
            b"irreplaceable"
        );
        assert_eq!(std::fs::read(&dest).expect("dest"), b"occupied");
        assert!(!staged.exists(), "the staging name is released");
    }

    /// When the source is still present the staged file is a partial, and
    /// removing it is the correct rollback.
    #[tokio::test]
    async fn a_failed_copy_removes_only_the_partial() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let staged = dir.path().join(".scryer-partial.partial");
        write(&source, b"payload");
        write(&staged, b"half");

        roll_back_staged_file(&source, &staged).await;

        assert_eq!(std::fs::read(&source).expect("source"), b"payload");
        assert!(!staged.exists(), "the partial copy is cleaned up");
    }

    #[test]
    fn blocking_claim_refuses_an_occupied_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let staged = dir.path().join("staged.tmp");
        let dest = dir.path().join("dest.mkv");
        write(&staged, b"staged-payload");
        write(&dest, b"dest-payload");

        let error = rename_into_claimed_destination_blocking(&staged, &dest)
            .expect_err("the importer must not replace an existing destination");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&dest).expect("dest"), b"dest-payload");
        assert_eq!(std::fs::read(&staged).expect("staged"), b"staged-payload");
    }
}
