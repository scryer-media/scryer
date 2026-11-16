use std::io;
use std::path::{Path, PathBuf};

use crate::disk::{
    Disk, DiskError, EntryKind, Link, LinkOutcome, Open, Parents, Removal, Remove, SyncOutcome,
    reason,
};
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

fn needs_copy(error: &DiskError) -> bool {
    error.reason() == reason::DISK_MOVE_NEEDS_COPY
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
) -> io::Result<()> {
    let disk = Disk::process();
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        disk.create_dir(parent, Parents::Create).await?;
    }

    // On a case-insensitive volume the destination name resolves to the source
    // file itself, so this is a rename to perform, not a collision to reject.
    if source != dest && paths_are_same_file(source, dest) {
        return move_via_intermediate_name(&disk, source, dest).await;
    }

    if options.overwrite {
        return match disk.move_replacing(source, dest).await {
            Ok(_) => Ok(()),
            Err(error) if needs_copy(&error) => {
                transfer_across_devices(&disk, source, dest, options).await
            }
            Err(error) => Err(error.into()),
        };
    }

    match exclusive_rename(source, dest).await {
        Some(Ok(())) => return Ok(()),
        Some(Err(error)) if needs_copy(&error) => {
            return transfer_across_devices(&disk, source, dest, options).await;
        }
        Some(Err(error)) => return Err(error.into()),
        // The platform or filesystem has no exclusive rename; claim instead.
        None => {}
    }

    // Claiming fails closed: if the destination cannot be reserved, the move
    // does not happen. Falling back to a plain rename here would reintroduce
    // the silent replace on exactly the network mounts that reject the claim.
    //
    // The claim is taken beside the destination rather than on it, so a media
    // scanner never sees an empty file under the real name.
    let staged = crate::disk::staging_path_for(dest);
    claim(&disk, &staged).await?;
    let (outcome, held) = match disk.move_replacing(source, &staged).await {
        Ok(_) => (
            promote_staged_file(&staged, dest).await,
            StagedHolds::OnlyCopy,
        ),
        Err(error) if needs_copy(&error) => {
            return copy_through_staging(&disk, source, &staged, dest, options).await;
        }
        Err(error) => (Err(error.into()), StagedHolds::EmptyClaim),
    };
    if outcome.is_err() {
        roll_back_staged_file(&disk, source, &staged, held).await;
    }
    outcome
}

/// Moves `source` to another device through `staged`, which the caller has
/// claimed: copy into it and prove the copy, promote it onto `dest` without
/// replacing, and only then unlink the source. Nothing is under the real name
/// while the copy is unproven, and a failure before the unlink leaves the
/// source as it was.
async fn copy_through_staging(
    disk: &Disk,
    source: &Path,
    staged: &Path,
    dest: &Path,
    options: MoveOptions,
) -> io::Result<()> {
    let (error, proven) = match copy_into_destination(disk, source, staged, options).await {
        Ok(()) => match promote_staged_file(staged, dest).await {
            Ok(()) => return unlink_moved_source(disk, source, dest).await,
            Err(error) => (error, true),
        },
        Err(error) => (error, false),
    };
    roll_back_staged_file(disk, source, staged, StagedHolds::Copy { proven }).await;
    Err(error)
}

/// What the staging name holds when a move through it fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StagedHolds {
    /// Only the empty claim: nothing of the source reached it.
    EmptyClaim,
    /// A copy of a source that may still exist, `proven` once it was written
    /// and verified in full.
    Copy { proven: bool },
    /// The source itself, renamed there.
    OnlyCopy,
}

/// Undoes a failed move without destroying the file.
///
/// Once the source has been renamed or copied into the staging name, that file
/// is the only copy, so deleting it on failure loses it. Sonarr draws the same
/// line: `RollbackPartialMove` deletes the target only after confirming the
/// source still exists, and `RollbackMove` moves the file back when it does
/// not. Leaving a stray file behind is always preferable to losing one.
async fn roll_back_staged_file(disk: &Disk, source: &Path, staged: &Path, held: StagedHolds) {
    let reading = disk.uncached(staged).await;
    let held = match held {
        // Only the empty claim this move created is removed. Anything else
        // there may be the source, moved by a rename that reported failure,
        // so it is treated as the only copy.
        StagedHolds::EmptyClaim if reading.is(EntryKind::File) && reading.size() == Some(0) => {
            let _ = disk.remove(staged, Remove::File).await;
            return;
        }
        StagedHolds::EmptyClaim if reading.exists() == Some(false) => return,
        StagedHolds::EmptyClaim => StagedHolds::OnlyCopy,
        held => held,
    };
    let proven = match held {
        StagedHolds::Copy { proven } => proven,
        _ => true,
    };
    if reading.exists() != Some(true) {
        tracing::error!(
            staged = %staged.display(),
            source = %source.display(),
            state = reading.describe_absence(),
            "the staged file of a failed move does not read as present; nothing was rolled back"
        );
        return;
    }

    // The only copy goes back whatever is at the source now: the promotion
    // refuses a file that has since appeared there.
    let source_present = match held {
        StagedHolds::OnlyCopy => Some(false),
        _ => disk.uncached(source).await.exists(),
    };
    match source_present {
        // The source survived, so the staged file is a copy of it.
        Some(true) => {
            let _ = disk.remove(staged, Remove::File).await;
        }
        // The staged file is the file: put it back where it came from, never
        // over a file that has since appeared there.
        Some(false) if proven => {
            if promote_staged_file(staged, source).await.is_err() {
                tracing::error!(
                    staged = %staged.display(),
                    source = %source.display(),
                    "failed to restore a staged file after a failed move; the file is left at the staging path rather than removed"
                );
            }
        }
        Some(false) => tracing::error!(
            staged = %staged.display(),
            source = %source.display(),
            "the source is gone and the staged copy of it was never proven; the copy is left at the staging path"
        ),
        None => tracing::error!(
            staged = %staged.display(),
            source = %source.display(),
            "could not tell whether the source survived a failed move; the staged file is left in place"
        ),
    }
}

/// Moves the finished file from its staging name onto the destination.
///
/// The destination was never claimed, so this is the moment another writer
/// could have taken it; the exclusive rename is tried first for that reason.
/// The verified copier stages its own partial the same way and promotes it
/// here, so a crash mid-copy never leaves a half-written file under the
/// destination's real name ([`crate::location::verify`]).
pub(crate) async fn promote_staged_file(staged: &Path, dest: &Path) -> io::Result<()> {
    if let Some(result) = exclusive_rename(staged, dest).await {
        return result.map_err(Into::into);
    }

    // No exclusive rename here, so link the staged file into place instead:
    // `link(2)` fails when the destination exists, which is the same refusal
    // without a window between checking and writing.
    let disk = Disk::process();
    match disk.link(staged, dest, Link::Hard).await {
        Ok(LinkOutcome::Linked) => {
            // The destination now holds the file. Failing to unlink the staging
            // name leaves a stray link, not a lost file, so the move is done.
            if !matches!(
                disk.remove(staged, Remove::File).await,
                Ok(Removal::Removed)
            ) {
                tracing::warn!(
                    staged = %staged.display(),
                    dest = %dest.display(),
                    "moved the file but could not remove its staging link"
                );
            }
            return Ok(());
        }
        Err(error) if error.reason() == reason::DISK_DESTINATION_EXISTS => {
            return Err(error.into());
        }
        Ok(LinkOutcome::NotPossible) | Err(_) => {}
    }

    // The volume has neither, so claim the name and rename over the claim. The
    // empty file exists only for the length of a rename, not a copy.
    disk.move_claimed(staged, dest).await?;
    Ok(())
}

/// Place a journaled recovery copy without the claim-and-replace fallback.
/// On volumes without exclusive rename, retain the staging link as well.
/// If neither operation is supported, preserve everything and refuse.
pub(crate) async fn promote_recovery_staged_file(staged: &Path, dest: &Path) -> io::Result<()> {
    if let Some(result) = exclusive_rename(staged, dest).await {
        return result.map_err(Into::into);
    }
    match Disk::process().link(staged, dest, Link::Hard).await? {
        LinkOutcome::Linked => Ok(()),
        LinkOutcome::NotPossible => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the volume offers neither an exclusive rename nor a hard link",
        )),
    }
}

/// Claims `path` so no other writer can take it, failing if it is already held.
async fn claim(disk: &Disk, path: &Path) -> io::Result<()> {
    disk.open(path, Open::create_new()).await?;
    Ok(())
}

async fn transfer_across_devices(
    disk: &Disk,
    source: &Path,
    dest: &Path,
    options: MoveOptions,
) -> io::Result<()> {
    let reading = disk.uncached(source).await;
    let Some(kind) = reading.kind() else {
        let kind = match reading.exists() {
            Some(false) => io::ErrorKind::NotFound,
            _ => io::ErrorKind::Other,
        };
        return Err(io::Error::new(
            kind,
            format!("{}: {}", source.display(), reading.describe_absence()),
        ));
    };
    // A symlinked media file is a pointer, not the payload: copying it would
    // silently duplicate the whole file and drop the link.
    if kind == EntryKind::Symlink {
        let link_target = disk.read_link(source).await?;
        if options.overwrite {
            let _ = disk.remove(dest, Remove::File).await;
        }
        if disk.link(&link_target, dest, Link::Symbolic).await? == LinkOutcome::NotPossible {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{} cannot hold a symbolic link", dest.display()),
            ));
        }
        return unlink_moved_source(disk, source, dest).await;
    }

    if !options.overwrite {
        let staged = crate::disk::staging_path_for(dest);
        claim(disk, &staged).await?;
        return copy_through_staging(disk, source, &staged, dest, options).await;
    }
    if let Err(error) = copy_into_destination(disk, source, dest, options).await {
        let _ = disk.remove(dest, Remove::File).await;
        return Err(error);
    }
    unlink_moved_source(disk, source, dest).await
}

/// Finishes a move whose destination is in place by unlinking the source.
/// When the unlink fails, one look at the source decides: gone means the move
/// finished; still there means the new destination is taken back, so no
/// duplicate refuses the retry; unknown leaves both and reports the failure.
async fn unlink_moved_source(disk: &Disk, source: &Path, dest: &Path) -> io::Result<()> {
    let Err(error) = remove_copied_source(disk, source).await else {
        return Ok(());
    };
    match disk.uncached(source).await.exists() {
        Some(false) => Ok(()),
        Some(true) => {
            let _ = disk.remove(dest, Remove::File).await;
            Err(error)
        }
        None => Err(error),
    }
}

/// Copies into a destination the caller already holds and proves it. Never
/// widens the destination: the handle is opened for writing
/// without creating, so the claim is what decided the file may be written.
async fn copy_into_destination(
    disk: &Disk,
    source: &Path,
    dest: &Path,
    options: MoveOptions,
) -> io::Result<()> {
    let mut source_file = disk.open(source, Open::read()).await?;
    let mut dest_file = disk.open(dest, Open::write_existing()).await?;
    dest_file.set_len(0).await?;
    tokio::io::copy(&mut source_file, &mut dest_file).await?;
    let sync = disk.finish(dest, dest_file).await?;

    // Carry the source's mode. Configured permissions are applied afterwards by
    // the caller and take precedence; this only stops a move from silently
    // changing access when nothing is configured.
    let _ = disk.copy_permissions(source, dest).await;

    // A copy the filesystem could not sync is proven here even when the
    // caller said it verifies for itself: the source goes next.
    let unconfirmed = sync == SyncOutcome::Unconfirmed;
    if options.verify_cross_device || unconfirmed {
        if unconfirmed {
            tracing::debug!(
                dest = %dest.display(),
                "move destination filesystem does not support fsync; verifying the copy"
            );
        }
        verify_copy_with_retry(source, dest).await?;
    }
    Ok(())
}

/// Unlinks the source of a copy that is written, synced and proven. The copy
/// is then the file, so a source already gone is a finished move, and a
/// failure here never costs the copy. A Windows read-only attribute that
/// refuses the unlink is cleared, and restored if the unlink still fails.
async fn remove_copied_source(disk: &Disk, source: &Path) -> io::Result<()> {
    disk.remove(source, Remove::FileEvenReadOnly).await?;
    Ok(())
}

/// Verifies a copy, retrying once after a pause.
///
/// A network filesystem can report a stale size immediately after a write, so
/// failing on the first look turns a good copy into a failed move. Sonarr does
/// the same thing for the same reason, calling it out as needed for remote NAS
/// devices.
async fn verify_copy_with_retry(source: &Path, dest: &Path) -> io::Result<()> {
    let first = crate::fs_integrity::verify_same_file_async(source, dest).await;
    if first.is_ok() {
        return Ok(());
    }

    tokio::time::sleep(COPY_VERIFY_RETRY_DELAY).await;
    match crate::fs_integrity::verify_same_file_async(source, dest).await {
        Ok(()) => Ok(()),
        Err(error) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    }
}

/// `Some` when the platform answered with an exclusive rename, `None` when the
/// caller should claim the destination instead.
///
/// On Windows a refusal that is neither a collision nor a move to another
/// volume also answers `None`, so the caller takes the fallback Windows used
/// before it had this rename, rather than failing where that fallback worked.
pub(crate) async fn exclusive_rename(source: &Path, dest: &Path) -> Option<Result<(), DiskError>> {
    match Disk::process().rename_no_replace(source, dest).await {
        Ok(true) => Some(Ok(())),
        Ok(false) => None,
        Err(error)
            if cfg!(windows)
                && !matches!(
                    error.reason(),
                    reason::DISK_DESTINATION_EXISTS
                        | reason::DISK_DIRECTORY_NOT_EMPTY
                        | reason::DISK_MOVE_NEEDS_COPY
                ) =>
        {
            None
        }
        Err(error) => Some(Err(error)),
    }
}

/// Whether `dest` is available to receive `source`.
///
/// A destination that *is* the source is a case-only rename on a
/// case-insensitive volume, not a collision, so it counts as free.
pub async fn destination_is_free_for(source: &Path, dest: &Path) -> bool {
    Disk::process().uncached(dest).await.exists() != Some(true) || paths_are_same_file(source, dest)
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
async fn move_via_intermediate_name(disk: &Disk, source: &Path, dest: &Path) -> io::Result<()> {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let mut last_error = None;
    for _ in 0..10 {
        let id = crate::Id::new().0;
        let intermediate = parent.join(format!(".scryer-move-{}", &id[..8]));
        if claim(disk, &intermediate).await.is_err() {
            continue;
        }
        // The claim only reserved the name; the rename replaces it.
        match disk.move_replacing(source, &intermediate).await {
            // Neither hop may replace a file: on a case-sensitive volume the
            // destination is a different file whose name differs only in case.
            Ok(_) => {
                return match promote_staged_file(&intermediate, dest).await {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        // Put the file back rather than stranding it.
                        if promote_staged_file(&intermediate, source).await.is_err() {
                            tracing::error!(
                                intermediate = %intermediate.display(),
                                source = %source.display(),
                                "failed to restore a file after a refused case-only rename; it is left at the intermediate name"
                            );
                        }
                        Err(error)
                    }
                };
            }
            Err(error) => {
                let _ = disk.remove(&intermediate, Remove::File).await;
                last_error = Some(error.into());
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::other("could not claim an intermediate name for a case-only rename")
    }))
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
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);

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
        let disk = Disk::process();

        // Force the staging path the way a filesystem without an exclusive
        // rename would, then fail promotion because the destination is taken.
        let staged = crate::disk::staging_path_for(&dest);
        claim(&disk, &staged).await.expect("claim staging");
        tokio::fs::rename(&source, &staged)
            .await
            .expect("stage the source");
        assert!(!source.exists(), "the source has moved to the staging name");

        roll_back_staged_file(&disk, &source, &staged, StagedHolds::OnlyCopy).await;

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

        roll_back_staged_file(
            &Disk::process(),
            &source,
            &staged,
            StagedHolds::Copy { proven: false },
        )
        .await;

        assert_eq!(std::fs::read(&source).expect("source"), b"payload");
        assert!(!staged.exists(), "the partial copy is cleaned up");
    }

    /// The staged file is the only copy, so a different file that has since
    /// appeared at the source's name neither causes its removal nor is
    /// replaced by it.
    #[tokio::test]
    async fn a_rollback_never_removes_the_only_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let staged = dir.path().join(".scryer-move.partial");
        write(&source, b"newcomer");
        write(&staged, b"the-only-copy");

        roll_back_staged_file(&Disk::process(), &source, &staged, StagedHolds::OnlyCopy).await;

        assert_eq!(std::fs::read(&staged).expect("staged"), b"the-only-copy");
        assert_eq!(std::fs::read(&source).expect("source"), b"newcomer");
    }

    /// An empty claim is released, and never put under the source's name.
    #[tokio::test]
    async fn a_rollback_of_an_empty_claim_never_creates_the_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let staged = dir.path().join(".scryer-move.partial");
        claim(&Disk::process(), &staged).await.expect("claim");

        roll_back_staged_file(&Disk::process(), &source, &staged, StagedHolds::EmptyClaim).await;

        assert!(!source.exists(), "no empty file appears at the source name");
        assert!(!staged.exists(), "the claim is released");
    }

    /// A copy that was never proven is not put under the source's name even
    /// when the source is gone.
    #[tokio::test]
    async fn a_rollback_never_promotes_an_unproven_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let staged = dir.path().join(".scryer-move.partial");
        write(&staged, b"half");

        let held = StagedHolds::Copy { proven: false };
        roll_back_staged_file(&Disk::process(), &source, &staged, held).await;

        assert!(!source.exists(), "the partial copy is not promoted");
        assert_eq!(std::fs::read(&staged).expect("staged"), b"half");
    }

    /// A claim that holds data is not the empty file the move created: it may
    /// be the source, moved by a rename that reported failure. It goes back
    /// to the source's name, never over a file there, and is never removed.
    #[tokio::test]
    async fn a_claim_that_holds_data_is_put_back_not_removed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let staged = dir.path().join(".scryer-move.partial");
        write(&staged, b"the-only-copy");

        roll_back_staged_file(&Disk::process(), &source, &staged, StagedHolds::EmptyClaim).await;
        assert_eq!(std::fs::read(&source).expect("source"), b"the-only-copy");
        assert!(!staged.exists());

        write(&staged, b"another-only-copy");
        roll_back_staged_file(&Disk::process(), &source, &staged, StagedHolds::EmptyClaim).await;
        assert_eq!(std::fs::read(&source).expect("source"), b"the-only-copy");
        assert_eq!(
            std::fs::read(&staged).expect("staged"),
            b"another-only-copy"
        );
    }

    /// A cross-device copy is written under the staging name, so nothing is
    /// under the real name while it is unproven, and a destination that
    /// appears meanwhile is neither replaced nor costs the source.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cross_device_copy_never_shows_an_unproven_file_or_replaces_a_newcomer() {
        use std::io::Write;
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().expect("tempdir");
        // A pipe as the source holds the copy open until the test lets it end.
        let source = dir.path().join("source.mkv");
        let source_c = std::ffi::CString::new(source.as_os_str().as_bytes()).expect("path");
        // SAFETY: the path is NUL-terminated and outlives the call.
        assert_eq!(unsafe { libc::mkfifo(source_c.as_ptr(), 0o600) }, 0);
        let dest = dir.path().join("dest.mkv");
        let names = || {
            let mut names = std::fs::read_dir(dir.path())
                .expect("list")
                .map(|entry| {
                    entry
                        .expect("entry")
                        .file_name()
                        .into_string()
                        .expect("name")
                })
                .collect::<Vec<_>>();
            names.sort();
            names
        };

        let mover = tokio::spawn({
            let (source, dest) = (source.clone(), dest.clone());
            async move {
                transfer_across_devices(
                    &Disk::process(),
                    &source,
                    &dest,
                    MoveOptions {
                        verify_cross_device: false,
                        ..MoveOptions::default()
                    },
                )
                .await
            }
        });
        // Opening the pipe for writing returns once the copy has opened it.
        let mut writer = tokio::task::spawn_blocking({
            let source = source.clone();
            move || std::fs::OpenOptions::new().write(true).open(source)
        })
        .await
        .expect("join")
        .expect("open the pipe");
        writer.write_all(b"payload").expect("feed the copy");

        // Only the source and the staged copy: nothing under the real name.
        let mid_copy = names();
        assert_eq!(mid_copy.len(), 2, "{mid_copy:?}");
        assert!(mid_copy[0].starts_with(".scryer-") && mid_copy[0].ends_with(".partial"));
        assert_eq!(mid_copy[1], "source.mkv");
        write(&dest, b"newcomer");
        drop(writer);

        let error = mover.await.expect("join").expect_err("the newcomer wins");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&dest).expect("dest"), b"newcomer");
        // The source is untouched and the staged copy is taken back.
        assert_eq!(names(), ["dest.mkv", "source.mkv"]);
    }

    /// On a case-sensitive volume two names that differ only in case are two
    /// files, so the case-only path must refuse rather than replace. Runs only
    /// where the temporary directory is case-sensitive.
    #[tokio::test]
    async fn a_case_only_name_of_a_different_file_is_not_replaced() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("case-probe"), b"probe");
        if dir.path().join("CASE-PROBE").exists() {
            return;
        }
        std::fs::remove_file(dir.path().join("case-probe")).expect("remove probe");
        let source = dir.path().join("sample show.mkv");
        let dest = dir.path().join("SAMPLE SHOW.mkv");
        write(&source, b"source-payload");
        write(&dest, b"dest-payload");

        let error = move_file_exclusive(&source, &dest, MoveOptions::default())
            .await
            .expect_err("a different file under the case-only name must not be replaced");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);

        assert_eq!(std::fs::read(&source).expect("source"), b"source-payload");
        assert_eq!(std::fs::read(&dest).expect("dest"), b"dest-payload");
        assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 2);
    }

    /// Once the copy is written and proven it is the file: a source already
    /// gone finishes the move. A source that cannot be unlinked but is still
    /// there fails the move and takes the copy back, leaving no duplicate.
    #[tokio::test]
    async fn a_proven_copy_is_dropped_only_while_the_source_is_still_there() {
        let dir = tempfile::tempdir().expect("tempdir");
        let disk = Disk::process();
        assert!(
            remove_copied_source(&disk, &dir.path().join("already-gone.mkv"))
                .await
                .is_ok()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // SAFETY: geteuid has no preconditions.
            if unsafe { libc::geteuid() } == 0 {
                // Root unlinks anywhere, so the locked folder would refuse nothing.
                return;
            }
            let held = dir.path().join("held");
            std::fs::create_dir(&held).expect("source folder");
            let source = held.join("source.mkv");
            let dest = dir.path().join("dest.mkv");
            write(&source, b"payload");
            std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o555))
                .expect("lock the source folder");

            let outcome = transfer_across_devices(
                &disk,
                &source,
                &dest,
                MoveOptions {
                    verify_cross_device: true,
                    ..MoveOptions::default()
                },
            )
            .await;
            std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o755))
                .expect("unlock the source folder");

            assert!(outcome.is_err(), "the source could not be unlinked");
            assert_eq!(std::fs::read(&source).expect("source"), b"payload");
            assert!(!dest.exists(), "the source survives, so no duplicate stays");
        }
    }

    /// A source whose own permissions forbid writing it is unlinked after its
    /// copy is proven, as an unlink has always done on Unix, where only the
    /// folder decides.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_cross_device_move_unlinks_a_source_without_write_permission() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("dest.mkv");
        write(&source, b"payload");
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o444))
            .expect("make the source unwritable");

        transfer_across_devices(&Disk::process(), &source, &dest, MoveOptions::default())
            .await
            .expect("the move succeeds");

        assert!(!source.exists());
        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
    }

    /// On Windows a read-only source refuses the unlink until its attribute
    /// is cleared, which a cross-device move does once the copy is in place.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_windows_cross_device_move_unlinks_a_read_only_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("dest.mkv");
        write(&source, b"payload");
        let mut permissions = std::fs::metadata(&source).expect("source").permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&source, permissions).expect("mark read-only");

        transfer_across_devices(&Disk::process(), &source, &dest, MoveOptions::default())
            .await
            .expect("the move succeeds");

        assert!(!source.exists());
        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
        let mut permissions = std::fs::metadata(&dest).expect("dest").permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&dest, permissions).expect("clear read-only");
    }

    /// A source that cannot be read might still be there or might not, so the
    /// staged file is neither removed nor moved over it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_rollback_that_cannot_read_the_source_leaves_the_staged_file() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            // Root reads anywhere, so the locked folder would hide nothing.
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let held = dir.path().join("held");
        std::fs::create_dir(&held).expect("source folder");
        let source = held.join("source.mkv");
        let staged = dir.path().join(".scryer-partial.partial");
        write(&source, b"payload");
        write(&staged, b"staged-payload");
        std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o000))
            .expect("lock the source folder");

        let proven = StagedHolds::Copy { proven: true };
        roll_back_staged_file(&Disk::process(), &source, &staged, proven).await;
        std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o755))
            .expect("unlock the source folder");

        assert_eq!(std::fs::read(&staged).expect("staged"), b"staged-payload");
        assert_eq!(std::fs::read(&source).expect("source"), b"payload");
    }

    /// NTFS renames without replacing, so an occupied destination is refused
    /// in the one call and both files are left as they were.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_windows_exclusive_rename_refuses_an_occupied_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("dest.mkv");
        write(&source, b"source-payload");
        write(&dest, b"dest-payload");

        let refused = exclusive_rename(&source, &dest).await;

        let Some(Err(error)) = refused else {
            panic!("an occupied destination must be refused: {refused:?}");
        };
        assert_eq!(error.reason(), reason::DISK_DESTINATION_EXISTS);
        assert_eq!(std::fs::read(&source).expect("source"), b"source-payload");
        assert_eq!(std::fs::read(&dest).expect("dest"), b"dest-payload");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_windows_exclusive_rename_moves_to_a_free_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("dest.mkv");
        write(&source, b"payload");

        assert!(matches!(
            exclusive_rename(&source, &dest).await,
            Some(Ok(()))
        ));
        assert!(!source.exists());
        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
    }

    /// A read-only file moves by rename, so no staging link is left behind
    /// that its attribute would keep from being unlinked.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_windows_read_only_file_moves_without_a_staging_leftover() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("dest.mkv");
        write(&source, b"payload");
        let mut permissions = std::fs::metadata(&source).expect("source").permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&source, permissions).expect("mark read-only");

        move_file_exclusive(&source, &dest, MoveOptions::default())
            .await
            .expect("the move succeeds");

        assert!(!source.exists());
        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
        let names = std::fs::read_dir(dir.path())
            .expect("list")
            .map(|entry| entry.expect("entry").file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, vec![std::ffi::OsString::from("dest.mkv")]);
        let mut permissions = std::fs::metadata(&dest).expect("dest").permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&dest, permissions).expect("clear read-only");
    }

    #[test]
    fn blocking_claim_refuses_an_occupied_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let staged = dir.path().join("staged.tmp");
        let dest = dir.path().join("dest.mkv");
        write(&staged, b"staged-payload");
        write(&dest, b"dest-payload");

        let error = Disk::process()
            .move_claimed_blocking(&staged, &dest)
            .expect_err("the importer must not replace an existing destination");
        assert_eq!(error.reason(), reason::DISK_DESTINATION_EXISTS);
        assert_eq!(std::fs::read(&dest).expect("dest"), b"dest-payload");
        assert_eq!(std::fs::read(&staged).expect("staged"), b"staged-payload");
    }
}
