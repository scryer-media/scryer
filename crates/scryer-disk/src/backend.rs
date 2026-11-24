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

#[cfg(windows)]
pub(crate) mod by_handle;
#[cfg(any(windows, test))]
pub(crate) mod win_facts;

/// The identity of the file an open handle holds, where the disk gives a
/// usable one. One read of the handle, never of a path.
pub(crate) fn identity_of_handle(file: &std::fs::File) -> Option<FileIdentity> {
    #[cfg(windows)]
    {
        by_handle::ByHandle::of(file)
            .ok()
            .and_then(|read| read.identity())
    }
    #[cfg(not(windows))]
    {
        file.metadata()
            .ok()
            .and_then(|metadata| RawStat::of(&metadata).identity)
    }
}

/// What one stat call said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RawStat {
    pub kind: EntryKind,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub identity: Option<FileIdentity>,
    pub links: Option<u64>,
    pub extra: Extra,
}

/// The rest of what the same stat call said, which only a
/// [`super::mark::Mark`] keeps. The inode, or the Windows file index, is
/// carried as given, zero included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Extra {
    /// The unix device.
    pub device: Option<u64>,
    /// The Windows volume serial, absent when it read as zero.
    pub serial: Option<u64>,
    pub inode: Option<u64>,
    /// Whether the volume's file numbers do not tell its files apart: a
    /// Windows volume that does not keep them for a file's life, or one
    /// whose index is truncated. Its number is then not a mark's.
    pub unnumbered: bool,
    /// The inode change time, as seconds and nanoseconds.
    pub changed: Option<(i64, i64)>,
    pub created: Option<SystemTime>,
    pub owner: Option<u32>,
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

    pub(crate) fn of(metadata: &std::fs::Metadata) -> Self {
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
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut extra = Extra {
            created: metadata.created().ok(),
            ..Extra::default()
        };
        #[cfg(unix)]
        let (identity, links) = {
            use std::os::unix::fs::MetadataExt;
            extra.device = Some(metadata.dev());
            extra.inode = Some(metadata.ino());
            extra.changed = Some((metadata.ctime(), metadata.ctime_nsec()));
            extra.owner = Some(metadata.uid());
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
            extra,
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

/// One entry of a visit: its name in hand, its kind read only when asked.
pub(crate) trait RawVisit: Send + Sync {
    fn name(&self) -> OsString;
    /// The listing's own kind where it carries one, else one probe that
    /// does not follow a link.
    fn kind(&self) -> io::Result<EntryKind>;
}

impl RawVisit for std::fs::DirEntry {
    fn name(&self) -> OsString {
        self.file_name()
    }

    fn kind(&self) -> io::Result<EntryKind> {
        let file_type = self.file_type()?;
        Ok(if file_type.is_symlink() {
            EntryKind::Symlink
        } else if file_type.is_dir() {
            EntryKind::Directory
        } else if file_type.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        })
    }
}

/// The entries of an open directory, read one at a time.
pub(crate) type DirEntries = Box<dyn Iterator<Item = io::Result<Box<dyn RawVisit>>> + Send>;

/// The entries of a directory opened for a listing, read one at a time.
pub(crate) type Listing = Box<dyn Iterator<Item = io::Result<RawEntry>> + Send>;

/// Whether a written file reached stable storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    /// The bytes are on stable storage.
    Durable,
    /// The bytes were written but the destination could not confirm they are
    /// on stable storage. Verify the file before relying on it.
    Unconfirmed,
}

impl SyncOutcome {
    /// For a caller that must not go on without durability: an unconfirmed
    /// sync is an [`io::ErrorKind::Unsupported`] error.
    pub fn required(self) -> io::Result<()> {
        match self {
            Self::Durable => Ok(()),
            Self::Unconfirmed => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the filesystem could not confirm the data reached stable storage",
            )),
        }
    }
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
    /// A second open of the name, made for a reparse point that is no link,
    /// held another file than the first.
    #[cfg(windows)]
    #[error("the name held another file when it was opened again")]
    Swapped,
    /// A second open of the name, made for a reparse point that is no link,
    /// cannot be shown to hold the file the first held: the volume gives no
    /// file numbers to compare.
    #[cfg(windows)]
    #[error("the file opened again cannot be confirmed as the one first opened")]
    Unconfirmed,
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

/// An exclusive rename refused with an answer a filesystem without one gives,
/// but that these paths alone can also draw: carried inside an
/// [`io::ErrorKind::Unsupported`] error, so the rename falls back without
/// concluding that the filesystem lacks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the exclusive rename was refused for these paths")]
pub(crate) struct AmbiguousRefusal;

impl AmbiguousRefusal {
    pub(crate) fn is(error: &io::Error) -> bool {
        error
            .get_ref()
            .is_some_and(|inner| inner.downcast_ref::<Self>().is_some())
    }
}

impl From<AmbiguousRefusal> for io::Error {
    fn from(refusal: AmbiguousRefusal) -> Self {
        io::Error::new(io::ErrorKind::Unsupported, refusal)
    }
}

/// A claimed rename that failed, and whether something is still at the
/// destination name it reserved: the empty reservation, which may be the
/// source moved by a rename that reported failure, or whatever the name holds
/// that is not the reservation.
#[derive(Debug)]
pub(crate) struct ClaimedRenameFailed {
    pub(crate) error: io::Error,
    pub(crate) reservation_left: bool,
}

/// A failure before anything was reserved.
impl From<io::Error> for ClaimedRenameFailed {
    fn from(error: io::Error) -> Self {
        Self {
            error,
            reservation_left: false,
        }
    }
}

/// The disk seam. One method per platform operation.
pub(crate) trait DiskBackend: Send + Sync + 'static {
    /// Follows links.
    fn stat(&self, path: &Path) -> io::Result<RawStat>;
    /// Does not follow links.
    fn lstat(&self, path: &Path) -> io::Result<RawStat>;
    /// Opens a directory for a listing; its entries are read as the
    /// iterator is driven.
    fn list(&self, path: &Path) -> io::Result<Listing>;
    /// Opens a directory. Its entries are read in listing order as the
    /// iterator is driven, an entry the listing failed on as its error.
    fn open_dir(&self, path: &Path) -> io::Result<DirEntries>;
    /// Whether a directory holds any entry, reading at most one.
    fn has_entries(&self, path: &Path) -> io::Result<bool>;
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    fn read_link(&self, path: &Path) -> io::Result<PathBuf>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    /// [`Self::remove_file`], clearing a Windows read-only attribute that
    /// refuses it.
    fn remove_file_clearing_readonly(&self, path: &Path) -> io::Result<()> {
        self.remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> io::Result<()>;
    /// [`Self::remove_dir`], clearing a Windows read-only attribute that
    /// refuses it.
    fn remove_dir_clearing_readonly(&self, path: &Path) -> io::Result<()> {
        self.remove_dir(path)
    }
    fn remove_tree(&self, path: &Path) -> io::Result<()>;
    /// [`Self::remove_tree`] after clearing every Windows read-only
    /// attribute in the tree.
    fn remove_tree_clearing_readonly(&self, path: &Path) -> io::Result<()> {
        self.remove_tree(path)
    }
    /// Replaces an existing destination of the source's kind. A folder
    /// onto a file fails as `NotADirectory`, and a file onto a folder as
    /// `IsADirectory`, on every platform.
    fn rename(&self, source: &Path, dest: &Path) -> io::Result<()>;
    /// Fails with `AlreadyExists` when `dest` exists, and with `Unsupported`
    /// when the filesystem has no exclusive rename.
    fn rename_no_replace(&self, source: &Path, dest: &Path) -> io::Result<()>;
    /// Reserves `dest` with an exclusive create, then renames over the
    /// reservation, so a destination that appeared is never replaced. The
    /// reservation is a file, so a folder is refused as [`Self::rename`]
    /// refuses it, and the reservation released.
    fn rename_claimed(&self, source: &Path, dest: &Path) -> Result<(), ClaimedRenameFailed>;
    fn hard_link(&self, source: &Path, dest: &Path) -> io::Result<()>;
    fn symlink(&self, target: &Path, at: &Path) -> io::Result<()>;
    /// Creates `path` and its missing parents, each with `access`.
    fn create_dir_all(&self, path: &Path, access: Access) -> io::Result<()>;
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
    /// The rest of the contents of a handle from [`Self::open`].
    fn read_from(&self, file: std::fs::File) -> io::Result<Vec<u8>>;
    /// Writes `bytes` to `path` in place, without syncing: created, or
    /// truncated and rewritten when `replace` allows it.
    fn write(&self, path: &Path, bytes: &[u8], replace: Replace, access: Access) -> io::Result<()>;
    /// Syncs a file through a handle the caller wrote with.
    fn sync_handle(&self, file: &std::fs::File) -> io::Result<SyncOutcome>;
    /// Makes the entries of a directory durable: renames and creations in it.
    fn sync_dir(&self, path: &Path) -> io::Result<SyncOutcome>;
    /// Narrows an existing path's permissions to its owner.
    fn restrict_to_owner(&self, path: &Path, what: OwnerOnly) -> io::Result<()>;
    /// [`Self::restrict_to_owner`] through a handle, which a link put at
    /// the path afterwards cannot redirect.
    fn restrict_handle_to_owner(&self, file: &std::fs::File, what: OwnerOnly) -> io::Result<()>;
    /// Sets a file's modification time, following links.
    fn set_modified(&self, path: &Path, at: SystemTime) -> io::Result<()>;
    /// Gives `dest` the permission bits of `source`, following links.
    fn copy_permissions(&self, source: &Path, dest: &Path) -> io::Result<()> {
        let _ = (source, dest);
        Ok(())
    }
}

/// The real filesystem.
#[derive(Debug, Default)]
pub(crate) struct SystemBackend;

/// A directory builder that creates with `access`: a private directory keeps
/// its owner's search permission.
fn dir_builder(access: Access) -> std::fs::DirBuilder {
    #[allow(unused_mut)]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match access {
            Access::Default => {}
            Access::OwnerOnly => {
                builder.mode(0o700);
            }
            Access::Like(_) => {
                if let Some(mode) = access.mode() {
                    builder.mode(mode);
                }
            }
        }
    }
    #[cfg(not(unix))]
    let _ = access;
    builder
}

impl SystemBackend {
    /// After a claimed rename failed, removes the reservation that rename
    /// made, and only that, and answers whether something is left at `dest`.
    ///
    /// A share can apply the rename and lose the reply, leaving the only copy
    /// at the name. So the empty file there goes only on a read of it, made
    /// immediately before the removal, that finds a regular empty file which
    /// is the entry the reservation created (`claim`), whatever the source
    /// reads as. Where the entry cannot be told (`claim` is none, or the
    /// volume has no numbers), it goes only while the source still reads as
    /// present. Anything else at the name is left in place.
    ///
    /// Called once no handle on the reservation is open: a share refuses to
    /// remove a file its own client holds. A removal that fails is logged
    /// and the file is left in place.
    ///
    /// Probes, on the failure alone: one read of `dest`, then the removal; one
    /// read of the source only where the entry cannot be told.
    fn release_failed_claim(
        &self,
        source: &Path,
        dest: &Path,
        claim: Option<FileIdentity>,
    ) -> bool {
        let remove = || match std::fs::remove_file(dest) {
            Ok(()) => false,
            Err(error) => {
                tracing::warn!(
                    path = %dest.display(),
                    error = %error,
                    "a rename reported failure and its reservation could not be removed; it is left in place"
                );
                true
            }
        };
        match self.lstat(dest) {
            Ok(stat) if stat.kind == EntryKind::File && stat.len == 0 => {
                match (claim, stat.identity) {
                    (Some(claim), Some(found)) if claim == found => remove(),
                    (Some(_), Some(_)) => {
                        tracing::warn!(
                            path = %dest.display(),
                            "a rename reported failure and the empty file at its reserved name is not the one it made; it is left in place"
                        );
                        true
                    }
                    _ => match self.lstat(source) {
                        Ok(_) => remove(),
                        Err(error) => {
                            tracing::error!(
                                path = %dest.display(),
                                source = %source.display(),
                                error = %error,
                                "a rename reported failure and its source does not read as present; the empty file at its reserved name is left in place"
                            );
                            true
                        }
                    },
                }
            }
            Ok(_) => {
                tracing::warn!(
                    path = %dest.display(),
                    "a rename reported failure and its reserved name is not an empty file; it is left in place"
                );
                true
            }
            // Nothing is at the name, so nothing was left there.
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                tracing::warn!(
                    path = %dest.display(),
                    error = %error,
                    "a rename reported failure and its reserved name could not be read; it is left in place"
                );
                true
            }
        }
    }
}

impl DiskBackend for SystemBackend {
    fn stat(&self, path: &Path) -> io::Result<RawStat> {
        #[cfg(windows)]
        return by_handle::stat(path, true);
        #[cfg(not(windows))]
        std::fs::metadata(path).map(|metadata| RawStat::of(&metadata))
    }

    fn lstat(&self, path: &Path) -> io::Result<RawStat> {
        #[cfg(windows)]
        return by_handle::stat(path, false);
        #[cfg(not(windows))]
        std::fs::symlink_metadata(path).map(|metadata| RawStat::of(&metadata))
    }

    fn has_entries(&self, path: &Path) -> io::Result<bool> {
        Ok(std::fs::read_dir(path)?.next().transpose()?.is_some())
    }

    fn list(&self, path: &Path) -> io::Result<Listing> {
        Ok(Box::new(std::fs::read_dir(path)?.map(|entry| {
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
                None => RawVisit::kind(&entry).ok(),
            };
            Ok(RawEntry {
                name: entry.file_name(),
                kind,
                stat,
            })
        })))
    }

    fn open_dir(&self, path: &Path) -> io::Result<DirEntries> {
        let entries = std::fs::read_dir(path)?;
        Ok(Box::new(entries.map(|entry| {
            entry.map(|entry| Box::new(entry) as Box<dyn RawVisit>)
        })))
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::read_link(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn remove_file_clearing_readonly(&self, path: &Path) -> io::Result<()> {
        let remove = |path: &Path| std::fs::remove_file(path);
        if !cfg!(windows) {
            return remove(path);
        }
        removing_readonly(path, remove, clear_readonly, set_readonly)
    }

    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir(path)
    }

    fn remove_dir_clearing_readonly(&self, path: &Path) -> io::Result<()> {
        let remove = |path: &Path| std::fs::remove_dir(path);
        if !cfg!(windows) {
            return remove(path);
        }
        removing_readonly(path, remove, clear_readonly, set_readonly)
    }

    fn remove_tree(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(path)
    }

    fn remove_tree_clearing_readonly(&self, path: &Path) -> io::Result<()> {
        if cfg!(windows) {
            clear_readonly_tree(path)?;
        }
        std::fs::remove_dir_all(path)
    }

    fn rename(&self, source: &Path, dest: &Path) -> io::Result<()> {
        #[cfg(windows)]
        refuse_another_kind(source, dest)?;
        std::fs::rename(source, dest)
    }

    fn rename_no_replace(&self, source: &Path, dest: &Path) -> io::Result<()> {
        exclusive_rename(source, dest)
    }

    fn rename_claimed(&self, source: &Path, dest: &Path) -> Result<(), ClaimedRenameFailed> {
        // The reservation's own entry, to tell it from anything a failed
        // rename left at the name, is read through its handle, and the handle
        // closes before the rename: a share refuses to replace, or to remove,
        // a file its own client still holds open. One handle read per claimed
        // rename.
        let claim = {
            let held = self.open(dest, Open::create_new())?;
            identity_of_handle(&held)
        };
        // The name holds the empty file just made, so beyond that read only
        // the source's kind is read before the rename: on Windows one more
        // handle read, links not followed; none on unix.
        #[cfg(windows)]
        let renamed = refuse_folder_onto_file(source).and_then(|()| std::fs::rename(source, dest));
        #[cfg(not(windows))]
        let renamed = std::fs::rename(source, dest);
        renamed.map_err(|error| ClaimedRenameFailed {
            error,
            reservation_left: self.release_failed_claim(source, dest, claim),
        })
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

    fn create_dir_all(&self, path: &Path, access: Access) -> io::Result<()> {
        dir_builder(access).recursive(true).create(path)
    }

    fn create_dir(&self, path: &Path, access: Access) -> io::Result<()> {
        dir_builder(access).create(path)
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
        if open.read_back {
            options.read(true);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            if let Some(mode) = open.access.mode() {
                options.mode(mode);
            }
            if !open.follow_links {
                options.custom_flags(libc::O_NOFOLLOW);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_FLAG_SEQUENTIAL_SCAN.
            let sequential = if open.sequential { 0x0800_0000 } else { 0 };
            // Open a link itself, not its target, and a folder too, so the
            // handle's own facts refuse it as one.
            let no_follow = if open.follow_links {
                0
            } else {
                NO_FOLLOW_FLAGS
            };
            if sequential | no_follow != 0 {
                options.custom_flags(sequential | no_follow);
            }
        }
        let file = options.open(path).map_err(|error| {
            if !open.follow_links && is_link_loop(&error) {
                Refused::Link.into()
            } else {
                error
            }
        })?;
        #[cfg(target_os = "linux")]
        if open.sequential {
            use std::os::fd::AsRawFd;
            // Advisory only: a filesystem that does not take it is still read.
            // SAFETY: the file owns this live descriptor; zero length covers
            // the whole file.
            let _ =
                unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_SEQUENTIAL) };
        }
        // On unix O_NOFOLLOW has already refused a link and a write open has
        // refused a folder, so only a read open, which opens a folder, needs
        // the handle's kind. Windows opens a reparse point itself, so every
        // open that does not follow links reads it.
        #[cfg(windows)]
        if !open.follow_links {
            return windows_held(file, path, open, options);
        }
        let needs_kind = !open.follow_links && open.mode == OpenMode::Read;
        if needs_kind {
            // The handle's own metadata: no further call on the path.
            let metadata = file.metadata()?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                return Err(Refused::Link.into());
            }
            if !file_type.is_file() {
                return Err(Refused::NotAFile.into());
            }
        }
        Ok(file)
    }

    fn read_from(&self, mut file: std::fs::File) -> io::Result<Vec<u8>> {
        use std::io::Read;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
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
            std::fs::set_permissions(path, owner_only(what))
        }
        // Permissions here are inherited access lists, not owner modes.
        #[cfg(not(unix))]
        {
            let _ = (path, what);
            Ok(())
        }
    }

    fn restrict_handle_to_owner(&self, file: &std::fs::File, what: OwnerOnly) -> io::Result<()> {
        #[cfg(unix)]
        {
            file.set_permissions(owner_only(what))
        }
        #[cfg(not(unix))]
        {
            let _ = (file, what);
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

    fn copy_permissions(&self, source: &Path, dest: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(source)?.permissions().mode();
            std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode))
        }
        #[cfg(not(unix))]
        {
            let _ = (source, dest);
            Ok(())
        }
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

/// macOS `sync_all` is `F_FULLFSYNC`, which network filesystems such as smbfs
/// reject, so plain `fsync` is tried next. A filesystem that rejects both has
/// no sync at all: unconfirmed, not an error.
fn settled(file: &std::fs::File) -> io::Result<SyncOutcome> {
    #[cfg(target_os = "macos")]
    {
        settle_sync(file.sync_all(), || plain_fsync(file))
    }
    #[cfg(not(target_os = "macos"))]
    {
        settle_sync(file.sync_all(), || Err(io::ErrorKind::Unsupported.into()))
    }
}

fn settle_sync(
    full_sync: io::Result<()>,
    fallback: impl FnOnce() -> io::Result<()>,
) -> io::Result<SyncOutcome> {
    match full_sync {
        Ok(()) => Ok(SyncOutcome::Durable),
        Err(error) if is_unsupported_sync_error(&error) => match fallback() {
            Ok(()) => Ok(SyncOutcome::Durable),
            Err(error) if is_unsupported_sync_error(&error) => Ok(SyncOutcome::Unconfirmed),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }
}

/// Matched on the errno as well as the kind: std reports macOS `ENOTSUP`
/// (errno 45) as `Uncategorized`, not `Unsupported`.
fn is_unsupported_sync_error(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    #[cfg(unix)]
    {
        matches!(
            error.raw_os_error(),
            Some(code) if code == libc::ENOTSUP || code == libc::EOPNOTSUPP || code == libc::ENOSYS
        )
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
fn plain_fsync(file: &std::fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    loop {
        // SAFETY: the descriptor belongs to `file`, which outlives the call.
        if unsafe { libc::fsync(file.as_raw_fd()) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// Clear the read-only attribute throughout a tree, links left alone and
/// not followed. A missing root has nothing to clear.
fn clear_readonly_tree(root: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut pending = vec![(root.to_path_buf(), metadata)];
    while let Some((path, metadata)) = pending.pop() {
        match clear_readonly_of(&path, &metadata) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
        if metadata.is_dir() {
            for entry in std::fs::read_dir(&path)? {
                let entry = entry?;
                // A directory entry's metadata does not follow links.
                pending.push((entry.path(), entry.metadata()?));
            }
        }
    }
    Ok(())
}

/// Windows refuses to remove an entry carrying the DOS read-only attribute.
/// The removal is tried first; only a refusal clears the attribute (`clear`
/// says whether there was one to clear) for one more try, and a failed
/// second try sets it again (`restore`), so a directory that is not empty
/// keeps it. Compiled everywhere so it is checked everywhere.
fn removing_readonly(
    path: &Path,
    remove: impl Fn(&Path) -> io::Result<()>,
    clear: impl FnOnce(&Path) -> io::Result<bool>,
    restore: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let refused = match remove(path) {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => error,
        other => return other,
    };
    match clear(path) {
        Ok(true) => {}
        // Gone since the refusal: the caller reads that as already absent.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(error),
        Ok(false) | Err(_) => return Err(refused),
    }
    remove(path).inspect_err(|_| {
        // Best effort: the second refusal is the caller's error.
        let _ = restore(path);
    })
}

/// Clear the read-only attribute of `path`, a link left alone; whether
/// there was one.
fn clear_readonly(path: &Path) -> io::Result<bool> {
    clear_readonly_of(path, &std::fs::symlink_metadata(path)?)
}

/// Set the read-only attribute [`clear_readonly`] cleared.
fn set_readonly(path: &Path) -> io::Result<()> {
    let mut permissions = std::fs::symlink_metadata(path)?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(path, permissions)
}

fn clear_readonly_of(path: &Path, metadata: &std::fs::Metadata) -> io::Result<bool> {
    let mut permissions = metadata.permissions();
    if metadata.file_type().is_symlink() || !permissions.readonly() {
        return Ok(false);
    }
    // Only reached on Windows, where this clears the DOS attribute.
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions)?;
    Ok(true)
}

#[cfg(unix)]
fn owner_only(what: OwnerOnly) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(match what {
        OwnerOnly::File => 0o600,
        OwnerOnly::Directory | OwnerOnly::Program => 0o700,
    })
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
        Some(libc::ENOSYS) | Some(libc::ENOTSUP) => Err(io::ErrorKind::Unsupported.into()),
        // A filesystem without the flag, or a rename these paths cannot take,
        // such as a directory into itself.
        Some(libc::EINVAL) => Err(AmbiguousRefusal.into()),
        _ => Err(error),
    }
}

#[cfg(windows)]
fn exclusive_rename(source: &Path, dest: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    // Win32 error codes a filesystem returns for an operation it lacks, and
    // a filter or a reparse point for one path alone.
    const ERROR_INVALID_FUNCTION: i32 = 1;
    const ERROR_NOT_SUPPORTED: i32 = 50;

    let source = win32_path(source)?;
    let dest = win32_path(dest)?;
    // SAFETY: both UTF-16 buffers are NUL-terminated and outlive the call.
    // No flags, as std's own no-replace rename: without
    // MOVEFILE_REPLACE_EXISTING an existing destination is refused, and
    // without MOVEFILE_COPY_ALLOWED a move to another volume fails with
    // ERROR_NOT_SAME_DEVICE instead of becoming an unverified copy.
    let moved = unsafe { MoveFileExW(source.as_ptr(), dest.as_ptr(), 0) };
    if moved != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(ERROR_INVALID_FUNCTION) | Some(ERROR_NOT_SUPPORTED) => Err(AmbiguousRefusal.into()),
        _ => Err(error),
    }
}

/// Refuses a replacing rename between two kinds, as unix refuses it. A
/// Windows rename that replaces puts a folder over a file where the volume
/// lacks POSIX rename semantics, such as FAT and many shares. Where the
/// destination exists, two handle reads, links not followed; where it does
/// not, one. Between these reads and the rename the names can change: the
/// rename then does what Windows does.
#[cfg(windows)]
fn refuse_another_kind(source: &Path, dest: &Path) -> io::Result<()> {
    let Ok(dest_is_folder) = is_folder(dest) else {
        return Ok(());
    };
    match (is_folder(source)?, dest_is_folder) {
        (true, false) => Err(io::ErrorKind::NotADirectory.into()),
        (false, true) => Err(io::ErrorKind::IsADirectory.into()),
        _ => Ok(()),
    }
}

/// [`refuse_another_kind`] where the destination is known to be a file:
/// a folder source is refused. One handle read, links not followed.
#[cfg(windows)]
fn refuse_folder_onto_file(source: &Path) -> io::Result<()> {
    match is_folder(source)? {
        true => Err(io::ErrorKind::NotADirectory.into()),
        false => Ok(()),
    }
}

/// Whether a Windows handle read of `path`, links not followed, is a folder.
#[cfg(windows)]
fn is_folder(path: &Path) -> io::Result<bool> {
    by_handle::stat(path, false).map(|read| read.kind == EntryKind::Directory)
}

/// The flags of a Windows open that does not follow links:
/// FILE_FLAG_OPEN_REPARSE_POINT opens a link or junction itself, and
/// FILE_FLAG_BACKUP_SEMANTICS opens a folder, which a file open is otherwise
/// denied, so the handle's facts say what the name is.
#[cfg(windows)]
const NO_FOLLOW_FLAGS: u32 = 0x0020_0000 | 0x0200_0000;

/// No flags, for the reopen that follows a reparse point that is no link:
/// the plain open the filter of a deduplicated, compressed or
/// cloud-placeholder file answers as it answers any reader. A folder that
/// took the name between the opens is refused either way: this open is
/// denied it, and a handle to one is refused by its facts.
#[cfg(windows)]
const REOPEN_FLAGS: u32 = 0;

/// What a Windows open that does not follow links keeps of the handle it
/// opened on the reparse point itself. A link or a junction is refused, and
/// anything but a regular file. A write open refuses any other reparse point
/// too. A read open of one, a deduplicated, compressed or cloud-placeholder
/// file, opens the name again without the flag: with it, CreateFileW skips
/// the filter that reads such a file, and the handle would read the raw
/// stream. The second handle must hold the file the first one held, one
/// volume serial and one file index: another file is refused as swapped,
/// and a volume with no numbers to compare as unconfirmed. One open and one
/// read of the handle for every such open, and a second of each for that
/// reparse point alone.
#[cfg(windows)]
fn windows_held(
    file: std::fs::File,
    path: &Path,
    open: Open,
    mut options: std::fs::OpenOptions,
) -> io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    let held = by_handle::ByHandle::of(&file)?;
    if held.is_name_surrogate() {
        return Err(Refused::Link.into());
    }
    if held.kind() != EntryKind::File {
        return Err(Refused::NotAFile.into());
    }
    if !held.is_reparse_point() {
        return Ok(file);
    }
    if open.mode != OpenMode::Read {
        return Err(Refused::Link.into());
    }
    drop(file);
    let reopened = options.custom_flags(REOPEN_FLAGS).open(path)?;
    let read = by_handle::ByHandle::of(&reopened)?;
    // Another kind is another file, numbered or not.
    if read.kind() != EntryKind::File {
        return Err(Refused::Swapped.into());
    }
    if !held.tells_files_apart() {
        return Err(Refused::Unconfirmed.into());
    }
    if !held.same_file_as(&read) {
        return Err(Refused::Swapped.into());
    }
    Ok(reopened)
}

/// `path` as a NUL-terminated UTF-16 buffer a Win32 call takes, in its long
/// form where it needs one.
#[cfg(windows)]
pub(crate) fn win32_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let value: Vec<u16> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains an interior NUL",
        ));
    }
    let mut value = long_path_form(value, || {
        Ok(std::path::absolute(path)?
            .as_os_str()
            .encode_wide()
            .collect())
    })?;
    value.push(0);
    Ok(value)
}

/// `path` in the form Win32 calls accept past `MAX_PATH`, prepared as std
/// prepares its own: a verbatim path stays as it is, and so does a short
/// drive or `\\` path, with either separator. Anything else is made absolute
/// by `absolute`, and that is given the `\\?\` prefix when it is long, or
/// `\\?\UNC\` for a share; a long `\\.\` device path becomes `\\?\`.
#[cfg(any(windows, test))]
fn long_path_form(
    path: Vec<u16>,
    absolute: impl FnOnce() -> io::Result<Vec<u16>>,
) -> io::Result<Vec<u16>> {
    // std's bound, which counts the terminating NUL these buffers lack.
    const LEGACY_MAX_PATH: usize = 248;
    const SEP: u16 = b'\\' as u16;
    const ALT: u16 = b'/' as u16;
    const COLON: u16 = b':' as u16;
    const QUERY: u16 = b'?' as u16;
    const DOT: u16 = b'.' as u16;
    let prefixed =
        |prefix: &str, rest: &[u16]| [&prefix.encode_utf16().collect::<Vec<_>>(), rest].concat();
    if path.is_empty()
        || matches!(
            path[..],
            [SEP, SEP, QUERY, SEP, ..] | [SEP, QUERY, QUERY, SEP, ..]
        )
    {
        return Ok(path);
    }
    if path.len() + 1 < LEGACY_MAX_PATH {
        match path[..] {
            [drive, COLON] | [drive, COLON, SEP | ALT, ..] if drive != SEP && drive != ALT => {
                return Ok(path);
            }
            [SEP | ALT, SEP | ALT, ..] => return Ok(path),
            _ => {}
        }
    }
    // The absolute form has only `\` separators.
    let full = absolute()?;
    if full.len() + 1 < LEGACY_MAX_PATH {
        return Ok(full);
    }
    Ok(match full[..] {
        [_, COLON, SEP, ..] => prefixed(r"\\?\", &full),
        [SEP, SEP, DOT, SEP, ref rest @ ..] => prefixed(r"\\?\", rest),
        [SEP, SEP, QUERY, SEP, ..] | [SEP, QUERY, QUERY, SEP, ..] => full,
        [SEP, SEP, ref rest @ ..] => prefixed(r"\\?\UNC\", rest),
        _ => full,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
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

        let mut listed = backend
            .list(dir.path())
            .unwrap()
            .collect::<io::Result<Vec<_>>>()
            .unwrap();
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
    fn only_a_durable_sync_meets_a_requirement_for_one() {
        assert!(SyncOutcome::Durable.required().is_ok());
        assert_eq!(
            SyncOutcome::Unconfirmed.required().unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[test]
    fn full_sync_falls_back_only_when_unsupported() {
        use io::ErrorKind;
        assert_eq!(
            settle_sync(Ok(()), || panic!("successful full sync needs no fallback")).unwrap(),
            SyncOutcome::Durable
        );
        assert_eq!(
            settle_sync(Err(ErrorKind::Unsupported.into()), || Ok(())).unwrap(),
            SyncOutcome::Durable
        );
        let error = settle_sync(Err(ErrorKind::PermissionDenied.into()), || {
            panic!("real sync errors must not fall back")
        })
        .expect_err("permission failure must propagate");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        let error = settle_sync(Err(ErrorKind::Unsupported.into()), || {
            Err(ErrorKind::Other.into())
        })
        .expect_err("fallback sync failure must propagate");
        assert_eq!(error.kind(), ErrorKind::Other);
    }

    #[cfg(unix)]
    #[test]
    fn enotsup_from_both_syncs_reports_a_filesystem_without_sync() {
        // smbfs on macOS can reject both F_FULLFSYNC and fsync with ENOTSUP.
        let enotsup = || io::Error::from_raw_os_error(libc::ENOTSUP);
        assert_eq!(
            settle_sync(Err(enotsup()), || Err(enotsup())).unwrap(),
            SyncOutcome::Unconfirmed
        );
        assert_eq!(
            settle_sync(Err(enotsup()), || Ok(())).unwrap(),
            SyncOutcome::Durable
        );
    }

    #[test]
    fn sync_preserves_written_contents() {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("copied-video.mkv");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"verified-copy").unwrap();
        settled(&file).expect("sync destination");
        assert_eq!(std::fs::read(path).unwrap(), b"verified-copy");
    }

    #[test]
    fn clearing_a_tree_reaches_every_entry_but_never_follows_a_link() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("Invented Show");
        let outside = directory.path().join("outside.nfo");
        std::fs::create_dir_all(tree.join("Season 01")).unwrap();
        let inner = tree.join("Season 01").join("episode.mkv");
        std::fs::write(&inner, b"a").unwrap();
        std::fs::write(&outside, b"b").unwrap();
        for path in [&inner, &outside] {
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(path, permissions).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, tree.join("linked.nfo")).unwrap();

        clear_readonly_tree(&tree).unwrap();
        clear_readonly_tree(&directory.path().join("gone")).expect("a missing tree is clear");

        assert!(!std::fs::metadata(&inner).unwrap().permissions().readonly());
        assert!(
            std::fs::metadata(&outside)
                .unwrap()
                .permissions()
                .readonly()
        );
    }

    #[test]
    fn a_claimed_rename_refuses_an_occupied_destination_and_releases_its_claim() {
        let directory = tempfile::tempdir().unwrap();
        let staged = directory.path().join("staged.tmp");
        let dest = directory.path().join("dest.mkv");
        backend_write(&staged, b"staged");
        backend_write(&dest, b"dest");
        let failed = SystemBackend.rename_claimed(&staged, &dest).unwrap_err();
        assert_eq!(failed.error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!failed.reservation_left, "nothing was reserved");
        assert_eq!(std::fs::read(&dest).unwrap(), b"dest");

        let free = directory.path().join("free.mkv");
        SystemBackend.rename_claimed(&staged, &free).unwrap();
        assert_eq!(std::fs::read(&free).unwrap(), b"staged");
        assert!(!staged.exists());
    }

    #[test]
    fn a_read_only_attribute_is_cleared_only_after_a_refusal_and_set_again_if_still_refused() {
        use std::cell::RefCell;
        let path = Path::new("invented-folder");
        let denied = || io::Error::from(io::ErrorKind::PermissionDenied);
        let run = |answers: Vec<io::Result<()>>, readonly: bool| {
            let steps = RefCell::new(Vec::new());
            let answers = RefCell::new(answers.into_iter());
            let result = removing_readonly(
                path,
                |_| {
                    steps.borrow_mut().push("remove");
                    answers.borrow_mut().next().expect("an answer")
                },
                |_| {
                    steps.borrow_mut().push("clear");
                    Ok(readonly)
                },
                |_| {
                    steps.borrow_mut().push("restore");
                    Ok(())
                },
            );
            (result.map_err(|error| error.kind()), steps.into_inner())
        };

        assert_eq!(run(vec![Ok(())], true), (Ok(()), vec!["remove"]));
        let not_found = run(vec![Err(io::ErrorKind::NotFound.into())], true);
        assert_eq!(not_found, (Err(io::ErrorKind::NotFound), vec!["remove"]));
        assert_eq!(
            run(vec![Err(denied()), Ok(())], true),
            (Ok(()), vec!["remove", "clear", "remove"])
        );
        // Refused without the attribute: nothing to clear, no second try.
        assert_eq!(
            run(vec![Err(denied())], false),
            (
                Err(io::ErrorKind::PermissionDenied),
                vec!["remove", "clear"]
            )
        );
        // Gone between the refusal and the clear: absent, not refused.
        let vanished = removing_readonly(
            path,
            |_| Err(denied()),
            |_| Err(io::ErrorKind::NotFound.into()),
            |_| panic!("nothing was cleared"),
        );
        assert_eq!(vanished.unwrap_err().kind(), io::ErrorKind::NotFound);
        // A folder that is not empty keeps its attribute.
        assert_eq!(
            run(
                vec![Err(denied()), Err(io::ErrorKind::DirectoryNotEmpty.into())],
                true
            ),
            (
                Err(io::ErrorKind::DirectoryNotEmpty),
                vec!["remove", "clear", "remove", "restore"]
            )
        );
    }

    #[test]
    fn long_paths_take_the_verbatim_prefix_and_short_or_verbatim_ones_stay() {
        fn wide(text: &str) -> Vec<u16> {
            text.encode_utf16().collect()
        }
        fn not_needed() -> io::Result<Vec<u16>> {
            panic!("a short or verbatim path is not made absolute")
        }
        // std's bound, the terminating NUL included.
        const LEGACY_BOUND: usize = 248;
        let short = r"C:\Library\Invented Show\invented.mkv";
        assert_eq!(
            long_path_form(wide(short), not_needed).unwrap(),
            wide(short)
        );

        let tail = "x".repeat(260);
        let drive = format!(r"C:\Library\{tail}");
        let prefixed = format!(r"\\?\{drive}");
        let made = long_path_form(wide(&drive), || Ok(wide(&drive))).unwrap();
        assert_eq!(made, wide(&prefixed));

        let relative = format!(r"Library\{tail}");
        let absolute = format!(r"D:\{relative}");
        let made = long_path_form(wide(&relative), || Ok(wide(&absolute))).unwrap();
        assert_eq!(made, wide(&format!(r"\\?\{absolute}")));

        let share = format!(r"\\invented-nas\media\{tail}");
        let made = long_path_form(wide(&share), || Ok(wide(&share))).unwrap();
        assert_eq!(made, wide(&format!(r"\\?\UNC\invented-nas\media\{tail}")));

        for verbatim in [
            prefixed,
            format!(r"\\?\UNC\invented-nas\media\{tail}"),
            format!(r"\??\C:\{tail}"),
        ] {
            assert_eq!(
                long_path_form(wide(&verbatim), not_needed).unwrap(),
                wide(&verbatim)
            );
        }

        // Short paths with either separator stay; so does a short device path.
        for short in [
            r"C:/Library/invented.mkv",
            r"C:",
            r"//invented-nas/media/invented.mkv",
            r"\\.\C:\invented.mkv",
        ] {
            assert_eq!(
                long_path_form(wide(short), not_needed).unwrap(),
                wide(short)
            );
        }

        // The bound is on the absolute form: a short relative path is made
        // absolute and prefixed only when that is long.
        let near = "x".repeat(LEGACY_BOUND - 1 - r"D:\".len());
        let at_bound = format!(r"D:\{near}");
        let made = long_path_form(wide("invented.mkv"), || Ok(wide(&at_bound))).unwrap();
        assert_eq!(made, wide(&format!(r"\\?\{at_bound}")));
        let under = format!(r"D:\{}", &near[1..]);
        let made = long_path_form(wide("invented.mkv"), || Ok(wide(&under))).unwrap();
        assert_eq!(made, wide(&under));

        // A long path with `/` separators, and a long device path, are made
        // absolute and take the verbatim prefix.
        let slashed = format!(r"C:/Library/{tail}");
        let made = long_path_form(wide(&slashed), || Ok(wide(&drive))).unwrap();
        assert_eq!(made, wide(&format!(r"\\?\{drive}")));
        let device = format!(r"\\.\C:\{tail}");
        let made = long_path_form(wide(&device), || Ok(wide(&device))).unwrap();
        assert_eq!(made, wide(&format!(r"\\?\C:\{tail}")));
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
        // NTFS has an exclusive rename, so this platform never falls back.
        #[cfg(windows)]
        assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists);
        let claimed = SystemBackend
            .rename_claimed(&source, &dest)
            .expect_err("a claimed rename refuses too");
        assert_eq!(claimed.error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!claimed.reservation_left);
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep me");
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
    }

    /// A claimed rename that fails releases its empty reservation, the file
    /// it made itself, whether its source is present, gone or unreadable: no
    /// empty file is left to refuse the next attempt as occupied.
    #[cfg(unix)]
    #[test]
    fn a_failed_claimed_rename_releases_the_empty_claim_it_made() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");

        // Present: a folder cannot be renamed onto the claimed file.
        let folder = dir.path().join("invented-folder");
        std::fs::create_dir(&folder).unwrap();
        let dest = dir.path().join("invented-present.bin");
        assert!(
            !SystemBackend
                .rename_claimed(&folder, &dest)
                .unwrap_err()
                .reservation_left
        );
        assert!(std::fs::symlink_metadata(&dest).is_err(), "claim released");
        assert!(folder.is_dir());

        // Absent: nothing to rename.
        let missing = dir.path().join("invented-missing.bin");
        let dest = dir.path().join("invented-absent.bin");
        assert!(
            !SystemBackend
                .rename_claimed(&missing, &dest)
                .unwrap_err()
                .reservation_left
        );
        assert!(std::fs::symlink_metadata(&dest).is_err(), "claim released");

        // Unreadable: the source's folder cannot be searched.
        let locked = dir.path().join("invented-locked");
        std::fs::create_dir(&locked).unwrap();
        let hidden = locked.join("invented-source.bin");
        std::fs::write(&hidden, b"source").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // A user the permissions do not bind reads it anyway; nothing to test.
        if std::fs::symlink_metadata(&hidden).is_err() {
            let dest = dir.path().join("invented-unread.bin");
            assert!(
                !SystemBackend
                    .rename_claimed(&hidden, &dest)
                    .unwrap_err()
                    .reservation_left
            );
            assert!(std::fs::symlink_metadata(&dest).is_err(), "claim released");
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(std::fs::read(&hidden).unwrap(), b"source");
    }

    /// What a failed claimed rename finds at the name in place of its own
    /// reservation stays: a nonempty file, and an empty file that is another
    /// entry, with the source gone, as a share that applied the rename and
    /// lost the reply leaves an empty source.
    #[test]
    fn a_failed_claim_leaves_what_is_not_its_own_reservation() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("invented-missing.bin");
        let dest = dir.path().join("invented-claimed.bin");
        std::fs::write(&dest, b"").unwrap();
        let claim = SystemBackend.lstat(&dest).unwrap().identity;

        // A nonempty file now holds the name.
        std::fs::write(&dest, b"arrived").unwrap();
        assert!(SystemBackend.release_failed_claim(&missing, &dest, claim));
        assert_eq!(std::fs::read(&dest).unwrap(), b"arrived");

        // Another empty entry now holds the name.
        let other = dir.path().join("invented-other.bin");
        std::fs::write(&other, b"").unwrap();
        std::fs::rename(&other, &dest).unwrap();
        if claim.is_some() && SystemBackend.lstat(&dest).unwrap().identity != claim {
            assert!(SystemBackend.release_failed_claim(&missing, &dest, claim));
            assert_eq!(std::fs::read(&dest).unwrap(), b"");
        }

        // With no entry to tell, the source's absence keeps it.
        assert!(SystemBackend.release_failed_claim(&missing, &dest, None));
        assert_eq!(std::fs::read(&dest).unwrap(), b"");
    }

    /// A claimed rename that fails, on every platform, leaves no reservation
    /// behind and says so: the handle it reserved the name with is closed
    /// before the rename, and the empty file it made is the one released.
    #[test]
    fn a_failed_claimed_rename_leaves_no_reservation_on_every_platform() {
        let dir = tempfile::tempdir().expect("temp dir");
        for (source, dest) in [
            ("invented-missing.bin", "invented-absent.bin"),
            ("invented-folder", "invented-present.bin"),
        ] {
            let (source, dest) = (dir.path().join(source), dir.path().join(dest));
            if source.extension().is_none() {
                std::fs::create_dir(&source).unwrap();
            }
            let failed = SystemBackend
                .rename_claimed(&source, &dest)
                .expect_err("the rename fails");
            assert!(!failed.reservation_left, "{source:?}");
            assert!(
                std::fs::symlink_metadata(&dest).is_err(),
                "the reservation is gone: {source:?}"
            );
            // The name is free for the next attempt.
            std::fs::write(&dest, b"next").unwrap();
        }
    }

    /// The reservation a failed claimed rename finds as its own is removed;
    /// one it cannot remove is kept and said to be left.
    #[cfg(unix)]
    #[test]
    fn a_failed_claim_removes_its_own_reservation_and_keeps_one_it_cannot() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("invented-missing.bin");
        let dest = dir.path().join("invented-claimed.bin");
        std::fs::write(&dest, b"").unwrap();
        let claim = SystemBackend.lstat(&dest).unwrap().identity;
        assert!(claim.is_some(), "a unix volume numbers its files");
        assert!(!SystemBackend.release_failed_claim(&missing, &dest, claim));
        assert!(std::fs::symlink_metadata(&dest).is_err(), "released");

        // A folder that refuses the removal keeps the reservation.
        let locked = dir.path().join("invented-locked");
        std::fs::create_dir(&locked).unwrap();
        let dest = locked.join("invented-claimed.bin");
        std::fs::write(&dest, b"").unwrap();
        let claim = SystemBackend.lstat(&dest).unwrap().identity;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        // A user the permissions do not bind removes it anyway; nothing to
        // test then.
        let probe = locked.join("invented-probe.bin");
        if std::fs::write(&probe, b"").is_err() {
            assert!(SystemBackend.release_failed_claim(&missing, &dest, claim));
            assert_eq!(std::fs::read(&dest).unwrap(), b"", "kept");
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
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
