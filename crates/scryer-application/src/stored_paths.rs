use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

const STORED_PATH_PREFIX: &str = "scryer-path-v1:";
const STORED_PATH_UNIX_PREFIX: &str = "scryer-path-v1:u:";
const STORED_PATH_WINDOWS_PREFIX: &str = "scryer-path-v1:w:";

pub fn path_to_stored_string(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    if let Some(value) = path.to_str()
        && !value.starts_with(STORED_PATH_PREFIX)
    {
        return value.to_string();
    }

    encode_path(path)
}

pub fn stored_path_to_path_buf(stored: &str) -> PathBuf {
    decode_path(stored).unwrap_or_else(|| PathBuf::from(stored))
}

pub fn stored_path_to_display_string(stored: &str) -> String {
    if !stored.starts_with(STORED_PATH_PREFIX) {
        return stored.to_string();
    }

    stored_path_to_path_buf(stored)
        .to_string_lossy()
        .into_owned()
}

/// Resolve `.` and `..` without touching the filesystem. One definition for
/// every caller that has to compare two paths the user typed against each
/// other, or against a configured root, before either is known to exist.
pub fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

/// The last segment of a stored path, or the stored path itself when it has
/// none. One definition for every caller that reads a file name back out of a
/// path the catalog stored.
pub fn stored_file_name(stored_path: &str) -> String {
    stored_path_to_path_buf(stored_path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| stored_path.to_string())
}

pub fn folder_path_identity_key(path: &str) -> Option<String> {
    folder_path_identity_key_for_platform(path, cfg!(windows))
}

/// Identity key for any path, folder or file.
///
/// Decodes the stored encoding, collapses path components, and normalizes
/// Unicode to NFC. That last step matters because filesystems disagree about
/// which form they hand back: a name written as NFC comes back decomposed from
/// an SMB share and precomposed from APFS, so comparing the raw strings makes
/// one file look like two and plans a rename that changes nothing.
pub fn path_identity_key(path: &str) -> Option<String> {
    folder_path_identity_key(path)
}

/// Whether two paths name the same location.
pub fn paths_match(left: &str, right: &str) -> bool {
    folder_paths_match(left, right)
}

/// Whether two paths name the same location ignoring case, on every platform.
///
/// Case-insensitive volumes are not a Windows-only concern: APFS and SMB are
/// case-insensitive too, so a rename that only changes case has to be
/// recognized as one wherever it runs.
pub fn paths_match_ignoring_case(left: &str, right: &str) -> bool {
    match (path_identity_key(left), path_identity_key(right)) {
        (Some(left), Some(right)) => left.to_lowercase() == right.to_lowercase(),
        _ => false,
    }
}

pub fn folder_paths_match(left: &str, right: &str) -> bool {
    match (
        folder_path_identity_key(left),
        folder_path_identity_key(right),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

pub(crate) fn stored_path_is_within_folder(folder: &str, path: &str) -> bool {
    if cfg!(windows) {
        let Some(folder) = folder_path_identity_key(folder) else {
            return false;
        };
        let Some(path) = folder_path_identity_key(path) else {
            return false;
        };
        return path == folder
            || path
                .strip_prefix(&folder)
                .is_some_and(|suffix| suffix.starts_with('/'));
    }

    stored_path_to_path_buf(path).starts_with(stored_path_to_path_buf(folder))
}

fn folder_path_identity_key_for_platform(path: &str, windows: bool) -> Option<String> {
    let decoded = stored_path_to_path_buf(path);
    if decoded.as_os_str().is_empty() {
        return None;
    }

    if !windows {
        let normalized = decoded.components().collect::<PathBuf>();
        return Some(normalize_identity_unicode(&path_to_stored_string(
            normalized,
        )));
    }

    let display = decoded.to_string_lossy();
    let mut normalized = String::with_capacity(display.len());
    let mut previous_was_separator = false;
    for character in display.chars() {
        let is_separator = character == '/' || character == '\\';
        if is_separator {
            if !previous_was_separator {
                normalized.push('/');
            }
        } else {
            normalized.push(character);
        }
        previous_was_separator = is_separator;
    }

    while normalized.len() > 1 && normalized.ends_with('/') {
        if normalized.len() == 3 && normalized.as_bytes().get(1) == Some(&b':') {
            break;
        }
        normalized.pop();
    }

    Some(normalize_identity_unicode(&normalized.to_lowercase()))
}

/// Normalizes to NFC, leaving the stored-path escape form untouched: those keys
/// are already ASCII and re-composing them would change their meaning.
fn normalize_identity_unicode(value: &str) -> String {
    if value.starts_with(STORED_PATH_PREFIX) || value.is_ascii() {
        return value.to_string();
    }
    value.nfc().collect()
}

#[cfg(unix)]
fn encode_path(path: &Path) -> String {
    encode_percent_bytes(path.as_os_str().as_bytes(), STORED_PATH_UNIX_PREFIX)
}

#[cfg(windows)]
fn encode_path(path: &Path) -> String {
    let mut encoded = String::from(STORED_PATH_WINDOWS_PREFIX);
    for unit in path.as_os_str().encode_wide() {
        if is_safe_ascii(unit) {
            encoded.push(char::from_u32(unit as u32).unwrap_or_default());
        } else {
            encoded.push_str(&format!("%u{unit:04X}"));
        }
    }
    encoded
}

#[cfg(not(any(unix, windows)))]
fn encode_path(path: &Path) -> String {
    let mut encoded = String::from(STORED_PATH_UNIX_PREFIX);
    encoded.push_str(&path.to_string_lossy());
    encoded
}

fn decode_path(stored: &str) -> Option<PathBuf> {
    if let Some(encoded) = stored.strip_prefix(STORED_PATH_UNIX_PREFIX) {
        return decode_unix_path(encoded);
    }

    if let Some(encoded) = stored.strip_prefix(STORED_PATH_WINDOWS_PREFIX) {
        return decode_windows_path(encoded);
    }

    None
}

fn decode_unix_path(encoded: &str) -> Option<PathBuf> {
    let bytes = decode_percent_bytes(encoded)?;

    #[cfg(unix)]
    {
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }

    #[cfg(not(unix))]
    {
        Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
    }
}

fn decode_windows_path(encoded: &str) -> Option<PathBuf> {
    let units = decode_windows_units(encoded)?;

    #[cfg(windows)]
    {
        Some(PathBuf::from(std::ffi::OsString::from_wide(&units)))
    }

    #[cfg(not(windows))]
    {
        let lossy = String::from_utf16_lossy(&units).replace('\\', "/");
        Some(PathBuf::from(lossy))
    }
}

#[cfg(unix)]
fn encode_percent_bytes(bytes: &[u8], prefix: &str) -> String {
    let mut encoded = String::from(prefix);
    for &byte in bytes {
        if is_safe_ascii(byte as u16) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn decode_percent_bytes(encoded: &str) -> Option<Vec<u8>> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = *bytes.get(index + 1)?;
                let low = *bytes.get(index + 2)?;
                decoded.push((hex_value(high)? << 4) | hex_value(low)?);
                index += 3;
            }
            byte if byte.is_ascii() => {
                decoded.push(byte);
                index += 1;
            }
            _ => return None,
        }
    }

    Some(decoded)
}

fn decode_windows_units(encoded: &str) -> Option<Vec<u16>> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            if bytes.get(index + 1).copied()? != b'u' {
                return None;
            }

            let h0 = u16::from(hex_value(*bytes.get(index + 2)?)?);
            let h1 = u16::from(hex_value(*bytes.get(index + 3)?)?);
            let h2 = u16::from(hex_value(*bytes.get(index + 4)?)?);
            let h3 = u16::from(hex_value(*bytes.get(index + 5)?)?);
            decoded.push((h0 << 12) | (h1 << 8) | (h2 << 4) | h3);
            index += 6;
            continue;
        }

        let byte = *bytes.get(index)?;
        if !byte.is_ascii() {
            return None;
        }
        decoded.push(u16::from(byte));
        index += 1;
    }

    Some(decoded)
}

fn is_safe_ascii(value: u16) -> bool {
    matches!(value, 0x20..=0x7E) && value != u16::from(b'%')
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {

    /// Written precomposed, handed back decomposed by SMB. Both spellings name
    /// one file, so they have to match or the planner renames forever.
    #[test]
    fn paths_match_across_unicode_forms() {
        let nfc = "/Volumes/Media/TV/Pok\u{e9}mon/Pok\u{e9}mon - S20E01.mkv";
        let nfd = "/Volumes/Media/TV/Poke\u{301}mon/Poke\u{301}mon - S20E01.mkv";
        assert_ne!(nfc, nfd);
        assert!(super::paths_match(nfc, nfd));
        assert!(super::paths_match_ignoring_case(nfc, nfd));
    }

    #[test]
    fn paths_match_still_distinguishes_real_differences() {
        assert!(!super::paths_match("/media/one.mkv", "/media/two.mkv"));
        #[cfg(not(windows))]
        assert!(!super::paths_match("/media/One.mkv", "/media/one.mkv"));
        assert!(super::paths_match_ignoring_case(
            "/media/One.mkv",
            "/media/one.mkv"
        ));
    }
    use super::*;

    #[test]
    fn utf8_paths_stay_plain() {
        let path = Path::new("/library/Movie (2024)/Movie.mkv");
        assert_eq!(
            path_to_stored_string(path),
            "/library/Movie (2024)/Movie.mkv"
        );
    }

    #[test]
    fn reserved_prefix_round_trips() {
        let path = Path::new("scryer-path-v1:/library/Movie.mkv");
        let stored = path_to_stored_string(path);

        assert_ne!(stored, "scryer-path-v1:/library/Movie.mkv");
        assert_eq!(stored_path_to_path_buf(&stored), path);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_unix_paths_round_trip() {
        let bytes = b"/library/\xFFmovie.mkv".to_vec();
        let path = PathBuf::from(std::ffi::OsString::from_vec(bytes.clone()));
        let stored = path_to_stored_string(&path);

        assert!(stored.starts_with(STORED_PATH_UNIX_PREFIX));
        assert_eq!(stored_path_to_path_buf(&stored), path);
        assert_eq!(
            stored_path_to_display_string(&stored),
            path.to_string_lossy().into_owned()
        );
    }

    #[cfg(unix)]
    #[test]
    fn windows_paths_decode_lossily_on_unix() {
        let stored = "scryer-path-v1:w:C:\\Media\\%uD800.mkv";
        let decoded = stored_path_to_path_buf(stored);

        assert_eq!(decoded, PathBuf::from("C:/Media/\u{FFFD}.mkv"));
        assert_eq!(
            decoded
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Some("\u{FFFD}.mkv".to_string())
        );
        assert_eq!(
            stored_path_to_display_string(stored),
            "C:/Media/\u{FFFD}.mkv"
        );
    }

    #[test]
    fn folder_identity_normalizes_separators_and_trailing_separators() {
        assert_eq!(
            folder_path_identity_key_for_platform("/library//Show/", false),
            Some("/library/Show".to_string())
        );
        assert_eq!(
            folder_path_identity_key_for_platform(r"C:\\Media\\Show\\", true),
            Some("c:/media/show".to_string())
        );
    }

    #[test]
    fn folder_identity_preserves_case_except_on_windows() {
        assert_ne!(
            folder_path_identity_key_for_platform("/library/Case Split Fixture", false),
            folder_path_identity_key_for_platform("/library/CASE SPLIT FIXTURE", false)
        );
        assert_eq!(
            folder_path_identity_key_for_platform(r"C:\Media\Case Split Fixture", true),
            folder_path_identity_key_for_platform(r"c:/media/CASE SPLIT FIXTURE", true)
        );
    }

    #[test]
    fn posix_folder_identity_preserves_backslashes_and_whitespace() {
        assert_ne!(
            folder_path_identity_key_for_platform(r"/library/Show\Name", false),
            folder_path_identity_key_for_platform("/library/Show/Name", false)
        );
        assert_ne!(
            folder_path_identity_key_for_platform("/library/Show ", false),
            folder_path_identity_key_for_platform("/library/Show", false)
        );
    }

    #[test]
    fn folder_containment_obeys_native_case_rules() {
        let owned = "/library/CASE SPLIT FIXTURE";
        assert!(stored_path_is_within_folder(
            owned,
            "/library/CASE SPLIT FIXTURE/Season 01/E01.mkv"
        ));
        assert_eq!(
            stored_path_is_within_folder(owned, "/library/Case Split Fixture/Season 01/E01.mkv"),
            cfg!(windows)
        );
        assert!(!stored_path_is_within_folder(
            owned,
            "/library/CASE SPLIT FIXTURE 2/E01.mkv"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn non_utf8_windows_paths_round_trip() {
        let path = PathBuf::from(std::ffi::OsString::from_wide(&[
            u16::from(b'C'),
            u16::from(b':'),
            u16::from(b'\\'),
            0xD800,
            u16::from(b'.'),
            u16::from(b'm'),
            u16::from(b'k'),
            u16::from(b'v'),
        ]));
        let stored = path_to_stored_string(&path);

        assert!(stored.starts_with(STORED_PATH_WINDOWS_PREFIX));
        assert_eq!(stored_path_to_path_buf(&stored), path);
    }

    #[cfg(windows)]
    #[test]
    fn unix_paths_decode_lossily_on_windows() {
        let stored = "scryer-path-v1:u:/library/%FFmovie.mkv";
        let decoded = stored_path_to_path_buf(stored);
        let display = stored_path_to_display_string(stored);

        assert_eq!(
            decoded
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Some("\u{FFFD}movie.mkv".to_string())
        );
        assert!(display.contains("\u{FFFD}movie.mkv"));
        assert!(!display.starts_with(STORED_PATH_PREFIX));
    }
}
