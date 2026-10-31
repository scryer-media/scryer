//! Evidence-based repair of root ownership. Recovery never disposes of sources.

mod execute;
#[cfg(test)]
mod tests;

use super::model::*;
use crate::stored_paths::{folder_paths_match, path_to_stored_string, stored_path_to_path_buf};
use crate::title_folder_rules::{
    path_is_strictly_within, stored_path_is_inside_folder, title_folder_from_media_paths,
};
use crate::{AppError, AppResult, AppUseCase};
use scryer_domain::{LibraryRoot, Title};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const RECOVERY_VERSION: &str = "title-folder-recovery-v1";
const MAX_DISCOVERY_ENTRIES: usize = 100_000;
pub const MAX_CATALOG_ROWS: usize = 250_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryOwner {
    pub id: String,
    pub library_id: String,
    pub folder_path: Option<String>,
    pub revision: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryMedia {
    pub id: String,
    pub title_id: String,
    pub file_path: String,
    pub size_bytes: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryCatalog {
    pub roots: Vec<LibraryRoot>,
    pub owners: Vec<RecoveryOwner>,
    pub media: Vec<RecoveryMedia>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryFile {
    pub media: RecoveryMedia,
    pub destination: String,
    pub source_version: Option<String>,
    pub expected_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecoveryPlan {
    pub version: String,
    pub title: Title,
    pub owner_revision: Option<chrono::DateTime<chrono::Utc>>,
    pub roots: Vec<LibraryRoot>,
    pub expected_media: Vec<RecoveryMedia>,
    pub destination_root: Option<LibraryRoot>,
    pub destination: Option<String>,
    #[serde(default)]
    pub generated_destination: bool,
    pub files: Vec<RecoveryFile>,
    pub unresolved: Option<String>,
}

pub fn eligible(folder: Option<&str>, roots: &[LibraryRoot]) -> bool {
    folder
        .filter(|s| !s.trim().is_empty())
        .is_some_and(|folder| {
            roots.iter().any(|root| {
                folder_paths_match(folder, &root.path)
                    || path_is_strictly_within(&root.path, folder)
            })
        })
}

fn fail(reason: &'static str, message: impl Into<String>) -> AppError {
    AppError::validation_refused(reason, message)
}

// Filesystem and serialization diagnostics are relayed at this boundary; no
// application condition is inferred from the external sentence.
fn relay_failure(message: String) -> AppError {
    AppError::Validation(message)
}

fn path_key(path: &str) -> String {
    crate::stored_paths::folder_path_identity_key(path).unwrap_or_else(|| path.to_owned())
}

fn overlaps(left: &str, right: &str) -> bool {
    folder_paths_match(left, right)
        || path_is_strictly_within(left, right)
        || path_is_strictly_within(right, left)
}

/// Shared by planning and the transactional recheck. Invalid ancestor claims
/// are not evidence that another title owns these exact files.
pub fn validate_catalog(plan: &RecoveryPlan, catalog: &RecoveryCatalog) -> AppResult<()> {
    if plan.version != RECOVERY_VERSION || plan.roots != catalog.roots {
        return Err(fail(
            crate::error_reasons::location::RECOVERY_ROOTS_CHANGED,
            "Library roots changed; recovery needs a new plan",
        ));
    }
    let owner = catalog
        .owners
        .iter()
        .find(|o| o.id == plan.title.id)
        .ok_or_else(|| {
            fail(
                crate::error_reasons::location::RECOVERY_TITLE_NOT_FOUND,
                "The title no longer exists",
            )
        })?;
    if owner.library_id != plan.title.library_id
        || owner.folder_path != plan.title.folder_path
        || owner.revision != plan.owner_revision
    {
        return Err(fail(
            crate::error_reasons::location::RECOVERY_OWNERSHIP_CHANGED,
            "Title ownership changed; originals preserved",
        ));
    }
    let mut media: Vec<_> = catalog
        .media
        .iter()
        .filter(|m| m.title_id == plan.title.id)
        .cloned()
        .collect();
    media.sort_by(|a, b| a.id.cmp(&b.id));
    if media != plan.expected_media {
        return Err(fail(
            crate::error_reasons::location::RECOVERY_TRACKED_MEDIA_CHANGED,
            "Tracked media changed; originals preserved",
        ));
    }
    let destination = plan.destination.as_deref().ok_or_else(|| {
        fail(
            crate::error_reasons::location::RECOVERY_DESTINATION_UNPROVEN,
            "No proven destination",
        )
    })?;
    let root = plan.destination_root.as_ref().ok_or_else(|| {
        fail(
            crate::error_reasons::location::RECOVERY_DESTINATION_ROOT_UNPROVEN,
            "No proven destination root",
        )
    })?;
    if root.library_id != plan.title.library_id
        || !path_is_strictly_within(destination, &root.path)
        || eligible(Some(destination), &catalog.roots)
    {
        return Err(fail(
            crate::error_reasons::location::RECOVERY_DESTINATION_BOUNDARY_INVALID,
            "Recovery destination is not a valid title folder",
        ));
    }
    for other in &catalog.owners {
        if other.id != plan.title.id
            && let Some(folder) = other.folder_path.as_deref()
            && !eligible(Some(folder), &catalog.roots)
            && overlaps(destination, folder)
        {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_DESTINATION_OWNERSHIP_CONFLICT,
                "Another title owns an overlapping destination folder",
            ));
        }
    }
    for media in &catalog.media {
        if media.title_id != plan.title.id
            && (stored_path_is_inside_folder(destination, &media.file_path)
                || plan
                    .files
                    .iter()
                    .any(|f| folder_paths_match(&f.media.file_path, &media.file_path)))
        {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_FILE_OWNERSHIP_CONFLICT,
                "Another title tracks a source or destination file",
            ));
        }
        if plan
            .files
            .iter()
            .any(|f| f.media.id != media.id && folder_paths_match(&f.destination, &media.file_path))
        {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_DESTINATION_RECORD_CONFLICT,
                "A destination is already associated with another media record",
            ));
        }
    }
    let mut destinations = BTreeSet::new();
    for file in &plan.files {
        if !path_is_strictly_within(&file.destination, destination)
            || !destinations.insert(path_key(&file.destination))
        {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_DESTINATION_PATH_INVALID,
                "Recovery file destinations collide or escape the title folder",
            ));
        }
    }
    Ok(())
}

/// Reject links in every existing component, including Windows junctions.
/// Missing final components are allowed only for a destination about to be created.
pub(super) async fn inspect_path(path: &Path, allow_missing: bool) -> AppResult<()> {
    if !path.is_absolute() {
        return Err(fail(
            crate::error_reasons::location::RECOVERY_NATIVE_ABSOLUTE_PATH_REQUIRED,
            "Recovery requires an absolute native filesystem path",
        ));
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_DOT_SEGMENT_PATH_FORBIDDEN,
                "Recovery does not follow dot-segment paths",
            ));
        }
        current.push(component);
        match tokio::fs::symlink_metadata(&current).await {
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err(fail(
                            crate::error_reasons::location::RECOVERY_REPARSE_POINT_FORBIDDEN,
                            "Recovery does not follow reparse points",
                        ));
                    }
                }
                if meta.file_type().is_symlink() {
                    return Err(fail(
                        crate::error_reasons::location::RECOVERY_SYMLINK_FORBIDDEN,
                        "Recovery does not follow symbolic links",
                    ));
                }
            }
            Err(e) if allow_missing && e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_PATH_UNAVAILABLE,
                    format!("Recovery path is unavailable: {e}"),
                ));
            }
        }
    }
    Ok(())
}

/// A content version also includes the filesystem identity, so replacing a file
/// with equal length and timestamps cannot reuse an old verification receipt.
async fn recovery_file_version(path: &Path) -> AppResult<String> {
    inspect_path(path, false).await?;
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| relay_failure(e.to_string()))?;
    if !metadata.is_file() {
        return Err(fail(
            crate::error_reasons::location::RECOVERY_REGULAR_FILE_REQUIRED,
            "Recovery requires a regular file",
        ));
    }
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec()
        )
    };
    #[cfg(windows)]
    let identity = {
        let (volume, index) = super::transfer::windows_file_identity(path).ok_or_else(|| {
            fail(
                crate::error_reasons::location::RECOVERY_FILE_IDENTITY_UNAVAILABLE,
                "Filesystem identity is unavailable",
            )
        })?;
        format!("{volume}:{index}")
    };
    #[cfg(not(any(unix, windows)))]
    return Err(fail(
        crate::error_reasons::location::RECOVERY_FILE_IDENTITY_UNSUPPORTED,
        "Filesystem identity is unsupported",
    ));
    #[cfg(any(unix, windows))]
    Ok(format!(
        "{identity}:{}",
        super::resolution::file_version(path).await?
    ))
}

fn unambiguous_layout(folder: &str, media: &str) -> bool {
    let folder = stored_path_to_path_buf(folder);
    let media = stored_path_to_path_buf(media);
    let Some(parent) = media.parent() else {
        return false;
    };
    if parent == folder {
        return true;
    }
    if parent.parent() != Some(folder.as_path()) {
        return false;
    }
    let Some(name) = parent.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower == "specials"
        || lower
            .strip_prefix("season ")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn external_ids_match(title: &Title, ids: &[(&str, Option<String>)]) -> bool {
    let mut matched = false;
    for (source, value) in ids {
        let Some(value) = value else { continue };
        let known: Vec<_> = title
            .external_ids
            .iter()
            .filter(|id| {
                id.source.eq_ignore_ascii_case(source)
                    && crate::normalize::external_id_kind_fits_facet(id, &title.facet)
            })
            .collect();
        if known.is_empty() {
            continue;
        }
        if known.iter().any(|id| id.value.eq_ignore_ascii_case(value)) {
            matched = true;
        } else {
            return false;
        }
    }
    matched
}

async fn folder_matches(title: &Title, path: &Path) -> AppResult<bool> {
    folder_identity_matches(title, path, false).await
}

async fn folder_identity_matches(
    title: &Title,
    path: &Path,
    require_consistency: bool,
) -> AppResult<bool> {
    let parsed = crate::library_filename_parser::parse_library_filename(
        &crate::library_filename_parser::LibraryFilenameParseInput::title_only(path, path.parent()),
    );
    let parsed = parsed.parsed_release;
    let mut ids = vec![
        ("tvdb", parsed.tvdb_id),
        ("tmdb", parsed.tmdb_id),
        ("imdb", parsed.imdb_id),
    ];
    let mut identity_errors = Vec::new();
    for name in ["tvshow.nfo", "movie.nfo"] {
        let file = path.join(name);
        let content: AppResult<Option<String>> = async {
            match tokio::fs::symlink_metadata(&file).await {
                Ok(meta)
                    if meta.is_file()
                        && !meta.file_type().is_symlink()
                        && meta.len() <= 1024 * 1024 =>
                {
                    inspect_path(&file, false).await?;
                    tokio::fs::read_to_string(&file)
                        .await
                        .map(Some)
                        .map_err(|e| relay_failure(e.to_string()))
                }
                Ok(_) => Err(fail(
                    crate::error_reasons::location::RECOVERY_IDENTITY_SIDECAR_INVALID,
                    "Identity sidecar is unsupported or too large",
                )),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(relay_failure(e.to_string())),
            }
        }
        .await;
        match content {
            Ok(Some(content)) => {
                let meta = crate::nfo::parse_nfo(&content);
                ids.extend([
                    ("tvdb", meta.tvdb_id),
                    ("tmdb", meta.tmdb_id),
                    ("imdb", meta.imdb_id),
                ]);
            }
            Ok(None) => {}
            Err(error) => identity_errors.push(error),
        }
    }
    let relevant = require_consistency
        || ids
            .iter()
            .any(|id| external_ids_match(title, std::slice::from_ref(id)));
    if let Some(error) = identity_errors.into_iter().next() {
        return if relevant { Err(error) } else { Ok(false) };
    }
    // Two sources naming different IDs are ambiguous even when one matches.
    for source in ["tvdb", "tmdb", "imdb"] {
        let values: BTreeSet<_> = ids
            .iter()
            .filter(|(s, _)| *s == source)
            .filter_map(|(_, v)| v.as_deref())
            .collect();
        if values.len() > 1 {
            return if relevant {
                Err(fail(
                    crate::error_reasons::location::RECOVERY_IDENTITY_EVIDENCE_CONFLICT,
                    "Folder identity evidence conflicts",
                ))
            } else {
                Ok(false)
            };
        }
    }
    if require_consistency
        && ids.iter().any(|(source, value)| {
            value.as_ref().is_some_and(|value| {
                let known: Vec<_> = title
                    .external_ids
                    .iter()
                    .filter(|id| {
                        id.source.eq_ignore_ascii_case(source)
                            && crate::normalize::external_id_kind_fits_facet(id, &title.facet)
                    })
                    .collect();
                !known.is_empty() && !known.iter().any(|id| id.value.eq_ignore_ascii_case(value))
            })
        })
    {
        return Err(fail(
            crate::error_reasons::location::RECOVERY_FOLDER_TITLE_IDENTITY_MISMATCH,
            "Tracked folder identity contradicts the title",
        ));
    }
    Ok(external_ids_match(title, &ids))
}

async fn locate_hashed_destination(
    folder: &Path,
    size: u64,
    hash: &str,
    recycle: Option<&str>,
) -> AppResult<PathBuf> {
    let mut pending = vec![folder.to_path_buf()];
    let mut found = None;
    let mut seen = 0usize;
    while let Some(directory) = pending.pop() {
        inspect_path(&directory, false).await?;
        let mut entries = tokio::fs::read_dir(&directory)
            .await
            .map_err(|e| relay_failure(e.to_string()))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| relay_failure(e.to_string()))?
        {
            seen += 1;
            if seen > MAX_DISCOVERY_ENTRIES {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_CONTENT_DISCOVERY_LIMIT_REACHED,
                    "Content discovery limit reached",
                ));
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == ".scryer-recycle"
                || name.starts_with(".scryer-upgrade-replacement-")
                || name.starts_with(".scryer-recovery-")
            {
                continue;
            }
            let path = entry.path();
            if recycle
                .is_some_and(|r| stored_path_is_inside_folder(r, &path_to_stored_string(&path)))
            {
                continue;
            }
            inspect_path(&path, false).await?;
            let metadata = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|e| relay_failure(e.to_string()))?;
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() || metadata.len() != size {
                continue;
            }
            let before = recovery_file_version(&path).await?;
            let proof = super::verify::hash_existing_file_with_progress(
                &path,
                super::verify::CopyProgress::none(),
            )
            .await?;
            if recovery_file_version(&path).await? != before {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_DESTINATION_CHANGED,
                    "Destination changed during content discovery",
                ));
            }
            if proof.full_blake3 == hash {
                if found.is_some() {
                    return Err(fail(
                        crate::error_reasons::location::RECOVERY_DESTINATION_HASH_AMBIGUOUS,
                        "Multiple destination files match the missing source hash",
                    ));
                }
                found = Some(path);
            }
        }
    }
    found.ok_or_else(|| {
        fail(
            crate::error_reasons::location::RECOVERY_DESTINATION_HASH_NOT_FOUND,
            "No destination matches the missing source's full-content hash",
        )
    })
}

impl AppUseCase {
    pub(crate) async fn plan_title_folder_recovery(
        &self,
        title: &Title,
    ) -> AppResult<RecoveryPlan> {
        let catalog = self
            .services
            .library
            .location_operations
            .recovery_catalog()
            .await?;
        let mut expected_media: Vec<_> = catalog
            .media
            .iter()
            .filter(|m| m.title_id == title.id)
            .cloned()
            .collect();
        expected_media.sort_by(|a, b| a.id.cmp(&b.id));
        let mut plan = RecoveryPlan {
            version: RECOVERY_VERSION.into(),
            title: title.clone(),
            owner_revision: catalog
                .owners
                .iter()
                .find(|o| o.id == title.id)
                .and_then(|o| o.revision),
            roots: catalog.roots.clone(),
            expected_media,
            destination_root: None,
            destination: None,
            generated_destination: false,
            files: Vec::new(),
            unresolved: None,
        };
        if let Err(error) = self.populate_recovery_plan(&mut plan, &catalog).await {
            plan.unresolved = Some(error.to_string());
            plan.files.clear();
        }
        Ok(plan)
    }

    async fn populate_recovery_plan(
        &self,
        plan: &mut RecoveryPlan,
        catalog: &RecoveryCatalog,
    ) -> AppResult<()> {
        if !eligible(plan.title.folder_path.as_deref(), &catalog.roots) {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_ROOT_CORRUPTION_REQUIRED,
                "Title ownership is not a root corruption case",
            ));
        }
        if let Some(reason) = self.active_work_blocking_a_move(&plan.title).await? {
            return Err(fail(crate::error_reasons::location::OWNERSHIP_BUSY, reason));
        }
        let roots: Vec<_> = catalog
            .roots
            .iter()
            .filter(|r| r.library_id == plan.title.library_id)
            .cloned()
            .collect();
        if roots.is_empty() {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_ROOTS_REQUIRED,
                "Title library has no configured roots",
            ));
        }
        let tracked = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&plan.title.id)
            .await?;
        let custom_recycle = self
            .read_setting_string_value_for_scope(
                crate::SETTINGS_SCOPE_MEDIA,
                "recycle_bin.path",
                None,
            )
            .await?;
        let live: Vec<_> = tracked
            .iter()
            .filter(|m| {
                let path = m.file_path.replace('\\', "/");
                !path.contains("/.scryer-recycle/")
                    && !path.contains("/.scryer-upgrade-replacement-")
                    && !path.contains("/.scryer-recovery-")
                    && !custom_recycle
                        .as_ref()
                        .is_some_and(|r| stored_path_is_inside_folder(r, &m.file_path))
            })
            .collect();
        let root_paths: Vec<_> = roots.iter().map(|r| r.path.clone()).collect();
        let implied =
            title_folder_from_media_paths(&root_paths, live.iter().map(|m| m.file_path.as_str()));
        let mut candidates = BTreeMap::<String, String>::new();
        if let Some(implied) = implied
            && live
                .iter()
                .all(|m| unambiguous_layout(&implied, &m.file_path))
        {
            folder_identity_matches(&plan.title, &stored_path_to_path_buf(&implied), true).await?;
            candidates.insert(path_key(&implied), implied);
        }
        let mut seen = 0usize;
        for root in &roots {
            let path = stored_path_to_path_buf(&root.path);
            inspect_path(&path, false).await?;
            let mut entries = tokio::fs::read_dir(&path)
                .await
                .map_err(|e| relay_failure(e.to_string()))?;
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| relay_failure(e.to_string()))?
            {
                seen += 1;
                if seen > MAX_DISCOVERY_ENTRIES {
                    return Err(fail(
                        crate::error_reasons::location::RECOVERY_FOLDER_DISCOVERY_LIMIT_REACHED,
                        "Folder discovery limit reached; no automatic repair",
                    ));
                }
                let kind = entry
                    .file_type()
                    .await
                    .map_err(|e| relay_failure(e.to_string()))?;
                if !kind.is_dir() || kind.is_symlink() {
                    continue;
                }
                let folder = entry.path();
                let stored = path_to_stored_string(&folder);
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if name.starts_with(".scryer-")
                    || custom_recycle.as_ref().is_some_and(|recycle| {
                        folder_paths_match(recycle, &stored)
                            || stored_path_is_inside_folder(recycle, &stored)
                    })
                {
                    continue;
                }
                inspect_path(&folder, false).await?;
                if folder_matches(&plan.title, &folder).await? {
                    let folder = path_to_stored_string(&folder);
                    candidates.insert(path_key(&folder), folder);
                }
            }
        }
        if candidates.len() > 1 {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_FOLDER_IDENTITY_AMBIGUOUS,
                "Multiple folders identify this title",
            ));
        }
        let destination = if let Some(folder) = candidates.into_values().next() {
            folder
        } else {
            plan.generated_destination = true;
            if live.is_empty() {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_FOLDER_OWNERSHIP_UNPROVEN,
                    "No media or exact-ID folder establishes ownership",
                ));
            }
            let root = roots
                .iter()
                .find(|r| r.id == plan.title.root_folder_id)
                .ok_or_else(|| {
                    fail(
                        crate::error_reasons::location::RECOVERY_SELECTED_ROOT_NOT_FOUND,
                        "Selected root is missing; no default-root substitution",
                    )
                })?;
            let corrupt = plan.title.folder_path.as_deref().unwrap_or_default();
            if !path_is_strictly_within(&root.path, corrupt)
                && !folder_paths_match(&root.path, corrupt)
            {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_SELECTED_ROOT_OUTSIDE_ANCESTOR,
                    "Selected root is not below the corrupt ancestor",
                ));
            }
            for media in &live {
                let source = stored_path_to_path_buf(&media.file_path);
                let parent = source.parent().ok_or_else(|| {
                    fail(
                        crate::error_reasons::location::RECOVERY_MEDIA_PARENT_REQUIRED,
                        "Media has no parent folder",
                    )
                })?;
                if !folder_paths_match(&path_to_stored_string(parent), corrupt) {
                    return Err(fail(
                        crate::error_reasons::location::RECOVERY_MEDIA_NOT_LOOSE_AT_ANCESTOR,
                        "Media is not loose at the corrupt ancestor",
                    ));
                }
            }
            let template = self.title_folder_template_for(&plan.title.facet).await?;
            path_to_stored_string(crate::library::rename::configured_title_folder_path(
                &root.path,
                &plan.title,
                &template,
                plan.title.year,
            ))
        };
        if custom_recycle
            .as_ref()
            .is_some_and(|recycle| overlaps(&destination, recycle))
        {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_RECYCLE_STORAGE_OVERLAP,
                "Recovery destination overlaps configured recycle storage",
            ));
        }
        let root = roots
            .iter()
            .filter(|r| path_is_strictly_within(&destination, &r.path))
            .max_by_key(|r| r.path.len())
            .ok_or_else(|| {
                fail(
                    crate::error_reasons::location::RECOVERY_DESTINATION_OUTSIDE_LIBRARY,
                    "Destination is outside the title library",
                )
            })?;
        plan.destination = Some(destination.clone());
        plan.destination_root = Some(root.clone());
        if let Some(previous) = self
            .services
            .library
            .location_operations
            .get_location_operation_plan_json(&plan.operation_id())
            .await?
        {
            let previous: RecoveryPlan = serde_json::from_str(&previous)
                .map_err(|error| relay_failure(error.to_string()))?;
            if previous.generated_destination && previous.destination == plan.destination {
                plan.generated_destination = true;
            }
        }
        let destination_path = stored_path_to_path_buf(&destination);
        inspect_path(&destination_path, true).await?;
        for media in live {
            let source = stored_path_to_path_buf(&media.file_path);
            let mut target = if stored_path_is_inside_folder(&destination, &media.file_path) {
                source.clone()
            } else {
                let parent = source.parent().ok_or_else(|| {
                    fail(
                        crate::error_reasons::location::RECOVERY_SOURCE_PARENT_UNAVAILABLE,
                        "Source parent is unavailable",
                    )
                })?;
                if !folder_paths_match(
                    &path_to_stored_string(parent),
                    plan.title.folder_path.as_deref().unwrap_or_default(),
                ) {
                    return Err(fail(
                        crate::error_reasons::location::RECOVERY_SOURCE_LOCATION_UNPROVEN,
                        "Tracked source is neither in the proven folder nor loose at the corrupt ancestor",
                    ));
                }
                destination_path.join(source.file_name().ok_or_else(|| {
                    fail(
                        crate::error_reasons::location::RECOVERY_SOURCE_FILENAME_REQUIRED,
                        "Source filename is unavailable",
                    )
                })?)
            };
            let source_version = match tokio::fs::symlink_metadata(&source).await {
                Ok(meta) => {
                    inspect_path(&source, false).await?;
                    if !meta.is_file() || meta.len() != media.size_bytes.max(0) as u64 {
                        return Err(fail(
                            crate::error_reasons::location::RECOVERY_SOURCE_CHANGED,
                            "Source size or type changed",
                        ));
                    }
                    Some(recovery_file_version(&source).await?)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(relay_failure(e.to_string())),
            };
            let expected_hash = media
                .content_hashes
                .as_ref()
                .filter(|h| h.hash_computed_at.is_some())
                .map(|h| h.full_blake3.clone());
            if source_version.is_none() && expected_hash.is_none() {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_SOURCE_FULL_HASH_REQUIRED,
                    "Missing source has no attested full hash",
                ));
            }
            if source_version.is_none() {
                target = locate_hashed_destination(
                    &destination_path,
                    media.size_bytes.max(0) as u64,
                    expected_hash.as_deref().ok_or_else(|| {
                        fail(
                            crate::error_reasons::location::RECOVERY_FULL_HASH_REQUIRED,
                            "Missing full hash",
                        )
                    })?,
                    custom_recycle.as_deref(),
                )
                .await?;
            }
            plan.files.push(RecoveryFile {
                media: RecoveryMedia {
                    id: media.id.clone(),
                    title_id: media.title_id.clone(),
                    file_path: media.file_path.clone(),
                    size_bytes: media.size_bytes,
                },
                destination: path_to_stored_string(target),
                source_version,
                expected_hash,
            });
        }
        validate_catalog(plan, catalog)?;
        self.validate_generated_recovery_destination(plan).await
    }

    pub(super) async fn validate_generated_recovery_destination(
        &self,
        plan: &RecoveryPlan,
    ) -> AppResult<()> {
        let destination = plan.destination.as_deref().ok_or_else(|| {
            fail(
                crate::error_reasons::location::RECOVERY_DESTINATION_NOT_FOUND,
                "Recovery destination missing",
            )
        })?;
        if self
            .read_setting_string_value_for_scope(
                crate::SETTINGS_SCOPE_MEDIA,
                "recycle_bin.path",
                None,
            )
            .await?
            .as_ref()
            .is_some_and(|recycle| overlaps(destination, recycle))
        {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_RECYCLE_STORAGE_OVERLAP,
                "Recovery destination overlaps configured recycle storage",
            ));
        }
        if !plan.generated_destination {
            return Ok(());
        }
        let path = stored_path_to_path_buf(destination);
        match tokio::fs::symlink_metadata(&path).await {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(relay_failure(error.to_string())),
            Ok(meta) if !meta.is_dir() => {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_GENERATED_DESTINATION_NOT_DIRECTORY,
                    "Generated destination is not a directory",
                ));
            }
            Ok(_) => {}
        }
        inspect_path(&path, false).await?;
        let store = &self.services.library.location_operations;
        let previous = store
            .get_location_operation_plan_json(&plan.operation_id())
            .await?
            .ok_or_else(|| {
                fail(
                    crate::error_reasons::location::RECOVERY_GENERATED_DESTINATION_UNCLAIMED,
                    "Generated destination already exists without ownership evidence",
                )
            })?;
        let previous: RecoveryPlan =
            serde_json::from_str(&previous).map_err(|error| relay_failure(error.to_string()))?;
        if !previous.generated_destination
            || previous.operation_id() != plan.operation_id()
            || previous.destination.as_deref() != Some(destination)
        {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_GENERATED_DESTINATION_IDENTITY_MISMATCH,
                "Generated destination is not this recovery's recorded folder",
            ));
        }
        let mut allowed: BTreeSet<_> = plan
            .files
            .iter()
            .map(|file| path_key(&file.destination))
            .collect();
        allowed.extend(
            store
                .recovery_staging_paths(&plan.operation_id())
                .await?
                .into_iter()
                .map(|path| path_key(&path)),
        );
        let mut entries = tokio::fs::read_dir(&path)
            .await
            .map_err(|error| relay_failure(error.to_string()))?;
        let mut seen = 0usize;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| relay_failure(error.to_string()))?
        {
            seen += 1;
            if seen > MAX_DISCOVERY_ENTRIES {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_DESTINATION_INVENTORY_LIMIT_REACHED,
                    "Generated destination inventory limit exceeded",
                ));
            }
            let entry_path = entry.path();
            inspect_path(&entry_path, false).await?;
            if !entry
                .file_type()
                .await
                .map_err(|error| relay_failure(error.to_string()))?
                .is_file()
                || !allowed.contains(&path_key(&path_to_stored_string(&entry_path)))
            {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_DESTINATION_UNRELATED_CONTENT,
                    "Generated destination contains unrelated content; ownership remains unresolved",
                ));
            }
        }
        Ok(())
    }
}
