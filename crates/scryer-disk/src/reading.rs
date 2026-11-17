//! What the owner can say about a path, and why an operation failed.
//!
//! Every answer is typed. A path that cannot be read is never reported as
//! absent: only the platform's "not found" makes a path [`PathReading::Missing`];
//! a dead mount, a permission error or a timeout is
//! [`PathReading::Unreachable`]. Nothing here names a filesystem or needs
//! filesystem knowledge to interpret.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};

/// Stable codes for readings that are not an answer and for failed
/// operations. Released codes are API: never rename, reuse or repurpose one.
pub mod reason {
    /// Nothing current is known about the path; the caller did not ask the
    /// disk.
    pub const DISK_NOT_OBSERVED: &str = "DISK_NOT_OBSERVED";
    /// The storage could not be reached: a stale handle, a dropped connection,
    /// a host or network that is down. Also the generic reason of a reading
    /// that got no answer for a cause with no code of its own, a bounded read
    /// that was refused or failed included.
    pub const DISK_PATH_UNREACHABLE: &str = "DISK_PATH_UNREACHABLE";
    /// The disk did not answer within the owner's bound.
    pub const DISK_TIMED_OUT: &str = "DISK_TIMED_OUT";
    /// The path does not exist.
    pub const DISK_PATH_MISSING: &str = "DISK_PATH_MISSING";
    /// The operation was refused for lack of permission.
    pub const DISK_PERMISSION_DENIED: &str = "DISK_PERMISSION_DENIED";
    /// The destination already holds something, and the operation never
    /// replaces it.
    pub const DISK_DESTINATION_EXISTS: &str = "DISK_DESTINATION_EXISTS";
    /// A directory still has entries, so it was not removed.
    pub const DISK_DIRECTORY_NOT_EMPTY: &str = "DISK_DIRECTORY_NOT_EMPTY";
    /// The path is a directory where a file was expected, or the reverse.
    pub const DISK_WRONG_KIND: &str = "DISK_WRONG_KIND";
    /// The source and destination cannot be joined by a rename; the move
    /// needs a copy.
    pub const DISK_MOVE_NEEDS_COPY: &str = "DISK_MOVE_NEEDS_COPY";
    /// The destination has no room left, or the account's quota on it is
    /// used up.
    pub const DISK_NO_SPACE: &str = "DISK_NO_SPACE";
    /// The path is a link, and the operation was asked not to follow links.
    pub const DISK_LINK_NOT_FOLLOWED: &str = "DISK_LINK_NOT_FOLLOWED";
    /// The destination is on another volume than the source, so it cannot
    /// hold a hard link to it.
    pub const DISK_LINK_DIFFERENT_VOLUME: &str = "DISK_LINK_DIFFERENT_VOLUME";
    /// The destination's filesystem has no hard links, or no room for
    /// another one to this file.
    pub const DISK_LINK_NOT_SUPPORTED: &str = "DISK_LINK_NOT_SUPPORTED";
    /// A copy did not match its source, so it does not count and the source
    /// was kept.
    pub const DISK_COPY_NOT_VERIFIED: &str = "DISK_COPY_NOT_VERIFIED";
    /// The name a staged copy writes under is held by something, so nothing
    /// was copied.
    pub const DISK_COPY_STAGING_OCCUPIED: &str = "DISK_COPY_STAGING_OCCUPIED";
    /// Whoever watches a copy stopped it.
    pub const DISK_COPY_CANCELLED: &str = "DISK_COPY_CANCELLED";
    /// Any other failure.
    pub const DISK_OPERATION_FAILED: &str = "DISK_OPERATION_FAILED";

    // Read by the distinctness test.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const ALL: &[&str] = &[
        DISK_NOT_OBSERVED,
        DISK_PATH_UNREACHABLE,
        DISK_TIMED_OUT,
        DISK_PATH_MISSING,
        DISK_PERMISSION_DENIED,
        DISK_DESTINATION_EXISTS,
        DISK_DIRECTORY_NOT_EMPTY,
        DISK_WRONG_KIND,
        DISK_MOVE_NEEDS_COPY,
        DISK_NO_SPACE,
        DISK_LINK_NOT_FOLLOWED,
        DISK_LINK_DIFFERENT_VOLUME,
        DISK_LINK_NOT_SUPPORTED,
        DISK_COPY_NOT_VERIFIED,
        DISK_COPY_STAGING_OCCUPIED,
        DISK_COPY_CANCELLED,
        DISK_OPERATION_FAILED,
    ];
}

/// What sits at a path. A link is reported as a link, never as what it
/// points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// An opaque identity for the file at a path, present only where the disk
/// gives a usable one. Two readings with equal identities are the same file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    /// `None` for a zero inode, which some network and FUSE filesystems
    /// report for every file: equality on it would prove nothing.
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) fn from_raw(device: u64, inode: u64) -> Option<Self> {
        (inode != 0).then_some(Self { device, inode })
    }

    /// Whether both files are on the same device, so they share its space.
    pub fn same_device(self, other: Self) -> bool {
        self.device == other.device
    }
}

/// What may be said about a path at a given instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathReading {
    Known {
        kind: EntryKind,
        /// Absent when the reading came from a directory listing that did not
        /// carry it.
        len: Option<u64>,
        modified: Option<SystemTime>,
        identity: Option<FileIdentity>,
        /// How many names the file has, where the disk reports it. Absent
        /// when the reading came from a listing or the platform does not say.
        links: Option<u64>,
        observed_at: DateTime<Utc>,
    },
    Missing {
        observed_at: DateTime<Utc>,
    },
    Unreachable {
        observed_at: DateTime<Utc>,
        /// The [`reason`] code for the failure that left it unread.
        cause: &'static str,
    },
    /// Nothing current is known; only [`super::Disk::observed`] answers this.
    NotObserved,
}

impl PathReading {
    /// The [`reason`] code for a reading that does not say what is there.
    pub fn unknown_reason(self) -> Option<&'static str> {
        match self {
            Self::Known { .. } | Self::Missing { .. } => None,
            Self::Unreachable { .. } => Some(reason::DISK_PATH_UNREACHABLE),
            Self::NotObserved => Some(reason::DISK_NOT_OBSERVED),
        }
    }

    /// Why a reading offers nothing to use, for a sentence: the unknown
    /// reason, or "missing".
    pub fn absence(self) -> &'static str {
        self.unknown_reason().unwrap_or("missing")
    }

    /// Why a reading offers nothing to use, in words for a log line or an
    /// error sentence, never a reason code. A reading that found something
    /// but lacks the detail asked for says so.
    pub fn describe_absence(self) -> &'static str {
        match self {
            Self::Known { .. } => "present, but the detail was not read",
            Self::Missing { .. } => "missing",
            Self::NotObserved => "not observed",
            Self::Unreachable { cause, .. } => match cause {
                reason::DISK_TIMED_OUT => "the disk did not answer in time",
                reason::DISK_PERMISSION_DENIED => "permission denied",
                reason::DISK_PATH_MISSING => "missing",
                reason::DISK_PATH_UNREACHABLE => "the path could not be read",
                reason::DISK_WRONG_KIND => "a parent component is not a directory",
                reason::DISK_LINK_NOT_FOLLOWED => "a link that is not followed",
                _ => "the read failed",
            },
        }
    }

    /// Why an unreachable reading could not be taken, as a [`reason`] code.
    pub fn unreachable_cause(self) -> Option<&'static str> {
        match self {
            Self::Unreachable { cause, .. } => Some(cause),
            _ => None,
        }
    }

    /// `Some(true)` when something is there, `Some(false)` when nothing is,
    /// `None` when that is not known. Never collapse `None` into either.
    pub fn exists(self) -> Option<bool> {
        match self {
            Self::Known { .. } => Some(true),
            Self::Missing { .. } => Some(false),
            Self::Unreachable { .. } | Self::NotObserved => None,
        }
    }

    pub fn kind(self) -> Option<EntryKind> {
        match self {
            Self::Known { kind, .. } => Some(kind),
            _ => None,
        }
    }

    /// Whether the reading shows `kind` there; `false` when it is unknown.
    pub fn is(self, kind: EntryKind) -> bool {
        self.kind() == Some(kind)
    }

    /// The modification time the reading carries, when it carries one.
    pub fn modified(self) -> Option<SystemTime> {
        match self {
            Self::Known { modified, .. } => modified,
            _ => None,
        }
    }

    /// The length the reading carries, when it carries one.
    pub fn size(self) -> Option<u64> {
        match self {
            Self::Known { len, .. } => len,
            _ => None,
        }
    }

    /// How many names the file has, when the reading carries it.
    pub fn link_count(self) -> Option<u64> {
        match self {
            Self::Known { links, .. } => links,
            _ => None,
        }
    }

    pub fn observed_at(self) -> Option<DateTime<Utc>> {
        match self {
            Self::Known { observed_at, .. }
            | Self::Missing { observed_at }
            | Self::Unreachable { observed_at, .. } => Some(observed_at),
            Self::NotObserved => None,
        }
    }

    /// A probe of the path itself, not evidence seeded by a listing.
    pub(crate) fn is_complete(self) -> bool {
        !matches!(self, Self::Known { len: None, .. } | Self::NotObserved)
    }

    /// A reading from one call's outcome. Only "not found" is missing.
    pub(crate) fn from_result(
        result: &io::Result<super::backend::RawStat>,
        observed_at: DateTime<Utc>,
    ) -> Self {
        match result {
            Ok(stat) => stat.reading(observed_at),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::Missing { observed_at },
            Err(error) => Self::Unreachable {
                observed_at,
                cause: classify(error),
            },
        }
    }
}

/// The operation a failure belongs to, named in the failure's sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskVerb {
    Read,
    List,
    Resolve,
    Remove,
    Move,
    Link,
    CreateDir,
    Write,
    Sync,
    /// Permissions or times of an existing path.
    Change,
}

impl DiskVerb {
    fn sentence(self) -> &'static str {
        match self {
            Self::Read => "could not read",
            Self::List => "could not list",
            Self::Resolve => "could not resolve",
            Self::Remove => "could not remove",
            Self::Move => "could not move",
            Self::Link => "could not link",
            Self::CreateDir => "could not create directory",
            Self::Write => "could not write",
            Self::Sync => "could not sync",
            Self::Change => "could not change",
        }
    }
}

/// A failed operation: a stable [`reason`] code, the path, and the
/// platform's own words for people.
#[derive(Debug, thiserror::Error)]
#[error("{} {}: {source}", verb.sentence(), path.display())]
pub struct DiskError {
    verb: DiskVerb,
    reason: &'static str,
    path: PathBuf,
    #[source]
    source: io::Error,
}

impl DiskError {
    pub(crate) fn new(verb: DiskVerb, path: &Path, source: io::Error) -> Self {
        Self {
            verb,
            reason: classify(&source),
            path: path.to_path_buf(),
            source,
        }
    }

    pub(crate) fn timed_out(verb: DiskVerb, path: &Path, timeout: std::time::Duration) -> Self {
        Self::new(
            verb,
            path,
            io::Error::new(
                io::ErrorKind::TimedOut,
                format!("the disk did not answer within {} ms", timeout.as_millis()),
            ),
        )
    }

    pub fn reason(&self) -> &'static str {
        self.reason
    }

    #[cfg(test)]
    pub(crate) fn verb(&self) -> DiskVerb {
        self.verb
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The platform's own words for the failure, without the verb or path.
    pub fn words(&self) -> impl std::fmt::Display + '_ {
        &self.source
    }
}

impl From<DiskError> for io::Error {
    fn from(error: DiskError) -> Self {
        error.source
    }
}

/// The one place an I/O failure becomes a reason code. Typed kinds only.
fn classify(error: &io::Error) -> &'static str {
    use io::ErrorKind as K;
    if is_cross_device_error(error) {
        return reason::DISK_MOVE_NEEDS_COPY;
    }
    // Win32's two answers for an occupied name, read from the raw code so the
    // no-replace rename keeps its refusal whatever kind std gives them.
    #[cfg(windows)]
    {
        const ERROR_FILE_EXISTS: i32 = 80;
        const ERROR_ALREADY_EXISTS: i32 = 183;
        if matches!(
            error.raw_os_error(),
            Some(ERROR_FILE_EXISTS | ERROR_ALREADY_EXISTS)
        ) {
            return reason::DISK_DESTINATION_EXISTS;
        }
    }
    if let Some(refusal) = super::backend::Refused::of(error) {
        return match refusal {
            super::backend::Refused::Link => reason::DISK_LINK_NOT_FOLLOWED,
            super::backend::Refused::NotAFile => reason::DISK_WRONG_KIND,
        };
    }
    match error.kind() {
        K::NotFound => reason::DISK_PATH_MISSING,
        K::PermissionDenied | K::ReadOnlyFilesystem => reason::DISK_PERMISSION_DENIED,
        K::AlreadyExists => reason::DISK_DESTINATION_EXISTS,
        K::DirectoryNotEmpty => reason::DISK_DIRECTORY_NOT_EMPTY,
        K::NotADirectory | K::IsADirectory => reason::DISK_WRONG_KIND,
        K::TimedOut => reason::DISK_TIMED_OUT,
        K::StorageFull | K::QuotaExceeded => reason::DISK_NO_SPACE,
        K::StaleNetworkFileHandle
        | K::NetworkDown
        | K::NetworkUnreachable
        | K::HostUnreachable
        | K::ConnectionReset
        | K::ConnectionAborted
        | K::NotConnected => reason::DISK_PATH_UNREACHABLE,
        _ => reason::DISK_OPERATION_FAILED,
    }
}

/// Cross-device rename.
///
/// The bare numbers differ per platform: 18 is `EXDEV` on Unix, while 17 is
/// `ERROR_NOT_SAME_DEVICE` on Windows and `EEXIST` on Unix. Matching both
/// everywhere sent existing-destination failures down copy-and-delete paths.
pub(crate) fn is_cross_device_error(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::CrossesDevices {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_codes_are_distinct_area_prefixed_and_screaming_snake() {
        let mut seen = std::collections::HashSet::new();
        for code in reason::ALL {
            assert!(seen.insert(*code), "{code} repeated");
            assert!(code.starts_with("DISK_"), "{code}");
            assert!(
                code.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
                "{code}"
            );
        }
        for reading in [
            PathReading::NotObserved,
            PathReading::Unreachable {
                observed_at: Utc::now(),
                cause: reason::DISK_PERMISSION_DENIED,
            },
        ] {
            assert!(reason::ALL.contains(&reading.unknown_reason().unwrap()));
        }
    }

    #[test]
    fn only_not_found_is_missing() {
        let at = Utc::now();
        let missing: io::Result<super::super::backend::RawStat> =
            Err(io::ErrorKind::NotFound.into());
        assert_eq!(
            PathReading::from_result(&missing, at),
            PathReading::Missing { observed_at: at }
        );
        for (kind, cause) in [
            (
                io::ErrorKind::PermissionDenied,
                reason::DISK_PERMISSION_DENIED,
            ),
            (io::ErrorKind::TimedOut, reason::DISK_TIMED_OUT),
            (
                io::ErrorKind::StaleNetworkFileHandle,
                reason::DISK_PATH_UNREACHABLE,
            ),
            (io::ErrorKind::Other, reason::DISK_OPERATION_FAILED),
        ] {
            let failed: io::Result<super::super::backend::RawStat> = Err(kind.into());
            let reading = PathReading::from_result(&failed, at);
            assert_eq!(
                reading,
                PathReading::Unreachable {
                    observed_at: at,
                    cause
                },
                "{kind:?}"
            );
            // The reading's own reason stays the same for every cause.
            assert_eq!(
                reading.unknown_reason(),
                Some(reason::DISK_PATH_UNREACHABLE)
            );
            assert_eq!(PathReading::from_result(&failed, at).exists(), None);
        }
    }

    #[test]
    fn failures_classify_by_kind() {
        let path = Path::new("/srv/fixture-library/Invented Title");
        let cases = [
            (io::ErrorKind::NotFound, reason::DISK_PATH_MISSING),
            (
                io::ErrorKind::PermissionDenied,
                reason::DISK_PERMISSION_DENIED,
            ),
            (
                io::ErrorKind::AlreadyExists,
                reason::DISK_DESTINATION_EXISTS,
            ),
            (
                io::ErrorKind::DirectoryNotEmpty,
                reason::DISK_DIRECTORY_NOT_EMPTY,
            ),
            (io::ErrorKind::IsADirectory, reason::DISK_WRONG_KIND),
            (io::ErrorKind::CrossesDevices, reason::DISK_MOVE_NEEDS_COPY),
            (io::ErrorKind::TimedOut, reason::DISK_TIMED_OUT),
            (io::ErrorKind::StorageFull, reason::DISK_NO_SPACE),
            (io::ErrorKind::QuotaExceeded, reason::DISK_NO_SPACE),
            (io::ErrorKind::InvalidData, reason::DISK_OPERATION_FAILED),
            (io::ErrorKind::Other, reason::DISK_OPERATION_FAILED),
        ];
        for (kind, code) in cases {
            let error = DiskError::new(DiskVerb::Remove, path, kind.into());
            assert_eq!(error.reason(), code, "{kind:?}");
            assert_eq!(error.path(), path);
            assert_eq!(io::Error::from(error).kind(), kind);
        }
    }

    /// 17 is `EEXIST` on Unix and `ERROR_NOT_SAME_DEVICE` on Windows. Reading it
    /// as cross-device on Unix sent existing-destination failures into
    /// copy-and-delete, which is how an import could overwrite a file.
    #[test]
    fn cross_device_detection_does_not_confuse_eexist() {
        #[cfg(unix)]
        {
            assert!(is_cross_device_error(&io::Error::from_raw_os_error(
                libc::EXDEV
            )));
            assert!(!is_cross_device_error(&io::Error::from_raw_os_error(
                libc::EEXIST
            )));
        }
        assert!(!is_cross_device_error(&io::Error::from_raw_os_error(5)));
        // What a caller sees is the reason code, never the raw number.
        let moved = |error| DiskError::new(DiskVerb::Move, Path::new("fixture"), error);
        #[cfg(unix)]
        assert_eq!(
            moved(io::Error::from_raw_os_error(libc::EXDEV)).reason(),
            reason::DISK_MOVE_NEEDS_COPY
        );
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        assert_eq!(moved(denied).reason(), reason::DISK_PERMISSION_DENIED);
    }

    #[test]
    fn identity_needs_a_real_inode() {
        assert_eq!(FileIdentity::from_raw(7, 0), None);
        assert_eq!(FileIdentity::from_raw(7, 9), FileIdentity::from_raw(7, 9));
        assert_ne!(FileIdentity::from_raw(7, 9), FileIdentity::from_raw(8, 9));
    }
}
