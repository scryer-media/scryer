use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{AppError, AppResult};
use scryer_domain::VIDEO_EXTENSIONS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkEntryKind {
    Directory { via_symlink: bool },
    File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkFilterPolicy {
    None,
    LibraryScanJunkOnly,
    EpisodicTitleScan,
    MovieTitleScan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkFilePolicy {
    All,
    SupportedVideoOnly,
    SupportedVideoAndNfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkReadBehavior {
    Fail,
    SkipUnreadableSubdirectories,
}

#[derive(Clone, Debug, Default)]
pub struct WalkedDirectory {
    pub path: PathBuf,
    pub subdirs: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
    pub filenames_lower: HashSet<String>,
    symlinked_subdirs: HashSet<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct FilesystemWalker {
    read_behavior: WalkReadBehavior,
    filter_policy: WalkFilterPolicy,
    file_policy: WalkFilePolicy,
    max_depth: Option<usize>,
    follow_symlinked_directories: bool,
    confine_to_root: bool,
}

impl Default for FilesystemWalker {
    fn default() -> Self {
        Self::new()
    }
}

impl FilesystemWalker {
    pub fn new() -> Self {
        Self {
            read_behavior: WalkReadBehavior::Fail,
            filter_policy: WalkFilterPolicy::None,
            file_policy: WalkFilePolicy::All,
            max_depth: None,
            follow_symlinked_directories: true,
            confine_to_root: false,
        }
    }

    pub fn skip_unreadable_subdirectories(mut self) -> Self {
        self.read_behavior = WalkReadBehavior::SkipUnreadableSubdirectories;
        self
    }

    pub fn skip_library_scan_junk(mut self) -> Self {
        self.filter_policy = WalkFilterPolicy::LibraryScanJunkOnly;
        self
    }

    pub fn skip_episodic_scan_junk_and_trailers(mut self) -> Self {
        self.filter_policy = WalkFilterPolicy::EpisodicTitleScan;
        self
    }

    pub fn skip_movie_scan_junk_and_extras(mut self) -> Self {
        self.filter_policy = WalkFilterPolicy::MovieTitleScan;
        self
    }

    pub fn supported_video_files_only(mut self) -> Self {
        self.file_policy = WalkFilePolicy::SupportedVideoOnly;
        self
    }

    pub fn supported_video_and_nfo_files(mut self) -> Self {
        self.file_policy = WalkFilePolicy::SupportedVideoAndNfo;
        self
    }

    pub fn max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = Some(max_depth);
        self
    }

    pub fn skip_symlinked_directories(mut self) -> Self {
        self.follow_symlinked_directories = false;
        self
    }

    /// Follow symlinked directories only when their canonical targets remain
    /// within the canonical walk root.
    pub fn confine_to_root(mut self) -> Self {
        self.confine_to_root = true;
        self
    }

    pub fn list_child_directories(&self, root: &Path) -> AppResult<Vec<PathBuf>> {
        let mut deduped = Vec::new();
        self.visit_child_directories(root, |path| {
            deduped.push(path);
            Ok(())
        })?;
        deduped.sort();

        Ok(deduped)
    }

    pub fn visit_child_directories<F>(&self, root: &Path, mut visitor: F) -> AppResult<()>
    where
        F: FnMut(PathBuf) -> AppResult<()>,
    {
        let listing = self.read_directory(root)?;
        let symlinked_subdirs = listing.symlinked_subdirs.clone();
        let subdirs = listing.subdirs;
        let root_visit_key = fs::canonicalize(root).map_err(|err| {
            AppError::Repository(format!(
                "failed to resolve directory {}: {err}",
                root.display()
            ))
        })?;
        let mut seen = HashSet::new();

        for path in subdirs {
            let visit_key =
                visit_key_for_child(&root_visit_key, &path, symlinked_subdirs.contains(&path))
                    .map_err(|err| {
                        AppError::Repository(format!(
                            "failed to resolve directory {}: {err}",
                            path.display()
                        ))
                    })?;
            if seen.insert(visit_key) {
                visitor(path)?;
            }
        }

        Ok(())
    }

    pub fn walk_with<F>(&self, root: &Path, mut visitor: F) -> AppResult<()>
    where
        F: FnMut(WalkedDirectory) -> AppResult<bool>,
    {
        let root_visit_key = fs::canonicalize(root).map_err(|err| {
            AppError::Repository(format!(
                "failed to resolve directory {}: {err}",
                root.display()
            ))
        })?;
        let mut stack = vec![(root.to_path_buf(), root_visit_key.clone(), 0usize)];
        let mut visited = HashSet::new();
        let mut has_visited_root = false;

        while let Some((dir, visit_key, depth)) = stack.pop() {
            if !visited.insert(visit_key.clone()) {
                continue;
            }

            let mut listing = match self.read_directory(&dir) {
                Ok(listing) => listing,
                Err(error)
                    if has_visited_root
                        && self.read_behavior == WalkReadBehavior::SkipUnreadableSubdirectories =>
                {
                    tracing::warn!(
                        path = %dir.display(),
                        error = %error,
                        "skipping unreadable path during filesystem walk"
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };

            has_visited_root = true;
            self.apply_walk_filter(root, &mut listing);
            if self.max_depth.is_none_or(|max_depth| depth < max_depth) {
                stack.extend(
                    listing
                        .subdirs
                        .iter()
                        .rev()
                        .filter(|path| {
                            self.follow_symlinked_directories
                                || !listing.symlinked_subdirs.contains(*path)
                        })
                        .cloned()
                        .filter_map(|path| {
                            let child_visit_key = visit_key_for_child(
                                &visit_key,
                                &path,
                                listing.symlinked_subdirs.contains(&path),
                            )
                            .unwrap_or_else(|_| path.clone());
                            (!self.confine_to_root || child_visit_key.starts_with(&root_visit_key))
                                .then_some((path, child_visit_key, depth.saturating_add(1)))
                        }),
                );
            }
            if !visitor(listing)? {
                return Ok(());
            }
        }

        Ok(())
    }

    pub fn walk(&self, root: &Path) -> AppResult<Vec<WalkedDirectory>> {
        let mut walked = Vec::new();
        self.walk_with(root, |listing| {
            walked.push(listing);
            Ok(true)
        })?;
        Ok(walked)
    }

    fn read_directory(&self, dir: &Path) -> AppResult<WalkedDirectory> {
        let entries = fs::read_dir(dir).map_err(|err| {
            AppError::Repository(format!("failed to read directory {}: {err}", dir.display()))
        })?;

        let mut subdirs = Vec::new();
        let mut files = Vec::new();
        let mut filenames_lower = HashSet::new();
        let mut symlinked_subdirs = HashSet::new();

        for entry in entries {
            let entry = entry.map_err(|err| {
                AppError::Repository(format!("failed to read entry in {}: {err}", dir.display()))
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|err| {
                AppError::Repository(format!(
                    "failed to inspect filesystem entry {}: {err}",
                    path.display()
                ))
            })?;

            let Some(kind) = classify_entry_kind(&path, &file_type) else {
                continue;
            };

            match kind {
                WalkEntryKind::Directory { via_symlink } => {
                    if via_symlink {
                        symlinked_subdirs.insert(path.clone());
                    }
                    subdirs.push(path);
                }
                WalkEntryKind::File => {
                    if !self.should_include_file(&path) {
                        continue;
                    }
                    if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                        filenames_lower.insert(name.to_ascii_lowercase());
                    }
                    files.push(path);
                }
            }
        }

        subdirs.sort();
        files.sort();

        Ok(WalkedDirectory {
            path: dir.to_path_buf(),
            subdirs,
            files,
            filenames_lower,
            symlinked_subdirs,
        })
    }

    fn apply_walk_filter(&self, root: &Path, listing: &mut WalkedDirectory) {
        match self.filter_policy {
            WalkFilterPolicy::None => {}
            WalkFilterPolicy::LibraryScanJunkOnly => {
                listing.subdirs.retain(|path| {
                    !crate::library_discovery::should_skip_library_subpath(root, path, true)
                });
                listing.files.retain(|path| {
                    !crate::library_discovery::should_skip_library_subpath(root, path, false)
                });
            }
            WalkFilterPolicy::EpisodicTitleScan => {
                listing.subdirs.retain(|path| {
                    !crate::library_discovery::should_skip_episodic_library_subpath(
                        root, path, true,
                    )
                });
                listing.files.retain(|path| {
                    !crate::library_discovery::should_skip_episodic_library_subpath(
                        root, path, false,
                    )
                });
            }
            WalkFilterPolicy::MovieTitleScan => {
                listing.subdirs.retain(|path| {
                    !crate::library_discovery::should_skip_movie_library_subpath(root, path, true)
                });
                listing.files.retain(|path| {
                    !crate::library_discovery::should_skip_movie_library_subpath(root, path, false)
                });
            }
        }

        listing
            .symlinked_subdirs
            .retain(|path| listing.subdirs.contains(path));
    }

    fn should_include_file(&self, path: &Path) -> bool {
        match self.file_policy {
            WalkFilePolicy::All => true,
            WalkFilePolicy::SupportedVideoOnly => is_supported_video_file(path),
            WalkFilePolicy::SupportedVideoAndNfo => {
                is_supported_video_file(path) || path_has_extension(path, "nfo")
            }
        }
    }
}

fn is_supported_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| VIDEO_EXTENSIONS.contains(&ext.as_str()))
}

fn path_has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(expected))
}

fn classify_entry_kind(path: &Path, file_type: &fs::FileType) -> Option<WalkEntryKind> {
    if file_type.is_dir() {
        return Some(WalkEntryKind::Directory { via_symlink: false });
    }

    if file_type.is_file() {
        return Some(WalkEntryKind::File);
    }

    if !file_type.is_symlink() {
        return None;
    }

    let metadata = fs::metadata(path).ok()?;
    if metadata.is_dir() {
        Some(WalkEntryKind::Directory { via_symlink: true })
    } else if metadata.is_file() {
        Some(WalkEntryKind::File)
    } else {
        None
    }
}

fn visit_key_for_child(
    parent_visit_key: &Path,
    path: &Path,
    via_symlink: bool,
) -> std::io::Result<PathBuf> {
    if via_symlink {
        fs::canonicalize(path)
    } else {
        Ok(match path.file_name() {
            Some(name) => parent_visit_key.join(name),
            None => path.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn walker_follows_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target");
        let link = dir.path().join("linked-target");
        fs::create_dir_all(&target).expect("target dir");
        fs::write(target.join("episode.mkv"), b"video").expect("video");
        symlink(&target, &link).expect("symlink");

        let walked = FilesystemWalker::new().walk(dir.path()).expect("walk");
        let files = walked
            .iter()
            .flat_map(|entry| entry.files.iter())
            .cloned()
            .collect::<Vec<_>>();

        assert!(files.iter().any(|path| path.ends_with("episode.mkv")));
    }

    #[cfg(unix)]
    #[test]
    fn walker_can_skip_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target");
        let link = dir.path().join("linked-target");
        fs::create_dir_all(&target).expect("target dir");
        fs::write(target.join("episode.mkv"), b"video").expect("video");
        symlink(&target, &link).expect("symlink");

        let walked = FilesystemWalker::new()
            .skip_symlinked_directories()
            .walk(dir.path())
            .expect("walk");

        assert!(walked.iter().any(|entry| entry.path == dir.path()));
        assert!(!walked.iter().any(|entry| entry.path == link));
    }

    #[cfg(unix)]
    #[test]
    fn walker_can_follow_only_symlinked_directories_contained_by_root() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("root tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        fs::write(outside.path().join("episode.mkv"), b"video").expect("video");
        let link = root.path().join("outside-link");
        symlink(outside.path(), &link).expect("symlink");

        let walked = FilesystemWalker::new()
            .confine_to_root()
            .walk(root.path())
            .expect("walk");

        assert!(walked.iter().any(|entry| entry.path == root.path()));
        assert!(!walked.iter().any(|entry| entry.path == link));
        assert!(walked.iter().all(|entry| entry.files.is_empty()));
    }

    #[cfg(unix)]
    #[test]
    fn child_directories_include_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("real-series");
        let link = dir.path().join("linked-series");
        fs::create_dir_all(&target).expect("target dir");
        symlink(&target, &link).expect("symlink");

        let child_dirs = FilesystemWalker::new()
            .list_child_directories(dir.path())
            .expect("child dirs");

        assert_eq!(child_dirs, vec![link]);
    }

    #[cfg(unix)]
    #[test]
    fn walker_avoids_symlink_loops() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let series = dir.path().join("series");
        fs::create_dir_all(&series).expect("series dir");
        fs::write(series.join("episode.mkv"), b"video").expect("video");
        symlink(dir.path(), series.join("loop")).expect("symlink");

        let walked = FilesystemWalker::new().walk(dir.path()).expect("walk");
        let file_count = walked.iter().map(|entry| entry.files.len()).sum::<usize>();

        assert_eq!(file_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn walk_with_can_stop_after_root_listing() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let root_file = dir.path().join("movie.mkv");
        let child_dir = dir.path().join("Season 1");
        let child_file = child_dir.join("episode.mkv");
        fs::write(&root_file, b"movie").expect("root file");
        fs::create_dir_all(&child_dir).expect("child dir");
        fs::write(&child_file, b"episode").expect("child file");
        symlink(&child_dir, dir.path().join("Linked Season 1")).expect("symlink");

        let mut visited = Vec::new();
        FilesystemWalker::new()
            .walk_with(dir.path(), |listing| {
                visited.push(listing.path);
                Ok(false)
            })
            .expect("walk with early stop");

        assert_eq!(visited, vec![dir.path().to_path_buf()]);
    }

    #[test]
    fn walker_can_filter_to_supported_video_files_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("episode.mkv"), b"video").expect("video");
        fs::write(dir.path().join("episode-thumb.jpg"), b"thumb").expect("thumb");
        fs::write(dir.path().join("episode.nfo"), b"<episodedetails />").expect("nfo");

        let walked = FilesystemWalker::new()
            .supported_video_files_only()
            .walk(dir.path())
            .expect("walk");

        assert_eq!(walked.len(), 1);
        assert_eq!(walked[0].files, vec![dir.path().join("episode.mkv")]);
    }

    #[test]
    fn walker_can_filter_to_supported_video_and_nfo_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("movie.mkv"), b"video").expect("video");
        fs::write(dir.path().join("movie.nfo"), b"<movie />").expect("nfo");
        fs::write(dir.path().join("poster.jpg"), b"poster").expect("poster");

        let walked = FilesystemWalker::new()
            .supported_video_and_nfo_files()
            .walk(dir.path())
            .expect("walk");

        assert_eq!(walked.len(), 1);
        assert_eq!(
            walked[0].files,
            vec![dir.path().join("movie.mkv"), dir.path().join("movie.nfo")]
        );
    }
}
