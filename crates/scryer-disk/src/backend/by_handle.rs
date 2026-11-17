//! A Windows stat read from one handle: what `GetFileInformationByHandle`
//! says, the volume serial and file index included, which std reads and
//! does not hand out.
//!
//! One open per stat, as std's own stat makes: the handle asks for no access
//! and opens a folder, and a read that does not follow links opens the link
//! itself. A reparse point's tag is one more call on the same handle. Whether
//! the volume keeps its file numbers is one call per volume serial, the first
//! time it is seen, and remembered for the life of the process whether the
//! volume answered or not. Where std falls back, this does too, with one call
//! each: a name the process may not open reads from its folder's listing, and
//! a name that cannot be followed is read as itself unless it is a link.

use std::fs::File;
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::time::SystemTime;

use windows_sys::Win32::Foundation::{FILETIME, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FileAttributeTagInfo, FindClose, FindExInfoBasic,
    FindExSearchNameMatch, FindFirstFileExW, GetFileInformationByHandle,
    GetFileInformationByHandleEx, GetVolumeInformationByHandleW, WIN32_FIND_DATAW,
};

use super::win_facts::{self, VolumeAnswers};
use super::{Extra, RawStat};
use crate::reading::{EntryKind, FileIdentity};

/// Win32 errors on which std's own stat reads the parent's listing instead
/// of a handle: a file another process holds without sharing, or one the
/// process may not open even for its attributes.
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SHARING_VIOLATION: i32 = 32;
/// The error a followed open gives for a reparse point whose filter is
/// absent, such as an app execution alias; std reads such a name as itself.
const ERROR_CANT_ACCESS_FILE: i32 = 1920;

/// What each volume said about its file numbers, by serial.
static VOLUMES: VolumeAnswers = VolumeAnswers::new();

/// What one handle says about the file it holds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ByHandle {
    attributes: u32,
    /// The reparse tag, read only when the attributes name a reparse point.
    tag: u32,
    serial: u32,
    index: u64,
    links: u32,
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    /// Whether the volume keeps file numbers that tell its files apart.
    numbered: bool,
}

impl ByHandle {
    pub(crate) fn of(file: &File) -> io::Result<Self> {
        let handle = file.as_raw_handle();
        let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // SAFETY: the handle stays open for the call, and the structure is
        // the size the call writes.
        if unsafe { GetFileInformationByHandle(handle, information.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call above succeeded and filled the structure.
        let information = unsafe { information.assume_init() };
        let attributes = information.dwFileAttributes;
        let tag = if attributes & win_facts::REPARSE_POINT != 0 {
            let mut tag_info = std::mem::MaybeUninit::<FILE_ATTRIBUTE_TAG_INFO>::uninit();
            // SAFETY: as above, with the buffer's own size.
            let read = unsafe {
                GetFileInformationByHandleEx(
                    handle,
                    FileAttributeTagInfo,
                    tag_info.as_mut_ptr().cast(),
                    std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
                )
            };
            if read == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the call above succeeded and filled the structure.
            unsafe { tag_info.assume_init() }.ReparseTag
        } else {
            0
        };
        let serial = information.dwVolumeSerialNumber;
        let index =
            (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
        Ok(Self {
            attributes,
            tag,
            serial,
            index,
            links: information.nNumberOfLinks,
            len: (u64::from(information.nFileSizeHigh) << 32) | u64::from(information.nFileSizeLow),
            modified: system_time(information.ftLastWriteTime),
            created: system_time(information.ftCreationTime),
            numbered: serial != 0 && VOLUMES.keeps_numbers(serial, || volume_flags(file)),
        })
    }

    /// Whether the handle holds a reparse point.
    pub(crate) fn is_reparse_point(&self) -> bool {
        self.attributes & win_facts::REPARSE_POINT != 0
    }

    /// Whether the handle holds a link or a junction: a reparse point whose
    /// tag names another path. std's own test for a link.
    pub(crate) fn is_name_surrogate(&self) -> bool {
        win_facts::is_name_surrogate(self.attributes, self.tag)
    }

    pub(crate) fn kind(&self) -> EntryKind {
        win_facts::kind(self.attributes, self.tag)
    }

    /// The volume serial and file index, where both tell files apart: a
    /// volume that keeps its numbers, a serial and an index that are not
    /// zero, and an index that is not the all-ones "no number".
    pub(crate) fn identity(&self) -> Option<FileIdentity> {
        if !self.numbered || self.index == u64::MAX {
            return None;
        }
        FileIdentity::from_raw(u64::from(self.serial), self.index)
    }

    /// Whether `later`, read from a handle opened on the same name after
    /// this one, holds the same file: one non-zero volume serial and one
    /// file index that is neither zero nor all ones. The index is taken as
    /// given, kept by its volume or not, since the two opens are moments
    /// apart; on ReFS two files can share a truncated index.
    pub(crate) fn same_file_as(&self, later: &Self) -> bool {
        self.serial != 0
            && self.index != 0
            && self.index != u64::MAX
            && (self.serial, self.index) == (later.serial, later.index)
    }

    pub(crate) fn raw_stat(&self) -> RawStat {
        let identity = self.identity();
        RawStat {
            kind: self.kind(),
            len: self.len,
            modified: self.modified,
            identity,
            links: Some(u64::from(self.links)),
            extra: Extra {
                serial: (self.serial != 0).then_some(u64::from(self.serial)),
                inode: Some(self.index),
                unnumbered: identity.is_none(),
                created: self.created,
                ..Extra::default()
            },
        }
    }
}

/// A stat of `path` from one handle, the link itself when `follow` is
/// false. A name the process may not open even for its attributes reads
/// from its folder's listing, without its volume or number; a followed name
/// whose reparse point cannot be followed reads as itself, unless it is a
/// link or a junction.
pub(crate) fn stat(path: &Path, follow: bool) -> io::Result<RawStat> {
    let error = match opened(path, follow) {
        Ok(file) => return ByHandle::of(&file).map(|read| read.raw_stat()),
        Err(error) => error,
    };
    match error.raw_os_error() {
        Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION) => {
            match listed(path) {
                // A followed name that is a link cannot be read past here.
                Some(read) if !(follow && read.kind == EntryKind::Symlink) => Ok(read),
                _ => Err(error),
            }
        }
        Some(ERROR_CANT_ACCESS_FILE) if follow => match opened(path, false) {
            Ok(file) => match ByHandle::of(&file)? {
                read if !read.is_name_surrogate() => Ok(read.raw_stat()),
                _ => Err(error),
            },
            Err(_) => Err(error),
        },
        _ => Err(error),
    }
}

/// The handle a stat reads: no access asked, a folder opened, and the link
/// itself when `follow` is false.
fn opened(path: &Path, follow: bool) -> io::Result<File> {
    let flags = if follow {
        FILE_FLAG_BACKUP_SEMANTICS
    } else {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    };
    std::fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(flags)
        .open(path)
}

/// What the listing of `path`'s folder says of it, in one call, as std
/// reads a name it may not open: no volume, no number, no link count.
/// `None` when the listing does not say.
fn listed(path: &Path) -> Option<RawStat> {
    let wide = super::win32_path(path).ok()?;
    let mut found = std::mem::MaybeUninit::<WIN32_FIND_DATAW>::uninit();
    // SAFETY: the name is NUL-terminated and outlives the call, and the
    // structure is the one FindExInfoBasic writes.
    let search = unsafe {
        FindFirstFileExW(
            wide.as_ptr(),
            FindExInfoBasic,
            found.as_mut_ptr().cast(),
            FindExSearchNameMatch,
            std::ptr::null(),
            0,
        )
    };
    if search == INVALID_HANDLE_VALUE {
        return None;
    }
    // SAFETY: the search handle is the valid one just returned.
    unsafe { FindClose(search) };
    // SAFETY: the call above succeeded and filled the structure.
    let found = unsafe { found.assume_init() };
    let attributes = found.dwFileAttributes;
    // The reparse tag, where the attributes name a reparse point.
    let tag = found.dwReserved0;
    Some(RawStat {
        kind: win_facts::kind(attributes, tag),
        len: (u64::from(found.nFileSizeHigh) << 32) | u64::from(found.nFileSizeLow),
        modified: system_time(found.ftLastWriteTime),
        identity: None,
        links: None,
        extra: Extra {
            unnumbered: true,
            created: system_time(found.ftCreationTime),
            ..Extra::default()
        },
    })
}

/// The flags of the volume `file` is on, or `None` when it does not say.
/// No name is asked for, so no buffer can be too short.
fn volume_flags(file: &File) -> Option<u32> {
    let mut flags = 0u32;
    // SAFETY: the handle stays open for the call; every out pointer but
    // the flags is null, with a zero length.
    let answered = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            std::ptr::null_mut(),
            0,
        )
    };
    (answered != 0).then_some(flags)
}

/// The instant a FILETIME names, as std converts one; `None` past what the
/// conversion can hold.
fn system_time(at: FILETIME) -> Option<SystemTime> {
    win_facts::filetime((u64::from(at.dwHighDateTime) << 32) | u64::from(at.dwLowDateTime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_reads_the_times_std_reads() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let read = ByHandle::of(file.as_file()).unwrap();
        let metadata = file.as_file().metadata().unwrap();
        assert_eq!(read.modified, metadata.modified().ok());
        assert_eq!(read.created, metadata.created().ok());
        // A FILETIME with the high bit set reads as no time, never a panic.
        let far = FILETIME {
            dwLowDateTime: u32::MAX,
            dwHighDateTime: u32::MAX,
        };
        let _ = system_time(far);
    }

    #[test]
    fn one_handle_reads_what_std_reads_and_the_file_number_besides() {
        let folder = tempfile::tempdir().unwrap();
        let first = folder.path().join("first.bin");
        let second = folder.path().join("second.bin");
        std::fs::write(&first, b"twelve bytes").unwrap();
        std::fs::write(&second, b"other").unwrap();

        let read = stat(&first, false).unwrap();
        let std_read = RawStat::of(&std::fs::symlink_metadata(&first).unwrap());
        assert_eq!(read.kind, std_read.kind);
        assert_eq!(read.len, std_read.len);
        assert_eq!(read.modified, std_read.modified);
        assert_eq!(read.extra.created, std_read.extra.created);
        assert_eq!(read.links, Some(1));
        assert!(read.extra.serial.is_some(), "an NTFS volume has a serial");
        assert!(read.identity.is_some(), "an NTFS volume numbers its files");
        assert_ne!(read.identity, stat(&second, false).unwrap().identity);
        assert_eq!(read.identity, stat(&first, true).unwrap().identity);
        assert_eq!(
            stat(folder.path(), false).unwrap().kind,
            EntryKind::Directory
        );

        let linked = folder.path().join("linked.bin");
        std::fs::hard_link(&first, &linked).unwrap();
        let after = stat(&linked, false).unwrap();
        assert_eq!(after.identity, read.identity);
        assert_eq!(after.links, Some(2));
        assert!(matches!(
            stat(&folder.path().join("missing"), false),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ));
    }

    /// A stat that follows a link reads its target, and one that does not
    /// reads the link: two files. A link or a junction to a folder reads so
    /// too. Creating a symbolic link needs the right to; without it the test
    /// shows nothing. A junction is not made here.
    #[test]
    fn a_link_reads_as_itself_unfollowed_and_as_its_target_followed() {
        let folder = tempfile::tempdir().unwrap();
        let target = folder.path().join("target.bin");
        std::fs::write(&target, b"target").unwrap();
        let (link, folder_link) = (folder.path().join("link.bin"), folder.path().join("linked"));
        if std::os::windows::fs::symlink_file(&target, &link).is_err() {
            return;
        }
        std::os::windows::fs::symlink_dir(folder.path(), &folder_link).unwrap();
        let unfollowed = stat(&link, false).unwrap();
        let followed = stat(&link, true).unwrap();
        assert_eq!(unfollowed.kind, EntryKind::Symlink);
        assert_eq!(followed.kind, EntryKind::File);
        assert_eq!(followed.identity, stat(&target, false).unwrap().identity);
        assert_ne!(unfollowed.identity, followed.identity);
        assert_eq!(stat(&folder_link, false).unwrap().kind, EntryKind::Symlink);
        assert_eq!(stat(&folder_link, true).unwrap().kind, EntryKind::Directory);
    }

    /// The system's paging file is held without sharing and cannot be opened
    /// even for its attributes: it reads from the listing of its folder, as
    /// std reads it, a file with no volume or number. A machine without one
    /// at the usual name shows nothing.
    #[test]
    fn a_file_held_without_sharing_reads_from_its_folder_listing() {
        let held = Path::new(r"C:\pagefile.sys");
        let Ok(std_read) = std::fs::symlink_metadata(held) else {
            return;
        };
        let read = stat(held, false).unwrap();
        assert_eq!(read.kind, EntryKind::File);
        assert_eq!(read.len, std_read.len());
        assert_eq!(read.identity, None);
        assert_eq!(read.links, None);
        assert_eq!(stat(held, true).unwrap().len, std_read.len());
    }

    /// An app execution alias is a reparse point no followed open can read:
    /// std reads it as itself, and so does this. A machine without one
    /// shows nothing.
    #[test]
    fn a_name_that_cannot_be_followed_reads_as_itself() {
        let Some(local) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        let aliases = Path::new(&local).join("Microsoft").join("WindowsApps");
        let Some(alias) = std::fs::read_dir(&aliases).ok().and_then(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .find(|path| path.extension().is_some_and(|extension| extension == "exe"))
        }) else {
            return;
        };
        let std_read = std::fs::metadata(&alias).unwrap();
        let read = stat(&alias, true).unwrap();
        assert_eq!(read.len, std_read.len());
        assert_eq!(read.kind == EntryKind::File, std_read.is_file());
    }

    /// A cloud placeholder, a deduplicated file and a WOF-compressed file are
    /// reparse points that read as files.
    #[test]
    fn a_reparse_point_that_is_not_a_link_reads_as_a_file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let plain = ByHandle::of(file.as_file()).unwrap();
        assert!(!plain.is_reparse_point() && plain.kind() == EntryKind::File);
        let tagged = |tag: u32| ByHandle {
            attributes: plain.attributes | win_facts::REPARSE_POINT,
            tag,
            ..plain
        };
        assert!(tagged(0xA000_000C).is_name_surrogate());
        for tag in [0x9000_001A, 0x8000_0013, 0x8000_0017] {
            assert!(tagged(tag).is_reparse_point(), "{tag:#x}");
            assert!(!tagged(tag).is_name_surrogate(), "{tag:#x}");
            assert_eq!(tagged(tag).kind(), EntryKind::File, "{tag:#x}");
        }
    }

    /// The second handle of a reopened name holds the first one's file only
    /// with one known serial and one usable index.
    #[test]
    fn a_reopened_handle_holds_the_same_file_only_by_a_known_number() {
        let folder = tempfile::tempdir().unwrap();
        let (first, other) = (folder.path().join("a.bin"), folder.path().join("b.bin"));
        std::fs::write(&first, b"a").unwrap();
        std::fs::write(&other, b"b").unwrap();
        let read = |path: &Path| ByHandle::of(&std::fs::File::open(path).unwrap()).unwrap();
        let held = read(&first);
        assert!(held.same_file_as(&read(&first)));
        assert!(!held.same_file_as(&read(&other)));
        for unknown in [
            ByHandle { serial: 0, ..held },
            ByHandle { index: 0, ..held },
            ByHandle {
                index: u64::MAX,
                ..held
            },
        ] {
            assert!(!unknown.same_file_as(&unknown), "{unknown:?}");
        }
    }

    /// A read open that does not follow links holds a plain file as it is,
    /// with one open, and refuses a folder.
    #[test]
    fn a_no_follow_read_keeps_a_plain_file_and_refuses_a_folder() {
        use crate::backend::{DiskBackend, Refused, SystemBackend};
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("plain.bin");
        std::fs::write(&path, b"plain").unwrap();
        let open = crate::Open::read().without_following_links();
        let file = SystemBackend.open(&path, open).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 5);
        let refused = SystemBackend.open(folder.path(), open).unwrap_err();
        assert_eq!(Refused::of(&refused), Some(Refused::NotAFile));
    }
}
