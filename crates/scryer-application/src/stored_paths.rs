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

/// The stored-path spellings a repository can compare by plain equality to
/// narrow a folder-ownership lookup to the rows [`folder_paths_match`] would
/// accept for `folder_path`.
///
/// This is a filter, not a replacement for the matcher: the caller still runs
/// `folder_paths_match` over whatever comes back, so a row this set lets
/// through is never accepted on the strength of the equality alone. It covers
/// the spellings the application actually writes for one folder — the value as
/// given, its component-normalized form, the NFC and NFD forms of both, and
/// each of those with a trailing separator.
pub fn folder_path_match_candidates(folder_path: &str) -> Vec<String> {
    // An empty path has no identity key, so `folder_paths_match` rejects every
    // row against it; there is nothing to look up.
    if folder_path.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::<String>::new();
    let mut push = |value: String| {
        if !value.is_empty() && !candidates.contains(&value) {
            candidates.push(value);
        }
    };

    let mut roots = vec![folder_path.to_string()];
    let component_normalized = path_to_stored_string(
        stored_path_to_path_buf(folder_path)
            .components()
            .collect::<PathBuf>(),
    );
    if !roots.contains(&component_normalized) {
        roots.push(component_normalized);
    }

    for root in roots {
        for form in unicode_identity_forms(&root) {
            for separator in folder_path_trailing_separators() {
                if !form.ends_with(*separator) {
                    push(format!("{form}{separator}"));
                }
            }
            push(form);
        }
    }
    candidates
}

/// The fold a repository applies to both sides of a narrowed folder lookup so
/// that plain equality still accepts every spelling [`folder_paths_match`]
/// accepts.
///
/// On Windows the matcher treats `/` and `\` as one separator and ignores case,
/// so raw equality against [`folder_path_match_candidates`] misses a stored
/// `C:\Media\Show` when a scan supplies `c:/media/show` — and a missed owner
/// lets a second title claim an owned folder. Folding both the stored value and
/// the candidates through this function restores the equivalence. Off Windows
/// the matcher is case- and separator-sensitive, so the fold is the identity and
/// the lookup keeps using the stored spelling exactly.
///
/// The repository's SQL twin of this is `lower(replace(folder_path, '/', '\'))`,
/// which both dialects understand; it is only applied to plain stored paths,
/// because the escape form is compared by exact equality instead.
pub fn folder_path_lookup_key(folder_path: &str) -> String {
    folder_path_lookup_key_for_platform(folder_path, cfg!(windows))
}

/// [`folder_path_lookup_key`] with the platform rule chosen explicitly, so the
/// Windows rule can be asserted and applied from any host.
pub fn folder_path_lookup_key_for_platform(folder_path: &str, windows: bool) -> String {
    if !windows {
        return folder_path.to_string();
    }
    folder_path.replace('/', "\\").to_lowercase()
}

/// Whether a stored path is the escape form, which the fold must not touch.
pub fn is_escaped_stored_path(stored: &str) -> bool {
    stored.starts_with(STORED_PATH_PREFIX)
}

/// The escape form is ASCII by construction and re-composing it would change
/// its meaning, so it is never re-normalized.
fn unicode_identity_forms(value: &str) -> Vec<String> {
    if value.starts_with(STORED_PATH_PREFIX) || value.is_ascii() {
        return vec![value.to_string()];
    }
    let mut forms = vec![value.to_string()];
    for form in [
        value.nfc().collect::<String>(),
        value.nfd().collect::<String>(),
    ] {
        if !forms.contains(&form) {
            forms.push(form);
        }
    }
    forms
}

const fn folder_path_trailing_separators() -> &'static [char] {
    if cfg!(windows) { &['/', '\\'] } else { &['/'] }
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

    /// Every candidate has to name the same folder as the value it came from,
    /// or the narrowed lookup would hand the caller rows it then rejects.
    #[test]
    fn folder_path_candidates_all_match_the_folder_they_came_from() {
        for folder in [
            "/media/movies/Arrival (2016)",
            "/media/movies/Arrival (2016)/",
            "/media//movies/./Arrival (2016)",
            "/Volumes/Media/TV/Pok\u{e9}mon",
            "/Volumes/Media/TV/Poke\u{301}mon",
        ] {
            let candidates = super::folder_path_match_candidates(folder);
            assert!(
                candidates.contains(&folder.to_string()),
                "{folder} is not among its own candidates"
            );
            for candidate in &candidates {
                assert!(
                    super::folder_paths_match(candidate, folder),
                    "candidate {candidate} does not match {folder}"
                );
            }
        }
    }

    /// The spellings one folder actually reaches the store as: written with a
    /// trailing separator, with a redundant component, or in the other unicode
    /// form. Each has to be reachable from the others' candidate sets.
    #[test]
    fn folder_path_candidates_cover_the_spellings_of_one_folder() {
        let canonical = "/media/movies/Arrival (2016)";
        for stored in [
            "/media/movies/Arrival (2016)/",
            "/media/movies/./Arrival (2016)",
        ] {
            assert!(
                super::folder_path_match_candidates(canonical).contains(&stored.to_string())
                    || super::folder_path_match_candidates(stored).contains(&canonical.to_string()),
                "{stored} is not reachable from {canonical}"
            );
        }

        let nfc = "/Volumes/Media/TV/Pok\u{e9}mon";
        let nfd = "/Volumes/Media/TV/Poke\u{301}mon";
        assert!(super::folder_path_match_candidates(nfc).contains(&nfd.to_string()));
        assert!(super::folder_path_match_candidates(nfd).contains(&nfc.to_string()));
    }

    /// The narrowed folder-ownership lookup has to reach every row
    /// `folder_paths_match` would accept. On Windows the matcher lowercases and
    /// treats `/` and `\` as one separator, so a title stored as `C:\Media\Show`
    /// has to be found when a scan or a move supplies `c:/media/show`; missing
    /// it reports no owner and lets another title claim an owned folder.
    ///
    /// The Windows rule is asserted through the platform-parameterized
    /// functions, so this runs on every host.
    #[test]
    fn windows_folder_lookup_keys_span_case_and_separator_spellings() {
        let stored = r"C:\Media\Show";
        let scanned = "c:/media/show";

        // What the matcher already accepts.
        assert_eq!(
            super::folder_path_identity_key_for_platform(stored, true),
            super::folder_path_identity_key_for_platform(scanned, true),
        );

        // What the narrowing has to preserve, from either spelling.
        for (query, other) in [(scanned, stored), (stored, scanned)] {
            let keys = super::folder_path_match_candidates(query)
                .iter()
                .map(|candidate| super::folder_path_lookup_key_for_platform(candidate, true))
                .collect::<Vec<_>>();
            assert!(
                keys.contains(&super::folder_path_lookup_key_for_platform(other, true)),
                "lookup keys for {query} ({keys:?}) do not reach {other}"
            );
        }
    }

    /// The Windows matcher folds case with full Unicode rules and both Unicode
    /// normal forms, so a repository narrowing cannot rely on SQL `lower()`
    /// (ASCII-only in sqlite) for non-ASCII folders; the title store adds a
    /// non-ASCII arm for exactly these pairs.
    #[test]
    fn windows_folder_matcher_folds_non_ascii_case_and_normal_form() {
        let scanned = "c:/m\u{e9}dia/show";
        for stored in ["C:\\M\u{c9}DIA\\Show", "C:\\Me\u{301}dia\\Show"] {
            assert_eq!(
                super::folder_path_identity_key_for_platform(stored, true),
                super::folder_path_identity_key_for_platform(scanned, true),
                "{stored:?}"
            );
        }
        assert_ne!(
            super::folder_path_identity_key_for_platform(r"C:\Media\Show", true),
            super::folder_path_identity_key_for_platform(scanned, true),
        );
    }

    /// The fold widens the lookup only where the matcher is already lenient.
    #[test]
    fn posix_folder_lookup_keys_keep_case_and_separators() {
        assert_eq!(
            super::folder_path_lookup_key_for_platform(r"C:\Media\Show", false),
            r"C:\Media\Show"
        );
        assert_ne!(
            super::folder_path_lookup_key_for_platform("/library/Show", false),
            super::folder_path_lookup_key_for_platform("/library/show", false),
        );
        assert_ne!(
            super::folder_path_lookup_key_for_platform(r"/library/Show\Name", false),
            super::folder_path_lookup_key_for_platform("/library/Show/Name", false),
        );
    }

    /// Folding still has to separate folders the matcher separates, or the
    /// narrowed read hands the caller an unrelated library's folder.
    #[test]
    fn windows_folder_lookup_keys_exclude_other_folders() {
        let keys = super::folder_path_match_candidates(r"C:\Media\Show")
            .iter()
            .map(|candidate| super::folder_path_lookup_key_for_platform(candidate, true))
            .collect::<Vec<_>>();
        assert!(!keys.contains(&super::folder_path_lookup_key_for_platform(
            r"C:\Media\Show 2",
            true
        )));
        assert!(!keys.contains(&super::folder_path_lookup_key_for_platform(
            r"C:\Media",
            true
        )));
    }

    #[test]
    fn folder_path_candidates_exclude_other_folders() {
        let candidates = super::folder_path_match_candidates("/media/movies/Arrival (2016)");
        assert!(!candidates.contains(&"/media/movies/Arrival (2017)".to_string()));
        assert!(!candidates.contains(&"/media/movies".to_string()));
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
