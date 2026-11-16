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
    let reading = disk.fresh_blocking(root);
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
            format!("is unavailable: {}", reading.absence()),
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
    let outcome = match disk.move_replacing(source, &staged).await {
        Ok(_) => promote_staged_file(&staged, dest).await,
        Err(error) if needs_copy(&error) => {
            match copy_into_destination(&disk, source, &staged, options).await {
                Ok(()) => promote_staged_file(&staged, dest).await,
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error.into()),
    };
    if outcome.is_err() {
        roll_back_staged_file(&disk, source, &staged).await;
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
async fn roll_back_staged_file(disk: &Disk, source: &Path, staged: &Path) {
    if disk.fresh(staged).await.exists() != Some(true) {
        return;
    }

    if disk.fresh(source).await.exists() == Some(true) {
        // The source survived, so the staged file is a partial copy.
        let _ = disk.remove(staged, Remove::File).await;
        return;
    }

    // The staged file is the only copy: put it back where it came from.
    if disk.move_replacing(staged, source).await.is_err() {
        tracing::error!(
            staged = %staged.display(),
            source = %source.display(),
            "failed to restore a staged file after a failed move; the file is left at the staging path rather than removed"
        );
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

/// Remove the file at `path`, a missing one being an error.
async fn remove_present_file(disk: &Disk, path: &Path) -> io::Result<()> {
    match disk.remove(path, Remove::File).await? {
        Removal::Removed => Ok(()),
        Removal::AlreadyAbsent => Err(io::ErrorKind::NotFound.into()),
    }
}

async fn transfer_across_devices(
    disk: &Disk,
    source: &Path,
    dest: &Path,
    options: MoveOptions,
) -> io::Result<()> {
    let reading = disk.fresh(source).await;
    let Some(kind) = reading.kind() else {
        let kind = match reading.exists() {
            Some(false) => io::ErrorKind::NotFound,
            _ => io::ErrorKind::Other,
        };
        return Err(io::Error::new(
            kind,
            format!("{} is {}", source.display(), reading.absence()),
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
        return remove_present_file(disk, source).await;
    }

    if !options.overwrite {
        claim(disk, dest).await?;
    }
    let result = copy_into_destination(disk, source, dest, options).await;
    if result.is_err() {
        let _ = disk.remove(dest, Remove::File).await;
    }
    result
}

/// Copies into a destination the caller already holds, proves it, then unlinks
/// the source. Never widens the destination: the handle is opened for writing
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

    remove_present_file(disk, source).await
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
/// caller should claim the destination instead. Windows callers keep their
/// own fallbacks.
pub(crate) async fn exclusive_rename(source: &Path, dest: &Path) -> Option<Result<(), DiskError>> {
    if cfg!(not(any(target_os = "linux", target_os = "macos"))) {
        return None;
    }
    match Disk::process().rename_no_replace(source, dest).await {
        Ok(true) => Some(Ok(())),
        Ok(false) => None,
        Err(error) => Some(Err(error)),
    }
}

/// Whether `dest` is available to receive `source`.
///
/// A destination that *is* the source is a case-only rename on a
/// case-insensitive volume, not a collision, so it counts as free.
pub async fn destination_is_free_for(source: &Path, dest: &Path) -> bool {
    Disk::process().fresh(dest).await.exists() != Some(true) || paths_are_same_file(source, dest)
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
            Ok(_) => {
                return match disk.move_replacing(&intermediate, dest).await {
                    Ok(_) => Ok(()),
                    Err(error) => {
                        // Put the file back rather than stranding it.
                        let _ = disk.move_replacing(&intermediate, source).await;
                        Err(error.into())
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

        roll_back_staged_file(&disk, &source, &staged).await;

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

        roll_back_staged_file(&Disk::process(), &source, &staged).await;

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

        let error = Disk::process()
            .move_claimed_blocking(&staged, &dest)
            .expect_err("the importer must not replace an existing destination");
        assert_eq!(error.reason(), reason::DISK_DESTINATION_EXISTS);
        assert_eq!(std::fs::read(&dest).expect("dest"), b"dest-payload");
        assert_eq!(std::fs::read(&staged).expect("staged"), b"staged-payload");
    }
}
