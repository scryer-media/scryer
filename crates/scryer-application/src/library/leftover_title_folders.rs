//! One-time removal of the empty title folders an old rename left behind.
//!
//! A rename that changed a title's folder name moved the files into the new
//! folder and, when the title's folder record was wrong, left the old folder
//! standing with nothing but empty season folders in it. This removes those
//! folders, and only those. A folder is removed when every one of these holds:
//!
//! - it sits directly under a configured library root;
//! - its name is a title's `Name (Year)`, alone or followed by text carrying
//!   one of that title's own external ids, and no other title's name fits it;
//! - that title's recorded folder is directly under the same root, is named
//!   the same way, is on disk as an ordinary directory, and is a different
//!   directory;
//! - no title records it or a folder inside it, no library root is it or sits
//!   inside it, and no title tracks a file in it, comparing paths without
//!   regard to case or Unicode form;
//! - no location operation is unfinished;
//! - it holds nothing but empty directories named like season folders.
//!
//! Anything else is left alone. It runs while the application is serving, so
//! the catalog is read again before each removal. Directories are removed
//! with `remove_dir`, which refuses a directory that still holds anything, so
//! an entry that appears after the check stops the removal.

use std::path::{Path, PathBuf};

use scryer_domain::Title;
use tracing::{info, warn};
use unicode_normalization::UnicodeNormalization;

use crate::library::rename::{build_title_folder_tokens, sanitize_filesystem_component};
use crate::stored_paths::{
    folder_paths_match, is_escaped_stored_path, path_identity_key, path_to_stored_string,
    stored_path_to_path_buf,
};
use crate::title_folder_rules::{path_is_strictly_within, stored_path_is_inside_folder};
use crate::{AppResult, AppUseCase};

/// What the one-time cleanup did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EmptyDuplicateTitleFolderReport {
    /// Folders removed.
    pub removed: Vec<String>,
    /// Folders named like a leftover that were not removed because they did
    /// not meet every condition.
    pub kept: Vec<String>,
    /// Folders that could not be inspected or removed.
    pub failed: Vec<String>,
}

enum FolderOutcome {
    Removed,
    Kept(&'static str),
    Failed {
        error: std::io::Error,
        /// Empty season folders already removed when the error stopped it.
        season_folders_removed: usize,
    },
}

impl FolderOutcome {
    fn failed(error: std::io::Error) -> Self {
        Self::Failed {
            error,
            season_folders_removed: 0,
        }
    }
}

impl AppUseCase {
    /// Remove the empty leftover title folders described in the module docs.
    pub async fn remove_empty_duplicate_title_folders(
        &self,
    ) -> AppResult<EmptyDuplicateTitleFolderReport> {
        let roots = self
            .all_library_root_folders()
            .await?
            .into_iter()
            .map(|root| root.path)
            .collect::<Vec<_>>();
        let titles = self.services.catalog.titles.list(None, None).await?;
        let recorded = titles
            .iter()
            .filter_map(|title| {
                let folder = title.folder_path.as_deref().map(str::trim)?;
                (!folder.is_empty()).then_some((title, folder))
            })
            .collect::<Vec<_>>();
        // Every title's name rule, with or without a folder record.
        let name_rules = titles
            .iter()
            .filter_map(|title| Some((title.id.as_str(), NameRule::for_title(title)?)))
            .collect::<Vec<_>>();
        // Loaded on the first folder that gets far enough to need it.
        let mut tracked_files: Option<Vec<String>> = None;

        let mut report = EmptyDuplicateTitleFolderReport::default();
        if !self.location_ownership_open_claims().await?.is_empty() {
            info!("one-time title folder cleanup skipped: a location operation is unfinished");
            return Ok(report);
        }
        info!(
            roots = roots.len(),
            titles = recorded.len(),
            "one-time cleanup of empty title folders left by an old rename started"
        );

        let mut visited_roots = Vec::<&str>::new();
        for root in &roots {
            if is_escaped_stored_path(root)
                || visited_roots
                    .iter()
                    .any(|visited| same_folder(visited, root))
            {
                continue;
            }
            visited_roots.push(root);

            // Titles whose recorded folder is directly under this root and is
            // itself named after the title.
            let siblings = recorded
                .iter()
                .filter(|(_, folder)| {
                    stored_path_to_path_buf(folder)
                        .parent()
                        .is_some_and(|parent| {
                            folder_paths_match(&path_to_stored_string(parent), root)
                        })
                })
                .filter_map(|(title, folder)| {
                    let rule = NameRule::for_title(title)?;
                    let recorded_path = stored_path_to_path_buf(folder);
                    let recorded_name = recorded_path.file_name()?.to_str()?;
                    rule.matches(recorded_name)
                        .then_some((*title, recorded_path.clone(), rule))
                })
                .collect::<Vec<_>>();
            if siblings.is_empty() {
                continue;
            }

            let root_path = stored_path_to_path_buf(root);
            let directories = match run_blocking(move || child_directories(&root_path)).await {
                Ok(directories) => directories,
                Err(error) => {
                    warn!(root = %root, error = %error, "skipping leftover title folder cleanup for a root that could not be listed");
                    continue;
                }
            };

            for directory in directories {
                let Some(name) = directory.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                // Windows resolves a name ending in a dot or a space to the
                // name without it, which is a different folder.
                if name.ends_with(['.', ' ']) {
                    continue;
                }
                let owners = siblings
                    .iter()
                    .filter(|(_, _, rule)| rule.matches(name))
                    .collect::<Vec<_>>();
                if owners.is_empty() {
                    continue;
                }
                let stored = path_to_stored_string(&directory);
                // The name could as well be the folder of a title that is not
                // one of the owners: one with no folder record, or recorded
                // somewhere else.
                if name_rules.iter().any(|(id, rule)| {
                    rule.matches(name) && !owners.iter().any(|(title, _, _)| title.id == *id)
                }) {
                    info!(folder = %stored, reason = "another title has the same name", "not removed by the one-time title folder cleanup");
                    report.kept.push(stored);
                    continue;
                }
                if is_escaped_stored_path(&stored)
                    || recorded
                        .iter()
                        .any(|(_, folder)| is_or_is_inside(folder, &stored))
                    || roots.iter().any(|other| is_or_is_inside(other, &stored))
                {
                    continue;
                }

                let owner_titles = owners
                    .iter()
                    .map(|(title, _, _)| *title)
                    .collect::<Vec<_>>();
                if let Some(reason) = self
                    .reason_to_keep_folder(&owner_titles, &stored, &mut tracked_files)
                    .await?
                {
                    info!(folder = %stored, reason, "not removed by the one-time title folder cleanup");
                    report.kept.push(stored);
                    continue;
                }

                let target = directory.clone();
                let root_path = stored_path_to_path_buf(root);
                let recorded_folders = owners
                    .iter()
                    .map(|(_, folder, _)| folder.clone())
                    .collect::<Vec<_>>();
                let outcome = run_blocking(move || {
                    Ok(remove_leftover_folder(
                        &root_path,
                        &target,
                        &recorded_folders,
                    ))
                })
                .await;
                match outcome {
                    Ok(FolderOutcome::Removed) => {
                        warn!(folder = %stored, "removed an empty title folder a rename left behind");
                        report.removed.push(stored);
                    }
                    Ok(FolderOutcome::Kept(reason)) => {
                        info!(folder = %stored, reason, "not removed by the one-time title folder cleanup");
                        report.kept.push(stored);
                    }
                    Ok(FolderOutcome::Failed {
                        error,
                        season_folders_removed,
                    }) => {
                        warn!(folder = %stored, error = %error, season_folders_removed, "could not finish removing an empty title folder a rename left behind");
                        report.failed.push(stored);
                    }
                    Err(error) => {
                        warn!(folder = %stored, error = %error, "could not inspect an empty title folder a rename left behind");
                        report.failed.push(stored);
                    }
                }
            }
        }
        Ok(report)
    }

    /// The catalog's reason to leave `folder` alone, read at the moment the
    /// folder is about to be removed.
    async fn reason_to_keep_folder(
        &self,
        owners: &[&Title],
        folder: &str,
        tracked_files: &mut Option<Vec<String>>,
    ) -> AppResult<Option<&'static str>> {
        if !self.location_ownership_open_claims().await?.is_empty() {
            return Ok(Some("a location operation is unfinished"));
        }
        // Read again: this runs while the application is serving requests.
        let titles = self.services.catalog.titles.list(None, None).await?;
        for owner in owners {
            let Some(current) = titles.iter().find(|title| title.id == owner.id) else {
                return Ok(Some("the title is gone"));
            };
            if current.folder_path != owner.folder_path {
                return Ok(Some("the title's folder record changed"));
            }
        }
        if titles
            .iter()
            .filter_map(|title| title.folder_path.as_deref())
            .any(|recorded| is_or_is_inside(recorded, folder))
        {
            return Ok(Some("a title records it"));
        }

        if tracked_files.is_none() {
            let paths = self
                .services
                .workflow
                .housekeeping
                .list_all_media_file_paths()
                .await?
                .into_iter()
                .map(|(_, path)| path)
                .collect();
            *tracked_files = Some(paths);
        }
        let tracked = tracked_files
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|path| is_or_is_inside(path, folder));
        Ok(tracked.then_some("a title tracks a file in it"))
    }
}

async fn run_blocking<T, F>(work: F) -> std::io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> std::io::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(std::io::Error::other)?
}

/// A path's spelling for comparison without regard to case or Unicode form.
fn folded(path: &str) -> Option<String> {
    path_identity_key(path).map(|key| key.nfc().collect::<String>().to_lowercase())
}

fn same_folder(left: &str, right: &str) -> bool {
    folder_paths_match(left, right)
        || matches!((folded(left), folded(right)), (Some(left), Some(right)) if left == right)
}

/// Whether `path` is `folder` or lies inside it under any of the comparisons
/// the application uses, or when compared without regard to case or Unicode
/// form. A path that cannot be compared counts as inside.
fn is_or_is_inside(path: &str, folder: &str) -> bool {
    if folder_paths_match(path, folder)
        || path_is_strictly_within(path, folder)
        || stored_path_is_inside_folder(folder, path)
    {
        return true;
    }
    let (Some(path), Some(folder)) = (folded(path), folded(folder)) else {
        return true;
    };
    let folder = folder.trim_end_matches('/');
    path.strip_prefix(folder)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Whether `metadata` describes an ordinary directory: not a symlink, and on
/// Windows not a reparse point of any kind (junction, mount point, cloud
/// placeholder).
fn is_plain_directory(metadata: &std::fs::Metadata) -> bool {
    if !metadata.file_type().is_dir() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }
    true
}

/// Whether two directories are on the same filesystem. Always true where the
/// platform gives no cheap way to tell.
fn on_the_same_filesystem(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev()
    }
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        true
    }
}

/// The ordinary directories directly inside `root`.
fn child_directories(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

/// Whether `name` is a season folder's name: `Specials`, or `Season` and a
/// number.
fn is_season_folder_name(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    if name == "specials" {
        return true;
    }
    name.strip_prefix("season").is_some_and(|rest| {
        let number = rest.trim_start();
        !number.is_empty() && number.len() <= 4 && number.chars().all(|ch| ch.is_ascii_digit())
    })
}

/// The season folders inside `directory`, when it holds nothing but empty
/// season folders. `Err` carries why it is something else.
fn empty_season_folders(directory: &Path) -> std::io::Result<Result<Vec<PathBuf>, &'static str>> {
    let own = std::fs::symlink_metadata(directory)?;
    if !is_plain_directory(&own) {
        return Ok(Err("it is not an ordinary directory"));
    }
    let mut seasons = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if !is_plain_directory(&metadata) {
            return Ok(Err("it holds something that is not a directory"));
        }
        if !on_the_same_filesystem(&own, &metadata) {
            return Ok(Err("it holds a mount point"));
        }
        if !entry
            .file_name()
            .to_str()
            .is_some_and(is_season_folder_name)
        {
            return Ok(Err("it holds a folder that is not a season folder"));
        }
        if std::fs::read_dir(&path)?.next().is_some() {
            return Ok(Err("a season folder in it is not empty"));
        }
        seasons.push(path);
    }
    Ok(Ok(seasons))
}

/// Whether two existing paths are one directory, however each is spelled.
fn same_directory(left: &Path, right: &Path) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (left, right) = (std::fs::metadata(left)?, std::fs::metadata(right)?);
        if left.dev() == right.dev() && left.ino() == right.ino() {
            return Ok(true);
        }
    }
    Ok(std::fs::canonicalize(left)? == std::fs::canonicalize(right)?)
}

/// Why `directory` is not a duplicate of the folders its titles record:
/// every one of those must be on disk as an ordinary directory, and be a
/// different directory.
fn reason_it_is_not_a_duplicate(
    directory: &Path,
    recorded_folders: &[PathBuf],
) -> std::io::Result<Option<&'static str>> {
    if recorded_folders.is_empty() {
        return Ok(Some("no title records a folder beside it"));
    }
    for recorded in recorded_folders {
        match std::fs::symlink_metadata(recorded) {
            Ok(metadata) if is_plain_directory(&metadata) => {}
            Ok(_) => {
                return Ok(Some(
                    "the title's recorded folder is not an ordinary directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Some("the title's recorded folder is not on disk"));
            }
            Err(error) => return Err(error),
        }
        if same_directory(directory, recorded)? {
            return Ok(Some("it is the title's recorded folder"));
        }
    }
    Ok(None)
}

/// Remove `directory` when it holds nothing but empty season folders and the
/// folders in `recorded_folders` are all on disk elsewhere.
fn remove_leftover_folder(
    root: &Path,
    directory: &Path,
    recorded_folders: &[PathBuf],
) -> FolderOutcome {
    let same_filesystem = std::fs::symlink_metadata(root).and_then(|root| {
        Ok(on_the_same_filesystem(
            &root,
            &std::fs::symlink_metadata(directory)?,
        ))
    });
    match same_filesystem {
        Ok(true) => {}
        Ok(false) => return FolderOutcome::Kept("it is a mount point"),
        Err(error) => return FolderOutcome::failed(error),
    }
    let seasons = match empty_season_folders(directory) {
        Ok(Ok(seasons)) => seasons,
        Ok(Err(reason)) => return FolderOutcome::Kept(reason),
        Err(error) => return FolderOutcome::failed(error),
    };
    match reason_it_is_not_a_duplicate(directory, recorded_folders) {
        Ok(None) => {}
        Ok(Some(reason)) => return FolderOutcome::Kept(reason),
        Err(error) => return FolderOutcome::failed(error),
    }
    for (removed, season) in seasons.iter().enumerate() {
        if let Err(error) = std::fs::remove_dir(season) {
            return FolderOutcome::Failed {
                error,
                season_folders_removed: removed,
            };
        }
    }
    match std::fs::remove_dir(directory) {
        Ok(()) => FolderOutcome::Removed,
        Err(error) => FolderOutcome::Failed {
            error,
            season_folders_removed: seasons.len(),
        },
    }
}

fn comparable_name(name: &str) -> String {
    name.nfc().collect::<String>().to_lowercase()
}

/// The external id sources a folder template can render into a folder name.
const FOLDER_NAME_ID_SOURCES: [&str; 6] = ["imdb", "tmdb", "tvdb", "anidb", "mal", "anilist"];

/// An id shorter than this is too easily an ordinary number in a folder name.
const SHORTEST_FOLDER_NAME_ID: usize = 3;

/// Which folder names are one title's: its `Name (Year)`, alone or followed
/// by text that carries one of the title's own external ids.
struct NameRule {
    prefix: String,
    ids: Vec<String>,
}

impl NameRule {
    /// `None` for a title with no name or no year, which matches nothing.
    fn for_title(title: &Title) -> Option<Self> {
        let tokens = build_title_folder_tokens(title, title.year);
        let token = |name: &str| {
            tokens
                .get(name)
                .map(|value| value.trim())
                .unwrap_or_default()
        };
        let ids = title
            .external_ids
            .iter()
            .filter(|id| {
                FOLDER_NAME_ID_SOURCES
                    .iter()
                    .any(|source| id.source.eq_ignore_ascii_case(source))
            })
            .map(|id| id.value.as_str())
            .chain(title.imdb_id.as_deref())
            .collect::<Vec<_>>();
        Self::new(token("title"), token("year"), &ids)
    }

    fn new(title_name: &str, year: &str, ids: &[&str]) -> Option<Self> {
        if title_name.is_empty() || year.is_empty() {
            return None;
        }
        Some(Self {
            prefix: comparable_name(&sanitize_filesystem_component(&format!(
                "{title_name} ({year})"
            ))),
            ids: ids
                .iter()
                .map(|id| comparable_name(id.trim()))
                .filter(|id| id.chars().count() >= SHORTEST_FOLDER_NAME_ID)
                .collect(),
        })
    }

    fn matches(&self, folder_name: &str) -> bool {
        let folder_name = comparable_name(folder_name);
        let Some(rest) = folder_name.strip_prefix(&self.prefix) else {
            return false;
        };
        rest.is_empty() || self.ids.iter().any(|id| contains_whole_word(rest, id))
    }
}

/// Whether `text` contains `word` with no letter or digit on either side.
fn contains_whole_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(start, matched)| {
        let before = text[..start].chars().next_back();
        let after = text[start + matched.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder_name_matches(folder_name: &str, title_name: &str, year: &str, ids: &[&str]) -> bool {
        NameRule::new(title_name, year, ids).is_some_and(|rule| rule.matches(folder_name))
    }

    /// The folder a title records, beside the leftover.
    fn recorded(temp: &tempfile::TempDir) -> Vec<PathBuf> {
        let folder = temp.path().join("Synthetic Show (2024) 93077");
        std::fs::create_dir_all(&folder).expect("recorded folder");
        vec![folder]
    }

    fn leftover(temp: &tempfile::TempDir) -> PathBuf {
        let folder = temp.path().join("Synthetic Show (2024)");
        std::fs::create_dir_all(folder.join("Season 1")).expect("season");
        std::fs::create_dir_all(folder.join("Specials")).expect("specials");
        folder
    }

    #[test]
    fn a_folder_of_empty_season_folders_is_removed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &recorded(&temp)),
            FolderOutcome::Removed
        ));
        assert!(!folder.exists());
        assert!(temp.path().is_dir());
    }

    #[test]
    fn a_folder_holding_one_file_is_left_whole() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        let kept = folder.join("Specials").join("poster.jpg");
        std::fs::write(&kept, b"x").expect("file");
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &recorded(&temp)),
            FolderOutcome::Kept(_)
        ));
        assert!(kept.is_file());
        assert!(folder.join("Season 1").is_dir(), "nothing is removed");
    }

    #[test]
    fn a_file_beside_the_season_folders_keeps_the_folder() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        std::fs::write(folder.join("desktop.ini"), b"x").expect("file");
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &recorded(&temp)),
            FolderOutcome::Kept(_)
        ));
        assert!(folder.join("Season 1").is_dir(), "nothing is removed");
    }

    #[test]
    fn a_folder_that_is_not_a_season_folder_keeps_the_folder() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        std::fs::create_dir_all(folder.join("Extras")).expect("extras");
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &recorded(&temp)),
            FolderOutcome::Kept(_)
        ));
        assert!(folder.join("Season 1").is_dir(), "nothing is removed");
        assert!(folder.join("Extras").is_dir());
    }

    #[test]
    fn a_directory_inside_a_season_folder_keeps_the_folder() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        std::fs::create_dir_all(folder.join("Season 1").join("Subs")).expect("nested");
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &recorded(&temp)),
            FolderOutcome::Kept(_)
        ));
        assert!(folder.join("Season 1").join("Subs").is_dir());
        assert!(folder.join("Specials").is_dir(), "nothing is removed");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_keeps_the_folder_and_its_target() {
        let temp = tempfile::tempdir().expect("tempdir");
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("elsewhere");
        let folder = leftover(&temp);
        std::os::unix::fs::symlink(&elsewhere, folder.join("Season 2")).expect("symlink");
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &recorded(&temp)),
            FolderOutcome::Kept(_)
        ));
        assert!(elsewhere.is_dir());
        assert!(folder.join("Season 1").is_dir(), "nothing is removed");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_under_the_root_is_not_a_candidate() {
        let temp = tempfile::tempdir().expect("tempdir");
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("elsewhere");
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join("Plain")).expect("plain");
        std::os::unix::fs::symlink(&elsewhere, root.join("Linked")).expect("symlink");
        assert_eq!(
            child_directories(&root).expect("list"),
            vec![root.join("Plain")]
        );
    }

    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run mklink");
        assert!(status.success(), "mklink /J failed");
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_keeps_the_folder_and_its_target() {
        let temp = tempfile::tempdir().expect("tempdir");
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("elsewhere");
        let folder = leftover(&temp);
        junction(&folder.join("Season 2"), &elsewhere);
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &recorded(&temp)),
            FolderOutcome::Kept(_)
        ));
        assert!(elsewhere.is_dir());
        assert!(folder.join("Season 2").exists(), "the junction is left");
        assert!(folder.join("Season 1").is_dir(), "nothing is removed");
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_is_never_a_candidate_or_removed_as_one() {
        let temp = tempfile::tempdir().expect("tempdir");
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("Season 1")).expect("elsewhere");
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join("Plain")).expect("plain");
        let linked = root.join("Synthetic Show (2024)");
        junction(&linked, &elsewhere);
        assert_eq!(
            child_directories(&root).expect("list"),
            vec![root.join("Plain")]
        );
        assert!(matches!(
            remove_leftover_folder(&root, &linked, &recorded(&temp)),
            FolderOutcome::Kept(_)
        ));
        assert!(linked.exists(), "the junction is left");
        assert!(elsewhere.join("Season 1").is_dir());
    }

    #[cfg(windows)]
    #[test]
    fn a_hidden_file_keeps_the_folder() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        let hidden = folder.join("Season 1").join("Thumbs.db");
        std::fs::write(&hidden, b"x").expect("file");
        let status = std::process::Command::new("attrib")
            .args(["+h", "+s"])
            .arg(&hidden)
            .status()
            .expect("run attrib");
        assert!(status.success());
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &recorded(&temp)),
            FolderOutcome::Kept(_)
        ));
        assert!(hidden.is_file());
        assert!(folder.join("Specials").is_dir(), "nothing is removed");
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_season_folder_fails_without_removing_anything() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        let locked = folder.join("Season 1");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("lock");
        let readable_anyway = std::fs::read_dir(&locked).is_ok();
        let outcome = remove_leftover_folder(temp.path(), &folder, &recorded(&temp));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("unlock");
        // A privileged test user can read the folder regardless.
        if !readable_anyway {
            assert!(matches!(outcome, FolderOutcome::Failed { .. }));
            assert!(folder.join("Specials").is_dir(), "nothing is removed");
        }
    }

    #[test]
    fn a_folder_is_kept_when_the_recorded_folder_is_not_on_disk() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        let missing = vec![temp.path().join("Synthetic Show (2024) 93077")];
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &missing),
            FolderOutcome::Kept(_)
        ));
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &[]),
            FolderOutcome::Kept(_)
        ));
        assert!(folder.join("Season 1").is_dir(), "nothing is removed");
    }

    #[test]
    fn a_folder_is_kept_when_any_recorded_folder_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        let mut folders = recorded(&temp);
        folders.push(temp.path().join("Synthetic Show (2024) 93078"));
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &folders),
            FolderOutcome::Kept(_)
        ));
        assert!(folder.join("Season 1").is_dir(), "nothing is removed");
    }

    #[test]
    fn a_folder_is_kept_when_it_is_the_recorded_folder_under_another_spelling() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        let respelled = vec![folder.join("Season 1").join("..")];
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &respelled),
            FolderOutcome::Kept(_)
        ));
        assert!(folder.join("Season 1").is_dir(), "nothing is removed");
    }

    #[cfg(windows)]
    #[test]
    fn a_recorded_folder_spelled_with_a_trailing_dot_is_the_same_folder() {
        let temp = tempfile::tempdir().expect("tempdir");
        let folder = leftover(&temp);
        let mut dotted = folder.clone().into_os_string();
        dotted.push(".");
        assert!(matches!(
            remove_leftover_folder(temp.path(), &folder, &[PathBuf::from(dotted)]),
            FolderOutcome::Kept(_)
        ));
        assert!(folder.join("Season 1").is_dir(), "nothing is removed");
    }

    #[test]
    fn an_id_must_be_long_enough_to_count() {
        let matches =
            |name: &str| folder_name_matches(name, "Synthetic Show", "2011", &["2", "42"]);
        assert!(matches("Synthetic Show (2011)"));
        for name in [
            "Synthetic Show (2011) - Copy (2)",
            "Synthetic Show (2011) 42",
        ] {
            assert!(!matches(name), "{name}");
        }
    }

    #[test]
    fn season_folder_names_are_recognised_narrowly() {
        for name in ["Season 1", "Season 01", "season 12", "Specials", "Season1"] {
            assert!(is_season_folder_name(name), "{name}");
        }
        for name in ["Season", "Season One", "Extras", "Season 1 Extras", "S01"] {
            assert!(!is_season_folder_name(name), "{name}");
        }
    }

    #[test]
    fn only_the_titles_own_name_and_year_match() {
        let matches = |name: &str| folder_name_matches(name, "Synthetic Show", "2011", &["245451"]);
        for name in [
            "Synthetic Show (2011)",
            "synthetic show (2011)",
            "Synthetic Show (2011) 245451",
            "Synthetic Show (2011) - [tvdbid-245451]",
            "Synthetic Show (2011) {tvdb-245451}",
        ] {
            assert!(matches(name), "{name}");
        }
        for name in [
            "Synthetic Show",
            "Synthetic Show (2019)",
            "Synthetic Show (2011) 2454510",
            "Synthetic Show (2011) 999",
            "Synthetic Show (2011) Extras",
            "Synthetic Show Reunion (2011)",
            "Synthetic Shows (2011)",
        ] {
            assert!(!matches(name), "{name}");
        }
    }

    #[test]
    fn a_short_title_does_not_match_a_longer_one() {
        let matches = |name: &str| folder_name_matches(name, "Dusk", "2023", &[]);
        assert!(matches("Dusk (2023)"));
        for name in [
            "Dusk Patrol (1997)",
            "Dusk-Zero (2023)",
            "Dusk's End (2023)",
        ] {
            assert!(!matches(name), "{name}");
        }
    }

    #[test]
    fn a_title_without_a_year_matches_nothing() {
        for name in [
            "Synthetic Show",
            "Synthetic Show ()",
            "Synthetic Show 245451",
        ] {
            assert!(
                !folder_name_matches(name, "Synthetic Show", "", &["245451"]),
                "{name}"
            );
        }
    }

    #[test]
    fn a_name_matches_across_unicode_forms() {
        assert!(folder_name_matches(
            "Cafe\u{301} Lumen (2020)",
            "Caf\u{e9} Lumen",
            "2020",
            &[]
        ));
    }

    #[test]
    fn containment_ignores_case_and_unicode_form() {
        assert!(is_or_is_inside(
            "/tv/Show Of The Year (2011)",
            "/tv/Show of the Year (2011)"
        ));
        assert!(is_or_is_inside(
            "/tv/Caf\u{e9} Lumen (2020)/Specials",
            "/tv/Cafe\u{301} Lumen (2020)"
        ));
        assert!(is_or_is_inside(
            "/tv/show (2011)/Season 1/episode.mkv",
            "/tv/Show (2011)/"
        ));
        assert!(!is_or_is_inside(
            "/tv/Show (2011) 245451",
            "/tv/Show (2011)"
        ));
        assert!(!is_or_is_inside("/tv/Show (2019)", "/tv/Show (2011)"));
        assert!(!is_or_is_inside("/tv", "/tv/Show (2011)"));
    }
}
