//! Where a title's recorded folder may point, relative to the configured
//! library roots.
//!
//! A title folder is a directory strictly inside one of its library's roots.
//! It is never a root itself, never a directory that contains a root, and
//! never somewhere outside every root of its library. Each of those values
//! turns an operation scoped to "the title's folder" into one scoped to a
//! whole library, so writers refuse to record them and readers that select
//! files by folder refuse to act on them.
//!
//! Everything here is pure string work on stored paths, so the same rule
//! serves the application, the location planner and data repairs that run
//! without a filesystem.

use scryer_domain::normalize_library_root_path;

use crate::catalog_workflow::library_path_is_under_root;
use crate::stored_paths::{
    is_escaped_stored_path, stored_path_is_within_folder, stored_path_to_display_string,
};

/// Why a folder cannot be a title's folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleFolderRootViolation {
    /// The folder is a configured library root.
    EqualsRoot,
    /// A configured library root lies inside the folder.
    ContainsRoot,
    /// The folder is not inside any root of the title's library.
    OutsideLibraryRoots,
}

impl TitleFolderRootViolation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EqualsRoot => "equals_library_root",
            Self::ContainsRoot => "contains_library_root",
            Self::OutsideLibraryRoots => "outside_library_roots",
        }
    }
}

fn windows_path_syntax(path: &str) -> bool {
    cfg!(windows)
        || path.starts_with(r"\\")
        || (path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic())
}

/// Whether a path spells a `.` or `..` component in either separator.
pub(crate) fn path_has_dot_segments(path: &str) -> bool {
    let windows = windows_path_syntax(path);
    path.split(|c| c == '/' || (windows && c == '\\'))
        .any(|segment| segment == "." || segment == "..")
}

/// `path` with its `.` and `..` components resolved lexically, so a folder
/// spelled `/media/tv/Show/..` compares as the `/media/tv` it names. A `..`
/// never climbs above a filesystem, drive or share root. Paths without such
/// components come back untouched.
fn collapse_dot_segments(path: &str) -> std::borrow::Cow<'_, str> {
    if !path_has_dot_segments(path) {
        return std::borrow::Cow::Borrowed(path);
    }
    let windows = windows_path_syntax(path);
    let is_path_separator = |c| c == '/' || (windows && c == '\\');
    let separator = path.chars().find(|c| is_path_separator(*c)).unwrap_or('/');
    let bytes = path.as_bytes();
    let starts_with_separator = |index: usize| {
        bytes
            .get(index)
            .is_some_and(|byte| is_path_separator(*byte as char))
    };
    // The anchor a `..` cannot climb past: a share (`\\server\share`), a
    // drive (`D:\`), the filesystem root, or nothing for a relative path.
    let (anchor, rest) = if starts_with_separator(0) && starts_with_separator(1) {
        let mut segments = path[2..].splitn(3, is_path_separator);
        let server = segments.next().unwrap_or_default();
        let share = segments.next().unwrap_or_default();
        let rest = segments.next().unwrap_or_default();
        (
            format!("{separator}{separator}{server}{separator}{share}"),
            rest,
        )
    } else if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let rest = path[2..].trim_start_matches(is_path_separator);
        (format!("{}{separator}", &path[..2]), rest)
    } else if starts_with_separator(0) {
        (separator.to_string(), &path[1..])
    } else {
        (String::new(), path)
    };
    let anchored = !anchor.is_empty();
    let mut kept: Vec<&str> = Vec::new();
    for segment in rest.split(is_path_separator) {
        match segment {
            "" | "." => {}
            ".." => {
                if kept.last().is_some_and(|last| *last != "..") {
                    kept.pop();
                } else if !anchored {
                    kept.push(segment);
                }
            }
            _ => kept.push(segment),
        }
    }
    let joined = kept.join(&separator.to_string());
    if anchor.is_empty() {
        return std::borrow::Cow::Owned(joined);
    }
    let needs_separator = !joined.is_empty() && !anchor.ends_with(separator);
    std::borrow::Cow::Owned(if needs_separator {
        format!("{anchor}{separator}{joined}")
    } else {
        format!("{anchor}{joined}")
    })
}

/// A stored path decoded for display with its dot components resolved.
fn resolved_display(path: &str) -> String {
    collapse_dot_segments(&stored_path_to_display_string(path)).into_owned()
}

/// The comparable spelling of a stored path: the escape form decoded for
/// display, dot components resolved, then normalized the way configured roots
/// are.
fn comparable(path: &str) -> String {
    let path = resolved_display(path);
    if windows_path_syntax(&path) {
        use unicode_normalization::UnicodeNormalization;
        normalize_library_root_path(&path)
            .replace('\\', "/")
            .nfc()
            .collect()
    } else {
        crate::stored_paths::path_identity_key(&path).unwrap_or_default()
    }
}

/// Whether `path` is `folder` or lies beneath it, across both separator
/// spellings, so a Windows-style record compares the same on every host.
/// Dot components are resolved first, so `/media/tv/Show/..` is the root
/// itself and `/media/tv/../movies` is not under `/media/tv` at all.
fn under_or_equal(path: &str, folder: &str) -> bool {
    if is_escaped_stored_path(path) || is_escaped_stored_path(folder) {
        return crate::stored_paths::stored_path_is_lexically_within_folder(folder, path);
    }
    let path = comparable(path);
    let folder = comparable(folder);
    !folder.is_empty()
        && (path == folder
            || path
                .strip_prefix(&folder)
                .is_some_and(|tail| folder.ends_with('/') || tail.starts_with('/')))
}

/// [`under_or_equal`] on the paths exactly as spelled.
fn spelled_under_or_equal(path: &str, folder: &str) -> bool {
    let path = stored_path_to_display_string(path);
    let folder = stored_path_to_display_string(folder);
    if windows_path_syntax(&path) || windows_path_syntax(&folder) {
        library_path_is_under_root(&path, &folder)
    } else {
        stored_path_is_within_folder(&folder, &path)
    }
}

fn same_location(left: &str, right: &str) -> bool {
    if is_escaped_stored_path(left) || is_escaped_stored_path(right) {
        return crate::stored_paths::stored_path_is_lexically_within_folder(left, right)
            && crate::stored_paths::stored_path_is_lexically_within_folder(right, left);
    }
    let left = comparable(left);
    !left.is_empty() && left == comparable(right)
}

/// Whether `path` lies strictly beneath `folder`.
pub fn path_is_strictly_within(path: &str, folder: &str) -> bool {
    under_or_equal(path, folder) && !same_location(path, folder)
}

/// Whether a stored file path lies inside a stored folder (or is it).
///
/// Plain stored paths are compared separator-agnostically; the escape form
/// for names that are not UTF-8 falls back to native path containment, which
/// is exact for it. A path that is inside the folder either as spelled or
/// with its dot components resolved counts as inside, so callers that keep
/// what is inside a folder never keep less than before dots were resolved.
pub fn stored_path_is_inside_folder(folder: &str, path: &str) -> bool {
    if is_escaped_stored_path(folder) || is_escaped_stored_path(path) {
        return stored_path_is_within_folder(folder, path);
    }
    under_or_equal(path, folder) || spelled_under_or_equal(path, folder)
}

/// The key a plain stored folder is compared under, for callers that index
/// many folders instead of comparing them pairwise. `None` for an empty path
/// and for the escape form, which is only ever compared natively.
pub fn folder_containment_key(folder: &str) -> Option<String> {
    if is_escaped_stored_path(folder) {
        return None;
    }
    let key = comparable(folder);
    (!key.is_empty()).then_some(key)
}

/// The key of every folder a plain stored path is, or lies inside: `folder`
/// holds `path` exactly when [`folder_containment_key`] of `folder` is one of
/// these. Empty for the escape form.
pub fn containing_folder_keys(path: &str) -> Vec<String> {
    let Some(key) = folder_containment_key(path) else {
        return Vec::new();
    };
    let separator = '/';
    let mut keys = Vec::new();
    for (index, _) in key.match_indices(separator) {
        if index > 0 {
            keys.push(key[..index].to_string());
        }
        // A filesystem or drive root keeps its separator when normalized.
        keys.push(key[..index + separator.len_utf8()].to_string());
    }
    if keys.last() != Some(&key) {
        keys.push(key);
    }
    keys
}

/// Check `folder` against the roots it must sit inside.
///
/// `library_roots` are the roots of the title's own library; `all_roots` are
/// every configured root of every library, because a folder that is another
/// library's root is just as wrong as one that is its own. An empty folder has
/// nothing to check and passes; callers decide what an empty value means.
pub fn title_folder_root_violation<L, A>(
    folder: &str,
    library_roots: L,
    all_roots: A,
) -> Option<TitleFolderRootViolation>
where
    L: IntoIterator,
    L::Item: AsRef<str>,
    A: IntoIterator,
    A::Item: AsRef<str>,
{
    if comparable(folder).is_empty() {
        return None;
    }
    let all_roots = all_roots
        .into_iter()
        .filter(|root| !comparable(root.as_ref()).is_empty())
        .collect::<Vec<_>>();
    if all_roots
        .iter()
        .any(|root| same_location(folder, root.as_ref()))
    {
        return Some(TitleFolderRootViolation::EqualsRoot);
    }
    if all_roots
        .iter()
        .any(|root| under_or_equal(root.as_ref(), folder))
    {
        return Some(TitleFolderRootViolation::ContainsRoot);
    }
    let inside_library = library_roots
        .into_iter()
        .any(|root| path_is_strictly_within(folder, root.as_ref()));
    if !inside_library {
        return Some(TitleFolderRootViolation::OutsideLibraryRoots);
    }
    None
}

/// The most specific of `roots` that `path` lies strictly beneath.
fn most_specific_root_strictly_containing<'a>(path: &str, roots: &'a [String]) -> Option<&'a str> {
    roots
        .iter()
        .map(String::as_str)
        .filter(|root| path_is_strictly_within(path, root))
        .max_by_key(|root| comparable(root).len())
}

/// The folder directly beneath its root that a stored file path sits in:
/// the root plus the path's first segment below it, in the file's own
/// spelling. `None` when the file is directly in a root, outside every root,
/// or stored in the escape form.
fn first_segment_folder(path: &str, library_roots: &[String]) -> Option<String> {
    // A dot component makes the spelled first segment something other than
    // the folder the file is really in.
    if is_escaped_stored_path(path) || path_has_dot_segments(path) {
        return None;
    }
    let root = most_specific_root_strictly_containing(path, library_roots)?;
    let original = path.trim();
    let windows = windows_path_syntax(original);
    let separator = |c| c == '/' || (windows && c == '\\');
    let root_segments = comparable(root)
        .split('/')
        .filter(|s| !s.is_empty())
        .count();
    let mut segments = 0;
    let mut in_segment = false;
    for (offset, character) in original.char_indices() {
        if separator(character) {
            if in_segment && segments == root_segments + 1 {
                return Some(original[..offset].to_string());
            }
            in_segment = false;
        } else if !in_segment {
            segments += 1;
            in_segment = true;
        }
    }
    None
}

/// The title folder implied by where a title's files are: the root plus the
/// first path segment below it, when every file agrees on it.
///
/// Returns `None` when there are no files, when any file is directly in a
/// root, outside every root of the library, or not interpretable, or when the
/// files disagree. Agreement is required because a folder chosen from some of
/// a title's files would strand the rest.
pub fn title_folder_from_media_paths<'a, I>(library_roots: &[String], paths: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut derived: Option<String> = None;
    for path in paths {
        let candidate = first_segment_folder(path, library_roots)?;
        match &derived {
            None => derived = Some(candidate),
            Some(existing) if same_location(existing, &candidate) => {}
            Some(_) => return None,
        }
    }
    derived
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn unicode_equivalent_roots_preserve_original_derived_spelling() {
        let library = roots(&["/media/Café"]);
        let path = "/media/Cafe\u{301}/Show/E01.mkv";
        assert!(path_is_strictly_within(path, &library[0]));
        assert_eq!(
            title_folder_from_media_paths(&library, [path]),
            Some("/media/Cafe\u{301}/Show".into())
        );
        assert!(
            containing_folder_keys(path).contains(&folder_containment_key(&library[0]).unwrap())
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_backslashes_do_not_cross_folder_boundaries() {
        let library = roots(&["/media/tv"]);
        for (path, violation) in [
            (
                r"/media/tv\Show",
                TitleFolderRootViolation::OutsideLibraryRoots,
            ),
            (r"/media/tv\Show/..", TitleFolderRootViolation::ContainsRoot),
            (
                r"/media/tv\../Show",
                TitleFolderRootViolation::OutsideLibraryRoots,
            ),
        ] {
            assert_eq!(
                title_folder_root_violation(path, &library, &library),
                Some(violation)
            );
            assert!(!stored_path_is_inside_folder(&library[0], path));
        }
        assert!(path_is_strictly_within(r"/media/tv/Show\Name", &library[0]));
    }

    #[test]
    fn a_folder_inside_a_root_is_valid() {
        let library = roots(&["/media/tv"]);
        assert_eq!(
            title_folder_root_violation("/media/tv/Synthetic Show", &library, &library),
            None
        );
        assert_eq!(
            title_folder_root_violation("/media/tv/Group/Synthetic Show", &library, &library),
            None
        );
    }

    #[test]
    fn a_root_is_not_a_title_folder() {
        let library = roots(&["/media/tv"]);
        assert_eq!(
            title_folder_root_violation("/media/tv", &library, &library),
            Some(TitleFolderRootViolation::EqualsRoot)
        );
        assert_eq!(
            title_folder_root_violation("/media/tv/", &library, &library),
            Some(TitleFolderRootViolation::EqualsRoot)
        );
    }

    #[test]
    fn another_librarys_root_is_not_a_title_folder() {
        let library = roots(&["/media/tv"]);
        let all = roots(&["/media/tv", "/media/tv/Anime"]);
        assert_eq!(
            title_folder_root_violation("/media/tv/Anime", &library, &all),
            Some(TitleFolderRootViolation::EqualsRoot)
        );
    }

    #[test]
    fn an_ancestor_of_a_root_is_not_a_title_folder() {
        let library = roots(&["/media/tv"]);
        assert_eq!(
            title_folder_root_violation("/media", &library, &library),
            Some(TitleFolderRootViolation::ContainsRoot)
        );
        assert_eq!(
            title_folder_root_violation("/", &library, &library),
            Some(TitleFolderRootViolation::ContainsRoot)
        );
    }

    #[test]
    fn a_folder_outside_the_library_roots_is_not_a_title_folder() {
        let library = roots(&["/media/tv"]);
        let all = roots(&["/media/tv", "/media/movies"]);
        assert_eq!(
            title_folder_root_violation("/elsewhere/Synthetic Show", &library, &all),
            Some(TitleFolderRootViolation::OutsideLibraryRoots)
        );
        assert_eq!(
            title_folder_root_violation("/media/movies/Synthetic Film", &library, &all),
            Some(TitleFolderRootViolation::OutsideLibraryRoots)
        );
        assert_eq!(
            title_folder_root_violation("/media/tv-other/Show", &library, &all),
            Some(TitleFolderRootViolation::OutsideLibraryRoots)
        );
    }

    #[test]
    fn windows_style_paths_follow_the_same_rule_on_every_host() {
        let library = roots(&[r"D:\Media\TV"]);
        assert_eq!(
            title_folder_root_violation(r"D:\Media\TV\Synthetic Show", &library, &library),
            None
        );
        assert_eq!(
            title_folder_root_violation(r"D:\Media\TV", &library, &library),
            Some(TitleFolderRootViolation::EqualsRoot)
        );
        assert_eq!(
            title_folder_root_violation(r"D:\", &library, &library),
            Some(TitleFolderRootViolation::ContainsRoot)
        );
        assert_eq!(
            title_folder_root_violation(r"D:\Media", &library, &library),
            Some(TitleFolderRootViolation::ContainsRoot)
        );
    }

    #[test]
    fn derives_the_first_segment_below_the_root() {
        let library = roots(&["/media/tv"]);
        assert_eq!(
            title_folder_from_media_paths(
                &library,
                [
                    "/media/tv/Synthetic Show/Season 01/Synthetic Show - S01E01.mkv",
                    "/media/tv/Synthetic Show/Season 02/Synthetic Show - S02E01.mkv",
                    "/media/tv/Synthetic Show/Synthetic Show - S03E01.mkv",
                ]
            ),
            Some("/media/tv/Synthetic Show".to_string())
        );
    }

    #[test]
    fn derivation_uses_the_most_specific_root() {
        let library = roots(&["/media", "/media/tv"]);
        assert_eq!(
            title_folder_from_media_paths(&library, ["/media/tv/Synthetic Show/E01.mkv"]),
            Some("/media/tv/Synthetic Show".to_string())
        );
    }

    #[test]
    fn derivation_refuses_disagreement_and_unplaceable_files() {
        let library = roots(&["/media/tv"]);
        assert_eq!(
            title_folder_from_media_paths(
                &library,
                ["/media/tv/Show A/E01.mkv", "/media/tv/Show B/E02.mkv"]
            ),
            None
        );
        assert_eq!(
            title_folder_from_media_paths(&library, ["/media/tv/E01.mkv"]),
            None
        );
        assert_eq!(
            title_folder_from_media_paths(&library, ["/elsewhere/Show/E01.mkv"]),
            None
        );
        assert_eq!(
            title_folder_from_media_paths(&library, std::iter::empty::<&str>()),
            None
        );
        assert_eq!(
            title_folder_from_media_paths(
                &library,
                [
                    "/media/tv/Show/E01.mkv",
                    "scryer-path-v1:u:/media/tv/Show/%FF.mkv"
                ]
            ),
            None
        );
    }

    #[test]
    fn derivation_keeps_the_windows_spelling() {
        let library = roots(&[r"D:\Media\TV"]);
        assert_eq!(
            title_folder_from_media_paths(
                &library,
                [
                    r"D:\Media\TV\Synthetic Show\Season 01\E01.mkv",
                    r"D:\Media\TV\Synthetic Show\E02.mkv",
                ]
            ),
            Some(r"D:\Media\TV\Synthetic Show".to_string())
        );
        let drive = roots(&[r"D:\"]);
        assert_eq!(
            title_folder_from_media_paths(&drive, [r"D:\Synthetic Show\E01.mkv"]),
            Some(r"D:\Synthetic Show".to_string())
        );
    }

    #[test]
    fn containment_keys_agree_with_pairwise_containment() {
        let folders = [
            "/",
            "/media",
            "/media/tv",
            "/media/tv/",
            "/media/tv/Show",
            "/media/tv/Show 2",
            "/media/tv/Show/Season 01",
            r"D:\",
            r"D:\Media",
            r"D:\Media\TV\Show",
            r"\\nas\share",
            r"\\nas\share\Show",
        ];
        let paths = [
            "/media/tv/Show/Season 01/E01.mkv",
            "/media/tv/Show 2/E01.mkv",
            "/media/tv/E01.mkv",
            "/media/tv/Show",
            r"D:\Media\TV\Show\Season 01\E01.mkv",
            r"D:\Show\E01.mkv",
            r"\\nas\share\Show\E01.mkv",
        ];
        for path in paths {
            let keys = containing_folder_keys(path);
            for folder in folders {
                let key = folder_containment_key(folder).expect("plain folder has a key");
                assert_eq!(
                    keys.contains(&key),
                    stored_path_is_inside_folder(folder, path),
                    "folder {folder:?} against path {path:?}"
                );
            }
        }
        assert!(containing_folder_keys("scryer-path-v1:u:/media/tv/Show/%FF.mkv").is_empty());
        assert_eq!(folder_containment_key(""), None);
    }

    #[test]
    fn dot_components_are_resolved_before_the_root_check() {
        let library = roots(&["/media/tv"]);
        let all = roots(&["/media/tv", "/media/movies"]);
        assert_eq!(
            title_folder_root_violation("/media/tv/Synthetic Show/..", &library, &all),
            Some(TitleFolderRootViolation::EqualsRoot)
        );
        assert_eq!(
            title_folder_root_violation("/media/tv/./Synthetic Show/../.", &library, &all),
            Some(TitleFolderRootViolation::EqualsRoot)
        );
        assert_eq!(
            title_folder_root_violation("/media/tv/Synthetic Show/../..", &library, &all),
            Some(TitleFolderRootViolation::ContainsRoot)
        );
        assert_eq!(
            title_folder_root_violation("/media/tv/../movies/Synthetic Film", &library, &all),
            Some(TitleFolderRootViolation::OutsideLibraryRoots)
        );
        assert_eq!(
            title_folder_root_violation("/media/tv/Other/../Synthetic Show/.", &library, &all),
            None
        );
        assert!(!path_is_strictly_within(
            "/media/tv/Synthetic Show/..",
            "/media/tv"
        ));
        assert!(!path_is_strictly_within(
            "/media/tv/../elsewhere",
            "/media/tv"
        ));
    }

    #[test]
    fn a_path_climbing_above_the_filesystem_root_stays_at_the_root() {
        let library = roots(&["/media/tv"]);
        assert_eq!(
            title_folder_root_violation("/media/tv/../../../..", &library, &library),
            Some(TitleFolderRootViolation::ContainsRoot)
        );
        assert_eq!(
            title_folder_root_violation(
                r"D:\Media\TV\Show\..\..\..\..",
                &roots(&[r"D:\Media\TV"]),
                &roots(&[r"D:\Media\TV"])
            ),
            Some(TitleFolderRootViolation::ContainsRoot)
        );
    }

    #[test]
    fn dot_components_resolve_across_trailing_and_mixed_separators() {
        let library = roots(&[r"D:\Media\TV"]);
        assert_eq!(
            title_folder_root_violation(r"D:\Media\TV\Show\..\", &library, &library),
            Some(TitleFolderRootViolation::EqualsRoot)
        );
        assert_eq!(
            title_folder_root_violation(r"D:\Media\TV/Show/..", &library, &library),
            Some(TitleFolderRootViolation::EqualsRoot)
        );
        assert_eq!(
            title_folder_root_violation(r"D:/Media/TV\Other\..\Show/", &library, &library),
            None
        );
        assert_eq!(
            title_folder_root_violation(
                "/media/tv/Synthetic Show/../",
                &roots(&["/media/tv"]),
                &roots(&["/media/tv"])
            ),
            Some(TitleFolderRootViolation::EqualsRoot)
        );
        assert_eq!(
            title_folder_root_violation(
                r"\\nas\share\Show\..\..",
                &roots(&[r"\\nas\share\TV"]),
                &roots(&[r"\\nas\share\TV"])
            ),
            Some(TitleFolderRootViolation::ContainsRoot)
        );
    }

    #[test]
    fn a_dotted_file_path_derives_no_folder() {
        let library = roots(&["/media/tv"]);
        assert_eq!(
            title_folder_from_media_paths(&library, ["/media/tv/Show A/../Show B/E01.mkv"]),
            None
        );
    }

    #[test]
    fn containment_is_separator_agnostic_and_segment_aware() {
        assert!(stored_path_is_inside_folder(
            r"D:\Media\TV\Show",
            r"D:\Media\TV\Show\Season 01\E01.mkv"
        ));
        assert!(stored_path_is_inside_folder(
            "/media/tv/Show",
            "/media/tv/Show/E01.mkv"
        ));
        assert!(!stored_path_is_inside_folder(
            "/media/tv/Show",
            "/media/tv/Show 2/E01.mkv"
        ));
        assert!(path_is_strictly_within("/media/tv/Show", "/media/tv"));
        assert!(!path_is_strictly_within("/media/tv", "/media/tv/"));
    }
}
