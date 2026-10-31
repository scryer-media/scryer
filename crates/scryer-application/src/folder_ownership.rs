use std::path::Path;

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
        let by_library: std::collections::HashMap<_, Vec<_>> = libraries
            .into_iter()
            .map(|library| {
                (
                    library.id,
                    library.roots.into_iter().map(|root| root.path).collect(),
                )
            })
            .collect();
        let all = by_library.values().flatten().cloned().collect();
        Ok(std::sync::Arc::new(Self { all, by_library }))
    }
}

tokio::task_local! {
    pub(crate) static SCAN_FOLDER_ROOTS: std::sync::Arc<ScanFolderRoots>;
}

fn non_empty_folder_path(path: Option<&str>) -> Option<&str> {
    path.filter(|path| !path.is_empty())
}

/// Resolve links in the existing prefix without requiring a destination to
/// exist yet. Callers must repeat this check after creating the destination.
pub(crate) fn resolved_folder_identity(path: &Path) -> AppResult<std::path::PathBuf> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
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

pub(crate) fn title_folders_overlap(left: &str, right: &str) -> AppResult<bool> {
    fn overlaps(left: &str, right: &str) -> bool {
        folder_paths_match(left, right)
            || crate::title_folder_rules::path_is_strictly_within(left, right)
            || crate::title_folder_rules::path_is_strictly_within(right, left)
    }
    if overlaps(left, right) {
        return Ok(true);
    }
    let left = resolved_folder_identity(&stored_path_to_path_buf(left))?;
    let right = match resolved_folder_identity(&stored_path_to_path_buf(right)) {
        Ok(path) => path,
        Err(error) => {
            let other = stored_path_to_path_buf(right);
            if !other.is_absolute()
                || other
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                tracing::debug!(%error, "invalid unrelated stored folder omitted from alias comparison");
                return Ok(false);
            }
            return Err(error);
        }
    };
    let left_string = path_to_stored_string(&left);
    let right_string = path_to_stored_string(&right);
    Ok(overlaps(&left_string, &right_string)
        || (overlaps(&left_string.to_lowercase(), &right_string.to_lowercase())
            && (folder_case_insensitive(&left) || folder_case_insensitive(&right))))
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
    let library_roots = if let Ok(roots) = SCAN_FOLDER_ROOTS.try_with(|snapshot| {
        snapshot
            .by_library
            .get(&title.library_id)
            .cloned()
            .unwrap_or_default()
    }) {
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
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &folder_path)
        .await?;
    title.folder_path = Some(folder_path.clone());
    tracing::info!(
        title_id = %title.id,
        previous_folder_path = %previous_folder_path,
        folder_path = %folder_path,
        "re-pointed stale title folder ownership to the scanned folder"
    );
    Ok(true)
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

/// Keys for comparing one move candidate against every recorded owner while
/// resolving the candidate and each library root only once.
pub(crate) struct FolderOverlapKeys {
    candidate: Vec<String>,
    /// Library root keys with their resolved spelling, or `None` when the
    /// root could not be resolved.
    roots: Vec<(String, Option<String>)>,
    /// Whether a folder below a library root is a link, shared by owners
    /// under the same organizing folders.
    links: std::collections::HashMap<std::path::PathBuf, bool>,
}

impl FolderOverlapKeys {
    async fn load(app: &AppUseCase, folder_path: &str) -> AppResult<Self> {
        let root_paths = match SCAN_FOLDER_ROOTS.try_with(|snapshot| snapshot.all.clone()) {
            Ok(roots) => roots,
            Err(_) => app
                .all_library_root_folders()
                .await?
                .into_iter()
                .map(|root| root.path)
                .collect(),
        };
        Self::from_roots(folder_path, &root_paths)
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
        let roots = root_paths
            .iter()
            .filter_map(|root| {
                let key = overlap_prefilter_key(root)?;
                let resolved = resolved_folder_identity(&stored_path_to_path_buf(root))
                    .ok()
                    .and_then(|resolved| overlap_prefilter_key(&path_to_stored_string(&resolved)));
                Some((key, resolved))
            })
            .collect();
        Ok(Self {
            candidate,
            roots,
            links: Default::default(),
        })
    }

    /// Whether `owned_path` needs the exact, resolving overlap comparison.
    /// An owner unrelated by key, also after substituting its resolved library
    /// root, with no link between that root and its folder, cannot overlap.
    pub(crate) fn needs_exact_comparison(&mut self, owned_path: &str) -> AppResult<bool> {
        let Some(owner) = overlap_prefilter_key(owned_path) else {
            return Ok(true);
        };
        let mut owner_keys = vec![owner.clone()];
        for (root, resolved) in &self.roots {
            if overlap_keys_related(&owner, root) && owner.len() >= root.len() {
                let Some(resolved) = resolved else {
                    return Ok(true);
                };
                let rest = owner[root.len()..].trim_start_matches('/');
                owner_keys.push(if rest.is_empty() {
                    resolved.clone()
                } else {
                    format!("{}/{rest}", resolved.trim_end_matches('/'))
                });
            }
        }
        if owner_keys.iter().any(|owner| {
            self.candidate
                .iter()
                .any(|candidate| overlap_keys_related(owner, candidate))
        }) {
            return Ok(true);
        }
        let path = stored_path_to_path_buf(owned_path);
        if !path.is_absolute() {
            return Ok(true);
        }
        for ancestor in path.ancestors() {
            let is_resolved_root = overlap_prefilter_key(&path_to_stored_string(ancestor))
                .is_some_and(|key| {
                    self.roots
                        .iter()
                        .any(|(root, resolved)| resolved.is_some() && *root == key)
                });
            if is_resolved_root {
                break;
            }
            let is_link = match self.links.get(ancestor) {
                Some(is_link) => *is_link,
                None => {
                    let is_link = match std::fs::symlink_metadata(ancestor) {
                        Ok(metadata) => metadata.file_type().is_symlink(),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                        Err(error) => {
                            return Err(AppError::validation_refused(crate::error_reasons::folder_ownership::CANNOT_INSPECT_FOLDER_OWNED_ANOTHER_TITLE, format!(
                                "cannot inspect folder {owned_path} owned by another title: {error}"
                            )));
                        }
                    };
                    self.links.insert(ancestor.to_path_buf(), is_link);
                    is_link
                }
            };
            if is_link {
                return Ok(true);
            }
        }
        Ok(false)
    }
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
    let mut keys = None;
    for owner in app
        .services
        .catalog
        .titles
        .list_delete_preview_info()
        .await?
    {
        let Some(owned_path) = owner.folder_path.as_deref().filter(|path| !path.is_empty()) else {
            continue;
        };
        if owner.title_id == title.id {
            continue;
        }
        if keys.is_none() {
            keys = Some(FolderOverlapKeys::load(app, &folder_path).await?);
        }
        if !keys
            .as_mut()
            .expect("overlap keys loaded")
            .needs_exact_comparison(owned_path)?
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
    ensure_unowned_folder_claimable(app, title, folder_path, TitleFolderClaimLayout::Configured)
        .await?;
    if !app
        .services
        .catalog
        .titles
        .claim_missing_folder_path(&title.id, &stored)
        .await?
    {
        return Err(AppError::validation_refused(
            crate::title_delete::FOLDER_CLAIM_LOST,
            "title folder ownership changed before deletion",
        ));
    }
    title.folder_path = Some(stored);
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
    claim_title_folder_with_layout(
        app,
        title,
        folder_path,
        TitleFolderClaimLayout::Conventional,
    )
    .await
}

enum TitleFolderClaimLayout {
    Configured,
    Conventional,
}

/// Every check [`claim_title_folder_if_missing`] makes before recording
/// `folder_path` for a title that has no folder, without recording it.
pub(crate) async fn ensure_title_folder_claimable(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<()> {
    ensure_folder_available_to_title(app, title, folder_path).await?;
    ensure_unowned_folder_claimable(app, title, folder_path, TitleFolderClaimLayout::Configured)
        .await
}

async fn claim_title_folder_with_layout(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
    layout: TitleFolderClaimLayout,
) -> AppResult<()> {
    ensure_folder_available_to_title(app, title, folder_path).await?;
    if non_empty_folder_path(title.folder_path.as_deref()).is_some() {
        return Ok(());
    }

    ensure_unowned_folder_claimable(app, title, folder_path, layout).await?;
    let folder_path = path_to_stored_string(folder_path);
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &folder_path)
        .await?;
    title.folder_path = Some(folder_path);
    Ok(())
}

async fn ensure_unowned_folder_claimable(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
    layout: TitleFolderClaimLayout,
) -> AppResult<()> {
    ensure_folder_move_available_to_title(app, title, folder_path).await?;
    // Imports reserve missing or empty destinations before writing media. A
    // destination with contents must not hold other titles' folders; stray
    // files or empty subfolders left by an earlier attempt do not block it.
    let has_contents = match tokio::fs::symlink_metadata(folder_path).await {
        Ok(_) => tokio::fs::read_dir(folder_path)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?
            .next_entry()
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?
            .is_some(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(AppError::Repository(error.to_string())),
    };
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
            TitleFolderClaimLayout::Conventional => {
                crate::library_discovery::ensure_scanned_folder_holds_no_child_titles(
                    folder_path,
                    title,
                )
                .await?;
            }
        }
    }

    let folder_path = path_to_stored_string(folder_path);
    if folder_spans_a_library_root(app, &folder_path).await? {
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
    if title_folder_is_outside_its_library(app, title, &folder_path).await? {
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
    Ok(())
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
            let filtered = if keys.needs_exact_comparison(&owner).unwrap_or(true) {
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
            assert!(keys.needs_exact_comparison(&owner).unwrap_or(true));
            let exact = title_folders_overlap(&candidate, &owner);
            if owner.ends_with("Loop") {
                assert!(exact.is_err());
            } else {
                assert!(exact.unwrap());
            }
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
                vec![crate::LibraryRootDraft {
                    id: None,
                    path: path_to_stored_string(root),
                    is_default: true,
                }],
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
}
