//! The key a title folder is unique under, and the rule every write of a
//! title's folder is checked against.
//!
//! The key is decided from the stored path's own shape, never from the host
//! the code runs on, so a catalog keyed on one machine compares the same on
//! every other. A path with a drive letter (`C:`) or a UNC prefix (`\\server`)
//! is a Windows path: its separators are folded to `\` and its case is folded,
//! because Windows treats `Show` and `show` as one folder. Every other path is
//! compared exactly as POSIX does, so `/library/Show` and `/library/show` are
//! two folders. Both kinds drop empty and `.` components, lose trailing
//! separators and trailing spaces, and are compared in Unicode NFC, so the
//! spellings a filesystem hands back for one folder share one key.
//!
//! The stored escape form for names that are not valid text is compared
//! exactly: it is already canonical, and decoding it could merge names the
//! filesystem keeps apart.

use std::collections::{BTreeMap, BTreeSet};

use unicode_normalization::UnicodeNormalization;

use crate::error_reasons::folder_ownership;
use crate::stored_paths::{is_escaped_stored_path, stored_path_to_display_string};
use crate::{AppError, AppResult};

const WINDOWS_SEPARATOR: char = '\\';
const POSIX_SEPARATOR: char = '/';

/// The key a stored title folder is unique under, or `None` for an empty or
/// whitespace-only value, which records no folder at all.
pub fn title_folder_key(stored: &str) -> Option<String> {
    if stored.trim().is_empty() {
        return None;
    }
    if is_escaped_stored_path(stored) {
        return Some(stored.to_string());
    }
    let path = strip_extended_length_prefix(stored);
    let key = if windows_kind(&path) {
        windows_key(&path)
    } else {
        posix_key(&path)
    };
    (!key.is_empty()).then_some(key)
}

/// `\\?\C:\x` names `C:\x` and `\\?\UNC\server\share` names
/// `\\server\share`; the prefix only lifts Win32 path-length limits.
fn strip_extended_length_prefix(path: &str) -> String {
    for prefix in [r"\\?\", "//?/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            if rest.len() >= 4 && rest[..4].eq_ignore_ascii_case(r"UNC\")
                || rest.len() >= 4 && rest[..4].eq_ignore_ascii_case("UNC/")
            {
                return format!(r"\\{}", &rest[4..]);
            }
            return rest.to_string();
        }
    }
    path.to_string()
}

fn has_drive_letter(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || bytes[2] == b'\\' || bytes[2] == b'/')
}

fn windows_kind(path: &str) -> bool {
    has_drive_letter(path) || path.starts_with(r"\\")
}

fn trim_trailing(path: &str, separator: impl Fn(char) -> bool) -> &str {
    path.trim_end_matches(|c: char| c == ' ' || separator(c))
}

fn windows_key(path: &str) -> String {
    let is_separator = |c: char| c == '\\' || c == '/';
    let (anchor, rest) = if has_drive_letter(path) {
        (format!("{}\\", &path[..2]), &path[2..])
    } else {
        (r"\\".to_string(), &path[2..])
    };
    let rest = trim_trailing(rest, is_separator);
    let components = rest
        .split(is_separator)
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>();
    let joined = format!(
        "{anchor}{}",
        components.join(&WINDOWS_SEPARATOR.to_string())
    );
    let composed = joined.nfc().collect::<String>();
    composed.to_lowercase().nfc().collect()
}

fn posix_key(path: &str) -> String {
    let is_separator = |c: char| c == POSIX_SEPARATOR;
    let absolute = path.starts_with(POSIX_SEPARATOR);
    let rest = trim_trailing(path, is_separator);
    let components = rest
        .split(is_separator)
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>();
    let joined = components.join(&POSIX_SEPARATOR.to_string());
    let joined = if absolute {
        format!("{POSIX_SEPARATOR}{joined}")
    } else {
        joined
    };
    joined.nfc().collect()
}

fn key_separator(key: &str) -> Option<char> {
    if is_escaped_stored_path(key) {
        None
    } else if windows_kind(key) {
        Some(WINDOWS_SEPARATOR)
    } else {
        Some(POSIX_SEPARATOR)
    }
}

/// The prefix every key strictly inside `key` starts with.
fn descendant_prefix(key: &str) -> Option<String> {
    let separator = key_separator(key)?;
    Some(if key.ends_with(separator) {
        key.to_string()
    } else {
        format!("{key}{separator}")
    })
}

/// Bounds `[low, high)` that select, by plain bytewise comparison, every key
/// strictly inside `key`. `None` for the escape form, which only ever
/// compares by equality.
pub fn title_folder_key_descendant_bounds(key: &str) -> Option<(String, String)> {
    let prefix = descendant_prefix(key)?;
    let mut high = prefix.clone();
    let last = high.pop()?;
    // `/` and `\` are ASCII, so the next code point is one byte up too.
    high.push(char::from_u32(last as u32 + 1)?);
    Some((prefix, high))
}

/// Every key `key` lies strictly inside, nearest last.
pub fn title_folder_key_ancestors(key: &str) -> Vec<String> {
    let Some(separator) = key_separator(key) else {
        return Vec::new();
    };
    let anchor_len = if has_drive_letter(key) {
        3
    } else if key.starts_with(r"\\") {
        2
    } else if key.starts_with(POSIX_SEPARATOR) {
        1
    } else {
        0
    };
    let mut ancestors = Vec::new();
    if anchor_len > 0 && key.len() > anchor_len && !key.starts_with(r"\\") {
        ancestors.push(key[..anchor_len].to_string());
    }
    for (index, _) in key.match_indices(separator) {
        if index >= anchor_len {
            ancestors.push(key[..index].to_string());
        }
    }
    ancestors
}

/// How one title folder key sits relative to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleFolderKeyRelation {
    /// The two keys name one folder.
    Same,
    /// The first folder lies inside the second.
    Inside,
    /// The second folder lies inside the first.
    Contains,
}

/// Where `key` sits relative to `other`, or `None` when the two folders are
/// disjoint.
pub fn title_folder_key_relation(key: &str, other: &str) -> Option<TitleFolderKeyRelation> {
    if key == other {
        return Some(TitleFolderKeyRelation::Same);
    }
    if descendant_prefix(other).is_some_and(|prefix| key.starts_with(&prefix)) {
        return Some(TitleFolderKeyRelation::Inside);
    }
    if descendant_prefix(key).is_some_and(|prefix| other.starts_with(&prefix)) {
        return Some(TitleFolderKeyRelation::Contains);
    }
    None
}

/// What a title's folder must currently be for a folder write to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedTitleFolder {
    /// Apply whatever the title's folder is now.
    Unchecked,
    /// Apply only while the title's folder is this one; `None` means the
    /// title records no folder. Compared by key.
    Exactly(Option<String>),
}

/// One change to one title's folder. A batch of writes is applied as one
/// unit: either every write lands or none does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleFolderWrite {
    pub title_id: String,
    /// The folder to record; `None` clears the title's folder.
    pub folder: Option<String>,
    pub expected_current: ExpectedTitleFolder,
}

impl TitleFolderWrite {
    /// Record `folder` for the title, whatever it records now.
    pub fn assign(title_id: impl Into<String>, folder: impl Into<String>) -> Self {
        Self {
            title_id: title_id.into(),
            folder: Some(folder.into()),
            expected_current: ExpectedTitleFolder::Unchecked,
        }
    }

    /// Clear the title's folder, whatever it records now.
    pub fn clear(title_id: impl Into<String>) -> Self {
        Self {
            title_id: title_id.into(),
            folder: None,
            expected_current: ExpectedTitleFolder::Unchecked,
        }
    }

    /// Apply only while the title's folder is still `current`.
    pub fn expecting(mut self, current: Option<String>) -> Self {
        self.expected_current = ExpectedTitleFolder::Exactly(current);
        self
    }
}

/// A title whose recorded folder a write could collide with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleFolderOccupant {
    pub title_id: String,
    pub title_name: String,
    pub folder_path: String,
    pub key: String,
}

/// A checked write, with the key the store records beside the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTitleFolderWrite {
    pub title_id: String,
    pub folder_path: Option<String>,
    pub key: Option<String>,
}

/// Check a batch of title folder writes against the folders other titles
/// record and the configured library roots, and return what to store.
///
/// `current_folders` holds the recorded folder of every title being written;
/// a write for a title missing from it is refused as not found. `occupants`
/// may include the titles being written: their current folders are about to
/// change, so they are judged by what the batch writes instead.
pub fn plan_title_folder_writes(
    writes: &[TitleFolderWrite],
    current_folders: &BTreeMap<String, Option<String>>,
    occupants: &[TitleFolderOccupant],
    library_root_paths: &[String],
) -> AppResult<Vec<PlannedTitleFolderWrite>> {
    let mut written = BTreeSet::new();
    let mut planned = Vec::with_capacity(writes.len());
    for write in writes {
        if !written.insert(write.title_id.as_str()) {
            return Err(AppError::Validation(format!(
                "title {} appears twice in one folder write",
                write.title_id
            )));
        }
        let current = current_folders
            .get(&write.title_id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", write.title_id)))?;
        if let ExpectedTitleFolder::Exactly(expected) = &write.expected_current {
            let expected_key = expected.as_deref().and_then(title_folder_key);
            let current_key = current.as_deref().and_then(title_folder_key);
            if expected_key != current_key {
                return Err(AppError::validation_refused(
                    folder_ownership::TITLE_FOLDER_CHANGED,
                    format!(
                        "the folder recorded for title {} changed before this write",
                        write.title_id
                    ),
                ));
            }
        }
        let key = match write.folder.as_deref() {
            None => None,
            Some(folder) => Some(title_folder_key(folder).ok_or_else(|| {
                AppError::validation_refused(
                    folder_ownership::TITLE_FOLDER_PATH_REQUIRED,
                    "title folder path is required",
                )
            })?),
        };
        planned.push(PlannedTitleFolderWrite {
            title_id: write.title_id.clone(),
            folder_path: write.folder.clone(),
            key,
        });
    }

    for (index, write) in planned.iter().enumerate() {
        let (Some(key), Some(folder)) = (write.key.as_deref(), write.folder_path.as_deref()) else {
            continue;
        };
        for root in library_root_paths {
            let Some(root_key) = title_folder_key(root) else {
                continue;
            };
            match title_folder_key_relation(key, &root_key) {
                Some(TitleFolderKeyRelation::Same) => {
                    return Err(AppError::validation_refused(
                        folder_ownership::CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER,
                        format!(
                            "{} is a library root, not a title folder",
                            stored_path_to_display_string(folder)
                        ),
                    ));
                }
                Some(TitleFolderKeyRelation::Contains) => {
                    return Err(AppError::validation_refused(
                        folder_ownership::CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER,
                        format!(
                            "{} holds a library root, so it cannot be a title folder",
                            stored_path_to_display_string(folder)
                        ),
                    ));
                }
                Some(TitleFolderKeyRelation::Inside) | None => {}
            }
        }
        for other in planned.iter().skip(index + 1) {
            if let Some(other_key) = other.key.as_deref()
                && let Some(relation) = title_folder_key_relation(key, other_key)
            {
                return Err(conflict_refusal(folder, relation, &other.title_id));
            }
        }
        for occupant in occupants {
            if written.contains(occupant.title_id.as_str()) {
                continue;
            }
            if let Some(relation) = title_folder_key_relation(key, &occupant.key) {
                return Err(conflict_refusal(folder, relation, &occupant.title_name));
            }
        }
    }
    Ok(planned)
}

fn conflict_refusal(folder: &str, relation: TitleFolderKeyRelation, owner: &str) -> AppError {
    let folder = stored_path_to_display_string(folder);
    match relation {
        TitleFolderKeyRelation::Same => AppError::validation_refused(
            folder_ownership::FOLDER_ALREADY_OWNED_TITLE,
            format!("folder {folder} is already owned by title {owner}"),
        ),
        TitleFolderKeyRelation::Inside | TitleFolderKeyRelation::Contains => {
            AppError::validation_refused(
                folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE,
                format!("folder {folder} overlaps the folder owned by title {owner}"),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(path: &str) -> String {
        title_folder_key(path).unwrap_or_else(|| panic!("{path:?} has no key"))
    }

    #[test]
    fn empty_and_whitespace_folders_have_no_key() {
        assert_eq!(title_folder_key(""), None);
        assert_eq!(title_folder_key("   "), None);
        assert_eq!(title_folder_key("\t\n"), None);
    }

    #[test]
    fn posix_keys_keep_case() {
        assert_ne!(key("/library/Emberfall"), key("/library/emberfall"));
        assert_eq!(key("/library/Emberfall"), "/library/Emberfall");
    }

    #[test]
    fn windows_keys_fold_case_and_separators_on_every_host() {
        assert_eq!(key(r"C:\Media\Emberfall"), key("c:/media/EMBERFALL"));
        assert_eq!(key(r"C:\Media\Emberfall"), r"c:\media\emberfall");
        assert_eq!(
            key(r"\\NAS\Share\Emberfall"),
            key(r"\\nas\share\emberfall\")
        );
    }

    #[test]
    fn trailing_separators_spaces_and_dot_components_collapse() {
        for spelling in [
            "/library/Emberfall/",
            "/library/Emberfall//",
            "/library/Emberfall ",
            "/library//./Emberfall",
            "/library/Emberfall/ /",
        ] {
            assert_eq!(key(spelling), "/library/Emberfall", "{spelling:?}");
        }
        assert_eq!(key(r"D:\TV\.\Cinderwake\\"), r"d:\tv\cinderwake");
    }

    #[test]
    fn unicode_forms_share_one_key() {
        assert_eq!(
            key("/library/Cinde\u{301}rwake"),
            key("/library/Cind\u{e9}rwake")
        );
        assert_eq!(
            key("C:\\Media\\CIND\u{c9}RWAKE"),
            key("c:\\media\\cinde\u{301}rwake")
        );
    }

    #[test]
    fn extended_length_prefixes_name_the_plain_path() {
        assert_eq!(key(r"\\?\C:\Media\Emberfall"), key(r"C:\Media\Emberfall"));
        assert_eq!(
            key(r"\\?\UNC\nas\share\Emberfall"),
            key(r"\\nas\share\Emberfall")
        );
    }

    #[test]
    fn roots_keep_their_separator() {
        assert_eq!(key("/"), "/");
        assert_eq!(key("C:"), r"c:\");
        assert_eq!(key(r"C:\"), r"c:\");
    }

    #[test]
    fn posix_backslashes_are_part_of_a_name() {
        assert_ne!(key(r"/library/Ember\fall"), key("/library/Ember/fall"));
    }

    #[test]
    fn escape_form_is_kept_exactly() {
        let escaped = "scryer-path-v1:u:/library/%FFEmberfall";
        assert_eq!(key(escaped), escaped);
    }

    #[test]
    fn relation_finds_equality_and_both_nesting_directions() {
        let outer = key("/library/Emberfall");
        let inner = key("/library/Emberfall/Season 01");
        let sibling = key("/library/Emberfall 2");
        assert_eq!(
            title_folder_key_relation(&outer, &outer),
            Some(TitleFolderKeyRelation::Same)
        );
        assert_eq!(
            title_folder_key_relation(&inner, &outer),
            Some(TitleFolderKeyRelation::Inside)
        );
        assert_eq!(
            title_folder_key_relation(&outer, &inner),
            Some(TitleFolderKeyRelation::Contains)
        );
        assert_eq!(title_folder_key_relation(&outer, &sibling), None);
        assert_eq!(
            title_folder_key_relation(&key("/library/x"), &key("/")),
            Some(TitleFolderKeyRelation::Inside)
        );
    }

    #[test]
    fn ancestors_and_descendant_bounds_match_the_relation() {
        assert_eq!(
            title_folder_key_ancestors("/library/Emberfall/Season 01"),
            vec!["/", "/library", "/library/Emberfall"]
        );
        assert_eq!(
            title_folder_key_ancestors(r"c:\media\emberfall"),
            vec![r"c:\", r"c:\media"]
        );
        assert_eq!(
            title_folder_key_ancestors(r"\\nas\share\emberfall"),
            vec![r"\\nas", r"\\nas\share"]
        );
        assert!(title_folder_key_ancestors("/").is_empty());
        assert!(title_folder_key_ancestors(r"c:\").is_empty());

        let (low, high) = title_folder_key_descendant_bounds("/library/Emberfall").unwrap();
        assert_eq!(low, "/library/Emberfall/");
        assert_eq!(high, "/library/Emberfall0");
        let inside = "/library/Emberfall/Season 01";
        assert!(inside >= low.as_str() && inside < high.as_str());
        let sibling = "/library/Emberfall 2";
        assert!(!(sibling >= low.as_str() && sibling < high.as_str()));

        let (low, high) = title_folder_key_descendant_bounds(r"c:\").unwrap();
        assert_eq!((low.as_str(), high.as_str()), (r"c:\", "c:]"));
    }

    fn occupant(id: &str, folder: &str) -> TitleFolderOccupant {
        TitleFolderOccupant {
            title_id: id.into(),
            title_name: id.into(),
            folder_path: folder.into(),
            key: key(folder),
        }
    }

    fn current(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        entries
            .iter()
            .map(|(id, folder)| (id.to_string(), folder.map(str::to_string)))
            .collect()
    }

    #[test]
    fn plan_refuses_conflicts_and_accepts_a_swap() {
        let roots = vec!["/library".to_string()];
        let occupants = vec![
            occupant("emberfall", "/library/Emberfall"),
            occupant("cinderwake", "/library/Cinderwake"),
        ];
        let folders = current(&[
            ("emberfall", Some("/library/Emberfall")),
            ("cinderwake", Some("/library/Cinderwake")),
            ("ashgrove", None),
        ]);
        let reason = |result: AppResult<Vec<PlannedTitleFolderWrite>>| {
            result.unwrap_err().validation_reason().map(str::to_string)
        };

        assert_eq!(
            reason(plan_title_folder_writes(
                &[TitleFolderWrite::assign("ashgrove", "/library/Emberfall/")],
                &folders,
                &occupants,
                &roots,
            )),
            Some(folder_ownership::FOLDER_ALREADY_OWNED_TITLE.to_string())
        );
        assert_eq!(
            reason(plan_title_folder_writes(
                &[TitleFolderWrite::assign(
                    "ashgrove",
                    "/library/Emberfall/Extras"
                )],
                &folders,
                &occupants,
                &roots,
            )),
            Some(folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE.to_string())
        );
        assert_eq!(
            reason(plan_title_folder_writes(
                &[TitleFolderWrite::assign("ashgrove", "/library/")],
                &folders,
                &[],
                &roots,
            )),
            Some(folder_ownership::CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER.to_string())
        );
        assert_eq!(
            reason(plan_title_folder_writes(
                &[TitleFolderWrite::assign("ashgrove", "/library/Ashgrove")
                    .expecting(Some("/library/Elsewhere".into()))],
                &folders,
                &occupants,
                &roots,
            )),
            Some(folder_ownership::TITLE_FOLDER_CHANGED.to_string())
        );

        let swap = plan_title_folder_writes(
            &[
                TitleFolderWrite::assign("emberfall", "/library/Cinderwake")
                    .expecting(Some("/library/Emberfall".into())),
                TitleFolderWrite::assign("cinderwake", "/library/Emberfall")
                    .expecting(Some("/library/Cinderwake".into())),
            ],
            &folders,
            &occupants,
            &roots,
        )
        .expect("a swap frees each folder for the other");
        assert_eq!(swap.len(), 2);

        assert_eq!(
            reason(plan_title_folder_writes(
                &[
                    TitleFolderWrite::assign("emberfall", "/library/Shared"),
                    TitleFolderWrite::assign("cinderwake", "/library/Shared/Inner"),
                ],
                &folders,
                &occupants,
                &roots,
            )),
            Some(folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE.to_string())
        );
    }
}
