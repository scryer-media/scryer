//! A Windows stat read from one handle: what `GetFileInformationByHandle`
//! says, the volume serial and file index included, which std reads and
//! does not hand out.
//!
//! One open per stat, as std's own stat makes: the handle asks for no access
//! and opens a folder, and a read that does not follow links opens the link
//! itself. A reparse point's tag is one more call on the same handle. Whether
//! the volume keeps its file numbers is one call per volume serial, the first
//! time it is seen, and remembered for the life of the process.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FileAttributeTagInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    GetVolumeInformationByHandleW,
};

use super::{Extra, RawStat};
use crate::reading::{EntryKind, FileIdentity};

/// `FILE_SUPPORTS_OPEN_BY_FILE_ID`: the filesystem keeps a file's number
/// for the file's life, so it can open a file by it.
const SUPPORTS_OPEN_BY_FILE_ID: u32 = 0x0100_0000;
/// Win32 errors on which std's own stat reads the parent's listing instead
/// of a handle: a file another process holds without sharing, or one the
/// process may not open even for its attributes.
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SHARING_VIOLATION: i32 = 32;
/// FILETIME counts 100 ns intervals from 1601; this many lie before 1970.
const INTERVALS_TO_UNIX_EPOCH: i64 = 116_444_736_000_000_000;

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
    modified: SystemTime,
    created: SystemTime,
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
        let tag = if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
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
            numbered: serial != 0 && keeps_file_numbers(file, serial),
        })
    }

    /// Whether the handle holds a reparse point.
    pub(crate) fn is_reparse_point(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    /// Whether the handle holds a link or a junction: a reparse point whose
    /// tag names another path. std's own test for a link.
    pub(crate) fn is_name_surrogate(&self) -> bool {
        self.is_reparse_point() && self.tag & 0x2000_0000 != 0
    }

    pub(crate) fn kind(&self) -> EntryKind {
        if self.is_name_surrogate() {
            EntryKind::Symlink
        } else if self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            EntryKind::Directory
        } else {
            EntryKind::File
        }
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

    pub(crate) fn raw_stat(&self) -> RawStat {
        let identity = self.identity();
        RawStat {
            kind: self.kind(),
            len: self.len,
            modified: Some(self.modified),
            identity,
            links: Some(u64::from(self.links)),
            extra: Extra {
                serial: (self.serial != 0).then_some(u64::from(self.serial)),
                inode: Some(self.index),
                unnumbered: identity.is_none(),
                created: Some(self.created),
                ..Extra::default()
            },
        }
    }
}

/// A stat of `path` from one handle, the link itself when `follow` is
/// false. A file the process may not open even for its attributes is read
/// as std reads it, without its volume or number.
pub(crate) fn stat(path: &Path, follow: bool) -> io::Result<RawStat> {
    let flags = if follow {
        FILE_FLAG_BACKUP_SEMANTICS
    } else {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    };
    let opened = std::fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(flags)
        .open(path);
    match opened {
        Ok(file) => ByHandle::of(&file).map(|read| read.raw_stat()),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION)
            ) =>
        {
            let metadata = if follow {
                std::fs::metadata(path)?
            } else {
                std::fs::symlink_metadata(path)?
            };
            Ok(RawStat::of(&metadata))
        }
        Err(error) => Err(error),
    }
}

/// Whether the volume `file` is on keeps the numbers of its files for their
/// life. FAT and exFAT derive a number from where the entry sits, so a
/// rename changes it, and do not say they keep them. ReFS keeps 128-bit
/// numbers, which the 64-bit index truncates, so two files there can share
/// one; it is not trusted either. One call for each serial, remembered when
/// it answers.
fn keeps_file_numbers(file: &File, serial: u32) -> bool {
    static KEPT: OnceLock<Mutex<HashMap<u32, bool>>> = OnceLock::new();
    let kept = KEPT.get_or_init(Mutex::default);
    if let Some(known) = kept.lock().ok().and_then(|kept| kept.get(&serial).copied()) {
        return known;
    }
    let mut flags = 0u32;
    let mut name = [0u16; 32];
    // SAFETY: the handle stays open for the call; the name buffer's length
    // is given in characters, and every other out pointer is null or valid.
    let answered = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    if answered == 0 {
        return false;
    }
    let end = name
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(name.len());
    let refs = String::from_utf16_lossy(&name[..end]).eq_ignore_ascii_case("ReFS");
    let keeps = flags & SUPPORTS_OPEN_BY_FILE_ID != 0 && !refs;
    if let Ok(mut kept) = kept.lock() {
        kept.insert(serial, keeps);
    }
    keeps
}

/// The instant a FILETIME names, as std converts one.
fn system_time(at: FILETIME) -> SystemTime {
    let intervals = ((u64::from(at.dwHighDateTime) << 32) | u64::from(at.dwLowDateTime)) as i64;
    let since = intervals.wrapping_sub(INTERVALS_TO_UNIX_EPOCH);
    let span = |count: i64| {
        let count = count.unsigned_abs();
        Duration::new(count / 10_000_000, ((count % 10_000_000) * 100) as u32)
    };
    if since >= 0 {
        UNIX_EPOCH + span(since)
    } else {
        UNIX_EPOCH - span(since)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filetime(intervals: u64) -> FILETIME {
        FILETIME {
            dwLowDateTime: intervals as u32,
            dwHighDateTime: (intervals >> 32) as u32,
        }
    }

    #[test]
    fn a_filetime_converts_as_std_converts_it() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let read = ByHandle::of(file.as_file()).unwrap();
        let metadata = file.as_file().metadata().unwrap();
        assert_eq!(Some(read.modified), metadata.modified().ok());
        assert_eq!(Some(read.created), metadata.created().ok());
        assert_eq!(
            system_time(filetime(INTERVALS_TO_UNIX_EPOCH as u64)),
            UNIX_EPOCH
        );
        assert_eq!(
            system_time(filetime(INTERVALS_TO_UNIX_EPOCH as u64 - 15)),
            UNIX_EPOCH - Duration::from_nanos(1_500)
        );
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
}
