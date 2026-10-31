//! Creating one new folder from the folder picker.
//!
//! The folder is made with a plain `create_dir`: no explicit mode and no
//! chmod afterwards. The operating system applies the umask Scryer runs with
//! (the launcher sets it from `UMASK`), and the folder belongs to the user
//! Scryer runs as (`PUID`/`PGID` in the container). On Windows it inherits the
//! parent's ACLs.
//!
//! Every refusal carries a `folder_create` reason. Names are checked up front
//! against the strictest common rules (Windows, SMB shares and FAT/exFAT
//! drives), so a name behaves the same on every host. What the operating
//! system refuses is classified once, in [`classify_create_dir_error`], and the
//! raw OS error only reaches the server log.

use std::io;
use std::path::{Path, PathBuf};

use scryer_domain::User;

use crate::error_reasons::folder_create as reason;
use crate::{AppError, AppResult, AppUseCase};

/// Characters Windows, SMB shares and FAT/exFAT drives reject in a name, plus
/// both path separators.
const FORBIDDEN_NAME_CHARACTERS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// No common file system allows a longer name; most count 255 bytes or
/// UTF-16 units, which the drive itself enforces.
const MAX_NAME_CHARS: usize = 255;

/// How often an interrupted `create_dir` is retried before giving up.
const INTERRUPTED_RETRIES: usize = 3;

impl AppUseCase {
    /// Create a folder named `name` inside `parent` for the folder picker,
    /// returning its path. Anyone who may browse folders may create one.
    pub async fn create_folder(
        &self,
        actor: &User,
        parent: &str,
        name: &str,
    ) -> AppResult<PathBuf> {
        self.require_library_settings_read_permission(actor).await?;
        let parent = PathBuf::from(parent);
        let name = name.to_owned();
        match tokio::task::spawn_blocking(move || create_folder_in(&parent, &name)).await {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(%error, "folder creation task did not finish");
                Err(failure_error(CreateFailure::Other))
            }
        }
    }
}

/// Create the folder `name` inside the absolute folder `parent`.
pub(crate) fn create_folder_in(parent: &Path, name: &str) -> AppResult<PathBuf> {
    if !parent.is_absolute() {
        return Err(AppError::validation_refused(
            reason::PARENT_NOT_ABSOLUTE,
            "Choose an absolute folder path.",
        ));
    }
    let name = check_folder_name(name)?;
    let target = parent.join(name);

    let mut retries = 0;
    loop {
        match std::fs::create_dir(&target) {
            Ok(()) => return Ok(target),
            Err(error)
                if error.kind() == io::ErrorKind::Interrupted && retries < INTERRUPTED_RETRIES =>
            {
                retries += 1;
            }
            Err(error) => {
                let failure = classify_create_dir_error(&error);
                match failure {
                    CreateFailure::Other | CreateFailure::DriveUnavailable => tracing::warn!(
                        path = %target.display(),
                        %error,
                        ?failure,
                        "could not create folder"
                    ),
                    _ => tracing::debug!(
                        path = %target.display(),
                        %error,
                        ?failure,
                        "could not create folder"
                    ),
                }
                return Err(failure_error(failure));
            }
        }
    }
}

/// The name to create, trimmed, or the rule it breaks.
fn check_folder_name(raw: &str) -> AppResult<&str> {
    let name = raw.trim();
    let refuse = |code, message: &str| Err(AppError::validation_refused(code, message.to_owned()));
    if name.is_empty() {
        return refuse(reason::NAME_REQUIRED, "Enter a folder name.");
    }
    if name == "." || name == ".." {
        return refuse(
            reason::NAME_RESERVED,
            "That name is reserved and can't be used for a folder.",
        );
    }
    if name
        .chars()
        .any(|ch| ch.is_control() || FORBIDDEN_NAME_CHARACTERS.contains(&ch))
    {
        return refuse(
            reason::NAME_INVALID_CHARACTERS,
            "Folder names can't contain / \\ : * ? \" < > | or control characters.",
        );
    }
    if name.starts_with('.') {
        return refuse(
            reason::NAME_HIDDEN,
            "Folder names can't start with a dot; the folder would be hidden.",
        );
    }
    if name.ends_with('.') {
        return refuse(
            reason::NAME_TRAILING_DOT,
            "Folder names can't end with a dot.",
        );
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return refuse(
            reason::NAME_TOO_LONG,
            "That name is too long for this drive.",
        );
    }
    if is_windows_reserved_name(name) {
        return refuse(
            reason::NAME_RESERVED,
            "That name is reserved and can't be used for a folder.",
        );
    }
    Ok(name)
}

/// Windows device names, which it reserves with any extension and any case:
/// `CON`, `nul.txt` and `com1 .x` are all refused there and on SMB shares.
fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let Some(number) = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
    else {
        return false;
    };
    let mut chars = number.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some('0'..='9' | '¹' | '²' | '³'), None)
    )
}

/// What stopped the operating system from creating the folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreateFailure {
    ParentMissing,
    ParentNotDirectory,
    AlreadyExists,
    PermissionDenied,
    ReadOnly,
    DiskFull,
    FolderFull,
    NameTooLong,
    NameNotAllowed,
    NotSupported,
    DriveUnavailable,
    Other,
}

/// The one place a `create_dir` error is read. The typed `ErrorKind` decides,
/// except for the few OS codes std leaves uncategorized or folds together.
fn classify_create_dir_error(error: &io::Error) -> CreateFailure {
    if let Some(failure) = error.raw_os_error().and_then(classify_raw_os_error) {
        return failure;
    }
    use io::ErrorKind as Kind;
    match error.kind() {
        Kind::NotFound => CreateFailure::ParentMissing,
        Kind::NotADirectory => CreateFailure::ParentNotDirectory,
        Kind::AlreadyExists => CreateFailure::AlreadyExists,
        Kind::PermissionDenied => CreateFailure::PermissionDenied,
        Kind::ReadOnlyFilesystem => CreateFailure::ReadOnly,
        Kind::StorageFull | Kind::QuotaExceeded => CreateFailure::DiskFull,
        Kind::TooManyLinks => CreateFailure::FolderFull,
        // A name the drive's own rules refuse: exFAT, vfat, CIFS and ntfs3
        // mounts answer EINVAL, Windows ERROR_INVALID_NAME.
        Kind::InvalidFilename | Kind::InvalidInput => CreateFailure::NameNotAllowed,
        Kind::Unsupported => CreateFailure::NotSupported,
        Kind::NotConnected
        | Kind::StaleNetworkFileHandle
        | Kind::NetworkDown
        | Kind::NetworkUnreachable
        | Kind::HostUnreachable
        | Kind::TimedOut
        | Kind::ConnectionAborted
        | Kind::ConnectionReset
        | Kind::ResourceBusy => CreateFailure::DriveUnavailable,
        _ => CreateFailure::Other,
    }
}

#[cfg(unix)]
fn classify_raw_os_error(code: i32) -> Option<CreateFailure> {
    match code {
        // std folds this into InvalidFilename with other name errors.
        libc::ENAMETOOLONG => Some(CreateFailure::NameTooLong),
        // Bytes the file system will not store, as on ZFS `utf8only`.
        libc::EILSEQ => Some(CreateFailure::NameNotAllowed),
        // A symlink loop in the path: the folder cannot be reached.
        libc::ELOOP => Some(CreateFailure::ParentMissing),
        // A failing or unplugged disk, or a FUSE mount whose daemon died.
        libc::EIO | libc::ENXIO | libc::ENODEV => Some(CreateFailure::DriveUnavailable),
        #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
        libc::EHOSTDOWN => Some(CreateFailure::DriveUnavailable),
        _ => None,
    }
}

#[cfg(windows)]
fn classify_raw_os_error(code: i32) -> Option<CreateFailure> {
    // Codes std maps to a kind already (access denied, path not found, disk
    // full, write protect, quota, invalid name, ...) are left to it, except
    // where it folds two conditions together or leaves one uncategorized.
    const ERROR_INVALID_FUNCTION: i32 = 1;
    const ERROR_NOT_READY: i32 = 21;
    const ERROR_NOT_SUPPORTED: i32 = 50;
    const ERROR_REM_NOT_LIST: i32 = 51;
    const ERROR_BAD_NETPATH: i32 = 53;
    const ERROR_NETWORK_BUSY: i32 = 54;
    const ERROR_DEV_NOT_EXIST: i32 = 55;
    const ERROR_UNEXP_NET_ERR: i32 = 59;
    const ERROR_NETNAME_DELETED: i32 = 64;
    const ERROR_NETWORK_ACCESS_DENIED: i32 = 65;
    const ERROR_BAD_NET_NAME: i32 = 67;
    const ERROR_FILENAME_EXCED_RANGE: i32 = 206;
    const ERROR_CANT_RESOLVE_FILENAME: i32 = 1921;
    match code {
        // std folds this into InvalidFilename with other name errors.
        ERROR_FILENAME_EXCED_RANGE => Some(CreateFailure::NameTooLong),
        ERROR_NETWORK_ACCESS_DENIED => Some(CreateFailure::PermissionDenied),
        // WebDAV and other redirectors that cannot make folders.
        ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED => Some(CreateFailure::NotSupported),
        // A symlink or junction loop: the folder cannot be reached.
        ERROR_CANT_RESOLVE_FILENAME => Some(CreateFailure::ParentMissing),
        // An empty card reader, an unplugged drive, or a share that is gone.
        // std calls the share codes NotFound, which would blame the folder.
        ERROR_NOT_READY
        | ERROR_REM_NOT_LIST
        | ERROR_BAD_NETPATH
        | ERROR_NETWORK_BUSY
        | ERROR_DEV_NOT_EXIST
        | ERROR_UNEXP_NET_ERR
        | ERROR_NETNAME_DELETED
        | ERROR_BAD_NET_NAME => Some(CreateFailure::DriveUnavailable),
        _ => None,
    }
}

#[cfg(not(any(unix, windows)))]
fn classify_raw_os_error(_code: i32) -> Option<CreateFailure> {
    None
}

fn failure_error(failure: CreateFailure) -> AppError {
    let (code, message) = match failure {
        CreateFailure::ParentMissing => (
            reason::PARENT_MISSING,
            "The folder you're in no longer exists or can't be reached.",
        ),
        CreateFailure::ParentNotDirectory => (
            reason::PARENT_NOT_DIRECTORY,
            "The path you're in is a file, not a folder.",
        ),
        CreateFailure::AlreadyExists => (
            reason::ALREADY_EXISTS,
            "A file or folder with that name is already here. Some drives treat upper- and lower-case names as the same.",
        ),
        CreateFailure::PermissionDenied => (
            reason::PERMISSION_DENIED,
            "Scryer isn't allowed to create folders here. Check the folder's permissions for the user Scryer runs as (PUID and PGID in Docker).",
        ),
        CreateFailure::ReadOnly => (
            reason::READ_ONLY,
            "This drive is read-only. If Scryer runs in Docker, check the volume isn't mounted read-only (:ro).",
        ),
        CreateFailure::DiskFull => (
            reason::DISK_FULL,
            "The drive is full, or its storage quota is used up.",
        ),
        CreateFailure::FolderFull => (
            reason::FOLDER_FULL,
            "This folder already holds as many subfolders as its drive allows.",
        ),
        CreateFailure::NameTooLong => (
            reason::NAME_TOO_LONG,
            "That name is too long for this drive.",
        ),
        CreateFailure::NameNotAllowed => (
            reason::NAME_NOT_ALLOWED,
            "This drive doesn't accept that name. Try a simpler one.",
        ),
        CreateFailure::NotSupported => (
            reason::NOT_SUPPORTED,
            "This location doesn't support creating folders.",
        ),
        CreateFailure::DriveUnavailable => (
            reason::DRIVE_UNAVAILABLE,
            "The drive or network share isn't responding. Check that it's connected and mounted.",
        ),
        CreateFailure::Other => (
            reason::FAILED,
            "Scryer couldn't create the folder. The server log has the details.",
        ),
    };
    AppError::validation_refused(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(result: AppResult<impl std::fmt::Debug>) -> &'static str {
        result
            .expect_err("expected a refusal")
            .validation_reason()
            .expect("refusal carries a reason")
    }

    #[test]
    fn creates_the_folder_with_a_trimmed_name() {
        let dir = tempfile::tempdir().unwrap();
        let created = create_folder_in(dir.path(), "  Season Packs  ").unwrap();
        assert_eq!(created, dir.path().join("Season Packs"));
        assert!(created.is_dir());
    }

    #[test]
    fn names_are_checked_the_same_on_every_host() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            ("", reason::NAME_REQUIRED),
            ("   ", reason::NAME_REQUIRED),
            (".", reason::NAME_RESERVED),
            ("..", reason::NAME_RESERVED),
            ("CON", reason::NAME_RESERVED),
            ("nul.txt", reason::NAME_RESERVED),
            ("com1 .backup", reason::NAME_RESERVED),
            ("LPT²", reason::NAME_RESERVED),
            ("a/b", reason::NAME_INVALID_CHARACTERS),
            ("a\\b", reason::NAME_INVALID_CHARACTERS),
            ("what?", reason::NAME_INVALID_CHARACTERS),
            ("tab\there", reason::NAME_INVALID_CHARACTERS),
            (".cache", reason::NAME_HIDDEN),
            ("ends.", reason::NAME_TRAILING_DOT),
        ];
        for (name, expected) in cases {
            assert_eq!(
                refusal(create_folder_in(dir.path(), name)),
                expected,
                "{name:?}"
            );
        }
        let long = "x".repeat(MAX_NAME_CHARS + 1);
        assert_eq!(
            refusal(create_folder_in(dir.path(), &long)),
            reason::NAME_TOO_LONG
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn device_names_only_match_whole_stems() {
        for name in ["CONSOLE", "com10", "LPT", "Auxiliary", "nul-backups"] {
            assert!(!is_windows_reserved_name(name), "{name}");
        }
    }

    #[test]
    fn a_relative_parent_is_refused() {
        assert_eq!(
            refusal(create_folder_in(Path::new("relative/dir"), "New")),
            reason::PARENT_NOT_ABSOLUTE
        );
    }

    #[test]
    fn an_existing_folder_or_file_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Movies")).unwrap();
        std::fs::write(dir.path().join("notes"), b"keep").unwrap();
        assert_eq!(
            refusal(create_folder_in(dir.path(), "Movies")),
            reason::ALREADY_EXISTS
        );
        assert_eq!(
            refusal(create_folder_in(dir.path(), "notes")),
            reason::ALREADY_EXISTS
        );
        assert_eq!(std::fs::read(dir.path().join("notes")).unwrap(), b"keep");
    }

    #[test]
    fn a_missing_parent_or_a_file_parent_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            refusal(create_folder_in(&dir.path().join("gone"), "New")),
            reason::PARENT_MISSING
        );
        let file = dir.path().join("file");
        std::fs::write(&file, b"").unwrap();
        let code = refusal(create_folder_in(&file, "New"));
        // Windows reports a file in the path as a missing path.
        assert!(
            code == reason::PARENT_NOT_DIRECTORY || cfg!(windows) && code == reason::PARENT_MISSING,
            "{code}"
        );
    }

    #[test]
    fn typed_error_kinds_map_to_their_reasons() {
        use io::ErrorKind as Kind;
        let cases = [
            (Kind::NotFound, CreateFailure::ParentMissing),
            (Kind::NotADirectory, CreateFailure::ParentNotDirectory),
            (Kind::AlreadyExists, CreateFailure::AlreadyExists),
            (Kind::PermissionDenied, CreateFailure::PermissionDenied),
            (Kind::ReadOnlyFilesystem, CreateFailure::ReadOnly),
            (Kind::StorageFull, CreateFailure::DiskFull),
            (Kind::QuotaExceeded, CreateFailure::DiskFull),
            (Kind::TooManyLinks, CreateFailure::FolderFull),
            (Kind::InvalidFilename, CreateFailure::NameNotAllowed),
            (Kind::InvalidInput, CreateFailure::NameNotAllowed),
            (Kind::Unsupported, CreateFailure::NotSupported),
            (Kind::NotConnected, CreateFailure::DriveUnavailable),
            (
                Kind::StaleNetworkFileHandle,
                CreateFailure::DriveUnavailable,
            ),
            (Kind::TimedOut, CreateFailure::DriveUnavailable),
            (Kind::Other, CreateFailure::Other),
        ];
        for (kind, expected) in cases {
            assert_eq!(
                classify_create_dir_error(&io::Error::from(kind)),
                expected,
                "{kind:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_os_errors_map_to_their_reasons() {
        let cases = [
            (libc::ENOENT, CreateFailure::ParentMissing),
            (libc::ENOTDIR, CreateFailure::ParentNotDirectory),
            (libc::EEXIST, CreateFailure::AlreadyExists),
            (libc::EACCES, CreateFailure::PermissionDenied),
            // macOS privacy controls and some FUSE mounts answer EPERM.
            (libc::EPERM, CreateFailure::PermissionDenied),
            (libc::EROFS, CreateFailure::ReadOnly),
            (libc::ENOSPC, CreateFailure::DiskFull),
            (libc::EDQUOT, CreateFailure::DiskFull),
            (libc::EMLINK, CreateFailure::FolderFull),
            (libc::ENAMETOOLONG, CreateFailure::NameTooLong),
            (libc::EINVAL, CreateFailure::NameNotAllowed),
            (libc::EILSEQ, CreateFailure::NameNotAllowed),
            (libc::ELOOP, CreateFailure::ParentMissing),
            (libc::EOPNOTSUPP, CreateFailure::NotSupported),
            (libc::EIO, CreateFailure::DriveUnavailable),
            (libc::ENOTCONN, CreateFailure::DriveUnavailable),
            (libc::ESTALE, CreateFailure::DriveUnavailable),
            (libc::ETIMEDOUT, CreateFailure::DriveUnavailable),
            (libc::ENODEV, CreateFailure::DriveUnavailable),
        ];
        for (code, expected) in cases {
            assert_eq!(
                classify_create_dir_error(&io::Error::from_raw_os_error(code)),
                expected,
                "errno {code}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_os_errors_map_to_their_reasons() {
        let cases = [
            (3, CreateFailure::ParentMissing),
            (5, CreateFailure::PermissionDenied),
            (19, CreateFailure::ReadOnly),
            (21, CreateFailure::DriveUnavailable),
            (53, CreateFailure::DriveUnavailable),
            (64, CreateFailure::DriveUnavailable),
            (65, CreateFailure::PermissionDenied),
            (50, CreateFailure::NotSupported),
            (1921, CreateFailure::ParentMissing),
            (39, CreateFailure::DiskFull),
            (1295, CreateFailure::DiskFull),
            (112, CreateFailure::DiskFull),
            (123, CreateFailure::NameNotAllowed),
            (183, CreateFailure::AlreadyExists),
            (206, CreateFailure::NameTooLong),
            (267, CreateFailure::ParentNotDirectory),
        ];
        for (code, expected) in cases {
            assert_eq!(
                classify_create_dir_error(&io::Error::from_raw_os_error(code)),
                expected,
                "Windows error {code}"
            );
        }
    }

    #[test]
    fn no_refusal_carries_os_error_text() {
        for failure in [
            CreateFailure::ParentMissing,
            CreateFailure::PermissionDenied,
            CreateFailure::DriveUnavailable,
            CreateFailure::Other,
        ] {
            let text = failure_error(failure).to_string();
            assert!(!text.contains("os error"), "{text}");
        }
    }

    // nextest runs each test in its own process, so changing the umask here
    // cannot leak into another test.
    #[cfg(unix)]
    #[test]
    fn the_new_folder_follows_the_process_umask() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for (mask, expected) in [(0o027, 0o750), (0o002, 0o775)] {
            // SAFETY: umask takes a plain mode and has no other preconditions.
            let previous = unsafe { libc::umask(mask) };
            let created = create_folder_in(dir.path(), &format!("mask-{mask:o}"));
            // SAFETY: as above, restoring the previous mask.
            unsafe { libc::umask(previous) };
            let mode = std::fs::metadata(created.unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, expected, "umask {mask:o}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_parent_reports_permission_denied() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            // root writes anywhere; nothing to observe.
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let code = refusal(create_folder_in(&locked, "New"));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(code, reason::PERMISSION_DENIED);
    }
}
