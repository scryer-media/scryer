//! The single place Scryer calls the operating system for a filesystem
//! operation. [`DiskBackend`] is the seam tests replace; [`SystemBackend`] is
//! the real one on `std::fs`. Every method is one platform operation (or the
//! smallest fixed sequence that makes one safe), and blocking: the owner moves
//! it off the async runtime.
//!
//! Platform differences are settled here and reported as typed kinds: a
//! filesystem without exclusive rename answers [`io::ErrorKind::Unsupported`],
//! a file the platform could not make durable answers
//! [`SyncOutcome::Unconfirmed`]. Nothing above this layer asks which
//! filesystem a path is on.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};

use super::reading::{EntryKind, FileIdentity, PathReading};
use super::{Access, Open, OpenMode, OwnerOnly, Replace};

/// What one stat call said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RawStat {
    pub kind: EntryKind,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub identity: Option<FileIdentity>,
    pub links: Option<u64>,
}

impl RawStat {
    pub(crate) fn reading(&self, observed_at: DateTime<Utc>) -> PathReading {
        PathReading::Known {
            kind: self.kind,
            len: Some(self.len),
            modified: self.modified,
            identity: self.identity,
            links: self.links,
            observed_at,
        }
    }

    fn of(metadata: &std::fs::Metadata) -> Self {
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            EntryKind::Symlink
        } else if file_type.is_dir() {
            EntryKind::Directory
        } else if file_type.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        #[cfg(unix)]
        let (identity, links) = {
            use std::os::unix::fs::MetadataExt;
            (
                FileIdentity::from_raw(metadata.dev(), metadata.ino()),
                Some(metadata.nlink()),
            )
        };
        #[cfg(not(unix))]
        let (identity, links) = (None, None);
        Self {
            kind,
            len: metadata.len(),
            modified: metadata.modified().ok(),
            identity,
            links,
        }
    }
}

/// One directory entry as the listing gave it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawEntry {
    pub name: OsString,
    /// `None` when the entry's kind could not be read.
    pub kind: Option<EntryKind>,
    /// Present only where the listing itself carries size and times, so no
    /// extra call was made for it.
    pub stat: Option<RawStat>,
}

/// Whether a written file reached stable storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    /// The bytes are on stable storage.
    Durable,
    /// The bytes were written but the destination could not confirm they are
    /// on stable storage. Verify the file before relying on it.
    Unconfirmed,
}

/// How an exclusive write failed.
#[derive(Debug)]
pub(crate) struct WriteFailure {
    /// The file did not exist before this call and this call created it:
    /// the failure came while writing or syncing it, or later. `false` when
    /// the exclusive create itself was refused, an existing file included.
    pub created: bool,
    pub error: io::Error,
}

impl WriteFailure {
    pub(crate) fn not_created(error: io::Error) -> Self {
        Self {
            created: false,
            error,
        }
    }

    pub(crate) fn created(error: io::Error) -> Self {
        Self {
            created: true,
            error,
        }
    }
}

/// Why the backend refused an open itself, carried inside the `io::Error` it
/// returns so the reason is read from a type, never from text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Refused {
    #[error("the path is a link, and links are not followed here")]
    Link,
    #[error("the path is not a regular file")]
    NotAFile,
}

impl Refused {
    pub(crate) fn of(error: &io::Error) -> Option<Self> {
        error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Self>())
            .copied()
    }
}

impl From<Refused> for io::Error {
    fn from(refused: Refused) -> Self {
        io::Error::new(io::ErrorKind::InvalidInput, refused)
    }
}

/// The disk seam. One method per platform operation.
pub(crate) trait DiskBackend: Send + Sync + 'static {
    /// Follows links.
    fn stat(&self, path: &Path) -> io::Result<RawStat>;
    /// Does not follow links.
    fn lstat(&self, path: &Path) -> io::Result<RawStat>;
    fn list(&self, path: &Path) -> io::Result<Vec<RawEntry>>;
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    fn read_link(&self, path: &Path) -> io::Result<PathBuf>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn remove_dir(&self, path: &Path) -> io::Result<()>;
    fn remove_tree(&self, path: &Path) -> io::Result<()>;
    /// Replaces an existing destination.
    fn rename(&self, source: &Path, dest: &Path) -> io::Result<()>;
    /// Fails with `AlreadyExists` when `dest` exists, and with `Unsupported`
    /// when the filesystem has no exclusive rename.
    fn rename_no_replace(&self, source: &Path, dest: &Path) -> io::Result<()>;
    /// Reserves `dest` with an exclusive create, then renames over the
    /// reservation, so a destination that appeared is never replaced.
    fn rename_claimed(&self, source: &Path, dest: &Path) -> io::Result<()>;
    fn hard_link(&self, source: &Path, dest: &Path) -> io::Result<()>;
    fn symlink(&self, target: &Path, at: &Path) -> io::Result<()>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn create_dir(&self, path: &Path, access: Access) -> io::Result<()>;
    /// Creates `path` exclusively, writes `bytes`, and syncs it. A failure
    /// says whether this call had created the file.
    fn write_new(
        &self,
        path: &Path,
        bytes: &[u8],
        access: Access,
    ) -> Result<SyncOutcome, WriteFailure>;
    /// Syncs an existing file.
    fn sync_file(&self, path: &Path) -> io::Result<SyncOutcome>;
    /// The whole contents of a file.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    /// Opens `path` as `open` says. When links are not followed, a link
    /// fails with [`Refused::Link`] and anything but a regular file with
    /// [`Refused::NotAFile`].
    fn open(&self, path: &Path, open: Open) -> io::Result<std::fs::File>;
    /// Writes `bytes` to `path` in place, without syncing: created, or
    /// truncated and rewritten when `replace` allows it.
    fn write(&self, path: &Path, bytes: &[u8], replace: Replace, access: Access) -> io::Result<()>;
    /// Syncs a file through a handle the caller wrote with.
    fn sync_handle(&self, file: &std::fs::File) -> io::Result<SyncOutcome>;
    /// Makes the entries of a directory durable: renames and creations in it.
    fn sync_dir(&self, path: &Path) -> io::Result<SyncOutcome>;
    /// Narrows an existing path's permissions to its owner.
    fn restrict_to_owner(&self, path: &Path, what: OwnerOnly) -> io::Result<()>;
    /// Sets a file's modification time, following links.
    fn set_modified(&self, path: &Path, at: SystemTime) -> io::Result<()>;
}

/// The real filesystem.
#[derive(Debug, Default)]
pub(crate) struct SystemBackend;

impl DiskBackend for SystemBackend {
    fn stat(&self, path: &Path) -> io::Result<RawStat> {
        std::fs::metadata(path).map(|metadata| RawStat::of(&metadata))
    }

    fn lstat(&self, path: &Path) -> io::Result<RawStat> {
        std::fs::symlink_metadata(path).map(|metadata| RawStat::of(&metadata))
    }

    fn list(&self, path: &Path) -> io::Result<Vec<RawEntry>> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            // Unix listings carry the entry's kind but not its size, and
            // `DirEntry::metadata` there is one more call per entry, so it is
            // only taken where the listing already holds it.
            #[cfg(windows)]
            let stat = entry.metadata().ok().map(|metadata| RawStat::of(&metadata));
            #[cfg(not(windows))]
            let stat: Option<RawStat> = None;
            let kind = match &stat {
                Some(stat) => Some(stat.kind),
                None => entry.file_type().ok().map(|file_type| {
                    if file_type.is_symlink() {
                        EntryKind::Symlink
                    } else if file_type.is_dir() {
                        EntryKind::Directory
                    } else if file_type.is_file() {
                        EntryKind::File
                    } else {
                        EntryKind::Other
                    }
                }),
            };
            entries.push(RawEntry {
                name: entry.file_name(),
                kind,
                stat,
            });
        }
        Ok(entries)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::read_link(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        if cfg!(windows) {
            clear_readonly(path)?;
        }
        std::fs::remove_file(path)
    }

    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        if cfg!(windows) {
            clear_readonly(path)?;
        }
        std::fs::remove_dir(path)
    }

    fn remove_tree(&self, path: &Path) -> io::Result<()> {
        if cfg!(windows) {
            clear_readonly_tree(path)?;
        }
        std::fs::remove_dir_all(path)
    }

    fn rename(&self, source: &Path, dest: &Path) -> io::Result<()> {
        std::fs::rename(source, dest)
    }

    fn rename_no_replace(&self, source: &Path, dest: &Path) -> io::Result<()> {
        exclusive_rename(source, dest)
    }

    fn rename_claimed(&self, source: &Path, dest: &Path) -> io::Result<()> {
        crate::fs_safety::rename_into_claimed_destination_blocking(source, dest)
    }

    fn hard_link(&self, source: &Path, dest: &Path) -> io::Result<()> {
        std::fs::hard_link(source, dest)
    }

    fn symlink(&self, target: &Path, at: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, at)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, at)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, at);
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn create_dir(&self, path: &Path, access: Access) -> io::Result<()> {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        if access == Access::OwnerOnly {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        #[cfg(not(unix))]
        let _ = access;
        builder.create(path)
    }

    fn write_new(
        &self,
        path: &Path,
        bytes: &[u8],
        access: Access,
    ) -> Result<SyncOutcome, WriteFailure> {
        use std::io::Write;
        let mut file = self
            .open(path, Open::create_new().with_access(access))
            .map_err(WriteFailure::not_created)?;
        file.write_all(bytes).map_err(WriteFailure::created)?;
        settled(&file).map_err(WriteFailure::created)
    }

    fn sync_file(&self, path: &Path) -> io::Result<SyncOutcome> {
        // Windows syncs only a handle opened for writing; elsewhere a read
        // handle is enough and works on files the process cannot write.
        let file = std::fs::OpenOptions::new()
            .read(!cfg!(windows))
            .write(cfg!(windows))
            .open(path)?;
        settled(&file)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn open(&self, path: &Path, open: Open) -> io::Result<std::fs::File> {
        let mut options = std::fs::OpenOptions::new();
        match open.mode {
            OpenMode::Read => options.read(true),
            OpenMode::WriteExisting => options.write(true),
            OpenMode::CreateNew => options.write(true).create_new(true),
            OpenMode::Create => options.write(true).create(true).truncate(true),
            OpenMode::CreateOrKeep => options.read(true).write(true).create(true).truncate(false),
            OpenMode::Append => options.append(true).create(true),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            if open.access == Access::OwnerOnly {
                options.mode(0o600);
            }
            if !open.follow_links {
                options.custom_flags(libc::O_NOFOLLOW);
            }
        }
        #[cfg(windows)]
        if !open.follow_links {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_FLAG_OPEN_REPARSE_POINT: open a link itself, not its target.
            options.custom_flags(0x0020_0000);
        }
        let file = options.open(path).map_err(|error| {
            if !open.follow_links && is_link_loop(&error) {
                Refused::Link.into()
            } else {
                error
            }
        })?;
        if !open.follow_links {
            // The handle's own metadata: no further call on the path.
            let file_type = file.metadata()?.file_type();
            if file_type.is_symlink() {
                return Err(Refused::Link.into());
            }
            if !file_type.is_file() {
                return Err(Refused::NotAFile.into());
            }
        }
        Ok(file)
    }

    fn write(&self, path: &Path, bytes: &[u8], replace: Replace, access: Access) -> io::Result<()> {
        use std::io::Write;
        let open = match replace {
            Replace::Allowed => Open::create(),
            Replace::Never => Open::create_new(),
        };
        self.open(path, open.with_access(access))?.write_all(bytes)
    }

    fn sync_handle(&self, file: &std::fs::File) -> io::Result<SyncOutcome> {
        settled(file)
    }

    fn sync_dir(&self, path: &Path) -> io::Result<SyncOutcome> {
        #[cfg(unix)]
        {
            settled(&std::fs::File::open(path)?)
        }
        // A directory cannot be opened as a file here; its entries are not
        // made durable this way.
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(SyncOutcome::Unconfirmed)
        }
    }

    fn restrict_to_owner(&self, path: &Path, what: OwnerOnly) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = match what {
                OwnerOnly::File => 0o600,
                OwnerOnly::Directory => 0o700,
            };
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        }
        // Permissions here are inherited access lists, not owner modes.
        #[cfg(not(unix))]
        {
            let _ = (path, what);
            Ok(())
        }
    }

    fn set_modified(&self, path: &Path, at: SystemTime) -> io::Result<()> {
        // Windows changes times only through a handle opened for writing;
        // elsewhere a read handle is enough and works on files the process
        // owns but cannot write.
        std::fs::OpenOptions::new()
            .read(!cfg!(windows))
            .write(cfg!(windows))
            .open(path)?
            .set_modified(at)
    }
}

/// The platform's answer to opening a link it was told not to follow.
fn is_link_loop(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::ELOOP)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

fn settled(file: &std::fs::File) -> io::Result<SyncOutcome> {
    Ok(match crate::fs_safety::sync_written_file(file)? {
        crate::fs_safety::FileSync::Synced => SyncOutcome::Durable,
        crate::fs_safety::FileSync::Unsupported => SyncOutcome::Unconfirmed,
    })
}

/// Windows refuses to remove a file carrying the DOS read-only attribute;
/// clear it first. Links are left alone. Compiled everywhere so it is
/// checked everywhere, called only on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
fn clear_readonly(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    clear_readonly_of(path, &metadata)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn clear_readonly_of(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        // Only reached on Windows, where this clears the DOS attribute.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

/// [`clear_readonly`] for every entry under a directory that is about to be
/// removed as a whole, without following links.
#[cfg_attr(not(windows), allow(dead_code))]
fn clear_readonly_tree(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return clear_readonly_of(path, &metadata);
    }
    let mut stack = vec![(path.to_path_buf(), metadata)];
    while let Some((dir, metadata)) = stack.pop() {
        clear_readonly_of(&dir, &metadata)?;
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            // Windows listings carry the entry's attributes, so this is no
            // extra call there, and it does not follow links.
            let child_metadata = entry.metadata()?;
            if child_metadata.is_dir() && !child_metadata.file_type().is_symlink() {
                stack.push((entry.path(), child_metadata));
            } else {
                clear_readonly_of(&entry.path(), &child_metadata)?;
            }
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn exclusive_rename(source: &Path, dest: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let nul = |_| io::Error::new(io::ErrorKind::InvalidInput, "path contains an interior NUL");
    let source_c = CString::new(source.as_os_str().as_bytes()).map_err(nul)?;
    let dest_c = CString::new(dest.as_os_str().as_bytes()).map_err(nul)?;
    #[cfg(target_os = "linux")]
    // SAFETY: both paths are NUL-terminated and outlive the call. The raw
    // syscall avoids a libc `renameat2` wrapper, which musl does not export.
    let code = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source_c.as_ptr(),
            libc::AT_FDCWD,
            dest_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        ) as libc::c_int
    };
    #[cfg(target_os = "macos")]
    // SAFETY: both paths are NUL-terminated and outlive the call.
    let code = unsafe { libc::renamex_np(source_c.as_ptr(), dest_c.as_ptr(), libc::RENAME_EXCL) };
    if code == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        // An old kernel, or a filesystem without exclusive rename.
        Some(libc::ENOSYS) | Some(libc::ENOTSUP) | Some(libc::EINVAL) => {
            Err(io::ErrorKind::Unsupported.into())
        }
        _ => Err(error),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn exclusive_rename(_source: &Path, _dest: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_lstat_and_list_report_kinds_on_synthetic_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("Invented Feature (2031).mkv");
        std::fs::write(&file, b"twelve bytes").unwrap();
        std::fs::create_dir(dir.path().join("Extras")).unwrap();
        let backend = SystemBackend;

        let stat = backend.lstat(&file).unwrap();
        assert_eq!(stat.kind, EntryKind::File);
        assert_eq!(stat.len, 12);
        assert!(stat.modified.is_some());
        #[cfg(unix)]
        assert!(stat.identity.is_some());
        assert_eq!(
            backend
                .lstat(&dir.path().join("absent"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );

        let mut listed = backend.list(dir.path()).unwrap();
        listed.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "Extras");
        assert_eq!(listed[0].kind, Some(EntryKind::Directory));
        assert_eq!(listed[1].kind, Some(EntryKind::File));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_a_link_to_lstat_and_its_target_to_stat() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("invented-episode.mkv");
        std::fs::write(&file, b"x").unwrap();
        let link = dir.path().join("invented-link.mkv");
        SystemBackend.symlink(&file, &link).unwrap();
        assert_eq!(SystemBackend.lstat(&link).unwrap().kind, EntryKind::Symlink);
        assert_eq!(SystemBackend.stat(&link).unwrap().kind, EntryKind::File);
        assert_eq!(
            SystemBackend.stat(&link).unwrap().identity,
            SystemBackend.lstat(&file).unwrap().identity
        );
        assert_eq!(SystemBackend.read_link(&link).unwrap(), file);
    }

    #[test]
    fn exclusive_renames_never_replace() {
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("invented-source.bin");
        let dest = dir.path().join("invented-dest.bin");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&dest, b"keep me").unwrap();
        let refused = SystemBackend
            .rename_no_replace(&source, &dest)
            .expect_err("an existing destination is refused");
        assert!(
            matches!(
                refused.kind(),
                io::ErrorKind::AlreadyExists | io::ErrorKind::Unsupported
            ),
            "{refused:?}"
        );
        let claimed = SystemBackend
            .rename_claimed(&source, &dest)
            .expect_err("a claimed rename refuses too");
        assert_eq!(claimed.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep me");
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
    }

    #[test]
    fn write_new_refuses_an_existing_file_and_keeps_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("invented-notes.txt");
        backend_write(&path, b"first");
        let refused = SystemBackend
            .write_new(&path, b"second", Access::Default)
            .unwrap_err();
        assert_eq!(refused.error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!refused.created, "the existing file was not this call's");
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        SystemBackend
            .sync_file(&path)
            .expect("sync an existing file");
    }

    fn backend_write(path: &Path, bytes: &[u8]) {
        SystemBackend
            .write_new(path, bytes, Access::Default)
            .expect("write new file");
    }
}
