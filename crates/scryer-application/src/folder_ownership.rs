use std::path::Path;

mod scan_cache;
pub(crate) use scan_cache::{ScanClaimReservation, ScanOwnershipContext, with_scan_ownership};

use scryer_domain::Title;

use crate::library::workflow::{LibraryRootState, library_root_state};
use crate::stored_paths::{
    folder_path_match_candidates, folder_paths_match, path_to_stored_string,
    stored_path_to_path_buf,
};
use crate::{AppError, AppResult, AppUseCase};

/// Immutable roots for one scan operation, shared by its title workers.
#[derive(Clone)]
pub(crate) struct ScanFolderRoots {
    all: Vec<String>,
    by_library: std::collections::HashMap<String, Vec<String>>,
}

impl ScanFolderRoots {
    pub(crate) async fn load(app: &AppUseCase) -> AppResult<std::sync::Arc<Self>> {
        let libraries = app.services.catalog.libraries.list(None).await?;
        // `all` is exactly what `AppUseCase::all_library_root_folders` reads,
        // so a check served from the snapshot matches a fresh read.
        let all = crate::catalog_workflow::library_root_folders_from_libraries(&libraries, None)
            .into_iter()
            .map(|root| root.path)
            .collect();
        let by_library = libraries
            .into_iter()
            .map(|library| {
                (
                    library.id,
                    library.roots.into_iter().map(|root| root.path).collect(),
                )
            })
            .collect();
        Ok(std::sync::Arc::new(Self { all, by_library }))
    }

    /// The scan's root snapshot, when the caller runs inside one.
    pub(crate) fn current_all() -> Option<Vec<String>> {
        SCAN_FOLDER_ROOTS
            .try_with(|snapshot| snapshot.all.clone())
            .ok()
    }
}

tokio::task_local! {
    pub(crate) static SCAN_FOLDER_ROOTS: std::sync::Arc<ScanFolderRoots>;
}

fn non_empty_folder_path(path: Option<&str>) -> Option<&str> {
    path.filter(|path| !path.is_empty())
}

/// Run a synchronous folder comparison off the async runtime, inside the
/// caller's tracing span. A panic in `check` resumes in the caller.
pub(crate) async fn run_folder_check_blocking<T: Send + 'static>(
    check: impl FnOnce() -> T + Send + 'static,
) -> T {
    let span = tracing::Span::current();
    match tokio::task::spawn_blocking(move || span.in_scope(check)).await {
        Ok(value) => value,
        Err(error) => std::panic::resume_unwind(error.into_panic()),
    }
}

/// Resolve links in the existing prefix without requiring a destination to
/// exist yet. Callers must repeat this check after creating the destination.
pub(crate) fn resolved_folder_identity(path: &Path) -> AppResult<std::path::PathBuf> {
    if !absolute_without_parent_traversal(path) {
        return Err(AppError::validation_refused(
            crate::error_reasons::folder_ownership::TITLE_FOLDER_INVALID,
            "title folder must be an absolute path without parent traversal",
        ));
    }
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let mut resolved = std::fs::canonicalize(ancestor).map_err(|error| {
                    AppError::validation_refused(
                        crate::error_reasons::folder_ownership::CANNOT_RESOLVE_TITLE_FOLDER,
                        format!(
                            "cannot resolve title folder {}: {error}",
                            ancestor.display(),
                        ),
                    )
                })?;
                for component in missing.into_iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = ancestor.file_name().ok_or_else(|| {
                    AppError::validation_refused(
                        crate::error_reasons::folder_ownership::TITLE_FOLDER_NO_EXISTING_ANCESTOR,
                        "title folder has no existing ancestor",
                    )
                })?;
                missing.push(component.to_os_string());
                ancestor = ancestor.parent().ok_or_else(|| {
                    AppError::validation_refused(
                        crate::error_reasons::folder_ownership::TITLE_FOLDER_NO_EXISTING_ANCESTOR,
                        "title folder has no existing ancestor",
                    )
                })?;
            }
            Err(error) => {
                return Err(AppError::validation_refused(
                    crate::error_reasons::folder_ownership::CANNOT_INSPECT_TITLE_FOLDER,
                    format!(
                        "cannot inspect title folder {}: {error}",
                        ancestor.display()
                    ),
                ));
            }
        }
    }
}

/// Whether two stored folders are the same folder or one holds the other, as
/// spelled. Nothing on disk is read, so a link or a case-insensitive
/// filesystem can make two folders overlap that this calls unrelated;
/// [`title_folders_overlap`] resolves those.
pub(crate) fn stored_title_folders_overlap(left: &str, right: &str) -> bool {
    folder_paths_match(left, right)
        || crate::title_folder_rules::path_is_strictly_within(left, right)
        || crate::title_folder_rules::path_is_strictly_within(right, left)
}

fn absolute_without_parent_traversal(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}

/// Whether two stored folders overlap once links are resolved.
///
/// `left` is the folder the caller is about to use, so it must resolve.
/// `right` is another title's folder: when part of it cannot be read it is
/// still compared, as [`UnreadableFolders`] describes, and never refuses.
pub(crate) fn title_folders_overlap(left: &str, right: &str) -> AppResult<bool> {
    OverlapLeft::new(left).overlaps(right)
}

/// One folder compared with many others by [`title_folders_overlap`],
/// resolving it on disk only once. A failed resolution is not kept, so the
/// next comparison tries again exactly as a fresh call would.
pub(crate) struct OverlapLeft {
    stored: String,
    resolved: Option<std::path::PathBuf>,
    others: UnreadableFolders,
}

impl OverlapLeft {
    pub(crate) fn new(stored: &str) -> Self {
        Self {
            stored: stored.to_string(),
            resolved: None,
            others: UnreadableFolders::default(),
        }
    }

    pub(crate) fn overlaps(&mut self, right: &str) -> AppResult<bool> {
        fn overlaps(left: &str, right: &str) -> bool {
            stored_title_folders_overlap(left, right)
        }
        if overlaps(&self.stored, right) {
            return Ok(true);
        }
        let left = match &self.resolved {
            Some(resolved) => resolved.clone(),
            None => {
                let resolved = resolved_folder_identity(&stored_path_to_path_buf(&self.stored))?;
                self.resolved = Some(resolved.clone());
                resolved
            }
        };
        let other = stored_path_to_path_buf(right);
        if !absolute_without_parent_traversal(&other) {
            tracing::debug!(
                folder = right,
                "invalid unrelated stored folder omitted from alias comparison"
            );
            return Ok(false);
        }
        let right = self.others.resolve(&other);
        let left_string = path_to_stored_string(&left);
        let right_string = path_to_stored_string(&right.path);
        // The case behaviour of a folder that cannot be read is unknown, so
        // it is compared without regard to case.
        Ok(overlaps(&left_string, &right_string)
            || (overlaps(&left_string.to_lowercase(), &right_string.to_lowercase())
                && (!right.whole
                    || folder_case_insensitive(&left)
                    || folder_case_insensitive(&right.path))))
    }
}

/// Another title's folder, resolved as far as it can be read.
struct OtherFolder {
    path: std::path::PathBuf,
    /// False when part of the folder could not be read and is kept as spelled.
    whole: bool,
}

/// Resolves the folders of other titles for one check.
///
/// Another title's folder can sit on a drive that is unplugged, a share that
/// is down or a directory this process may not read. That says nothing about
/// the folder being checked, so it never refuses the check: links are
/// resolved in the part of the folder that can be read, and the rest is
/// compared as spelled. A link at or below the unreadable point cannot be
/// seen.
///
/// What a folder answered is remembered for the rest of the check, so the
/// folders that many titles share are read once, and nothing below a folder
/// that is missing or dead is probed at all.
#[derive(Default)]
struct UnreadableFolders {
    seen: std::collections::HashMap<std::path::PathBuf, SeenFolder>,
    #[cfg(test)]
    probes: u64,
}

enum SeenFolder {
    /// It exists and resolves to this. Kept only for a folder reached
    /// through one below it, which other titles are likely to share.
    Resolved(std::path::PathBuf),
    /// It does not exist, so nothing below it does.
    Missing,
    /// It refused access, or it is there and does not resolve, as a link
    /// that cannot be followed. A folder inside it can still answer (on
    /// Windows, inside a folder that refuses), so only this folder is known
    /// to be unreadable.
    Refused,
    /// It could not be inspected at all, and nothing below it can be either.
    Dead,
}

impl UnreadableFolders {
    fn resolve(&mut self, path: &Path) -> OtherFolder {
        let mut whole = true;
        // The highest folder known missing or dead: nothing from `path` up
        // to it needs a probe.
        let mut skip_through = None;
        if !self.seen.is_empty() {
            for ancestor in path.ancestors() {
                match self.seen.get(ancestor) {
                    Some(SeenFolder::Dead) => {
                        whole = false;
                        skip_through = Some(ancestor);
                    }
                    Some(SeenFolder::Missing) => skip_through = Some(ancestor),
                    Some(SeenFolder::Resolved(_) | SeenFolder::Refused) | None => {}
                }
            }
        }
        let mut ancestor = path;
        let mut below = Vec::new();
        loop {
            if let Some(skipped) = skip_through {
                if ancestor == skipped {
                    skip_through = None;
                }
            } else {
                match self.seen.get(ancestor) {
                    Some(SeenFolder::Resolved(resolved)) => {
                        let mut resolved = resolved.clone();
                        resolved.extend(below.iter().rev());
                        return OtherFolder {
                            path: resolved,
                            whole,
                        };
                    }
                    Some(SeenFolder::Refused | SeenFolder::Dead) => whole = false,
                    Some(SeenFolder::Missing) => {}
                    None => match self.probe(ancestor, ancestor != path) {
                        Ok(mut resolved) => {
                            resolved.extend(below.iter().rev());
                            return OtherFolder {
                                path: resolved,
                                whole,
                            };
                        }
                        Err(readable_above) => whole &= readable_above,
                    },
                }
            }
            let (Some(name), Some(parent)) = (ancestor.file_name(), ancestor.parent()) else {
                return OtherFolder {
                    path: path.to_path_buf(),
                    whole: false,
                };
            };
            below.push(name);
            ancestor = parent;
        }
    }

    /// Read `folder` and remember what it answered. The error says whether
    /// the folder is only missing, which leaves the folder above it to answer
    /// for it in full.
    fn probe(&mut self, folder: &Path, shared: bool) -> Result<std::path::PathBuf, bool> {
        self.probed();
        let (failure, seen) = match std::fs::symlink_metadata(folder) {
            Ok(_) => {
                self.probed();
                match std::fs::canonicalize(folder) {
                    Ok(resolved) => {
                        if shared {
                            self.seen.insert(
                                folder.to_path_buf(),
                                SeenFolder::Resolved(resolved.clone()),
                            );
                        }
                        return Ok(resolved);
                    }
                    Err(error) => (error, SeenFolder::Refused),
                }
            }
            // A missing folder resolves through its parent. A missing drive
            // or share has none, and nothing on it can be read.
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && folder.parent().is_some() =>
            {
                self.seen.insert(folder.to_path_buf(), SeenFolder::Missing);
                return Err(true);
            }
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                (error, SeenFolder::Refused)
            }
            Err(error) => (error, SeenFolder::Dead),
        };
        tracing::debug!(
            folder = %folder.display(),
            error = %failure,
            "folder of another title cannot be read; compared as spelled"
        );
        self.seen.insert(folder.to_path_buf(), seen);
        Err(false)
    }

    /// Whether `folder` is at or below a folder that could not be inspected
    /// at all.
    fn dead(&self, folder: &Path) -> bool {
        folder
            .ancestors()
            .any(|ancestor| matches!(self.seen.get(ancestor), Some(SeenFolder::Dead)))
    }

    fn probed(&mut self) {
        #[cfg(test)]
        {
            self.probes += 1;
        }
    }
}

/// Probe existing directory entries without writing to the library. Unknown
/// case behavior is treated conservatively when reserving a destination.
pub(crate) fn folder_case_insensitive(path: &Path) -> bool {
    let Some(parent) = path.ancestors().find(|ancestor| ancestor.is_dir()) else {
        return true;
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return true;
    };
    for entry in entries.take(16).flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((index, character)) = name.char_indices().find(|(_, c)| c.is_ascii_alphabetic())
        else {
            continue;
        };
        let mut alternative = name.to_string();
        alternative.replace_range(
            index..index + 1,
            &if character.is_ascii_uppercase() {
                character.to_ascii_lowercase()
            } else {
                character.to_ascii_uppercase()
            }
            .to_string(),
        );
        let alternative = parent.join(alternative);
        match std::fs::metadata(&alternative) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
            Ok(other) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if let Ok(original) = std::fs::metadata(entry.path()) {
                        return original.dev() == other.dev() && original.ino() == other.ino();
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = other;
                    return true;
                }
            }
            Err(_) => {}
        }
    }
    true
}

pub(crate) fn title_owns_folder(title: &Title, folder_path: &Path) -> bool {
    let Some(owned_path) = non_empty_folder_path(title.folder_path.as_deref()) else {
        return false;
    };
    folder_paths_match(owned_path, &path_to_stored_string(folder_path))
}

pub(crate) fn title_owns_another_folder(title: &Title, folder_path: &Path) -> bool {
    non_empty_folder_path(title.folder_path.as_deref()).is_some()
        && !title_owns_folder(title, folder_path)
}

pub(crate) fn title_folder_path(title: &Title) -> Option<&str> {
    non_empty_folder_path(title.folder_path.as_deref())
}

/// Whether the folder a title records as its own is still on disk.
///
/// `None` means the answer is unknown (an unreachable mount, a permission
/// problem). Callers treat that as "not proven" and take no action on it.
async fn owned_folder_present(owned_path: &str) -> Option<bool> {
    match tokio::fs::metadata(stored_path_to_path_buf(owned_path)).await {
        Ok(_) => Some(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// A title's recorded folder is stale when it no longer exists while the
/// directory that held it still has entries.
///
/// `folder_path` is written once when a title first claims a folder and is
/// never re-validated, so files relocated outside Scryer (a root split, a
/// drive migration, another tool's root-folder move) leave every affected
/// title pointing at a folder that is gone. The parent-directory check mirrors
/// the mount guard in [`library_root_state`]: a missing or empty parent is the
/// signature of a vanished mount, and nothing under it can be called gone.
pub(crate) async fn title_folder_is_stale(title: &Title) -> bool {
    let Some(owned_path) = non_empty_folder_path(title.folder_path.as_deref()) else {
        return false;
    };
    if owned_folder_present(owned_path).await != Some(false) {
        return false;
    }
    let owned_path = stored_path_to_path_buf(owned_path);
    let Some(parent) = owned_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return false;
    };
    matches!(
        library_root_state(parent).await,
        Ok(LibraryRootState::Populated)
    )
}

/// Whether `folder_path` is a library root or holds one.
///
/// No title owns such a folder. Recording one turns everything scoped to "the
/// title's folder" into something scoped to a whole library.
pub(crate) async fn folder_spans_a_library_root(
    app: &AppUseCase,
    folder_path: &str,
) -> AppResult<bool> {
    let snapshot = SCAN_FOLDER_ROOTS.try_with(std::sync::Arc::clone).ok();
    let loaded;
    let all_roots = if let Some(snapshot) = snapshot.as_ref() {
        &snapshot.all
    } else {
        loaded = app
            .all_library_root_folders()
            .await?
            .into_iter()
            .map(|root| root.path)
            .collect::<Vec<_>>();
        &loaded
    };
    Ok(matches!(
        crate::title_folder_rules::title_folder_root_violation(folder_path, all_roots, all_roots),
        Some(
            crate::title_folder_rules::TitleFolderRootViolation::EqualsRoot
                | crate::title_folder_rules::TitleFolderRootViolation::ContainsRoot
        )
    ))
}

/// Whether `folder_path` lies outside every root of `title`'s own library.
///
/// [`folder_spans_a_library_root`] checks against every library's roots, so a
/// folder inside another library's root passes it. A title folder belongs to
/// its own library, so claiming one elsewhere would hand the title a folder
/// that library's scans, renames and moves never look at. A library with no
/// configured roots places titles under the facet default, so there is nothing
/// to hold the folder against and it passes.
pub(crate) async fn title_folder_is_outside_its_library(
    app: &AppUseCase,
    title: &Title,
    folder_path: &str,
) -> AppResult<bool> {
    let snapshot_roots = SCAN_FOLDER_ROOTS
        .try_with(|snapshot| {
            snapshot
                .by_library
                .get(&title.library_id)
                .cloned()
                .unwrap_or_default()
        })
        .ok();
    let library_roots = if let Some(roots) = snapshot_roots {
        roots
    } else {
        app.services
            .catalog
            .libraries
            .get_by_id(&title.library_id)
            .await?
            .map(|library| {
                library
                    .roots
                    .into_iter()
                    .map(|root| root.path)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    if library_roots.is_empty() {
        return Ok(false);
    }
    Ok(!library_roots
        .iter()
        .any(|root| crate::title_folder_rules::path_is_strictly_within(folder_path, root)))
}

/// Whether a title's recorded folder is a library root or holds one.
///
/// Such a record is damage, and treating it as ownership would make every
/// other folder the title's files are in look like a second copy.
pub(crate) async fn title_folder_spans_a_library_root(
    app: &AppUseCase,
    title: &Title,
) -> AppResult<bool> {
    match non_empty_folder_path(title.folder_path.as_deref()) {
        Some(owned_path) => folder_spans_a_library_root(app, owned_path).await,
        None => Ok(false),
    }
}

/// Refuse to place or look for a title's files by a recorded folder that is a
/// library root or holds one.
///
/// Importing by such a record drops the file straight into the library root,
/// and walking it reads every other title's files as this title's. The
/// operator corrects the title's folder first.
pub(crate) async fn ensure_title_folder_is_not_a_library_root(
    app: &AppUseCase,
    title: &Title,
) -> AppResult<()> {
    if let Some(folder) = non_empty_folder_path(title.folder_path.as_deref())
        && title_folder_is_outside_its_library(app, title, folder).await?
    {
        return Err(AppError::validation_refused(
            crate::error_reasons::folder_ownership::PATH_OUTSIDE_LIBRARY_ROOT,
            format!(
                "the recorded folder for \"{}\" is outside its library roots; correct the title's folder first",
                title.name
            ),
        ));
    }
    if !title_folder_spans_a_library_root(app, title).await? {
        return Ok(());
    }
    Err(AppError::validation_refused(
        crate::error_reasons::folder_ownership::FOLDER_NOT_TITLE_BOUNDARY,
        format!(
            "the recorded folder for \"{}\" ({}) is a library root, not a title folder; correct the title's folder first",
            title.name,
            crate::stored_paths::stored_path_to_display_string(
                title.folder_path.as_deref().unwrap_or_default()
            )
        ),
    ))
}

pub(crate) async fn title_folder_is_invalid(app: &AppUseCase, title: &Title) -> AppResult<bool> {
    let Some(folder) = non_empty_folder_path(title.folder_path.as_deref()) else {
        return Ok(false);
    };
    Ok(folder_spans_a_library_root(app, folder).await?
        || title_folder_is_outside_its_library(app, title, folder).await?)
}

/// Move a title's stale folder ownership to the folder a scan found its
/// files in. Returns `false`, changing nothing, when another title already
/// owns that folder — that is a real conflict, not a heal — or when the folder
/// is a library root or outside every root of the title's library.
pub(crate) async fn reclaim_stale_title_folder(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
) -> AppResult<bool> {
    let previous_folder_path = title.folder_path.clone().unwrap_or_default();
    let folder_path = path_to_stored_string(folder_path);
    if folder_spans_a_library_root(app, &folder_path).await?
        || title_folder_is_outside_its_library(app, title, &folder_path).await?
        || find_other_folder_owner(app, title, &folder_path)
            .await?
            .is_some()
    {
        return Ok(false);
    }
    let write = crate::TitleFolderWrite::assign(&title.id, &folder_path)
        .expecting(title.folder_path.clone());
    match app
        .services
        .catalog
        .titles
        .assign_title_folders(&[write])
        .await
    {
        Ok(()) => {}
        // Another writer took the folder, or changed this title's, after the
        // checks above: the same outcome as finding the conflict there.
        Err(error) if title_folder_write_lost(&error) => return Ok(false),
        Err(error) => return Err(error),
    }
    title.folder_path =
        Some(recorded_folder_after_claim(app, &title.id, folder_path.clone()).await?);
    tracing::info!(
        title_id = %title.id,
        previous_folder_path = %previous_folder_path,
        folder_path = %folder_path,
        "re-pointed stale title folder ownership to the scanned folder"
    );
    Ok(true)
}

/// Whether a title folder write was refused because the catalog moved on
/// between the caller's checks and the write: another title holds or nests
/// with the folder, or the title's own folder changed.
pub(crate) fn title_folder_write_lost(error: &AppError) -> bool {
    use crate::error_reasons::folder_ownership as reasons;
    matches!(
        error.validation_reason(),
        Some(
            reasons::FOLDER_ALREADY_OWNED_TITLE
                | reasons::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE
                | reasons::TITLE_FOLDER_CHANGED
        )
    )
}

pub(crate) async fn find_other_folder_owner(
    app: &AppUseCase,
    title: &Title,
    folder_path: &str,
) -> AppResult<Option<Title>> {
    // Ownership is decided from the id and the folder path alone, and a scan or
    // pending-import pass asks this question once per folder it touches —
    // 11,988 times over a three-library scan of a 111k-file test library.
    // Reading every title in the library per folder made that the single most
    // expensive statement of a scan, so the candidate spellings of the folder
    // go to the repository and it narrows the read; `folder_paths_match` below
    // is still what decides.
    let match_candidates = folder_path_match_candidates(folder_path);
    Ok(app
        .services
        .catalog
        .titles
        .list_folder_path_owner_candidates(&title.library_id, &title.id, &match_candidates)
        .await?
        .into_iter()
        .find(|candidate| {
            candidate.id != title.id
                && non_empty_folder_path(candidate.folder_path.as_deref())
                    .is_some_and(|owned_path| folder_paths_match(owned_path, folder_path))
        }))
}

/// Lowercased containment keys a folder is compared under before any
/// filesystem resolution. Lowercasing only widens the set of owners that go
/// on to the exact, resolving comparison.
fn overlap_prefilter_key(folder: &str) -> Option<String> {
    crate::title_folder_rules::folder_containment_key(folder).map(|key| key.to_lowercase())
}

fn overlap_keys_related(left: &str, right: &str) -> bool {
    fn within(path: &str, folder: &str) -> bool {
        path.strip_prefix(folder)
            .is_some_and(|rest| rest.is_empty() || folder.ends_with('/') || rest.starts_with('/'))
    }
    within(left, right) || within(right, left)
}

/// One library root as the overlap prefilter compares owners against it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RootOverlapKeys {
    key: String,
    /// The key of the root with its links resolved as far as it can be read,
    /// or `None` when the root is not a path that can be resolved.
    resolved: Option<String>,
    /// The root as spelled. Keys ignore case, so on a volume that does not, a
    /// folder can share a root's key and be another folder: only a folder
    /// spelled as the root is taken for it.
    spelled: std::path::PathBuf,
    /// Nothing at or below the root can be read: an owner under it is
    /// compared by its keys alone.
    dead: bool,
}

/// What one probe of a folder says to the link walk.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkProbe {
    /// Not a link, or a folder that refused access: whether that one is a
    /// link cannot be told, which is part of what an unreadable folder costs.
    /// A folder inside it can still answer on Windows, so the walk goes on.
    Plain,
    Link,
    /// Missing, or failed some other way: nothing at or below it can be
    /// inspected, so nothing there is probed.
    Gone,
}

impl LinkProbe {
    pub(crate) fn of(metadata: &std::io::Result<std::fs::Metadata>) -> Self {
        match metadata {
            Ok(metadata) if metadata.file_type().is_symlink() => Self::Link,
            Ok(_) => Self::Plain,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Self::Plain,
            Err(_) => Self::Gone,
        }
    }
}

/// Keys for comparing one move candidate against every recorded owner while
/// resolving the candidate and each library root only once.
pub(crate) struct FolderOverlapKeys {
    candidate: Vec<String>,
    candidate_resolved: std::path::PathBuf,
    roots: Vec<RootOverlapKeys>,
    /// What each folder walked for links answered, shared by owners under
    /// the same organizing folders.
    links: std::collections::HashMap<std::path::PathBuf, LinkProbe>,
    /// Whether `links` holds a folder that is gone.
    gone: bool,
    link_probes: u64,
}

impl FolderOverlapKeys {
    async fn root_paths(app: &AppUseCase) -> AppResult<Vec<String>> {
        Ok(match ScanFolderRoots::current_all() {
            Some(roots) => roots,
            None => app
                .all_library_root_folders()
                .await?
                .into_iter()
                .map(|root| root.path)
                .collect(),
        })
    }

    pub(crate) fn from_roots(folder_path: &str, root_paths: &[String]) -> AppResult<Self> {
        let resolved = resolved_folder_identity(&stored_path_to_path_buf(folder_path))?;
        let mut candidate: Vec<String> = [
            overlap_prefilter_key(folder_path),
            overlap_prefilter_key(&path_to_stored_string(&resolved)),
        ]
        .into_iter()
        .flatten()
        .collect();
        candidate.dedup();
        // A root that cannot be read does not refuse the check. It is
        // resolved as far as it can be read, as another title's folder is.
        let mut unreadable = UnreadableFolders::default();
        let roots = root_paths
            .iter()
            .filter_map(|root| {
                let key = overlap_prefilter_key(root)?;
                let path = stored_path_to_path_buf(root);
                if !absolute_without_parent_traversal(&path) {
                    return Some(RootOverlapKeys {
                        key,
                        resolved: None,
                        spelled: path,
                        dead: true,
                    });
                }
                let resolved = unreadable.resolve(&path);
                Some(RootOverlapKeys {
                    key,
                    resolved: overlap_prefilter_key(&path_to_stored_string(&resolved.path)),
                    dead: unreadable.dead(&path),
                    spelled: path,
                })
            })
            .collect();
        Ok(Self {
            candidate,
            candidate_resolved: resolved,
            roots,
            links: Default::default(),
            gone: false,
            link_probes: 0,
        })
    }

    /// The exact comparison for the candidate these keys were built for,
    /// `folder_path`, without resolving it a second time.
    pub(crate) fn exact_comparison(&self, folder_path: &str) -> OverlapLeft {
        OverlapLeft {
            stored: folder_path.to_string(),
            resolved: Some(self.candidate_resolved.clone()),
            others: UnreadableFolders::default(),
        }
    }

    /// Whether `owned_path` needs the exact, resolving overlap comparison.
    /// An owner unrelated by key, also after substituting its resolved library
    /// root, with no link between that root and its folder, cannot overlap.
    /// A folder on the way that cannot be inspected is not counted as a link:
    /// the exact comparison could not read through it either.
    pub(crate) fn needs_exact_comparison(&mut self, owned_path: &str) -> bool {
        self.needs_exact_comparison_with_evidence(owned_path, &mut |_| None)
    }

    /// `evidence` answers for a folder it already knows or probes itself, and
    /// `None` for one it did not probe.
    fn needs_exact_comparison_with_evidence(
        &mut self,
        owned_path: &str,
        evidence: &mut impl FnMut(&Path) -> Option<LinkProbe>,
    ) -> bool {
        let prepared = OwnerOverlapPrefilter::prepare(owned_path, &self.roots);
        self.needs_exact_comparison_prepared(&prepared, evidence)
    }

    /// The library root keys these keys were built with, which an
    /// [`OwnerOverlapPrefilter`] must have been prepared with.
    pub(crate) fn root_keys(&self) -> &[RootOverlapKeys] {
        &self.roots
    }

    /// [`Self::needs_exact_comparison_with_evidence`] for an owner whose
    /// candidate-independent part was prepared with [`Self::root_keys`].
    pub(crate) fn needs_exact_comparison_prepared(
        &mut self,
        prepared: &OwnerOverlapPrefilter,
        evidence: &mut impl FnMut(&Path) -> Option<LinkProbe>,
    ) -> bool {
        let OwnerOverlapPrefilter::Keys {
            keys: owner_keys,
            path,
            chain,
        } = prepared
        else {
            return true;
        };
        if owner_keys.iter().any(|owner| {
            self.candidate
                .iter()
                .any(|candidate| overlap_keys_related(owner, candidate))
        }) {
            return true;
        }
        // Nothing at or below a folder already found gone is probed: a share
        // that is down is asked once, not once for every owner on it.
        let mut skip = 0;
        if self.gone {
            for (depth, ancestor) in path.ancestors().take(*chain).enumerate() {
                match self.links.get(ancestor) {
                    Some(LinkProbe::Link) => return true,
                    Some(LinkProbe::Gone) => skip = depth + 1,
                    Some(LinkProbe::Plain) | None => {}
                }
            }
        }
        for ancestor in path.ancestors().take(*chain).skip(skip) {
            let probe = match self.links.get(ancestor) {
                Some(probe) => *probe,
                None => {
                    let probe = evidence(ancestor).unwrap_or_else(|| {
                        self.link_probes += 1;
                        LinkProbe::of(&std::fs::symlink_metadata(ancestor))
                    });
                    self.gone |= probe == LinkProbe::Gone;
                    self.links.insert(ancestor.to_path_buf(), probe);
                    probe
                }
            };
            if probe == LinkProbe::Link {
                return true;
            }
        }
        false
    }
}

/// The part of [`FolderOverlapKeys::needs_exact_comparison`] that depends only
/// on one owner's folder and the library root keys, computed once and reused
/// for every candidate compared with the same roots. Pure: preparing reads no
/// file and no catalog row.
pub(crate) enum OwnerOverlapPrefilter {
    /// The owner needs the exact comparison whatever the candidate: its folder
    /// has no key or is relative.
    Exact,
    Keys {
        /// The owner's key and its key under each root it lies in, with that
        /// root resolved as far as it can be read.
        keys: Vec<String>,
        path: std::path::PathBuf,
        /// How many of `path`'s ancestors, from `path` itself, lie below the
        /// nearest library root spelled as one of them, and are walked for
        /// links. Every ancestor when there is no such root: the scan index
        /// then asks this owner on every claim. Zero when nothing at or below
        /// that root can be read: the owner is then compared by its keys,
        /// which is all the exact comparison could do.
        chain: usize,
    },
}

impl OwnerOverlapPrefilter {
    pub(crate) fn prepare(owned_path: &str, roots: &[RootOverlapKeys]) -> Self {
        let Some(owner) = overlap_prefilter_key(owned_path) else {
            return Self::Exact;
        };
        let mut keys = vec![owner.clone()];
        for root in roots {
            let RootOverlapKeys {
                key: root,
                resolved,
                ..
            } = root;
            if let Some(resolved) = resolved
                && overlap_keys_related(&owner, root)
                && owner.len() >= root.len()
            {
                let rest = owner[root.len()..].trim_start_matches('/');
                keys.push(if rest.is_empty() {
                    resolved.clone()
                } else {
                    format!("{}/{rest}", resolved.trim_end_matches('/'))
                });
            }
        }
        let path = stored_path_to_path_buf(owned_path);
        if !path.is_absolute() {
            return Self::Exact;
        }
        let mut chain = 0;
        for ancestor in path.ancestors() {
            let key = overlap_prefilter_key(&path_to_stored_string(ancestor));
            // Root keys ignore case. A folder that shares a root's key and is
            // spelled another way can be another folder, and a link: the
            // root's resolved key says nothing of it, so the walk goes on
            // through it and above it.
            let mut here = roots
                .iter()
                .filter(|root| key.as_ref() == Some(&root.key) && root.spelled == ancestor)
                .peekable();
            if here.peek().is_some() {
                if here.all(|root| root.dead) {
                    chain = 0;
                }
                break;
            }
            chain += 1;
        }
        Self::Keys { keys, path, chain }
    }
}

/// [`ensure_folder_move_available_to_title`] ahead of recording the folder
/// for `title`. During a scan an accepted folder is also reserved for `title`
/// until the returned reservation settles, so a concurrent claim of a folder
/// that overlaps it is refused as if it were already recorded. Outside a scan
/// this is exactly [`ensure_folder_move_available_to_title`].
async fn ensure_folder_claim_available_to_title(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<Option<ScanClaimReservation>> {
    if !folder_path.as_os_str().is_empty()
        && let Some(context) = ScanOwnershipContext::current()
    {
        return context
            .check_and_reserve(app, title, path_to_stored_string(folder_path))
            .await
            .map(Some);
    }
    ensure_folder_move_available_to_title(app, title, folder_path)
        .await
        .map(|()| None)
}

pub(crate) async fn ensure_folder_move_available_to_title(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<()> {
    if folder_path.as_os_str().is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::folder_ownership::TITLE_FOLDER_PATH_REQUIRED,
            "title folder path is required",
        ));
    }
    let folder_path = path_to_stored_string(folder_path);
    if let Some(context) = ScanOwnershipContext::current() {
        return context.check(app, title, folder_path).await;
    }
    let owners = app
        .services
        .catalog
        .titles
        .list_delete_preview_info()
        .await?;
    let Some(first) = owners
        .iter()
        .position(|owner| move_check_compares_owner(owner, &title.id))
    else {
        return Ok(());
    };
    let root_paths = FolderOverlapKeys::root_paths(app).await?;
    let mut scan = MoveOverlapScan {
        title_id: title.id.clone(),
        left: OverlapLeft::new(&folder_path),
        folder_path,
        owners,
        next: first,
        keys: None,
        root_paths,
    };
    loop {
        let (returned, found) = run_folder_check_blocking(move || {
            let found = scan.next_overlap();
            (scan, found)
        })
        .await;
        scan = returned;
        let Some(index) = found? else {
            return Ok(());
        };
        let owner = &scan.owners[index];
        let owned_path = owner.folder_path.as_deref().unwrap_or_default();
        if !folder_spans_a_library_root(app, owned_path).await? {
            return Err(AppError::validation_refused(
                crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE,
                format!(
                    "folder {} overlaps the folder owned by title {}",
                    scan.folder_path, owner.title_id
                ),
            ));
        }
    }
}

fn move_check_compares_owner(owner: &crate::TitleDeletePreviewInfo, title_id: &str) -> bool {
    owner
        .folder_path
        .as_deref()
        .is_some_and(|path| !path.is_empty())
        && owner.title_id != title_id
}

/// The synchronous part of [`ensure_folder_move_available_to_title`]: walks
/// the owners in order and stops at each overlapping one so the caller can
/// apply the library-root exemption before going on.
struct MoveOverlapScan {
    title_id: String,
    folder_path: String,
    owners: Vec<crate::TitleDeletePreviewInfo>,
    next: usize,
    keys: Option<FolderOverlapKeys>,
    root_paths: Vec<String>,
    left: OverlapLeft,
}

impl MoveOverlapScan {
    fn next_overlap(&mut self) -> AppResult<Option<usize>> {
        while self.next < self.owners.len() {
            let index = self.next;
            self.next += 1;
            let owner = &self.owners[index];
            if !move_check_compares_owner(owner, &self.title_id) {
                continue;
            }
            let owned_path = owner.folder_path.as_deref().unwrap_or_default();
            if self.keys.is_none() {
                self.keys = Some(FolderOverlapKeys::from_roots(
                    &self.folder_path,
                    &self.root_paths,
                )?);
            }
            if !self
                .keys
                .as_mut()
                .expect("overlap keys loaded")
                .needs_exact_comparison(owned_path)
            {
                continue;
            }
            let folder_path = &self.folder_path;
            let overlaps = self.left.overlaps(owned_path)
                .map_err(|error| {
                    AppError::validation_refused(crate::error_reasons::folder_ownership::CANNOT_COMPARE_FOLDER_FOLDER_OWNED_TITLE, format!(
                        "cannot compare folder {folder_path} with folder {owned_path} owned by title {}: {error}",
                        owner.title_id
                    ))
                })?;
            if overlaps {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }
}

/// Recycle entries retain their original path even after a title is removed.
/// Restoring one must not write into a folder another title has since claimed.
pub(crate) async fn ensure_restore_ownership(
    app: &AppUseCase,
    title_id: Option<&str>,
    destination: &Path,
) -> AppResult<()> {
    let owners = app
        .services
        .catalog
        .titles
        .list_delete_preview_info()
        .await?;
    let destination = path_to_stored_string(destination);
    let title_id = title_id.map(str::to_string);
    run_folder_check_blocking(move || {
        restore_ownership_check(title_id.as_deref(), &destination, &owners)
    })
    .await
}

fn restore_ownership_check(
    title_id: Option<&str>,
    destination: &str,
    owners: &[crate::TitleDeletePreviewInfo],
) -> AppResult<()> {
    let current_folder = owners
        .iter()
        .find(|owner| Some(owner.title_id.as_str()) == title_id)
        .and_then(|owner| owner.folder_path.as_deref())
        .filter(|folder| !folder.is_empty());
    if let Some(folder) = current_folder {
        let resolved_folder = resolved_folder_identity(&stored_path_to_path_buf(folder))?;
        let resolved_destination = resolved_folder_identity(&stored_path_to_path_buf(destination))?;
        if !crate::title_folder_rules::path_is_strictly_within(destination, folder)
            || resolved_destination == resolved_folder
            || !resolved_destination.starts_with(&resolved_folder)
        {
            return Err(AppError::validation_refused(crate::error_reasons::folder_ownership::RECYCLE_RESTORE_PATH_OUTSIDE_TITLE_S_CURRENT_OWNED_FOLDER, "recycle restore path is outside the title's current owned folder; resolve the folder change before restoring"));
        }
    }
    let mut checked = OverlapLeft::new(current_folder.unwrap_or(destination));
    for owner in owners {
        if Some(owner.title_id.as_str()) == title_id {
            continue;
        }
        if let Some(folder) = owner
            .folder_path
            .as_deref()
            .filter(|folder| !folder.is_empty())
            && checked.overlaps(folder)?
        {
            return Err(AppError::validation_refused(crate::error_reasons::folder_ownership::RECYCLE_RESTORE_DESTINATION_OVERLAPS_ANOTHER_TITLE_S_OWNED_FOLDER, "recycle restore destination overlaps another title's owned folder; recycled files were preserved"));
        }
    }
    Ok(())
}

pub(crate) async fn ensure_folder_available_to_title(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<()> {
    if folder_path.as_os_str().is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::folder_ownership::TITLE_FOLDER_PATH_REQUIRED,
            "title folder path is required",
        ));
    }
    let folder_path = path_to_stored_string(folder_path);

    if non_empty_folder_path(title.folder_path.as_deref())
        .is_some_and(|owned_path| folder_paths_match(owned_path, &folder_path))
    {
        return Ok(());
    }

    if let Some(owner) = find_other_folder_owner(app, title, &folder_path).await? {
        let owned_path = owner.folder_path.as_deref().unwrap_or_default();
        return Err(AppError::validation_refused(
            crate::error_reasons::folder_ownership::FOLDER_ALREADY_OWNED_TITLE,
            format!(
                "folder {owned_path} is already owned by title {}",
                owner.name
            ),
        ));
    }

    if let Some(owned_path) = non_empty_folder_path(title.folder_path.as_deref()) {
        return Err(AppError::validation_refused(
            crate::error_reasons::folder_ownership::TITLE_ALREADY_OWNS_ANOTHER_FOLDER,
            format!(
                "title {} already owns another folder: {owned_path}",
                title.name
            ),
        ));
    }

    Ok(())
}

/// Recheck catalog ownership immediately before an explicit destructive action.
pub(crate) async fn claim_title_folder_for_delete(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
) -> AppResult<()> {
    ensure_folder_available_to_title(app, title, folder_path).await?;
    if non_empty_folder_path(title.folder_path.as_deref()).is_some() {
        return Ok(());
    }
    let stored = path_to_stored_string(folder_path);
    let owners = app
        .services
        .catalog
        .titles
        .list_delete_preview_info()
        .await?;
    let stored = {
        let title_id = title.id.clone();
        run_folder_check_blocking(move || {
            let mut checked = OverlapLeft::new(&stored);
            for owner in &owners {
                if owner.title_id == title_id {
                    continue;
                }
                if let Some(other) = owner.folder_path.as_deref().filter(|path| !path.is_empty())
                    && checked.overlaps(other)?
                {
                    return Err(AppError::validation_refused(
                        crate::title_delete::FOLDER_OVERLAPS_TITLE,
                        "title folder overlaps another title's recorded folder",
                    ));
                }
            }
            Ok(stored)
        })
        .await?
    };
    let mut reservation = ensure_unowned_folder_claimable(
        app,
        title,
        folder_path,
        TitleFolderClaimLayout::Configured,
    )
    .await?;
    let write = crate::TitleFolderWrite::assign(&title.id, &stored).expecting(None);
    match assign_claimed_title_folder(app, write, &mut reservation).await {
        Ok(()) => {}
        Err(error) if title_folder_write_lost(&error) => {
            return Err(AppError::validation_refused(
                crate::title_delete::FOLDER_CLAIM_LOST,
                "title folder ownership changed before deletion",
            ));
        }
        Err(error) => return Err(error),
    }
    title.folder_path =
        Some(recorded_folder_after_reserved_claim(app, &title.id, stored, reservation).await?);
    Ok(())
}

pub(crate) async fn claim_title_folder_if_missing(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
) -> AppResult<()> {
    claim_title_folder_with_layout(app, title, folder_path, TitleFolderClaimLayout::Configured)
        .await
}

pub(crate) async fn claim_title_folder_for_scan_if_missing(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
) -> AppResult<()> {
    claim_title_folder_for_scan_with_evidence(app, title, folder_path, None).await
}

pub(crate) async fn claim_title_folder_for_scan_with_evidence(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
    evidence: Option<&crate::library_discovery::DiscoveredBoundaryEvidence>,
) -> AppResult<()> {
    claim_title_folder_with_layout(
        app,
        title,
        folder_path,
        TitleFolderClaimLayout::Conventional(evidence),
    )
    .await
}

enum TitleFolderClaimLayout<'a> {
    Configured,
    Conventional(Option<&'a crate::library_discovery::DiscoveredBoundaryEvidence>),
}

/// Every check [`claim_title_folder_if_missing`] makes before recording
/// `folder_path` for a title that has no folder, without recording it.
pub(crate) async fn ensure_title_folder_claimable(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<()> {
    ensure_folder_available_to_title(app, title, folder_path).await?;
    // Nothing is recorded here, so nothing stays reserved.
    ensure_unowned_folder_claimable(app, title, folder_path, TitleFolderClaimLayout::Configured)
        .await
        .map(drop)
}

async fn claim_title_folder_with_layout(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
    layout: TitleFolderClaimLayout<'_>,
) -> AppResult<()> {
    let started = std::time::Instant::now();
    ensure_folder_available_to_title(app, title, folder_path).await?;
    let available_ms = elapsed_ms(started);
    if non_empty_folder_path(title.folder_path.as_deref()).is_some() {
        return Ok(());
    }

    let stage = std::time::Instant::now();
    let mut reservation = ensure_unowned_folder_claimable(app, title, folder_path, layout).await?;
    let claimable_ms = elapsed_ms(stage);
    let folder_path = path_to_stored_string(folder_path);
    let stage = std::time::Instant::now();
    assign_claimed_title_folder(
        app,
        crate::TitleFolderWrite::assign(&title.id, &folder_path).expecting(None),
        &mut reservation,
    )
    .await?;
    let write_ms = elapsed_ms(stage);
    let folder = folder_path.clone();
    let stage = std::time::Instant::now();
    title.folder_path =
        Some(recorded_folder_after_reserved_claim(app, &title.id, folder_path, reservation).await?);
    let reread_ms = elapsed_ms(stage);
    tracing::debug!(
        folder = %folder,
        available_ms,
        claimable_ms,
        write_ms,
        reread_ms,
        total_ms = elapsed_ms(started),
        "title folder claim stage timing"
    );
    Ok(())
}

fn elapsed_ms(started: std::time::Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// The folder a title records after a claim of `claimed`. A title that
/// already recorded that folder keeps its stored spelling, which can differ
/// from `claimed` while naming the same folder.
async fn recorded_folder_after_claim(
    app: &AppUseCase,
    title_id: &str,
    claimed: String,
) -> AppResult<String> {
    recorded_folder_after_reserved_claim(app, title_id, claimed, None).await
}

/// [`recorded_folder_after_claim`] for a claim that may hold a scan
/// reservation. The reservation settles with the folder the catalog now
/// records for the title, read once by the same lookup.
async fn recorded_folder_after_reserved_claim(
    app: &AppUseCase,
    title_id: &str,
    claimed: String,
    reservation: Option<ScanClaimReservation>,
) -> AppResult<String> {
    let recorded = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await?
        .and_then(|title| title.folder_path);
    if let Some(reservation) = reservation {
        reservation.recorded(recorded.clone());
    }
    Ok(recorded
        .filter(|recorded| crate::title_folder_keys::title_folder_key(recorded).is_some())
        .unwrap_or(claimed))
}

/// Records a claimed folder. A claim that holds a scan reservation also
/// learns which counter advance its own write produced, so the scan knows
/// that advance changed nothing it does not already hold.
async fn assign_claimed_title_folder(
    app: &AppUseCase,
    write: crate::TitleFolderWrite,
    reservation: &mut Option<ScanClaimReservation>,
) -> AppResult<()> {
    let titles = &app.services.catalog.titles;
    let Some(reservation) = reservation.as_mut() else {
        return titles.assign_title_folders(&[write]).await;
    };
    reservation.begin_write();
    let (result, generation) = titles.assign_title_folders_with_generation(&[write]).await;
    reservation.wrote(generation);
    result
}

async fn ensure_unowned_folder_claimable(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
    layout: TitleFolderClaimLayout<'_>,
) -> AppResult<Option<ScanClaimReservation>> {
    let started = std::time::Instant::now();
    let reservation = ensure_folder_claim_available_to_title(app, title, folder_path).await?;
    let ownership_ms = elapsed_ms(started);
    let stage = std::time::Instant::now();
    let boundary_verified = match &layout {
        TitleFolderClaimLayout::Conventional(Some(evidence)) => {
            evidence
                .verify(folder_path, &title.facet, &title.external_ids)
                .await?
        }
        _ => false,
    };
    let evidence_verify_ms = elapsed_ms(stage);
    let stage = std::time::Instant::now();
    // Imports reserve missing or empty destinations before writing media. A
    // destination with contents must not hold other titles' folders; stray
    // files or empty subfolders left by an earlier attempt do not block it.
    let has_contents = if boundary_verified {
        false
    } else {
        match tokio::fs::symlink_metadata(folder_path).await {
            Ok(_) => tokio::fs::read_dir(folder_path)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?
                .next_entry()
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?
                .is_some(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(AppError::Repository(error.to_string())),
        }
    };
    let contents_probe_ms = elapsed_ms(stage);
    let stage = std::time::Instant::now();
    if has_contents {
        match layout {
            TitleFolderClaimLayout::Configured => {
                let patterns =
                    crate::library_discovery::episode_folder_patterns(app, &title.facet).await?;
                crate::library_discovery::ensure_folder_holds_no_child_titles(
                    folder_path,
                    &patterns,
                )
                .await?;
            }
            TitleFolderClaimLayout::Conventional(_) => {
                crate::library_discovery::ensure_scanned_folder_holds_no_child_titles(
                    folder_path,
                    title,
                )
                .await?;
            }
        }
    }
    let child_titles_ms = elapsed_ms(stage);

    let folder_path = path_to_stored_string(folder_path);
    let stage = std::time::Instant::now();
    let spans_root = folder_spans_a_library_root(app, &folder_path).await?;
    let spans_root_ms = elapsed_ms(stage);
    if spans_root {
        tracing::warn!(
            title_id = %title.id,
            folder_path = %folder_path,
            "not recording a library root as a title's folder"
        );
        return Err(AppError::validation_refused(
            crate::error_reasons::folder_ownership::CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER,
            "cannot claim a library root as a title folder",
        ));
    }
    let stage = std::time::Instant::now();
    let outside_library = title_folder_is_outside_its_library(app, title, &folder_path).await?;
    let outside_library_ms = elapsed_ms(stage);
    if outside_library {
        tracing::warn!(
            title_id = %title.id,
            library_id = %title.library_id,
            folder_path = %folder_path,
            "not recording a folder outside the title's library roots as its folder"
        );
        return Err(AppError::validation_refused(
            crate::error_reasons::folder_ownership::PATH_OUTSIDE_LIBRARY_ROOT,
            "cannot claim a title folder outside its library roots",
        ));
    }
    tracing::debug!(
        folder = %folder_path,
        ownership_ms,
        evidence_verify_ms,
        evidence_verified = boundary_verified,
        contents_probe_ms,
        has_contents,
        child_titles_ms,
        spans_root_ms,
        outside_library_ms,
        total_ms = elapsed_ms(started),
        "title folder claimable stage timing"
    );
    Ok(reservation)
}

pub(crate) async fn unlink_title_media_in_folder(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<u32> {
    if !title_owns_another_folder(title, folder_path) {
        return Ok(0);
    }
    // Only a title whose owned folder is confirmed on disk holds a real
    // second copy. An absent or unreachable owned folder is stale ownership
    // or an unmounted root; unlinking there turns a monitored title fileless
    // and exposes it to automatic re-download.
    let owned_path = title.folder_path.as_deref().unwrap_or_default();
    if crate::title_folder_rules::path_is_strictly_within(
        owned_path,
        &path_to_stored_string(folder_path),
    ) {
        tracing::debug!(title_id = %title.id, "ignoring scan conflict at an ancestor of the owned title folder");
        return Ok(0);
    }
    if owned_folder_present(owned_path).await != Some(true) {
        return Ok(0);
    }
    // A library root is always on disk, so a record that climbed to one would
    // otherwise pass for a real second copy and strip the title's own files.
    if title_folder_spans_a_library_root(app, title).await? {
        return Ok(0);
    }

    detach_title_media_in_folder(app, &title.id, folder_path).await
}

/// Drop every catalog media row of `title_id` that lives inside `folder_path`,
/// without asking whether the title still owns some other folder.
///
/// [`unlink_title_media_in_folder`] guards on ownership because a scan only ever
/// detaches rows from a folder the title lost to someone else. Folder-match
/// correction detaches from a folder the title is *giving up* — including the
/// takeover case where the displaced title is left owning nothing at all — so it
/// needs the unguarded form (FR-003, FR-007).
pub(crate) async fn detach_title_media_in_folder(
    app: &AppUseCase,
    title_id: &str,
    folder_path: &Path,
) -> AppResult<u32> {
    detach_title_media_in_folder_except(app, title_id, folder_path, None).await
}

/// [`detach_title_media_in_folder`], keeping the rows inside `kept_folder`.
///
/// A title re-pointed at a folder inside the one it gives up keeps the rows
/// that are already in its new folder. That is the shape of correcting a
/// record that had climbed to a library root: the old folder holds everything
/// the title has, including the folder it is being pointed at.
pub(crate) async fn detach_title_media_in_folder_except(
    app: &AppUseCase,
    title_id: &str,
    folder_path: &Path,
    kept_folder: Option<&str>,
) -> AppResult<u32> {
    let folder_path = path_to_stored_string(folder_path);
    let media_file_ids = app
        .services
        .library
        .media_files
        .list_media_files_for_title(title_id)
        .await?
        .into_iter()
        .filter(|media_file| {
            crate::stored_paths::stored_path_is_within_folder(&folder_path, &media_file.file_path)
                && !kept_folder.is_some_and(|kept_folder| {
                    crate::stored_paths::stored_path_is_within_folder(
                        kept_folder,
                        &media_file.file_path,
                    )
                })
        })
        .map(|media_file| media_file.id)
        .collect::<Vec<_>>();
    for media_file_id in &media_file_ids {
        app.services
            .library
            .media_files
            .delete_media_file(media_file_id)
            .await?;
    }
    let deleted = media_file_ids.len() as u32;
    tracing::info!(
        title_id = %title_id,
        folder_path = %folder_path,
        unlinked_media_files = deleted,
        "unlinked catalog media outside the title-owned folder"
    );
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::MediaFacet;

    #[test]
    fn title_delete_overlap_prefilter_matches_exact_comparison() {
        let root = tempfile::tempdir().unwrap();
        let candidate = root.path().join("Bucket/Title");
        std::fs::create_dir_all(&candidate).unwrap();
        let candidate = path_to_stored_string(candidate);
        let roots = vec![path_to_stored_string(root.path())];
        let mut keys = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
        let owners = [
            "relative/unknown".into(),
            path_to_stored_string(root.path().join("Bucket")),
            path_to_stored_string(root.path().join("Bucket/Title/Season 01")),
            path_to_stored_string(root.path().join("Bucket/Other")),
            path_to_stored_string(root.path().join("Bucket/title")),
            path_to_stored_string(root.path().join("Bucket/../Invalid")),
        ];
        for owner in owners {
            let exact = title_folders_overlap(&candidate, &owner);
            let filtered = if keys.needs_exact_comparison(&owner) {
                title_folders_overlap(&candidate, &owner)
            } else {
                Ok(false)
            };
            assert_eq!(exact.as_ref().ok(), filtered.as_ref().ok(), "{owner}");
            assert_eq!(exact.is_err(), filtered.is_err(), "{owner}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn title_delete_overlap_prefilter_falls_back_for_links_and_resolution_errors() {
        let root = tempfile::tempdir().unwrap();
        let candidate = root.path().join("Title");
        std::fs::create_dir(&candidate).unwrap();
        let alias = root.path().join("Alias");
        std::os::unix::fs::symlink(&candidate, &alias).unwrap();
        let looping = root.path().join("Loop");
        std::os::unix::fs::symlink(&looping, &looping).unwrap();
        let candidate = path_to_stored_string(candidate);
        let mut keys =
            FolderOverlapKeys::from_roots(&candidate, &[path_to_stored_string(root.path())])
                .unwrap();
        for owner in [alias, looping] {
            let owner = path_to_stored_string(owner);
            assert!(keys.needs_exact_comparison(&owner));
            // A link that cannot be followed is compared as spelled.
            assert_eq!(
                title_folders_overlap(&candidate, &owner).unwrap(),
                !owner.ends_with("Loop")
            );
        }
    }

    #[test]
    fn nested_title_overlap_checks_missing_destinations_and_shared_buckets() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("E/Emberfall");
        for invalid_other in ["", "relative/old-folder", "../old-folder"] {
            assert!(!title_folders_overlap(&path_to_stored_string(&first), invalid_other).unwrap());
        }
        let second = root.path().join("E/Echo");
        assert!(
            !title_folders_overlap(
                &path_to_stored_string(&first),
                &path_to_stored_string(&second)
            )
            .unwrap()
        );
        assert!(
            title_folders_overlap(
                &path_to_stored_string(&first),
                &path_to_stored_string(first.join("Season 01"))
            )
            .unwrap()
        );
        assert!(resolved_folder_identity(&root.path().join("E/../Escape")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn nested_title_overlap_resolves_symlinked_existing_ancestors() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("E")).unwrap();
        std::os::unix::fs::symlink(root.path().join("E"), root.path().join("Alias")).unwrap();
        assert!(
            title_folders_overlap(
                &path_to_stored_string(root.path().join("E/Example")),
                &path_to_stored_string(root.path().join("Alias/Example/Season 01")),
            )
            .unwrap()
        );
    }

    /// `real` with a folder in it, `alias` linked to `real`, `real/Dead`, a
    /// link that cannot be followed, and `real/Blocker`, a file: nothing at
    /// or below the link, or below the file, can be read, whoever runs the
    /// test.
    #[cfg(unix)]
    fn unreadable_folder_fixture(temp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::fs::canonicalize(temp).unwrap();
        let real = base.join("real");
        std::fs::create_dir_all(real.join("Lantern")).unwrap();
        std::os::unix::fs::symlink(&real, base.join("alias")).unwrap();
        let dead = real.join("Dead");
        std::os::unix::fs::symlink(&dead, &dead).unwrap();
        // A file where a folder is recorded: nothing below it can even be
        // inspected, as on a drive that is unplugged.
        std::fs::write(real.join("Blocker"), b"").unwrap();
        (base, dead)
    }

    #[cfg(unix)]
    #[test]
    fn another_titles_unreadable_folder_is_compared_as_spelled() {
        let temp = tempfile::tempdir().unwrap();
        let (base, dead) = unreadable_folder_fixture(temp.path());
        let real = base.join("real");
        let overlap = |left: &Path, right: &Path| {
            title_folders_overlap(&path_to_stored_string(left), &path_to_stored_string(right))
        };
        // Unrelated to the unreadable folder: compared, and no overlap.
        assert!(!overlap(&real.join("Lantern"), &dead.join("Owned")).unwrap());
        assert!(!overlap(&real.join("Lantern"), &dead).unwrap());
        // As spelled, the folder holds the unreadable one.
        assert!(overlap(&real, &dead.join("Owned")).unwrap());
        // The readable part is still resolved: `alias` is `real`.
        assert!(overlap(&real, &base.join("alias/Dead/Owned")).unwrap());
        assert!(!overlap(&real.join("Lantern"), &base.join("alias/Dead/Owned")).unwrap());
        // The folder being checked must itself resolve.
        assert_eq!(
            overlap(&dead.join("Owned"), &real.join("Lantern"))
                .unwrap_err()
                .validation_reason(),
            Some(crate::error_reasons::folder_ownership::CANNOT_INSPECT_TITLE_FOLDER)
        );
    }

    #[cfg(unix)]
    #[test]
    fn another_titles_unreadable_folder_is_compared_without_regard_to_case() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(temp.path()).unwrap();
        let locked = base.join("Locked");
        std::fs::create_dir_all(locked.join("Owned")).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // A privileged test user reads the folder anyway and proves nothing.
        let unreadable = std::fs::symlink_metadata(locked.join("Owned")).is_err();
        let overlaps = title_folders_overlap(
            &path_to_stored_string(base.join("locked")),
            &path_to_stored_string(locked.join("Owned")),
        );
        let unrelated = title_folders_overlap(
            &path_to_stored_string(base.join("Lantern")),
            &path_to_stored_string(locked.join("Owned")),
        );
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!unrelated.unwrap());
        if unreadable {
            assert!(overlaps.unwrap());
        }
    }

    #[cfg(unix)]
    #[test]
    fn nothing_below_an_unreadable_folder_is_probed_twice() {
        let temp = tempfile::tempdir().unwrap();
        let (base, dead) = unreadable_folder_fixture(temp.path());
        let mut others = UnreadableFolders::default();
        let first = others.resolve(&dead.join("Owned"));
        assert_eq!(first.path, base.join("real/Dead/Owned"));
        assert!(!first.whole);
        // The folder, the link, following the link, then `real` twice.
        assert_eq!(others.probes, 5);
        let second = others.resolve(&base.join("alias/Lantern/Season 01"));
        assert_eq!(second.path, base.join("real/Lantern/Season 01"));
        assert!(second.whole);
        let before = others.probes;
        let third = others.resolve(&dead.join("Other/Season 01"));
        assert_eq!(third.path, base.join("real/Dead/Other/Season 01"));
        assert!(!third.whole);
        // The two folders below the link, once each. The link and `real`
        // above it have already answered.
        assert_eq!(others.probes - before, 2);
        let fourth = others.resolve(&dead.join("Other/Season 02"));
        assert_eq!(fourth.path, base.join("real/Dead/Other/Season 02"));
        assert!(!fourth.whole);
        assert_eq!(others.probes - before, 2, "nothing below `Other` is asked");
    }

    #[cfg(unix)]
    #[test]
    fn folders_shared_by_other_titles_are_read_once() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(temp.path()).unwrap();
        std::fs::create_dir_all(base.join("Present")).unwrap();
        let mut others = UnreadableFolders::default();
        // Titles whose organizing folder is missing: the folder is asked
        // once, then nothing under it is asked at all.
        for name in ["One", "Two", "Three"] {
            let other = others.resolve(&base.join("Missing/Movies").join(name));
            assert_eq!(other.path, base.join("Missing/Movies").join(name));
            assert!(other.whole, "a missing folder is not an unreadable one");
        }
        // One, Movies, Missing, then the folder above them twice.
        assert_eq!(others.probes, 5);

        // Titles under a folder that refuses access: each title's own folder
        // is asked, since a folder inside a refusing one can still answer on
        // Windows, but the folders they share are not asked again.
        let locked = base.join("Locked");
        std::fs::create_dir_all(locked.join("Movies")).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // A privileged test user reads the folder anyway and proves nothing.
        let refused = std::fs::symlink_metadata(locked.join("Movies")).is_err();
        let mut others = UnreadableFolders::default();
        let mut probes = Vec::new();
        let mut resolved = Vec::new();
        for name in ["One", "Two", "Three"] {
            let before = others.probes;
            resolved.push(others.resolve(&locked.join("Movies").join(name)));
            probes.push(others.probes - before);
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        if refused {
            // One, Movies, then `Locked` itself twice; then one probe each.
            assert_eq!(probes, [4, 1, 1]);
            for (other, name) in resolved.iter().zip(["One", "Two", "Three"]) {
                assert_eq!(other.path, locked.join("Movies").join(name));
                assert!(!other.whole);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn link_walk_asks_a_folder_that_is_gone_once_for_every_owner_under_it() {
        let temp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(temp.path()).unwrap();
        std::fs::create_dir_all(base.join("library/Lantern")).unwrap();
        // A file where a folder is recorded fails every path through it, as a
        // share that is down does.
        std::fs::write(base.join("down"), b"").unwrap();
        let roots = vec![path_to_stored_string(base.join("library"))];
        let candidate = path_to_stored_string(base.join("library/Lantern"));
        for other_library in ["down/share", "missing"] {
            // Owners of another library, outside the roots the check was given.
            let owner = |name: &str| {
                path_to_stored_string(base.join(other_library).join("Movies").join(name))
            };
            let mut keys = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
            assert!(!keys.needs_exact_comparison(&owner("First")));
            let first = keys.link_probes;
            assert!(first > 0);
            for index in 0..50 {
                assert!(!keys.needs_exact_comparison(&owner(&format!("Owner {index}"))));
            }
            assert_eq!(keys.link_probes, first, "{other_library}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn owner_under_a_dead_root_is_walked_only_when_it_shares_no_more_than_its_key() {
        let temp = tempfile::tempdir().unwrap();
        let (base, dead) = unreadable_folder_fixture(temp.path());
        let real = base.join("real");
        let blocked = real.join("Blocker/Library");
        let roots = vec![
            path_to_stored_string(&real),
            path_to_stored_string(&blocked),
            path_to_stored_string(&dead),
        ];
        let keys =
            FolderOverlapKeys::from_roots(&path_to_stored_string(real.join("Lantern")), &roots)
                .unwrap();
        let chain = |owner: std::path::PathBuf| match OwnerOverlapPrefilter::prepare(
            &path_to_stored_string(owner),
            keys.root_keys(),
        ) {
            OwnerOverlapPrefilter::Keys { chain, .. } => chain,
            OwnerOverlapPrefilter::Exact => panic!("an absolute owner has keys"),
        };
        assert_eq!(chain(blocked.join("Owned")), 0);
        assert_eq!(chain(blocked.join("Shelf/Owned")), 0);
        // Root keys ignore case. On a volume that does not, `library` is
        // another folder, under the readable root: it is walked up to that
        // root.
        assert_eq!(chain(real.join("Blocker/library/Owned")), 3);
        assert_eq!(chain(real.join("Lantern")), 1);
        // A root that is there but does not resolve is not dead.
        assert_eq!(chain(dead.join("Owned")), 1);
    }

    #[cfg(unix)]
    #[test]
    fn owner_under_an_unreadable_library_root_is_prefiltered_as_spelled() {
        let temp = tempfile::tempdir().unwrap();
        let (base, dead) = unreadable_folder_fixture(temp.path());
        let real = base.join("real");
        let blocked = real.join("Blocker/Library");
        let roots = vec![
            path_to_stored_string(&real),
            path_to_stored_string(&blocked),
            path_to_stored_string(&dead),
        ];
        let candidate = path_to_stored_string(real.join("Lantern"));
        let mut keys = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
        for owner in [blocked.join("Owned"), blocked.join("O/Owned")] {
            assert!(!keys.needs_exact_comparison(&path_to_stored_string(owner)));
        }
        assert_eq!(keys.link_probes, 0, "nothing under the root is probed");
        // A root that is a link that cannot be followed is there, so the
        // folders in it are asked, once each.
        for owner in [dead.join("Owned"), dead.join("Other")] {
            assert!(!keys.needs_exact_comparison(&path_to_stored_string(owner)));
        }
        assert_eq!(keys.link_probes, 2);
        // Related as spelled, so it still goes to the exact comparison.
        let mut holding =
            FolderOverlapKeys::from_roots(&path_to_stored_string(&real), &roots).unwrap();
        for owner in [blocked.join("Owned"), dead.join("Owned")] {
            assert!(holding.needs_exact_comparison(&path_to_stored_string(owner)));
        }
        assert_eq!(holding.link_probes, 0);
    }

    /// An import, rename or scan claim of `candidate` while another title's
    /// folder cannot be read, outside a scan and inside one.
    #[cfg(unix)]
    async fn move_check_with_unreadable_owner(
        owned: &Path,
        candidate: &Path,
        roots: &[&Path],
    ) -> [AppResult<()>; 2] {
        let (app, title) = unowned_scan_title_with_roots(MediaFacet::Movie, roots).await;
        let mut owner = title.clone();
        owner.id = "other-owner".into();
        owner.name = "Other Owner".into();
        owner.folder_path = Some(path_to_stored_string(owned));
        app.services
            .catalog
            .titles
            .create_or_get_existing(owner)
            .await
            .unwrap();
        let outside_scan = ensure_folder_move_available_to_title(&app, &title, candidate).await;
        let in_scan = with_scan_ownership(
            Some(ScanOwnershipContext::new()),
            ensure_folder_move_available_to_title(&app, &title, candidate),
        )
        .await;
        [outside_scan, in_scan]
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn move_check_is_not_refused_by_another_titles_unreadable_folder() {
        let temp = tempfile::tempdir().unwrap();
        let (base, dead) = unreadable_folder_fixture(temp.path());
        let real = base.join("real");
        let candidate = real.join("Lantern");
        let sentinel = candidate.join("preserved.mkv");
        std::fs::write(&sentinel, b"preserved").unwrap();
        // The other title's folder is under a library root that cannot be
        // read, then under an unreadable folder of the candidate's own root.
        for roots in [vec![real.as_path(), dead.as_path()], vec![real.as_path()]] {
            for outcome in
                move_check_with_unreadable_owner(&dead.join("Owned"), &candidate, &roots).await
            {
                outcome.unwrap();
            }
        }
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserved");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn move_check_still_refuses_a_folder_holding_an_unreadable_owner() {
        let temp = tempfile::tempdir().unwrap();
        let (base, _) = unreadable_folder_fixture(temp.path());
        let real = base.join("real");
        let shelf = real.join("Shelf");
        std::fs::create_dir(&shelf).unwrap();
        let dead = shelf.join("Dead");
        std::os::unix::fs::symlink(&dead, &dead).unwrap();
        // Recorded under the shelf as spelled, then through the linked root.
        for owned in [dead.join("Owned"), base.join("alias/Shelf/Dead/Owned")] {
            for outcome in move_check_with_unreadable_owner(&owned, &shelf, &[&real]).await {
                assert_eq!(
                    outcome.unwrap_err().validation_reason(),
                    Some(
                        crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE
                    )
                );
            }
        }
    }

    /// The other title is recorded under a root that cannot be read, and that
    /// root is spelled through a link above it. The readable part of the root
    /// is still resolved, so the folder that holds it is still refused.
    #[cfg(unix)]
    #[tokio::test]
    async fn move_check_refuses_holder_of_owner_under_unresolved_root_reached_through_link() {
        let temp = tempfile::tempdir().unwrap();
        let (base, _) = unreadable_folder_fixture(temp.path());
        let real = base.join("real");
        let shelf = real.join("Shelf");
        std::fs::create_dir(&shelf).unwrap();
        let dead = shelf.join("Dead");
        std::os::unix::fs::symlink(&dead, &dead).unwrap();
        let dead_root = base.join("alias/Shelf/Dead");
        let owned = dead_root.join("Owned");
        let stored = path_to_stored_string;
        assert!(title_folders_overlap(&stored(&shelf), &stored(&owned)).unwrap());
        let mut keys =
            FolderOverlapKeys::from_roots(&stored(&shelf), &[stored(&real), stored(&dead_root)])
                .unwrap();
        assert!(keys.needs_exact_comparison(&stored(&owned)));
        for outcome in move_check_with_unreadable_owner(&owned, &shelf, &[&real, &dead_root]).await
        {
            assert_eq!(
                outcome.unwrap_err().validation_reason(),
                Some(crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE)
            );
        }
    }

    /// Root keys ignore case. Where the volume does not, `srv/TV` beside the
    /// root `srv/tv` is another folder, here a link into the folder being
    /// claimed. The other title recorded through it must still be found.
    #[cfg(unix)]
    #[tokio::test]
    async fn move_check_refuses_owner_recorded_through_a_link_that_only_shares_a_roots_key() {
        let temp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(temp.path()).unwrap();
        let root = base.join("srv/tv");
        let data = base.join("data");
        let candidate = data.join("elsewhere/Show");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&candidate).unwrap();
        let sentinel = candidate.join("preserved.mkv");
        std::fs::write(&sentinel, b"preserved").unwrap();
        let owned = base.join("srv/TV/Show");
        let stored = |path: &Path| path_to_stored_string(path);
        let roots = [stored(&root), stored(&data)];

        // Whatever the volume: the folder that shares the root's key is
        // walked, and so is everything above it. A folder spelled as the
        // root is walked only below it.
        let keys = FolderOverlapKeys::from_roots(&stored(&candidate), &roots).unwrap();
        let chain =
            |owner: &Path| match OwnerOverlapPrefilter::prepare(&stored(owner), keys.root_keys()) {
                OwnerOverlapPrefilter::Keys { chain, .. } => chain,
                OwnerOverlapPrefilter::Exact => panic!("an absolute owner has keys"),
            };
        assert_eq!(chain(&owned), owned.ancestors().count());
        assert_eq!(chain(&root.join("Show")), 1);

        // The link can exist only where `TV` and `tv` are two names.
        let linked = std::os::unix::fs::symlink(data.join("elsewhere"), base.join("srv/TV"));
        if linked.is_ok() {
            assert!(title_folders_overlap(&stored(&candidate), &stored(&owned)).unwrap());
            let mut keys = FolderOverlapKeys::from_roots(&stored(&candidate), &roots).unwrap();
            assert!(keys.needs_exact_comparison(&stored(&owned)));
            for outcome in
                move_check_with_unreadable_owner(&owned, &candidate, &[&root, &data]).await
            {
                assert_eq!(
                    outcome.unwrap_err().validation_reason(),
                    Some(
                        crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE
                    )
                );
            }
        }
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserved");
    }

    fn title(folder_path: Option<&str>) -> Title {
        let facet = MediaFacet::Anime;
        Title {
            id: "title-1".to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            name: "Case Split Fixture".to_string(),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/Anime"),
            created_by: None,
            created_at: chrono::Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            canonical_tags: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            movie_release_dates: None,
            folder_path: folder_path.map(str::to_string),
        }
    }

    async fn unowned_scan_title(facet: MediaFacet, root: &Path) -> (AppUseCase, Title) {
        unowned_scan_title_with_roots(facet, &[root]).await
    }

    /// An unowned title in a library with `roots`, the first the default.
    async fn unowned_scan_title_with_roots(
        facet: MediaFacet,
        roots: &[&Path],
    ) -> (AppUseCase, Title) {
        let (app, user) = crate::lib_tests::bootstrap();
        let library_id = scryer_domain::default_library_id_for_facet(&facet);
        let library = app
            .services
            .catalog
            .libraries
            .get_by_id(&library_id)
            .await
            .unwrap()
            .unwrap();
        app.services
            .catalog
            .libraries
            .update(
                &library_id,
                library.name,
                library.slug,
                roots
                    .iter()
                    .enumerate()
                    .map(|(index, root)| crate::LibraryRootDraft {
                        id: None,
                        path: path_to_stored_string(root),
                        is_default: index == 0,
                    })
                    .collect(),
            )
            .await
            .unwrap();
        let created = app
            .create_title_without_hydration_in_library(
                &user,
                crate::NewTitle {
                    name: "Scan Fixture".into(),
                    facet,
                    ..Default::default()
                },
                library_id,
            )
            .await
            .unwrap();
        assert!(created.title.folder_path.is_none());
        (app, created.title)
    }

    #[tokio::test]
    async fn scan_ownership_context_reads_new_catalog_owners() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("candidate");
        std::fs::create_dir(&folder).unwrap();
        let sentinel = folder.join("preserved.mkv");
        std::fs::write(&sentinel, b"preserved").unwrap();
        let (app, title) = unowned_scan_title(MediaFacet::Movie, root.path()).await;
        let context = ScanOwnershipContext::new();
        with_scan_ownership(Some(context), async {
            ensure_folder_move_available_to_title(&app, &title, &folder)
                .await
                .unwrap();
            let mut owner = title.clone();
            owner.id = "new-owner".into();
            owner.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
            owner.name = "New Owner".into();
            owner.folder_path = Some(path_to_stored_string(&folder));
            app.services
                .catalog
                .titles
                .create_or_get_existing(owner)
                .await
                .unwrap();
            let error = ensure_folder_move_available_to_title(&app, &title, &folder)
                .await
                .unwrap_err();
            assert_eq!(
                error.validation_reason(),
                Some(crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE)
            );
            let other = root.path().join("other");
            std::fs::create_dir(&other).unwrap();
            app.services
                .catalog
                .titles
                .set_folder_path("new-owner", &path_to_stored_string(&other))
                .await
                .unwrap();
            ensure_folder_move_available_to_title(&app, &title, &folder)
                .await
                .unwrap();
            app.services
                .catalog
                .titles
                .set_folder_path("new-owner", &path_to_stored_string(&folder))
                .await
                .unwrap();
            assert!(
                ensure_folder_move_available_to_title(&app, &title, &folder)
                    .await
                    .is_err()
            );
        })
        .await;
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserved");
    }

    #[tokio::test]
    async fn discovery_evidence_claim_preserves_ownership_and_library_boundaries() {
        for mode in ["valid", "root", "outside", "owner", "descendant_owner"] {
            let root = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let folder = root.path().join("Scan Fixture");
            std::fs::create_dir(&folder).unwrap();
            let file = folder.join("Scan.Fixture.mkv");
            std::fs::write(&file, b"preserved media").unwrap();
            let mut discoveries = crate::library_discovery::stream_title_folder_boundaries(
                root.path().into(),
                crate::library_discovery::TitleFolderBoundaryPolicy {
                    facet: MediaFacet::Movie,
                    stored_folders: vec![],
                    other_roots: vec![],
                },
                1,
                Default::default(),
            );
            let evidence =
                tokio::time::timeout(std::time::Duration::from_secs(30), discoveries.recv())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .remove(0)
                    .evidence;
            assert_eq!(evidence.is_some(), cfg!(unix));
            let library_root = match mode {
                "root" => folder.as_path(),
                "outside" => outside.path(),
                _ => root.path(),
            };
            let (app, mut title) = unowned_scan_title(MediaFacet::Movie, library_root).await;
            let owner_path = match mode {
                "owner" => Some(folder.clone()),
                "descendant_owner" => Some(folder.join("Recorded Child")),
                _ => None,
            };
            if let Some(owner_path) = &owner_path {
                // Catalog ownership can change after discovery, without any
                // filesystem change that would invalidate the cached stamps.
                let mut owner = title.clone();
                owner.id = "other-owner".into();
                owner.name = "Other Owner".into();
                owner.folder_path = Some(path_to_stored_string(owner_path));
                app.services
                    .catalog
                    .titles
                    .create_or_get_existing(owner)
                    .await
                    .unwrap();
            }
            let result = claim_title_folder_for_scan_with_evidence(
                &app,
                &mut title,
                &folder,
                evidence.as_ref(),
            )
            .await;
            if mode == "valid" {
                result.unwrap();
                assert!(title_owns_folder(&title, &folder));
            } else {
                assert!(result.is_err());
                assert!(title.folder_path.is_none());
                assert!(
                    app.services
                        .catalog
                        .titles
                        .get_by_id(&title.id)
                        .await
                        .unwrap()
                        .unwrap()
                        .folder_path
                        .is_none()
                );
            }
            if let Some(owner_path) = owner_path {
                assert_eq!(
                    app.services
                        .catalog
                        .titles
                        .get_by_id("other-owner")
                        .await
                        .unwrap()
                        .unwrap()
                        .folder_path,
                    Some(path_to_stored_string(&owner_path)),
                );
            }
            assert_eq!(std::fs::read(file).unwrap(), b"preserved media");
        }
    }

    #[tokio::test]
    async fn scan_and_import_claims_accept_conventional_season_layouts() {
        for facet in [MediaFacet::Series, MediaFacet::Anime] {
            for (season, scan) in [
                "Series 1",
                "Special Episodes",
                "Season_1",
                "Season-1",
                "Season1",
            ]
            .into_iter()
            .flat_map(|season| [(season, true), (season, false)])
            {
                let root = tempfile::tempdir().unwrap();
                let folder = root.path().join("Scan Fixture");
                let file = folder.join(season).join("Scan.Fixture.S01E01.mkv");
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(&file, b"synthetic media").unwrap();
                let (app, mut title) = unowned_scan_title(facet.clone(), root.path()).await;

                if scan {
                    claim_title_folder_for_scan_if_missing(&app, &mut title, &folder)
                        .await
                        .unwrap();
                } else {
                    claim_title_folder_if_missing(&app, &mut title, &folder)
                        .await
                        .unwrap_or_else(|error| panic!("{season}: {error}"));
                }
                assert!(title_owns_folder(&title, &folder));
                let stored = app
                    .services
                    .catalog
                    .titles
                    .get_by_id(&title.id)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(title_owns_folder(&stored, &folder));
                assert_eq!(std::fs::read(&file).unwrap(), b"synthetic media");
            }
        }
    }

    #[tokio::test]
    async fn automatic_scan_claim_preserves_mixed_title_and_movie_boundaries() {
        for facet in [MediaFacet::Series, MediaFacet::Anime, MediaFacet::Movie] {
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join("Scan Fixture");
            let child = folder
                .join("Other Title")
                .join("Series 1")
                .join("S01E01.mkv");
            std::fs::create_dir_all(child.parent().unwrap()).unwrap();
            std::fs::write(&child, b"preserved child media").unwrap();
            let loose = folder.join("Scan.Fixture.2024.mkv");
            std::fs::write(&loose, b"preserved direct media").unwrap();
            // Movies do not recognize episode layouts, so use a direct child
            // file to prove their existing boundary remains guarded.
            let direct_child = (facet == MediaFacet::Movie)
                .then(|| folder.join("Other Title").join("Other.Title.2024.mkv"));
            if let Some(path) = &direct_child {
                std::fs::write(path, b"preserved child movie").unwrap();
            }
            let (app, mut title) = unowned_scan_title(facet, root.path()).await;
            assert!(
                claim_title_folder_for_scan_if_missing(&app, &mut title, &folder)
                    .await
                    .is_err()
            );
            assert!(title.folder_path.is_none());
            assert!(
                app.services
                    .catalog
                    .titles
                    .get_by_id(&title.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .folder_path
                    .is_none()
            );
            assert_eq!(std::fs::read(&child).unwrap(), b"preserved child media");
            assert_eq!(std::fs::read(&loose).unwrap(), b"preserved direct media");
            if let Some(path) = direct_child {
                assert_eq!(std::fs::read(path).unwrap(), b"preserved child movie");
            }
        }
    }

    #[tokio::test]
    async fn automatic_scan_claim_cour_aliases_preserve_files_and_settings() {
        for relative in [
            "Glass Meridian S03 Complete/Scan.Fixture.S03E01.mkv",
            "Arc One/Verified.Alias.S02E01.mkv",
        ] {
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join("Scan Fixture");
            let file = folder.join(relative);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, b"preserved episode").unwrap();
            let (app, mut title) = unowned_scan_title(MediaFacet::Anime, root.path()).await;
            title.aliases.push("Verified Alias".into());
            let tags = title.tags.clone();
            claim_title_folder_for_scan_if_missing(&app, &mut title, &folder)
                .await
                .unwrap();
            assert!(title_owns_folder(&title, &folder));
            assert_eq!(title.tags, tags);
            assert_eq!(std::fs::read(&file).unwrap(), b"preserved episode");
        }
    }

    #[tokio::test]
    async fn automatic_scan_claim_rejects_unproven_cour_and_foreign_identity() {
        for (relative, nfo) in [
            ("Arc One/01.mkv", None),
            (
                "Scan.Fixture.S03E01.mkv",
                Some("<tvshow><title>Other</title><tvdbid>456</tvdbid></tvshow>"),
            ),
            (
                "Arc One/Scan.Fixture.S03E01.mkv",
                Some("<tvshow><title>Other</title><tvdbid>456</tvdbid></tvshow>"),
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join("Scan Fixture");
            let file = folder.join(relative);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, b"preserved episode").unwrap();
            if let Some(nfo) = nfo {
                std::fs::write(file.parent().unwrap().join("tvshow.nfo"), nfo).unwrap();
            }
            let (app, mut title) = unowned_scan_title(MediaFacet::Anime, root.path()).await;
            title
                .external_ids
                .push(scryer_domain::ExternalId::new("tvdb", "123"));
            assert!(
                claim_title_folder_for_scan_if_missing(&app, &mut title, &folder)
                    .await
                    .is_err()
            );
            assert!(title.folder_path.is_none());
            assert_eq!(std::fs::read(&file).unwrap(), b"preserved episode");
        }
    }

    #[test]
    fn native_title_folder_ownership_uses_platform_case_rules() {
        let title = title(Some("/data/Anime/CASE SPLIT FIXTURE"));
        let candidate = Path::new("/data/Anime/Case Split Fixture");

        if cfg!(windows) {
            assert!(title_owns_folder(&title, candidate));
        } else {
            assert!(title_owns_another_folder(&title, candidate));
        }
    }

    /// The guards before their comparison loops moved off the runtime and
    /// stopped re-resolving the compared folder per owner, kept so the current
    /// guards can be held to their decisions. They share today's comparison,
    /// so another title's unreadable folder refuses neither.
    mod previous {
        use super::super::*;

        pub(super) async fn move_available(
            app: &AppUseCase,
            title: &Title,
            folder_path: &Path,
        ) -> AppResult<()> {
            if folder_path.as_os_str().is_empty() {
                return Err(AppError::validation_refused(
                    crate::error_reasons::folder_ownership::TITLE_FOLDER_PATH_REQUIRED,
                    "title folder path is required",
                ));
            }
            let folder_path = path_to_stored_string(folder_path);
            if let Some(context) = ScanOwnershipContext::current() {
                return context.check(app, title, folder_path).await;
            }
            let mut keys = None;
            for owner in app
                .services
                .catalog
                .titles
                .list_delete_preview_info()
                .await?
            {
                let Some(owned_path) = owner.folder_path.as_deref().filter(|path| !path.is_empty())
                else {
                    continue;
                };
                if owner.title_id == title.id {
                    continue;
                }
                if keys.is_none() {
                    let root_paths = FolderOverlapKeys::root_paths(app).await?;
                    keys = Some(FolderOverlapKeys::from_roots(&folder_path, &root_paths)?);
                }
                if !keys
                    .as_mut()
                    .expect("overlap keys loaded")
                    .needs_exact_comparison(owned_path)
                {
                    continue;
                }
                let overlaps = title_folders_overlap(&folder_path, owned_path).map_err(|error| {
                    AppError::validation_refused(crate::error_reasons::folder_ownership::CANNOT_COMPARE_FOLDER_FOLDER_OWNED_TITLE, format!(
                        "cannot compare folder {folder_path} with folder {owned_path} owned by title {}: {error}",
                        owner.title_id
                    ))
                })?;
                if overlaps && !folder_spans_a_library_root(app, owned_path).await? {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE,
                        format!(
                            "folder {folder_path} overlaps the folder owned by title {}",
                            owner.title_id
                        ),
                    ));
                }
            }
            Ok(())
        }

        pub(super) async fn restore_ownership(
            app: &AppUseCase,
            title_id: Option<&str>,
            destination: &Path,
        ) -> AppResult<()> {
            let owners = app
                .services
                .catalog
                .titles
                .list_delete_preview_info()
                .await?;
            let destination = path_to_stored_string(destination);
            let current_folder = owners
                .iter()
                .find(|owner| Some(owner.title_id.as_str()) == title_id)
                .and_then(|owner| owner.folder_path.as_deref())
                .filter(|folder| !folder.is_empty());
            if let Some(folder) = current_folder {
                let resolved_folder = resolved_folder_identity(&stored_path_to_path_buf(folder))?;
                let resolved_destination =
                    resolved_folder_identity(&stored_path_to_path_buf(&destination))?;
                if !crate::title_folder_rules::path_is_strictly_within(&destination, folder)
                    || resolved_destination == resolved_folder
                    || !resolved_destination.starts_with(&resolved_folder)
                {
                    return Err(AppError::validation_refused(crate::error_reasons::folder_ownership::RECYCLE_RESTORE_PATH_OUTSIDE_TITLE_S_CURRENT_OWNED_FOLDER, "recycle restore path is outside the title's current owned folder; resolve the folder change before restoring"));
                }
            }
            for owner in &owners {
                if Some(owner.title_id.as_str()) == title_id {
                    continue;
                }
                if let Some(folder) = owner
                    .folder_path
                    .as_deref()
                    .filter(|folder| !folder.is_empty())
                    && title_folders_overlap(current_folder.unwrap_or(&destination), folder)?
                {
                    return Err(AppError::validation_refused(crate::error_reasons::folder_ownership::RECYCLE_RESTORE_DESTINATION_OVERLAPS_ANOTHER_TITLE_S_OWNED_FOLDER, "recycle restore destination overlaps another title's owned folder; recycled files were preserved"));
                }
            }
            Ok(())
        }

        pub(super) async fn claim_for_delete(
            app: &AppUseCase,
            title: &mut Title,
            folder_path: &Path,
        ) -> AppResult<()> {
            ensure_folder_available_to_title(app, title, folder_path).await?;
            if non_empty_folder_path(title.folder_path.as_deref()).is_some() {
                return Ok(());
            }
            let stored = path_to_stored_string(folder_path);
            for owner in app
                .services
                .catalog
                .titles
                .list_delete_preview_info()
                .await?
            {
                if owner.title_id == title.id {
                    continue;
                }
                if let Some(other) = owner.folder_path.as_deref().filter(|path| !path.is_empty())
                    && title_folders_overlap(&stored, other)?
                {
                    return Err(AppError::validation_refused(
                        crate::title_delete::FOLDER_OVERLAPS_TITLE,
                        "title folder overlaps another title's recorded folder",
                    ));
                }
            }
            ensure_unowned_folder_claimable(
                app,
                title,
                folder_path,
                TitleFolderClaimLayout::Configured,
            )
            .await?;
            let write = crate::TitleFolderWrite::assign(&title.id, &stored).expecting(None);
            match app
                .services
                .catalog
                .titles
                .assign_title_folders(&[write])
                .await
            {
                Ok(()) => {}
                Err(error) if title_folder_write_lost(&error) => {
                    return Err(AppError::validation_refused(
                        crate::title_delete::FOLDER_CLAIM_LOST,
                        "title folder ownership changed before deletion",
                    ));
                }
                Err(error) => return Err(error),
            }
            title.folder_path = Some(recorded_folder_after_claim(app, &title.id, stored).await?);
            Ok(())
        }

        /// The folder-match overlap loop, given the catalog it read.
        pub(super) fn folder_match_overlaps(
            selected_folder: &str,
            title_id: &str,
            candidates: Vec<crate::TitleDeletePreviewInfo>,
        ) -> AppResult<bool> {
            for candidate in candidates {
                if candidate.title_id == title_id {
                    continue;
                }
                if let Some(folder) = candidate.folder_path
                    && !crate::stored_paths::folder_paths_match(selected_folder, &folder)
                    && title_folders_overlap(selected_folder, &folder)?
                {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }

    const EMBERFALL_OWNER: &str = "emberfall-owner";
    const ROOT_SPANNING_OWNER: &str = "root-spanning-owner";

    /// The decision a guard reached: allowed, refused with a reason, or
    /// failed without one. The message stays out of it only where a
    /// failure carries no reason code.
    fn decision(result: &AppResult<()>) -> Result<(), (Option<String>, String)> {
        result.as_ref().map(|_| ()).map_err(|error| {
            (
                error.validation_reason().map(str::to_string),
                error.to_string(),
            )
        })
    }

    struct GuardFixture {
        _root: tempfile::TempDir,
        root: std::path::PathBuf,
        app: AppUseCase,
        user: scryer_domain::User,
        subject: Title,
    }

    /// A Movie library rooted in a temp dir, a folderless subject title and
    /// one owner of `Shelf/Emberfall`. `root_spanning` adds an owner of the
    /// library root itself ahead of it.
    async fn guard_fixture(root_spanning: bool) -> GuardFixture {
        let root_dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root_dir.path()).unwrap();
        for folder in [
            "Shelf/Emberfall/Season 01",
            "Shelf/Ember",
            "Shelf/Cinderwake",
            "Shelf/Ashgrove",
            CINDERWAKE_NFC,
        ] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("Shelf/Emberfall"), root.join("Shelf/Ash Alias"))
            .unwrap();
        let (app, user) = crate::lib_tests::bootstrap();
        let facet = MediaFacet::Movie;
        let library_id = scryer_domain::default_library_id_for_facet(&facet);
        let library = app
            .services
            .catalog
            .libraries
            .get_by_id(&library_id)
            .await
            .unwrap()
            .unwrap();
        app.services
            .catalog
            .libraries
            .update(
                &library_id,
                library.name,
                library.slug,
                vec![crate::LibraryRootDraft {
                    id: None,
                    path: path_to_stored_string(&root),
                    is_default: true,
                }],
            )
            .await
            .unwrap();
        let subject = app
            .create_title_without_hydration_in_library(
                &user,
                crate::NewTitle {
                    name: "Ashgrove".into(),
                    facet,
                    ..Default::default()
                },
                library_id,
            )
            .await
            .unwrap()
            .title;
        let mut owners = Vec::new();
        if root_spanning {
            owners.push((ROOT_SPANNING_OWNER, root.clone()));
        }
        owners.push((EMBERFALL_OWNER, root.join("Shelf/Emberfall")));
        owners.push(("cinderwake-owner", root.join(CINDERWAKE_NFC)));
        for (id, folder) in owners {
            let mut owner = subject.clone();
            owner.id = id.into();
            owner.name = "Emberfall".into();
            owner.folder_path = None;
            app.services
                .catalog
                .titles
                .create_or_get_existing(owner)
                .await
                .unwrap();
            // The test catalog records a library-root folder as damage would.
            app.services
                .catalog
                .titles
                .set_folder_path(id, &path_to_stored_string(folder))
                .await
                .unwrap();
        }
        GuardFixture {
            _root: root_dir,
            root,
            app,
            user,
            subject,
        }
    }

    /// Folders each guard is asked about, relative to the library root,
    /// with whether they overlap `Shelf/Emberfall`.
    fn guard_cases() -> Vec<(&'static str, bool)> {
        let cases = vec![
            ("Shelf/Emberfall", true),
            ("Shelf/Emberfall/", true),
            ("Shelf/Emberfall/Season 01", true),
            ("Shelf/Emberfall/Season 02", true),
            ("Shelf", true),
            ("Shelf/Ember", false),
            ("Shelf/Emberfall 2", false),
            ("Shelf/Cinderwake", false),
            ("Shelf/Ashgrove", false),
        ];
        #[cfg(unix)]
        let cases = [
            cases,
            vec![
                ("Shelf/Ash Alias", true),
                ("Shelf/Ash Alias/Season 01", true),
            ],
        ]
        .concat();
        cases
    }

    /// A second owner whose folder name has a precomposed letter.
    const CINDERWAKE_NFC: &str = "Shelf/Cinderwak\u{e9}";

    /// Case and Unicode spellings, whose outcome can depend on the volume.
    const SPELLING_CASES: [&str; 4] = [
        "Shelf/emberfall",
        "Shelf/EMBERFALL/Season 01",
        "Shelf/Cinderwake\u{301}",
        "Shelf/Cinderwake\u{301}/Season 01",
    ];

    #[tokio::test]
    async fn folder_move_guard_matches_previous_decisions() {
        for root_spanning in [false, true] {
            let fixture = guard_fixture(root_spanning).await;
            let folders = guard_cases()
                .into_iter()
                .map(|(folder, overlaps)| (folder, Some(overlaps)))
                .chain(SPELLING_CASES.iter().map(|folder| (*folder, None)));
            for (folder, overlaps) in folders {
                let path = fixture.root.join(folder);
                let current =
                    ensure_folder_move_available_to_title(&fixture.app, &fixture.subject, &path)
                        .await;
                let previous =
                    previous::move_available(&fixture.app, &fixture.subject, &path).await;
                assert_eq!(decision(&current), decision(&previous), "{folder}");
                // The library-root owner overlaps everything and is exempt.
                if let Some(overlaps) = overlaps {
                    assert_eq!(
                        current.err().and_then(|error| error
                            .validation_reason()
                            .map(str::to_string)),
                        overlaps.then(|| crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE.to_string()),
                        "{folder}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn restore_guard_matches_previous_decisions() {
        let fixture = guard_fixture(false).await;
        let folders = guard_cases()
            .into_iter()
            .map(|(folder, overlaps)| (folder, Some(overlaps)))
            .chain(SPELLING_CASES.iter().map(|folder| (*folder, None)));
        for (folder, overlaps) in folders {
            let path = fixture.root.join(folder);
            for title_id in [None, Some(fixture.subject.id.as_str())] {
                let current = ensure_restore_ownership(&fixture.app, title_id, &path).await;
                let previous = previous::restore_ownership(&fixture.app, title_id, &path).await;
                assert_eq!(decision(&current), decision(&previous), "{folder}");
                if let Some(overlaps) = overlaps {
                    assert_eq!(current.is_err(), overlaps, "{folder}");
                }
            }
        }
        // The restoring title's own folder is the compared one when it has
        // one: a destination inside it, inside another title's folder, and
        // outside it.
        let fixture = guard_fixture(false).await;
        for folder in [
            "Shelf/Emberfall/Season 01",
            "Shelf/Ember/Season 01",
            "Shelf/Cinderwake/Season 01",
            "Shelf/Ember",
        ] {
            let path = fixture.root.join(folder);
            for title_id in [Some(EMBERFALL_OWNER), Some("missing-title")] {
                let current = ensure_restore_ownership(&fixture.app, title_id, &path).await;
                let previous = previous::restore_ownership(&fixture.app, title_id, &path).await;
                assert_eq!(decision(&current), decision(&previous), "{folder}");
            }
        }
    }

    #[tokio::test]
    async fn delete_claim_guard_matches_previous_decisions() {
        let folders = guard_cases()
            .into_iter()
            .map(|(folder, overlaps)| (folder, Some(overlaps)))
            .chain(SPELLING_CASES.iter().map(|folder| (*folder, None)));
        for (folder, overlaps) in folders {
            // The claim records the folder, so each side gets its own catalog.
            let current_fixture = guard_fixture(false).await;
            let previous_fixture = guard_fixture(false).await;
            let mut current_title = current_fixture.subject.clone();
            let mut previous_title = previous_fixture.subject.clone();
            let current = claim_title_folder_for_delete(
                &current_fixture.app,
                &mut current_title,
                &current_fixture.root.join(folder),
            )
            .await;
            let previous = previous::claim_for_delete(
                &previous_fixture.app,
                &mut previous_title,
                &previous_fixture.root.join(folder),
            )
            .await;
            let relative = |fixture: &GuardFixture, title: &Title| {
                title.folder_path.as_deref().map(|recorded| {
                    recorded
                        .strip_prefix(&path_to_stored_string(&fixture.root))
                        .unwrap_or(recorded)
                        .to_string()
                })
            };
            let message_free = |result: &AppResult<()>| {
                decision(result).map_err(|(reason, message)| {
                    (
                        reason,
                        message
                            .replace(&path_to_stored_string(&current_fixture.root), "<root>")
                            .replace(&path_to_stored_string(&previous_fixture.root), "<root>"),
                    )
                })
            };
            assert_eq!(message_free(&current), message_free(&previous), "{folder}");
            assert_eq!(
                relative(&current_fixture, &current_title),
                relative(&previous_fixture, &previous_title),
                "{folder}"
            );
            // An exact owner is refused by the ownership check ahead of the
            // overlap loop.
            if let Some(overlaps) = overlaps {
                assert_eq!(current.is_err(), overlaps, "{folder}");
            }
        }
    }

    #[tokio::test]
    async fn folder_match_guard_matches_previous_decisions() {
        let fixture = guard_fixture(false).await;
        let folders = guard_cases()
            .into_iter()
            .map(|(folder, _)| folder)
            .chain(SPELLING_CASES);
        for folder in folders {
            let selected = path_to_stored_string(fixture.root.join(folder));
            let catalog = fixture
                .app
                .services
                .catalog
                .titles
                .list_delete_preview_info()
                .await
                .unwrap();
            let previous = previous::folder_match_overlaps(
                &path_to_stored_string(stored_path_to_path_buf(&selected)),
                &fixture.subject.id,
                catalog,
            );
            let current = fixture
                .app
                .change_title_folder_preview(&fixture.user, &fixture.subject.id, &selected)
                .await;
            let refused_for_overlap = current
                .as_ref()
                .err()
                .and_then(|error| error.validation_reason())
                == Some(
                    crate::error_reasons::location::SELECTED_FOLDER_OVERLAPS_ANOTHER_TITLE_S_FOLDER,
                );
            // Checks ahead of the overlap loop may refuse first; the loop's
            // refusal appears exactly where the previous loop refused.
            match previous {
                Ok(true) => assert!(current.is_err(), "{folder}"),
                Ok(false) => assert!(!refused_for_overlap, "{folder}"),
                Err(_) => assert!(current.is_err(), "{folder}"),
            }
        }
    }

    #[test]
    fn one_folder_compared_with_many_matches_single_comparisons() {
        let root = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root.path()).unwrap();
        std::fs::create_dir_all(root.join("Shelf/Emberfall/Season 01")).unwrap();
        std::fs::create_dir_all(root.join("Shelf/Cinderwake")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("Shelf/Emberfall"), root.join("Shelf/Ash Alias"))
            .unwrap();
        let others = [
            "",
            "relative/Ashgrove",
            "../Ashgrove",
            "/Shelf/../Ashgrove",
            "Shelf/Emberfall",
            "Shelf/Emberfall/Season 01",
            "Shelf",
            "Shelf/Ember",
            "Shelf/Emberfall 2",
            "Shelf/Cinderwake",
            "Shelf/emberfall",
            "Shelf/Emberfall\u{0301}",
            "Shelf/Ash Alias",
            "Shelf/Ash Alias/Season 01",
        ]
        .map(|other| {
            if other.is_empty() || other.starts_with("relative") || other.starts_with("..") {
                other.to_string()
            } else {
                path_to_stored_string(root.join(other.trim_start_matches('/')))
            }
        });
        let lefts = [
            path_to_stored_string(root.join("Shelf/Emberfall")),
            path_to_stored_string(root.join("Shelf/Ember")),
            path_to_stored_string(root.join("Shelf/Missing/Ashgrove")),
            path_to_stored_string(root.join("Shelf/Ash Alias")),
            "relative/Ashgrove".to_string(),
        ];
        for left in lefts {
            let mut checked = OverlapLeft::new(&left);
            for other in &others {
                let single = title_folders_overlap(&left, other);
                let shared = checked.overlaps(other);
                assert_eq!(
                    single.as_ref().ok(),
                    shared.as_ref().ok(),
                    "{left} vs {other}"
                );
                assert_eq!(
                    single.err().map(|error| error.to_string()),
                    shared.err().map(|error| error.to_string()),
                    "{left} vs {other}"
                );
            }
        }
    }
}
