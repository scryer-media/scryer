//! Previewed, checkpointed resolution of duplicate movie folders.

use std::collections::{BTreeMap, BTreeSet};

use scryer_domain::{LibraryPermission, MediaFacet, Title, User};
use serde::{Deserialize, Serialize};

use super::classify::TitleLocationClass;
use super::collisions::{
    CollisionNaming, CollisionPlanRequest, DestinationItem, IncomingItem, PathCaseRule,
    plan_collisions,
};
use super::execution::SourceRecycler;
use super::model::{LocationExecutionMode, LocationOperation, LocationOperationType};
use super::operations::{LocationJobRunActor, LocationOperationAdmission};
use super::preview::{
    LocationPlan, LocationPlanBuilder, LocationPlanHeader, PlanItem, PlanItemKind,
};
use super::root_move::{RootMoveExecutionPlan, RootMoveFileExecution, RootMoveTitleExecution};
use crate::stored_paths::{folder_paths_match, path_to_stored_string, stored_path_to_path_buf};
use crate::{
    AppError, AppResult, AppUseCase, InsertMediaFileInput, MediaFileRole, PendingImportStatus,
    TitleMediaFile,
};

/// Stable refusal reasons for ownership-conflict resolution.
pub mod refusal {
    /// Selection is empty, repeated, or exceeds the bound.
    pub const SELECTION_INVALID: &str = "LOCATION_CONFLICT_SELECTION_INVALID";
    /// The pending conflict no longer exists.
    pub const NOT_FOUND: &str = "LOCATION_CONFLICT_NOT_FOUND";
    /// The selected item is not a pending movie ownership conflict.
    pub const UNSUPPORTED: &str = "LOCATION_CONFLICT_UNSUPPORTED";
    /// A source file or folder no longer exists.
    pub const PATH_MISSING: &str = "LOCATION_CONFLICT_PATH_MISSING";
    /// A directory is required at a source path.
    pub const PATH_NOT_DIRECTORY: &str = "LOCATION_CONFLICT_PATH_NOT_DIRECTORY";
    /// Filesystem access to a source was denied.
    pub const PATH_PERMISSION_DENIED: &str = "LOCATION_CONFLICT_PATH_PERMISSION_DENIED";
    /// A filesystem operation failed for another reason.
    pub const PATH_IO_FAILED: &str = "LOCATION_CONFLICT_PATH_IO_FAILED";
    /// Planning failed without a more specific refusal.
    pub const PLAN_FAILED: &str = "LOCATION_CONFLICT_PLAN_FAILED";
    /// A folder escapes its root, overlaps another folder, or contains links.
    pub const FOLDER_UNSAFE: &str = "LOCATION_CONFLICT_FOLDER_UNSAFE";
    /// A title or media ownership claim changed.
    pub const OWNERSHIP_CHANGED: &str = "LOCATION_CONFLICT_OWNERSHIP_CHANGED";
    /// No movie file was found in the conflict folders.
    pub const MEDIA_MISSING: &str = "LOCATION_CONFLICT_MEDIA_MISSING";
    /// The current preview differs from the confirmed preview.
    pub const PLAN_CHANGED: &str = "LOCATION_CONFLICT_PLAN_CHANGED";
    /// Permanent disposal needs an additional explicit confirmation.
    pub const PERMANENT_CONFIRMATION_REQUIRED: &str =
        "LOCATION_CONFLICT_PERMANENT_CONFIRMATION_REQUIRED";
    /// A previewed source or destination changed before disposal.
    pub const FILE_CHANGED: &str = "LOCATION_CONFLICT_FILE_CHANGED";
    /// A configured recycle bin cannot safely accept a file.
    pub const RECYCLE_UNAVAILABLE: &str = "LOCATION_CONFLICT_RECYCLE_UNAVAILABLE";
    /// An untracked media file in a conflict folder names a different work, so
    /// the folder is a collection rather than this title's duplicate.
    pub const FOLDER_HOLDS_OTHER_WORKS: &str = "LOCATION_CONFLICT_FOLDER_HOLDS_OTHER_WORKS";
    /// The title's media is split across parts, so keeping one file would
    /// dispose of the others.
    pub const MULTI_PART_MEDIA: &str = "LOCATION_CONFLICT_MULTI_PART_MEDIA";
    pub const ALL: &[&str] = &[
        SELECTION_INVALID,
        NOT_FOUND,
        UNSUPPORTED,
        PATH_MISSING,
        PATH_NOT_DIRECTORY,
        PATH_PERMISSION_DENIED,
        PATH_IO_FAILED,
        PLAN_FAILED,
        FOLDER_UNSAFE,
        OWNERSHIP_CHANGED,
        MEDIA_MISSING,
        PLAN_CHANGED,
        PERMANENT_CONFIRMATION_REQUIRED,
        FILE_CHANGED,
        RECYCLE_UNAVAILABLE,
        FOLDER_HOLDS_OTHER_WORKS,
        MULTI_PART_MEDIA,
    ];
}

/// Existing scanner reason identifying a duplicate folder bound to a title.
pub const PENDING_REASON: &str =
    crate::library_scan_unmatched::LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER;

const MAX_SELECTION: usize = 1_000;
const MAX_FILES: usize = 100_000;
const SAMPLE: usize = 100;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    AdoptAll,
    KeepOne,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictLeftovers {
    PreserveAll,
    MediaOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictRequest {
    pub pending_import_ids: Vec<String>,
    pub policy: ConflictPolicy,
    pub leftovers: ConflictLeftovers,
}

pub struct ResolveConflictRequest {
    pub request: ConflictRequest,
    pub plan_fingerprint: String,
    pub confirm_permanent_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictRename {
    pub source_path: String,
    pub destination_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictRow {
    pub pending_import_id: String,
    pub title_id: String,
    pub title_name: String,
    pub survivor_path: String,
    pub primary_path: String,
    pub move_bytes: u64,
    pub recycle_bytes: u64,
    pub recycle_count: i32,
    pub collision_renames: i32,
    pub collision_rename_samples: Vec<ConflictRename>,
    pub leftover_count: i32,
    pub leftover_bytes: u64,
    pub leftover_paths: Vec<String>,
    pub recycle_paths: Vec<String>,
    pub files_sampled: bool,
    pub error_reason: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictPreview {
    pub plan_fingerprint: String,
    pub total_count: i32,
    pub actionable_count: i32,
    pub failed_count: i32,
    pub sampled: bool,
    pub rows: Vec<ConflictRow>,
    pub permanent_delete_count: i32,
    pub permanent_delete_bytes: u64,
    pub permanent_delete_paths: Vec<String>,
    pub permanent_delete_sampled: bool,
    pub requires_permanent_delete_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictTask {
    pub job_run_id: String,
    pub total_count: i32,
    pub settled_count: i32,
    pub failed_count: i32,
    pub operation_ids: Vec<String>,
    pub completed: bool,
}

/// Every file is enumerated at confirmation. Recovery never discovers disposal targets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictFile {
    pub path: String,
    pub version: String,
    pub size_bytes: u64,
    pub media: bool,
    pub media_file_id: Option<String>,
    pub root_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictExecution {
    pub roots: Vec<scryer_domain::LibraryRoot>,
    pub pending_import_ids: Vec<String>,
    pub policy: ConflictPolicy,
    pub leftovers: ConflictLeftovers,
    pub folders: Vec<String>,
    pub files: Vec<ConflictFile>,
    pub dispose_paths: Vec<String>,
    pub primary_source: String,
    /// Explicit confirmation persists with the allowlist, never inferred from bin failure.
    pub permanent_delete_confirmed: bool,
    pub permanent_delete_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hardlink_proofs: Vec<ConflictHardlinkProof>,
}

/// Preview content proof for an inode whose selected pathnames may be disposed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictHardlinkProof {
    pub version: String,
    pub full_blake3: String,
    pub link_count: u64,
}

/// Durable intent and the resulting exact versions of this operation's aliases.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisposalProof {
    pub sequence: usize,
    pub before_link_count: u64,
    pub after_link_count: Option<u64>,
    pub after_versions: BTreeMap<String, String>,
    pub recycle: Option<crate::recycle_bin::PreparedRecycleFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlannedConflict {
    selected_count: i32,
    row: ConflictRow,
    plan: LocationPlan,
    execution: RootMoveExecutionPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConflictTaskJournal {
    task: ConflictTask,
    actor_id: String,
    plans: Vec<PlannedConflict>,
    next: usize,
    current_operation_id: Option<String>,
    library_ids: Vec<String>,
}

fn refused(reason: &'static str, message: impl Into<String>) -> AppError {
    AppError::validation_refused(reason, message)
}

fn conflict_io_error(error: std::io::Error) -> AppError {
    let (reason, message) = match error.kind() {
        std::io::ErrorKind::NotFound => (
            refusal::PATH_MISSING,
            "A conflict file or folder no longer exists",
        ),
        std::io::ErrorKind::NotADirectory => (
            refusal::PATH_NOT_DIRECTORY,
            "A conflict folder path is not a directory",
        ),
        std::io::ErrorKind::PermissionDenied => (
            refusal::PATH_PERMISSION_DENIED,
            "Scryer cannot access a conflict file or folder",
        ),
        _ => (
            refusal::PATH_IO_FAILED,
            "A conflict file or folder could not be read",
        ),
    };
    refused(reason, message)
}

async fn conflict_path_metadata(path: &str) -> AppResult<std::fs::Metadata> {
    if !crate::stored_paths::stored_path_is_native(path) {
        return Err(refused(
            refusal::FOLDER_UNSAFE,
            "The conflict path uses another platform's encoding",
        ));
    }
    let path = stored_path_to_path_buf(path);
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(conflict_io_error)?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return Err(refused(
            refusal::FOLDER_UNSAFE,
            "Only regular files and directories may be consolidated",
        ));
    }
    super::title_folder_recovery::inspect_path(&path, false).await?;
    Ok(metadata)
}

async fn pending_conflict_folder(path: &str) -> AppResult<String> {
    if conflict_path_metadata(path).await?.is_dir() {
        return Ok(path.to_owned());
    }
    let path = stored_path_to_path_buf(path);
    let parent = path.parent().ok_or_else(|| {
        refused(
            refusal::FOLDER_UNSAFE,
            "The conflict file has no parent folder",
        )
    })?;
    let folder = path_to_stored_string(parent);
    if !conflict_path_metadata(&folder).await?.is_dir() {
        return Err(refused(
            refusal::PATH_NOT_DIRECTORY,
            "The conflict file's parent is not a directory",
        ));
    }
    Ok(folder)
}

/// Execution-time check of one confirmed conflict folder against the current
/// catalog: roots may have been added and owners may have changed since the
/// preview was confirmed.
fn revalidate_conflict_folder(
    folder: &str,
    title: &Title,
    catalog: &super::title_folder_recovery::RecoveryCatalog,
) -> AppResult<()> {
    if folder_spans_any_root(folder, &catalog.roots) {
        return Err(refused(
            refusal::FOLDER_UNSAFE,
            "A conflict folder is or contains a library root",
        ));
    }
    if !catalog.roots.iter().any(|root| {
        root.library_id == title.library_id
            && crate::title_folder_rules::path_is_strictly_within(folder, &root.path)
    }) || another_title_holds_content(folder, &title.id, catalog)?
    {
        return Err(refused(
            refusal::OWNERSHIP_CHANGED,
            "Conflict folder ownership or roots changed",
        ));
    }
    Ok(())
}

/// Whether another title owns a folder overlapping `folder`, or tracks media
/// inside it. Owner folders compare the way ownership claims do, so a re-cased
/// spelling on a case-insensitive volume still counts as the same folder.
fn another_title_holds_content(
    folder: &str,
    title_id: &str,
    catalog: &super::title_folder_recovery::RecoveryCatalog,
) -> AppResult<bool> {
    for owner in &catalog.owners {
        if owner.id != title_id
            && let Some(owned) = owner.folder_path.as_deref()
            && other_folder_overlaps(folder, owned)?
        {
            return Ok(true);
        }
    }
    Ok(catalog
        .media
        .iter()
        .any(|media| media.title_id != title_id && other_media_within(&media.file_path, folder)))
}

/// [`overlaps`], widened to a case-folded match on a case-insensitive volume.
/// The case-folded spelling is only a cheap filter; the claim comparison
/// decides.
fn other_folder_overlaps(folder: &str, owned: &str) -> AppResult<bool> {
    if overlaps(folder, owned) {
        return Ok(true);
    }
    if !overlaps(&folder.to_lowercase(), &owned.to_lowercase()) {
        return Ok(false);
    }
    crate::folder_ownership::title_folders_overlap(folder, owned)
}

fn other_media_within(media: &str, folder: &str) -> bool {
    crate::title_folder_rules::path_is_strictly_within(media, folder)
        || (crate::title_folder_rules::path_is_strictly_within(
            &media.to_lowercase(),
            &folder.to_lowercase(),
        ) && crate::folder_ownership::folder_case_insensitive(&stored_path_to_path_buf(folder)))
}

/// The most specific root holding `folder`: with nested roots the inner one
/// owns it, the way title folder recovery resolves them.
fn deepest_containing_root<'a>(
    roots: &'a [scryer_domain::LibraryRoot],
    folder: &str,
) -> Option<&'a scryer_domain::LibraryRoot> {
    roots
        .iter()
        .filter(|root| {
            crate::stored_paths::stored_path_is_native(&root.path)
                && crate::title_folder_rules::path_is_strictly_within(folder, &root.path)
        })
        .max_by_key(|root| root.path.len())
}

/// Whether `folder` is, or contains, any configured root of any library.
fn folder_spans_any_root(folder: &str, roots: &[scryer_domain::LibraryRoot]) -> bool {
    let all = roots
        .iter()
        .map(|root| root.path.as_str())
        .collect::<Vec<_>>();
    matches!(
        crate::title_folder_rules::title_folder_root_violation(folder, &all, &all),
        Some(
            crate::title_folder_rules::TitleFolderRootViolation::EqualsRoot
                | crate::title_folder_rules::TitleFolderRootViolation::ContainsRoot
        )
    )
}

fn overlaps(a: &str, b: &str) -> bool {
    folder_paths_match(a, b)
        || crate::title_folder_rules::path_is_strictly_within(a, b)
        || crate::title_folder_rules::path_is_strictly_within(b, a)
}

/// Reuse the release parser's title and metadata identities; explicit disagreement loses.
///
/// Names compare in the catalog's lookup form (the same key the library scan
/// resolves folders with), so a trailing article or `&` for `and` still names
/// this title. A year one off still matches, since release and catalog years
/// routinely disagree by one around a festival or regional premiere.
fn folder_name_rank(title: &Title, folder: &str) -> i8 {
    let path = stored_path_to_path_buf(folder);
    name_rank(
        title,
        &path.file_name().unwrap_or_default().to_string_lossy(),
    )
}

/// [`folder_name_rank`] for a media file, judged by its own stem.
fn file_name_rank(title: &Title, file: &str) -> i8 {
    let path = stored_path_to_path_buf(file);
    name_rank(
        title,
        &path.file_stem().unwrap_or_default().to_string_lossy(),
    )
}

/// A stem's part number when it carries a stacking marker such as `cd1`,
/// `pt2`, `part 1` or `disc-2`.
fn stacked_part_number(stem: &str) -> Option<u32> {
    const MARKERS: &[&str] = &["cd", "pt", "part", "disc", "disk", "dvd"];
    let lower = stem.to_ascii_lowercase();
    let tokens = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let number = |digits: &str| {
        ((1..=2).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| digits.parse().ok())
            .flatten()
    };
    for (index, token) in tokens.iter().enumerate() {
        for marker in MARKERS {
            if let Some(rest) = token.strip_prefix(marker) {
                if rest.is_empty() {
                    if let Some(found) = tokens.get(index + 1).and_then(|next| number(next)) {
                        return Some(found);
                    }
                } else if let Some(found) = number(rest) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Whether the media is split across parts: a part folder holding video, or
/// media stems carrying two different part numbers. One stem saying `Part 2`
/// is a title, not a stack.
fn holds_multi_part_media(files: &[ConflictFile], folders: &[String]) -> bool {
    let media = files.iter().filter(|file| file.media).collect::<Vec<_>>();
    let in_part_folder = files.iter().any(|file| {
        let path = stored_path_to_path_buf(&file.path);
        // A part the feature filter left out is still part of the title.
        if !file.media && !scryer_domain::is_standalone_video_file(&path) {
            return false;
        }
        let Some(folder) = folders
            .iter()
            .map(|folder| stored_path_to_path_buf(folder))
            .find(|folder| path.strip_prefix(folder).is_ok())
        else {
            return false;
        };
        path.parent()
            .into_iter()
            .flat_map(std::path::Path::ancestors)
            .take_while(|ancestor| *ancestor != folder.as_path())
            .filter_map(|ancestor| ancestor.file_name()?.to_str())
            .any(crate::library_discovery::is_title_part_folder_name)
    });
    let parts = media
        .iter()
        .filter_map(|file| {
            stacked_part_number(
                &stored_path_to_path_buf(&file.path)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy(),
            )
        })
        .collect::<BTreeSet<_>>();
    in_part_folder || parts.len() > 1
}

fn name_rank(title: &Title, raw: &str) -> i8 {
    let parsed = crate::release_parser::parse_release_metadata(raw);
    let mut identity_match = false;
    let mut carries_id = false;
    for (source, value) in [
        ("tmdb", parsed.tmdb_id.as_deref()),
        ("imdb", parsed.imdb_id.as_deref()),
        ("tvdb", parsed.tvdb_id.as_deref()),
    ] {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            continue;
        };
        carries_id = true;
        // An id the title does not carry is no evidence either way.
        let Some(expected) = folder_rank_external_id(title, source) else {
            continue;
        };
        if !expected.eq_ignore_ascii_case(value) {
            return -1;
        }
        identity_match = true;
    }
    if identity_match {
        return 1;
    }
    // Spaces are dropped from the key because separators are not identity:
    // "10,000 BC" and "10000.BC" are one name, but the comma splits the first
    // into two words and the second stays one.
    let key = |name: &str| {
        crate::title_matching::canonical_lookup_key(&name.replace('&', " and "))
            .split_whitespace()
            .collect::<String>()
    };
    let candidates = std::iter::once(parsed.normalized_title.as_str())
        .chain(parsed.normalized_title_variants.iter().map(String::as_str))
        .map(key)
        .filter(|candidate| !candidate.is_empty())
        .collect::<Vec<_>>();
    let name_matches = std::iter::once(title.name.as_str())
        .chain(title.aliases.iter().map(String::as_str))
        .map(key)
        .filter(|expected| !expected.is_empty())
        .any(|expected| candidates.contains(&expected));
    let year_matches = parsed.year.is_none_or(|year| {
        title
            .year
            .is_none_or(|expected| (year - expected).abs() <= 1)
    });
    if name_matches && year_matches {
        return 1;
    }
    // A recognizable release, year or id tag identifies a different work, never junk.
    if (carries_id || parsed.year.is_some() || parsed.quality.is_some() || parsed.source.is_some())
        && !candidates.is_empty()
    {
        -1
    } else {
        0
    }
}

/// The title's id for `source`, trimmed, the way the library scan reads it.
fn folder_rank_external_id<'a>(title: &'a Title, source: &str) -> Option<&'a str> {
    if source == "imdb"
        && let Some(imdb_id) = title.imdb_id.as_deref().map(str::trim)
        && !imdb_id.is_empty()
    {
        return Some(imdb_id);
    }
    title
        .external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case(source) && !id.value.trim().is_empty())
        .map(|id| id.value.trim())
}

async fn enumerate(folder: &str, root: &str) -> AppResult<(Vec<ConflictFile>, Vec<String>)> {
    let base = stored_path_to_path_buf(folder);
    super::title_folder_recovery::inspect_path(&base, false).await?;
    let mut pending = vec![base.clone()];
    let mut files = Vec::new();
    let mut directories = Vec::new();
    while let Some(path) = pending.pop() {
        let mut entries = tokio::fs::read_dir(&path)
            .await
            .map_err(conflict_io_error)?;
        directories.push(path_to_stored_string(&path));
        while let Some(entry) = entries.next_entry().await.map_err(conflict_io_error)? {
            if files.len() + directories.len() + pending.len() >= MAX_FILES {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "The folder exceeds the safe preview bound",
                ));
            }
            let path = entry.path();
            let metadata = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(conflict_io_error)?;
            super::title_folder_recovery::inspect_path(&path, false).await?;
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                files.push(ConflictFile {
                    path: path_to_stored_string(&path),
                    version: super::resolution::file_version(&path).await?,
                    size_bytes: metadata.len(),
                    media: crate::library_discovery::is_movie_feature_media_candidate(&base, &path),
                    media_file_id: None,
                    root_path: root.to_owned(),
                });
            } else {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "Only regular files and directories may be consolidated",
                ));
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    directories.sort_by_key(|path| std::cmp::Reverse(path.len()));
    Ok((files, directories))
}

impl AppUseCase {
    async fn plan_pending_import_conflict(
        &self,
        actor: &User,
        id: &str,
        request: &ConflictRequest,
        catalog: &super::title_folder_recovery::RecoveryCatalog,
        authorized_title: &mut Option<(String, String)>,
    ) -> AppResult<PlannedConflict> {
        let repo = &self.services.library.library_scan_unmatched_items;
        let item = repo
            .get_library_scan_unmatched_item(id)
            .await?
            .ok_or_else(|| refused(refusal::NOT_FOUND, "The pending import no longer exists"))?;
        self.require_library_permission(actor, &item.library_id, LibraryPermission::ResolveImports)
            .await?;
        self.require_library_permission(actor, &item.library_id, LibraryPermission::ManageTitles)
            .await?;
        if item.facet != MediaFacet::Movie
            || item.status != PendingImportStatus::Pending
            || item.reason_code
                != crate::library_scan_unmatched::LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER
        {
            return Err(refused(
                refusal::UNSUPPORTED,
                "Select a pending movie folder ownership conflict",
            ));
        }
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(
                item.title_id
                    .as_deref()
                    .ok_or_else(|| refused(refusal::UNSUPPORTED, "This conflict has no title"))?,
            )
            .await?
            .ok_or_else(|| refused(refusal::NOT_FOUND, "The title no longer exists"))?;
        if title.library_id != item.library_id {
            return Err(refused(
                refusal::OWNERSHIP_CHANGED,
                "The title changed libraries",
            ));
        }
        // Only retain identity after authorizing its library, including on failure.
        *authorized_title = Some((title.id.clone(), title.name.clone()));
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(&item.library_id)
            .await?
            .ok_or_else(|| refused(refusal::NOT_FOUND, "The library no longer exists"))?;
        let mut folders = vec![
            title.folder_path.clone().ok_or_else(|| {
                refused(
                    refusal::OWNERSHIP_CHANGED,
                    "The title no longer owns a folder",
                )
            })?,
            pending_conflict_folder(&item.item_path).await?,
        ];
        let siblings = repo
            .list_pending_folder_conflicts_for_title(&item.library_id, &title.id)
            .await?;
        if siblings.len() > MAX_SELECTION {
            return Err(refused(
                refusal::SELECTION_INVALID,
                "This title has too many duplicate folders for one operation",
            ));
        }
        let mut pending_ids = vec![id.to_owned()];
        for sibling in siblings {
            if sibling.library_id == item.library_id
                && sibling.title_id.as_deref() == Some(&title.id)
                && sibling.reason_code == item.reason_code
            {
                folders.push(pending_conflict_folder(&sibling.item_path).await?);
                pending_ids.push(sibling.id);
            }
        }
        folders.sort();
        folders.dedup();
        pending_ids.sort();
        pending_ids.dedup();
        if folders.len() < 2 {
            return Err(refused(
                refusal::OWNERSHIP_CHANGED,
                "The conflict folders now coincide",
            ));
        }
        let mut files = Vec::new();
        let mut directories = Vec::new();
        // Distinct spellings can name one directory on a case-insensitive
        // volume. Planning them as two folders would let keep-one dispose the
        // only copy, so physical identity, not spelling, decides.
        let mut folder_identities = BTreeMap::new();
        for (index, folder) in folders.iter().enumerate() {
            if !conflict_path_metadata(folder).await?.is_dir() {
                return Err(refused(
                    refusal::PATH_NOT_DIRECTORY,
                    "A conflict folder path is not a directory",
                ));
            }
            if let Some(identity) = directory_identity(&stored_path_to_path_buf(folder)).await?
                && folder_identities.insert(identity, index).is_some()
            {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "Two conflict folders are the same directory under different spellings",
                ));
            }
        }
        let mut seen_inodes: BTreeMap<String, usize> = BTreeMap::new();
        for (index, folder) in folders.iter().enumerate() {
            if !crate::stored_paths::stored_path_is_native(folder) {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "The folder path uses another platform's native encoding; its files were preserved",
                ));
            }
            let root = deepest_containing_root(&library.roots, folder).ok_or_else(|| {
                refused(
                    refusal::FOLDER_UNSAFE,
                    "Every conflict folder must be strictly within this library's roots",
                )
            })?;
            // A root added to any library after the pending row was written
            // can make a conflict folder a whole root or hold one; consolidating
            // it would treat that root's titles as this title's files.
            if folder_spans_any_root(folder, &catalog.roots) {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "A conflict folder is or contains a library root",
                ));
            }
            if folders
                .iter()
                .any(|other| folder != other && overlaps(folder, other))
            {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "Conflict folders must not overlap",
                ));
            }
            if another_title_holds_content(folder, &title.id, catalog)? {
                return Err(refused(
                    refusal::OWNERSHIP_CHANGED,
                    "Another title owns content in a conflict folder",
                ));
            }
            let (mut entries, mut dirs) = enumerate(folder, &root.path).await?;
            for dir in &dirs {
                if let Some(identity) = directory_identity(&stored_path_to_path_buf(dir)).await?
                    && folder_identities
                        .get(&identity)
                        .is_some_and(|other| *other != index)
                {
                    return Err(refused(
                        refusal::FOLDER_UNSAFE,
                        "A conflict folder is reachable inside another under a different spelling",
                    ));
                }
            }
            for entry in &entries {
                let Some(inode) = inode_key(&entry.version) else {
                    continue;
                };
                match seen_inodes.get(&inode) {
                    Some(other)
                        if *other != index
                            && super::hardlinks::link_count(&stored_path_to_path_buf(
                                &entry.path,
                            ))?
                            .count()
                                == Some(1) =>
                    {
                        return Err(refused(
                            refusal::FOLDER_UNSAFE,
                            "One file is reachable from two conflict folders; its folders may be one directory",
                        ));
                    }
                    Some(_) => {}
                    None => {
                        seen_inodes.insert(inode, index);
                    }
                }
            }
            files.append(&mut entries);
            directories.append(&mut dirs);
        }
        let tracked = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await?;
        if tracked.iter().any(|media| {
            !files
                .iter()
                .any(|file| folder_paths_match(&file.path, &media.file_path))
        }) {
            return Err(refused(
                refusal::OWNERSHIP_CHANGED,
                "A tracked file is outside the conflict folders or unavailable",
            ));
        }
        for file in &mut files {
            file.media_file_id = tracked
                .iter()
                .find(|media| folder_paths_match(&media.file_path, &file.path))
                .map(|media| media.id.clone());
            // A tracked row is this title's media wherever it sits and whatever
            // its stem says; the feature filter only classifies untracked files.
            if file.media_file_id.is_some() {
                file.media = true;
            }
        }
        // An untracked film whose own name identifies another work means a
        // folder is a collection, not this title's duplicate: adopting it
        // would claim that film, and keep-one would dispose of it.
        if files.iter().any(|file| {
            file.media && file.media_file_id.is_none() && file_name_rank(&title, &file.path) < 0
        }) {
            return Err(refused(
                refusal::FOLDER_HOLDS_OTHER_WORKS,
                "A conflict folder holds media that names a different work; its files were preserved",
            ));
        }
        if request.policy == ConflictPolicy::KeepOne && holds_multi_part_media(&files, &folders) {
            return Err(refused(
                refusal::MULTI_PART_MEDIA,
                "This title's media is split into parts, so keeping one file would dispose of the others; its files were preserved",
            ));
        }
        let profile = self
            .resolve_quality_profile(crate::catalog::release_search::QualityProfileLookup {
                title_tags: &title.tags,
                library_id: Some(&title.library_id),
                imdb_id: title.imdb_id.as_deref(),
                tvdb_id: None,
                category_hint: Some("movie"),
            })
            .await?;
        let context = self
            .resolve_canonical_scoring_context(&title, &profile)
            .await;
        let mut ranked = files
            .iter()
            .filter(|file| file.media)
            .map(|file| {
                let candidate = tracked
                    .iter()
                    .find(|media| media.file_path == file.path)
                    .cloned()
                    .unwrap_or_else(|| candidate_file(&title.id, file));
                let bar = self.incumbent_bar(
                    &candidate,
                    &context,
                    crate::quality_profile::CoverageSizeBasis::default(),
                );
                (
                    crate::admission::tier_sort_key(bar.tier_index),
                    std::cmp::Reverse(bar.score),
                    std::cmp::Reverse(file.size_bytes),
                    file.path.clone(),
                )
            })
            .collect::<Vec<_>>();
        ranked.sort();
        let primary = ranked
            .first()
            .ok_or_else(|| refused(refusal::MEDIA_MISSING, "No movie files were found"))?
            .3
            .clone();
        let primary_folder = folders
            .iter()
            .find(|folder| crate::title_folder_rules::path_is_strictly_within(&primary, folder))
            .expect("enumerated file has a folder");
        let mut eligible = folders
            .iter()
            .filter(|folder| folder_name_rank(&title, folder) >= 0)
            .collect::<Vec<_>>();
        eligible.sort_by_key(|folder| {
            (
                std::cmp::Reverse(folder_name_rank(&title, folder)),
                *folder != primary_folder,
                (*folder).clone(),
            )
        });
        let survivor = (*eligible.first().ok_or_else(|| {
            refused(
                refusal::FOLDER_UNSAFE,
                "Every folder name identifies a different title",
            )
        })?)
        .clone();
        let destination_root =
            deepest_containing_root(&library.roots, &survivor).expect("validated folder root");
        let source_root = library
            .roots
            .iter()
            .find(|root| root.id == title.root_folder_id)
            .ok_or_else(|| refused(refusal::OWNERSHIP_CHANGED, "The source root changed"))?;
        let mut dispose = Vec::new();
        let mut moving = Vec::new();
        let mut leftovers = Vec::new();
        for file in &files {
            let in_survivor =
                crate::title_folder_rules::path_is_strictly_within(&file.path, &survivor);
            // Keep-one retains the best media across all folders, including when it must move.
            let dispose_media =
                file.media && request.policy == ConflictPolicy::KeepOne && file.path != primary;
            let attached_to_kept_media = !file.media
                && files.iter().any(|media| {
                    media.media
                        && (request.policy == ConflictPolicy::AdoptAll || media.path == primary)
                        && super::resolution::companion_suffix(
                            &stored_path_to_path_buf(&file.path),
                            &stored_path_to_path_buf(&media.path),
                        )
                        .is_some()
                });
            let is_leftover = !file.media && !in_survivor && !attached_to_kept_media;
            let dispose_leftover = is_leftover && request.leftovers == ConflictLeftovers::MediaOnly;
            if is_leftover {
                leftovers.push(file);
            }
            if dispose_media || dispose_leftover {
                dispose.push(file.path.clone());
            } else if !in_survivor {
                moving.push(file);
            }
        }
        let destination = files
            .iter()
            .filter(|file| {
                crate::title_folder_rules::path_is_strictly_within(&file.path, &survivor)
            })
            .map(|file| {
                DestinationItem::companion(relative_name(&survivor, &file.path), file.size_bytes)
                    .with_path(file.path.clone())
            })
            .collect();
        let incoming = moving
            .iter()
            .map(|file| {
                let folder = folders
                    .iter()
                    .find(|folder| {
                        crate::title_folder_rules::path_is_strictly_within(&file.path, folder)
                    })
                    .expect("enumerated folder");
                let name = relative_name(folder, &file.path);
                let mut item = if file.media {
                    IncomingItem::media(&file.path, name, file.size_bytes)
                } else {
                    IncomingItem::companion(&file.path, name, file.size_bytes)
                }
                .with_source_path(&file.path);
                if !file.media
                    && let Some(media) = moving
                        .iter()
                        .filter(|media| {
                            media.media
                                && super::resolution::companion_suffix(
                                    &stored_path_to_path_buf(&file.path),
                                    &stored_path_to_path_buf(&media.path),
                                )
                                .is_some()
                        })
                        .max_by_key(|media| media.path.len())
                {
                    item = item.with_companion_of(&media.path);
                }
                item
            })
            .collect();
        let collision = plan_collisions(
            &CollisionPlanRequest::new(
                PathCaseRule::platform_default(),
                CollisionNaming::from_source_library("duplicate"),
            )
            .with_stored_path_names()
            .with_destination(destination)
            .with_incoming(incoming),
        );
        let execution_files = moving
            .iter()
            .map(|file| {
                Ok(RootMoveFileExecution {
                    media_file_id: file.media_file_id.clone().or_else(|| {
                        file.media.then(|| {
                            format!(
                                "conflict-media:{}",
                                blake3::hash(file.path.as_bytes()).to_hex()
                            )
                        })
                    }),
                    source_path: file.path.clone(),
                    destination_path: join_relative_name(
                        &survivor,
                        &collision
                            .decision(&file.path)
                            .expect("planned incoming file")
                            .final_name,
                    )?,
                    size_bytes: file.size_bytes,
                    source_content: tracked
                        .iter()
                        .find(|media| media.file_path == file.path)
                        .and_then(super::model::KnownSourceContent::from_media_file),
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        let mut permanent_delete_paths = Vec::new();
        let mut hardlink_proofs: Vec<ConflictHardlinkProof> = Vec::new();
        for path in &dispose {
            let file = files
                .iter()
                .find(|file| &file.path == path)
                .expect("allowlisted file");
            if !self
                .recycle_bin_config_for_media_root(Some(&file.root_path))
                .await
                .enabled
            {
                permanent_delete_paths.push(path.clone());
            }
        }
        // Only an inode a disposal could unlink needs a content proof. Every
        // pathname of that inode shares the proof, so hashing the disposal
        // targets themselves covers the files linked to them.
        if !dispose.is_empty() {
            for file in files.iter().filter(|file| dispose.contains(&file.path)) {
                if let Some(link_count) =
                    super::hardlinks::link_count(&stored_path_to_path_buf(&file.path))?.count()
                    && link_count > 1
                    && !hardlink_proofs
                        .iter()
                        .any(|proof| same_inode_version(&proof.version, &file.version))
                {
                    hardlink_proofs.push(ConflictHardlinkProof {
                        version: file.version.clone(),
                        full_blake3: conflict_hash(&file.path, &file.version).await?,
                        link_count,
                    });
                }
            }
        }
        let conflict = ConflictExecution {
            roots: library.roots.clone(),
            pending_import_ids: pending_ids,
            policy: request.policy,
            leftovers: request.leftovers,
            folders: folders.clone(),
            files: files.clone(),
            dispose_paths: dispose.clone(),
            primary_source: primary.clone(),
            permanent_delete_confirmed: false,
            permanent_delete_paths,
            hardlink_proofs,
        };
        let operation_type = match request.policy {
            ConflictPolicy::AdoptAll => LocationOperationType::FolderConsolidation,
            ConflictPolicy::KeepOne => LocationOperationType::FolderKeepOne,
        };
        let mut builder = LocationPlanBuilder::new(
            LocationPlanHeader::new(operation_type, LocationExecutionMode::MoveWithScryer)
                .with_source(Some(title.library_id.clone()), Some(source_root.id.clone()))
                .with_destination(
                    Some(title.library_id.clone()),
                    Some(destination_root.id.clone()),
                )
                .with_selection(vec![title.id.clone()]),
        );
        builder.verification_depth(self.resolve_verification_depth().await);
        for file in &execution_files {
            builder.push(
                PlanItem::new(PlanItemKind::Move)
                    .with_title(&title.id)
                    .with_paths(
                        Some(file.source_path.clone()),
                        Some(file.destination_path.clone()),
                    )
                    .with_size(file.size_bytes),
            );
        }
        // The complete file versions and disposal allowlist participate in confirmation.
        builder.push(
            PlanItem::new(PlanItemKind::CatalogChange)
                .with_title(&title.id)
                .with_detail(
                    serde_json::to_string(&conflict)
                        .map_err(|e| AppError::Repository(e.to_string()))?,
                ),
        );
        let plan = builder.build();
        let renamed_destinations = execution_files
            .iter()
            .filter(|file| {
                collision
                    .decision(&file.source_path)
                    .is_some_and(|decision| decision.renamed())
            })
            .map(|file| file.destination_path.clone())
            .collect::<Vec<_>>();
        let primary_path = execution_files
            .iter()
            .find(|file| file.source_path == primary)
            .map(|file| file.destination_path.clone())
            .unwrap_or(primary);
        let collision_rename_samples = execution_files
            .iter()
            .filter(|file| renamed_destinations.contains(&file.destination_path))
            .take(SAMPLE)
            .map(|file| ConflictRename {
                source_path: file.source_path.clone(),
                destination_path: file.destination_path.clone(),
            })
            .collect();
        let row = ConflictRow {
            pending_import_id: id.to_owned(),
            title_id: title.id.clone(),
            title_name: title.name.clone(),
            survivor_path: survivor.clone(),
            primary_path,
            move_bytes: execution_files.iter().map(|file| file.size_bytes).sum(),
            recycle_bytes: files
                .iter()
                .filter(|file| dispose.contains(&file.path))
                .map(|file| file.size_bytes)
                .sum(),
            recycle_count: dispose.len() as i32,
            collision_renames: renamed_destinations.len() as i32,
            collision_rename_samples,
            leftover_count: leftovers.len() as i32,
            leftover_bytes: leftovers.iter().map(|file| file.size_bytes).sum(),
            leftover_paths: leftovers
                .iter()
                .take(SAMPLE)
                .map(|file| file.path.clone())
                .collect(),
            recycle_paths: dispose.iter().take(SAMPLE).cloned().collect(),
            files_sampled: leftovers.len() > SAMPLE
                || dispose.len() > SAMPLE
                || renamed_destinations.len() > SAMPLE,
            error_reason: None,
            error_message: None,
        };
        let keep_catalog = tracked
            .iter()
            .filter(|media| {
                !execution_files
                    .iter()
                    .any(|file| file.media_file_id.as_deref() == Some(&media.id))
            })
            .map(|media| RootMoveFileExecution {
                media_file_id: Some(media.id.clone()),
                source_path: media.file_path.clone(),
                destination_path: media.file_path.clone(),
                size_bytes: media.size_bytes.max(0) as u64,
                source_content: None,
            })
            .collect();
        directories.retain(|path| {
            !folder_paths_match(path, &survivor)
                && !crate::title_folder_rules::path_is_strictly_within(path, &survivor)
        });
        directories.sort_by_key(|path| std::cmp::Reverse(path.len()));
        let execution = RootMoveExecutionPlan {
            conflict: Some(conflict),
            catalog_paths: BTreeMap::from([(title.id.clone(), keep_catalog)]),
            titles: vec![RootMoveTitleExecution {
                title_id: title.id.clone(),
                title_name: title.name.clone(),
                sequence: 0,
                class: TitleLocationClass::RootMove,
                source_library_id: title.library_id.clone(),
                source_root_id: source_root.id.clone(),
                source_folder_path: title.folder_path.clone(),
                destination_library_id: title.library_id,
                destination_root_id: destination_root.id.clone(),
                destination_folder_path: Some(survivor),
                destination_root_path: Some(destination_root.path.clone()),
                source_root_path: Some(source_root.path.clone()),
                same_volume: None,
                files: execution_files,
                deduplicated_sources: Vec::new(),
                renamed_destinations,
                prune_directories: directories,
                warnings: Vec::new(),
                converted_facet: None,
                dropped_tag_prefixes: Vec::new(),
                merge_target_title_id: None,
            }],
            ..Default::default()
        };
        Ok(PlannedConflict {
            selected_count: 1,
            row,
            plan,
            execution,
        })
    }
}

fn relative_name(folder: &str, path: &str) -> String {
    path_to_stored_string(
        stored_path_to_path_buf(path)
            .strip_prefix(stored_path_to_path_buf(folder))
            .expect("validated descendant"),
    )
}

fn join_relative_name(folder: &str, name: &str) -> AppResult<String> {
    let relative = stored_path_to_path_buf(name);
    if !crate::stored_paths::stored_path_is_native(name)
        || relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        return Err(refused(
            refusal::FOLDER_UNSAFE,
            "A planned filename escapes its conflict folder",
        ));
    }
    Ok(path_to_stored_string(
        stored_path_to_path_buf(folder).join(relative),
    ))
}

#[cfg(test)]
mod native_name_tests {
    use super::*;

    #[test]
    fn conflict_io_reasons_use_kinds_not_sentences() {
        for (kind, reason) in [
            (std::io::ErrorKind::NotFound, refusal::PATH_MISSING),
            (
                std::io::ErrorKind::NotADirectory,
                refusal::PATH_NOT_DIRECTORY,
            ),
            (
                std::io::ErrorKind::PermissionDenied,
                refusal::PATH_PERMISSION_DENIED,
            ),
            (std::io::ErrorKind::Other, refusal::PATH_IO_FAILED),
        ] {
            let error = conflict_io_error(std::io::Error::new(kind, "arbitrary provider wording"));
            assert_eq!(error.validation_reason(), Some(reason));
        }
    }

    #[test]
    fn encoded_relative_names_cannot_escape_the_survivor() {
        let folder = std::path::Path::new("/library/survivor");
        for unsafe_name in [
            "../outside.mkv",
            "/outside.mkv",
            "nested/../../outside.mkv",
            "",
        ] {
            let platform = if cfg!(windows) { "w" } else { "u" };
            let encoded = format!("scryer-path-v1:{platform}:{unsafe_name}");
            assert_eq!(
                join_relative_name(&path_to_stored_string(folder), &encoded)
                    .unwrap_err()
                    .validation_reason(),
                Some(refusal::FOLDER_UNSAFE)
            );
        }
        let literal = "scryer-path-v1:u:movie.mkv";
        let joined = join_relative_name(
            &path_to_stored_string(folder),
            &path_to_stored_string(literal),
        )
        .unwrap();
        assert_eq!(stored_path_to_path_buf(&joined), folder.join(literal));
    }
}

#[cfg(test)]
mod folder_name_rank_tests {
    use super::*;
    use scryer_domain::{ExternalId, MediaFacet};

    fn movie(name: &str, year: i32, external_ids: &[(&str, &str)]) -> Title {
        Title {
            id: "title-1".into(),
            name: name.into(),
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            root_folder_id: scryer_domain::root_folder_id_for_path("/library"),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: external_ids
                .iter()
                .map(|(source, value)| ExternalId {
                    source: (*source).into(),
                    kind: None,
                    value: (*value).into(),
                })
                .collect(),
            created_by: None,
            created_at: chrono::Utc::now(),
            year: Some(year),
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
            folder_path: None,
        }
    }

    fn rank(title: &Title, name: &str) -> i8 {
        folder_name_rank(title, &path_to_stored_string(format!("/library/{name}")))
    }

    fn media(path: &str) -> ConflictFile {
        ConflictFile {
            path: path_to_stored_string(path),
            version: String::new(),
            size_bytes: 1,
            media: true,
            media_file_id: None,
            root_path: "/library".into(),
        }
    }

    fn root(id: &str, library_id: &str, path: &str) -> scryer_domain::LibraryRoot {
        scryer_domain::LibraryRoot {
            id: id.into(),
            library_id: library_id.into(),
            path: path_to_stored_string(path),
            is_default: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn catalog(
        roots: Vec<scryer_domain::LibraryRoot>,
    ) -> crate::location::title_folder_recovery::RecoveryCatalog {
        crate::location::title_folder_recovery::RecoveryCatalog {
            roots,
            owners: vec![],
            media: vec![],
        }
    }

    #[test]
    fn a_folder_that_is_or_holds_any_librarys_root_spans_a_root() {
        let roots = vec![
            root("movies", "movie-lib", "/library"),
            root("series", "series-lib", "/library/Bucket/Inner"),
            root("anime", "anime-lib", "/library/Exact"),
        ];
        assert!(folder_spans_any_root(
            &path_to_stored_string("/library/Bucket"),
            &roots
        ));
        assert!(folder_spans_any_root(
            &path_to_stored_string("/library/Exact"),
            &roots
        ));
        assert!(!folder_spans_any_root(
            &path_to_stored_string("/library/Alone"),
            &roots
        ));
    }

    #[test]
    fn revalidation_refuses_a_folder_that_became_another_librarys_root() {
        let mut title = movie("Emberfall", 2024, &[]);
        title.library_id = "movie-lib".into();
        let folder = path_to_stored_string("/library/Bucket");
        let before = catalog(vec![root("movies", "movie-lib", "/library")]);
        revalidate_conflict_folder(&folder, &title, &before).expect("still safe");
        for added in ["/library/Bucket", "/library/Bucket/Inner"] {
            let after = catalog(vec![
                root("movies", "movie-lib", "/library"),
                root("series", "series-lib", added),
            ]);
            let error = revalidate_conflict_folder(&folder, &title, &after)
                .expect_err("a root inside the folder is refused");
            assert_eq!(error.validation_reason(), Some(refusal::FOLDER_UNSAFE));
        }
    }

    /// A re-cased spelling of another title's folder is that folder exactly
    /// when the volume folds case; the planner follows the volume, not the
    /// spelling. The fixture's volume decides which answer is right.
    #[test]
    fn another_owner_under_a_re_cased_spelling_follows_the_volume() {
        let temp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(temp.path()).unwrap();
        let owned = base.join("Owner Folder");
        std::fs::create_dir(&owned).unwrap();
        std::fs::write(owned.join("Fixture.mkv"), b"x").unwrap();
        let recased = base.join("owner folder");
        let case_insensitive = crate::folder_ownership::folder_case_insensitive(&recased);
        let mut title = movie("Emberfall", 2024, &[]);
        title.id = "this-title".into();
        let mut held = catalog(vec![]);
        held.owners
            .push(crate::location::title_folder_recovery::RecoveryOwner {
                id: "other-title".into(),
                library_id: title.library_id.clone(),
                folder_path: Some(path_to_stored_string(&owned)),
                revision: None,
            });
        assert_eq!(
            another_title_holds_content(&path_to_stored_string(&recased), &title.id, &held)
                .unwrap(),
            case_insensitive
        );
        // An exact spelling always overlaps; an unrelated folder never does.
        assert!(
            another_title_holds_content(&path_to_stored_string(&owned), &title.id, &held).unwrap()
        );
        assert!(
            !another_title_holds_content(
                &path_to_stored_string(base.join("Elsewhere")),
                &title.id,
                &held
            )
            .unwrap()
        );
        let mut media = catalog(vec![]);
        media
            .media
            .push(crate::location::title_folder_recovery::RecoveryMedia {
                id: "media".into(),
                title_id: "other-title".into(),
                file_path: path_to_stored_string(owned.join("Fixture.mkv")),
                size_bytes: 1,
            });
        assert_eq!(
            another_title_holds_content(&path_to_stored_string(&recased), &title.id, &media)
                .unwrap(),
            case_insensitive
        );
    }

    #[test]
    fn nested_roots_resolve_to_the_deepest_one() {
        let roots = vec![
            root("outer", "movie-lib", "/library"),
            root("inner", "movie-lib", "/library/Nested"),
        ];
        let deepest = |folder: &str| {
            deepest_containing_root(&roots, &path_to_stored_string(folder)).map(|r| r.id.as_str())
        };
        assert_eq!(deepest("/library/Nested/Emberfall"), Some("inner"));
        assert_eq!(deepest("/library/Emberfall"), Some("outer"));
        assert_eq!(deepest("/elsewhere/Emberfall"), None);
    }

    #[test]
    fn a_file_stem_naming_another_work_ranks_as_another_work() {
        let title = movie("Emberfall", 2024, &[]);
        let file = |name: &str| file_name_rank(&title, &format!("/library/A/{name}"));
        assert_eq!(file("Emberfall.2024.2160p.BluRay.mkv"), 1);
        assert_eq!(file("Other.Picture.2011.1080p.BluRay.mkv"), -1);
        assert_eq!(file("movie.mkv"), 0);
    }

    #[test]
    fn a_file_stem_differing_only_in_separators_names_the_title() {
        let title = movie("40,000 Lanterns", 2011, &[]);
        let file = |name: &str| file_name_rank(&title, &format!("/library/ADEG78/{name}"));
        assert_eq!(file("40000.Lanterns.2011.1080p.BluRay.mkv"), 1);
        assert_eq!(file("40,000.Lanterns.2011.1080p.BluRay.mkv"), 1);
        assert_eq!(file("40000.Lantern.Keepers.2011.1080p.BluRay.mkv"), -1);
    }

    #[test]
    fn stacked_part_markers_are_read_from_stems() {
        assert_eq!(stacked_part_number("Emberfall.2024.cd1"), Some(1));
        assert_eq!(stacked_part_number("Emberfall 2024 pt2"), Some(2));
        assert_eq!(stacked_part_number("Emberfall-2024-part-1"), Some(1));
        assert_eq!(stacked_part_number("Emberfall.2024.Disc 2"), Some(2));
        assert_eq!(stacked_part_number("Emberfall.2024.2160p"), None);
        assert_eq!(stacked_part_number("Emberfall.Part.2001"), None);
    }

    #[test]
    fn multi_part_media_needs_two_part_numbers_or_a_part_folder() {
        let folders = vec![path_to_stored_string("/library/A")];
        assert!(holds_multi_part_media(
            &[
                media("/library/A/Emberfall.cd1.mkv"),
                media("/library/A/Emberfall.cd2.mkv")
            ],
            &folders
        ));
        // One stem saying `Part 2` names a sequel, not a stack.
        assert!(!holds_multi_part_media(
            &[
                media("/library/A/Emberfall Part 2.mkv"),
                media("/library/A/Emberfall Part 2.720p.mkv")
            ],
            &folders
        ));
        assert!(holds_multi_part_media(
            &[media("/library/A/CD2/Emberfall.mkv")],
            &folders
        ));
        // A part-looking conflict folder itself is not a part folder inside it.
        assert!(!holds_multi_part_media(
            &[media("/library/CD1/Emberfall.mkv")],
            &[path_to_stored_string("/library/CD1")]
        ));
    }

    #[test]
    fn a_trailing_article_still_names_the_title() {
        let title = movie("The Emberfall", 2024, &[]);
        assert_eq!(rank(&title, "Emberfall, The (2024)"), 1);
        assert_eq!(rank(&title, "The Emberfall (2024)"), 1);
    }

    #[test]
    fn an_ampersand_and_the_word_and_name_the_same_title() {
        let title = movie("Ash & Ember", 2024, &[]);
        assert_eq!(rank(&title, "Ash and Ember (2024)"), 1);
        let title = movie("Ash and Ember", 2024, &[]);
        assert_eq!(rank(&title, "Ash & Ember (2024)"), 1);
    }

    #[test]
    fn a_year_one_off_matches_but_a_distant_year_is_another_work() {
        let title = movie("Emberfall", 2024, &[]);
        assert_eq!(rank(&title, "Emberfall (2023)"), 1);
        assert_eq!(rank(&title, "Emberfall (2025)"), 1);
        assert_eq!(rank(&title, "Emberfall (2019)"), -1);
    }

    #[test]
    fn tag_ids_compare_trimmed_and_unknown_sources_are_no_evidence() {
        let title = movie("Emberfall", 2024, &[("tmdb", " 603 "), ("imdb", " ")]);
        assert_eq!(rank(&title, "Ashen Harbor (2024) {tmdb-603}"), 1);
        assert_eq!(rank(&title, "Emberfall (2024) {tmdb-604}"), -1);
        // The title's imdb id is blank, so an imdb tag decides nothing.
        assert_eq!(rank(&title, "Emberfall (2024) {imdb-tt9999999}"), 1);
        assert_eq!(rank(&title, "Different Feature {imdb-tt9999999}"), -1);
    }

    #[test]
    fn junk_names_are_neutral_and_other_works_lose() {
        let title = movie("Emberfall", 2024, &[]);
        assert_eq!(rank(&title, "ADEG78"), 0);
        assert_eq!(rank(&title, "Different Feature (2024)"), -1);
        assert_eq!(rank(&title, "Different.Feature.2160p.BluRay"), -1);
    }
}

fn candidate_file(title_id: &str, file: &ConflictFile) -> TitleMediaFile {
    let parsed = crate::release_parser::parse_release_metadata(
        &stored_path_to_path_buf(&file.path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
    );
    TitleMediaFile {
        title_id: title_id.to_owned(),
        file_path: file.path.clone(),
        size_bytes: file.size_bytes.min(i64::MAX as u64) as i64,
        quality_label: parsed.quality.clone(),
        source_type: crate::release_parser::parsed_release_source_type(&parsed),
        resolution: parsed.quality.clone(),
        video_codec_parsed: parsed.video_codec,
        scene_name: Some(parsed.raw_title),
        release_group: parsed.release_group,
        edition: parsed.edition,
        ..Default::default()
    }
}

/// The `dev:ino` prefix of a unix file version; `None` where versions carry no inode.
fn inode_key(version: &str) -> Option<String> {
    if !cfg!(unix) || version.starts_with("symlink:") {
        return None;
    }
    let mut parts = version.split(':');
    Some(format!("{}:{}", parts.next()?, parts.next()?))
}

/// A directory's physical `(dev, ino)`, or `None` on platforms without inodes.
async fn directory_identity(path: &std::path::Path) -> AppResult<Option<(u64, u64)>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = tokio::fs::symlink_metadata(path)
            .await
            .map_err(conflict_io_error)?;
        Ok(Some((metadata.dev(), metadata.ino())))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(None)
    }
}

fn same_inode_version(left: &str, right: &str) -> bool {
    cfg!(unix) && left.split(':').take(2).eq(right.split(':').take(2))
}

fn same_content_version(left: &str, right: &str) -> bool {
    cfg!(unix) && left.split(':').take(5).eq(right.split(':').take(5))
}

async fn conflict_hash(path: &str, version: &str) -> AppResult<String> {
    let native = stored_path_to_path_buf(path);
    if super::resolution::file_version(&native).await? != version {
        return Err(refused(
            refusal::FILE_CHANGED,
            "A file changed before its content could be verified",
        ));
    }
    let hashes = super::verify::hash_existing_file_with_progress(
        &native,
        super::verify::CopyProgress::none(),
    )
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;
    if super::resolution::file_version(&native).await? != version {
        return Err(refused(
            refusal::FILE_CHANGED,
            "A file changed while its content was verified",
        ));
    }
    Ok(hashes.full_blake3)
}

fn disposal_proof(row: &super::resolution::FileResolution) -> Option<&DisposalProof> {
    match &row.disposition {
        super::resolution::ResolutionDisposition::Disposal(proof) => Some(proof),
        _ => None,
    }
}

/// Only the newest recorded pathname effect for this inode can explain ctime.
async fn disposal_version_current(
    path: &str,
    expected: &str,
    resolutions: &[super::resolution::FileResolution],
) -> AppResult<bool> {
    let Some((row, proof)) = resolutions
        .iter()
        .filter_map(|row| {
            let proof = disposal_proof(row)?;
            row.source_version
                .as_deref()
                .filter(|version| same_inode_version(version, expected))?;
            Some((row, proof))
        })
        .max_by_key(|(_, proof)| proof.sequence)
    else {
        return Ok(false);
    };
    let native = stored_path_to_path_buf(path);
    let actual = super::resolution::file_version(&native).await?;
    if !same_content_version(&actual, expected) {
        return Ok(false);
    }
    let source_absent = tokio::fs::symlink_metadata(stored_path_to_path_buf(&row.source_path))
        .await
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
    if !source_absent {
        return Ok(!row.completed
            && proof.after_versions.get(path) == Some(&actual)
            && super::hardlinks::link_count(&native)?.count() == Some(proof.before_link_count));
    }
    if row.completed {
        if let Some(recycle) = &proof.recycle {
            let payload = stored_path_to_path_buf(&recycle.recycled_path);
            if super::resolution::file_version(&payload).await.ok() != row.destination_version {
                return Ok(false);
            }
        }
        return Ok(proof.after_versions.get(path) == Some(&actual)
            && super::hardlinks::link_count(&native)?.count() == proof.after_link_count);
    }
    let mut expected_count = proof.before_link_count.saturating_sub(1);
    if let Some(recycle) = &proof.recycle {
        let payload = stored_path_to_path_buf(&recycle.recycled_path);
        super::title_folder_recovery::inspect_path(&payload, false).await?;
        let version = super::resolution::file_version(&payload).await?;
        if conflict_hash(&recycle.recycled_path, &version)
            .await?
            .as_str()
            != row.identity_hash.as_deref().unwrap_or("")
        {
            return Ok(false);
        }
        if same_inode_version(&version, expected) {
            expected_count = proof.before_link_count;
        }
    }
    Ok(
        super::hardlinks::link_count(&native)?.count() == Some(expected_count)
            && conflict_hash(path, &actual).await?.as_str()
                == row.identity_hash.as_deref().unwrap_or(""),
    )
}

async fn conflict_resolution_current(
    row: &super::resolution::FileResolution,
    resolutions: &[super::resolution::FileResolution],
) -> AppResult<bool> {
    if row.proof_is_current().await {
        return Ok(true);
    }
    let Some(expected) = &row.destination_version else {
        return Ok(false);
    };
    if row.is_disposal()
        || !row.completed
        || !disposal_version_current(&row.destination_path, expected, resolutions).await?
    {
        return Ok(false);
    }
    if let Some(source_expected) = &row.source_version {
        match tokio::fs::symlink_metadata(stored_path_to_path_buf(&row.source_path)).await {
            Ok(_) => {
                let actual =
                    super::resolution::file_version(&stored_path_to_path_buf(&row.source_path))
                        .await?;
                if &actual != source_expected
                    && !disposal_version_current(&row.source_path, source_expected, resolutions)
                        .await?
                {
                    return Ok(false);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::Repository(error.to_string())),
        }
    }
    Ok(true)
}

async fn preview_version_current(
    file: &ConflictFile,
    resolutions: &[super::resolution::FileResolution],
    conflict: &ConflictExecution,
) -> AppResult<bool> {
    let actual = super::resolution::file_version(&stored_path_to_path_buf(&file.path)).await?;
    if actual == file.version {
        return Ok(true);
    }
    if disposal_version_current(&file.path, &file.version, resolutions).await? {
        return Ok(true);
    }
    if let Some(proof) = conflict
        .hardlink_proofs
        .iter()
        .find(|proof| same_inode_version(&proof.version, &file.version))
    {
        if resolutions.iter().any(|row| {
            row.is_disposal()
                && row
                    .source_version
                    .as_deref()
                    .is_some_and(|version| same_inode_version(version, &file.version))
        }) {
            return Ok(false);
        }
        let mut removed = BTreeSet::new();
        let mut added = BTreeSet::new();
        for row in resolutions.iter().filter(|row| {
            row.completed
                && row.disposition == super::resolution::ResolutionDisposition::Hardlink
                && row
                    .source_version
                    .as_deref()
                    .is_some_and(|version| same_inode_version(version, &file.version))
        }) {
            if !row.proof_is_current().await
                || !same_inode_version(
                    &super::resolution::file_version(&stored_path_to_path_buf(
                        &row.destination_path,
                    ))
                    .await?,
                    &file.version,
                )
            {
                return Ok(false);
            }
            if !conflict
                .files
                .iter()
                .any(|file| file.path == row.destination_path)
            {
                added.insert(&row.destination_path);
            }
            if tokio::fs::symlink_metadata(stored_path_to_path_buf(&row.source_path))
                .await
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                removed.insert(&row.source_path);
            }
        }
        return Ok((!added.is_empty() || !removed.is_empty())
            && same_content_version(&actual, &file.version)
            && super::hardlinks::link_count(&stored_path_to_path_buf(&file.path))?.count()
                == proof
                    .link_count
                    .checked_add(added.len() as u64)
                    .and_then(|count| count.checked_sub(removed.len() as u64))
            && conflict_hash(&file.path, &actual).await? == proof.full_blake3);
    }
    #[cfg(unix)]
    if actual
        .split(':')
        .take(5)
        .eq(file.version.split(':').take(5))
    {
        for resolution in resolutions.iter().filter(|row| {
            row.destination_path == file.path
                && row.disposition == super::resolution::ResolutionDisposition::Hardlink
        }) {
            // Removing this operation's incoming link changes only ctime on
            // the retained inode. Require its durable, current hardlink proof;
            // an unrelated ctime change never earns this exception.
            if resolution.proof_is_current().await {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

impl AppUseCase {
    async fn conflict_selection(
        &self,
        actor: &User,
        request: &ConflictRequest,
    ) -> AppResult<(ConflictPreview, Vec<PlannedConflict>)> {
        let unique: BTreeSet<_> = request.pending_import_ids.iter().collect();
        if unique.is_empty()
            || unique.len() > MAX_SELECTION
            || unique.len() != request.pending_import_ids.len()
            || unique.iter().any(|id| id.trim().is_empty())
        {
            return Err(refused(
                refusal::SELECTION_INVALID,
                "Select between one and one thousand distinct pending conflicts",
            ));
        }
        let catalog = self
            .services
            .library
            .location_operations
            .recovery_catalog()
            .await?;
        let mut plans: Vec<PlannedConflict> = Vec::new();
        let mut rows = Vec::new();
        let mut permanent = BTreeMap::new();
        let mut fingerprints = Vec::new();
        let mut title_ids = BTreeSet::new();
        for id in unique {
            if let Some(existing) = plans.iter_mut().find(|plan| {
                plan.execution
                    .conflict
                    .as_ref()
                    .is_some_and(|conflict| conflict.pending_import_ids.contains(id))
            }) {
                existing.selected_count += 1;
                let mut row = existing.row.clone();
                row.pending_import_id = id.clone();
                rows.push(row);
                fingerprints.push((id.clone(), existing.plan.fingerprint.0.clone()));
                continue;
            }
            let mut authorized_title = None;
            match self
                .plan_pending_import_conflict(actor, id, request, &catalog, &mut authorized_title)
                .await
            {
                Ok(plan) => {
                    fingerprints.push((id.clone(), plan.plan.fingerprint.0.clone()));
                    rows.push(plan.row.clone());
                    if title_ids.insert(plan.row.title_id.clone()) {
                        let conflict = plan.execution.conflict.as_ref().expect("conflict plan");
                        for path in &conflict.permanent_delete_paths {
                            permanent.insert(
                                path.clone(),
                                conflict
                                    .files
                                    .iter()
                                    .find(|file| &file.path == path)
                                    .expect("allowlisted file")
                                    .size_bytes,
                            );
                        }
                        plans.push(plan);
                        if plans
                            .iter()
                            .map(|plan| {
                                plan.execution
                                    .conflict
                                    .as_ref()
                                    .map_or(0, |conflict| conflict.files.len())
                            })
                            .sum::<usize>()
                            > MAX_FILES
                        {
                            return Err(refused(
                                refusal::SELECTION_INVALID,
                                "The selection exceeds the safe file preview bound; select fewer titles",
                            ));
                        }
                    } else if let Some(existing) = plans
                        .iter_mut()
                        .find(|existing| existing.row.title_id == plan.row.title_id)
                    {
                        existing.selected_count += 1;
                    }
                }
                Err(error @ (AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. })) => {
                    return Err(error);
                }
                Err(error) => {
                    if error.reason().is_none() {
                        tracing::warn!(pending_import_id = %id, error = %error, "Conflict preview planning failed");
                    }
                    let reason = error.reason().unwrap_or(refusal::PLAN_FAILED).to_owned();
                    fingerprints.push((id.clone(), reason.clone()));
                    let (title_id, title_name) = authorized_title.unwrap_or_default();
                    rows.push(ConflictRow {
                        pending_import_id: id.clone(),
                        title_id,
                        title_name,
                        survivor_path: String::new(),
                        primary_path: String::new(),
                        move_bytes: 0,
                        recycle_bytes: 0,
                        recycle_count: 0,
                        collision_renames: 0,
                        collision_rename_samples: Vec::new(),
                        leftover_count: 0,
                        leftover_bytes: 0,
                        leftover_paths: Vec::new(),
                        recycle_paths: Vec::new(),
                        files_sampled: false,
                        error_reason: Some(reason),
                        error_message: Some(error.to_string()),
                    });
                }
            }
        }
        let fingerprint = blake3::hash(
            &serde_json::to_vec(&(request, fingerprints))
                .map_err(|e| AppError::Repository(e.to_string()))?,
        )
        .to_hex()
        .to_string();
        let actionable_count: i32 = plans.iter().map(|plan| plan.selected_count).sum();
        let preview = ConflictPreview {
            plan_fingerprint: fingerprint,
            total_count: rows.len() as i32,
            actionable_count,
            failed_count: rows.len() as i32 - actionable_count,
            sampled: rows.len() > SAMPLE,
            rows: rows.into_iter().take(SAMPLE).collect(),
            permanent_delete_count: permanent.len() as i32,
            permanent_delete_bytes: permanent.values().sum(),
            permanent_delete_paths: permanent.keys().take(SAMPLE).cloned().collect(),
            permanent_delete_sampled: permanent.len() > SAMPLE,
            requires_permanent_delete_confirmation: !permanent.is_empty(),
        };
        Ok((preview, plans))
    }

    pub async fn preview_pending_import_conflicts(
        &self,
        actor: &User,
        request: ConflictRequest,
    ) -> AppResult<ConflictPreview> {
        self.conflict_selection(actor, &request)
            .await
            .map(|(preview, _)| preview)
    }

    pub async fn resolve_pending_import_conflicts(
        &self,
        actor: &User,
        request: ResolveConflictRequest,
    ) -> AppResult<ConflictTask> {
        let (preview, mut plans) = self.conflict_selection(actor, &request.request).await?;
        if preview.plan_fingerprint != request.plan_fingerprint {
            return Err(refused(
                refusal::PLAN_CHANGED,
                "The folders changed; review a fresh preview",
            ));
        }
        if plans.is_empty() {
            return Err(refused(
                refusal::UNSUPPORTED,
                "The selection contains no actionable movie folder conflicts",
            ));
        }
        if preview.requires_permanent_delete_confirmation && !request.confirm_permanent_delete {
            return Err(refused(
                refusal::PERMANENT_CONFIRMATION_REQUIRED,
                "Confirm permanent deletion of the listed files because no recycle bin is configured",
            ));
        }
        for plan in &mut plans {
            plan.execution
                .conflict
                .as_mut()
                .expect("conflict plan")
                .permanent_delete_confirmed = request.confirm_permanent_delete;
        }
        let mut run = self
            .open_location_job_run(
                "pending-import-conflicts",
                preview.total_count as i64,
                LocationJobRunActor::Confirmed(actor),
                "pending_import_conflicts".into(),
            )
            .await?;
        let planned_count: i32 = plans.iter().map(|plan| plan.selected_count).sum();
        let task = ConflictTask {
            job_run_id: run.id.clone(),
            total_count: preview.total_count,
            settled_count: 0,
            failed_count: preview.total_count - planned_count,
            operation_ids: Vec::new(),
            completed: false,
        };
        let journal = ConflictTaskJournal {
            task: task.clone(),
            actor_id: actor.id.clone(),
            library_ids: plans
                .iter()
                .filter_map(|plan| plan.plan.header.source_library_id.clone())
                .collect(),
            plans,
            next: 0,
            current_operation_id: None,
        };
        run.progress_json =
            Some(serde_json::to_string(&journal).map_err(|e| AppError::Repository(e.to_string()))?);
        self.services.events.job_runs.update_job_run(&run).await?;
        self.spawn_conflict_task(run.id);
        Ok(task)
    }

    pub async fn pending_import_conflict_task(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<ConflictTask>> {
        let Some(run) = self.services.events.job_runs.get_job_run(id).await? else {
            return Ok(None);
        };
        if run.operation_type != "pending_import_conflicts" {
            return Ok(None);
        }
        let journal: ConflictTaskJournal =
            serde_json::from_str(run.progress_json.as_deref().unwrap_or(""))
                .map_err(|e| AppError::Repository(e.to_string()))?;
        // A journal naming no library authorizes nobody: hide it rather than
        // let an empty list skip the permission check.
        if journal.library_ids.is_empty() {
            return Ok(None);
        }
        for library in &journal.library_ids {
            self.require_library_permission(actor, library, LibraryPermission::ResolveImports)
                .await?;
        }
        Ok(Some(journal.task))
    }

    fn spawn_conflict_task(&self, id: String) {
        let key = format!("conflict-task:{id}");
        let Some(guard) = self.runtime.library.location_runners.begin(&key) else {
            return;
        };
        let app = self.clone();
        self.restore_coordinator().clone().spawn_worker(async move {
            let _guard = guard;
            if let Err(error) = app.run_conflict_task(&id).await {
                tracing::error!(job_run_id = %id, %error, "Conflict resolution task stopped");
                if let Ok(Some(mut run)) = app.services.events.job_runs.get_job_run(&id).await {
                    if let Some(json) = &run.progress_json
                        && let Ok(mut journal) = serde_json::from_str::<ConflictTaskJournal>(json)
                    {
                        journal.task.failed_count =
                            journal.task.total_count - journal.task.settled_count;
                        journal.task.completed = true;
                        run.progress_json = serde_json::to_string(&journal).ok();
                        if let Ok(updated) = app.services.events.job_runs.update_job_run(&run).await
                        {
                            run = updated;
                        }
                    }
                    app.close_location_operation_job_run(
                        &run,
                        crate::JobRunStatus::Failed,
                        "Folder conflict resolution stopped; review its operations in Activity."
                            .into(),
                        Some(error.to_string()),
                        None,
                    )
                    .await;
                    app.runtime.library.location_runners.transfers.changed();
                }
            }
        });
    }

    pub(super) async fn resumable_conflict_task_job_run_ids(&self) -> AppResult<Vec<String>> {
        let mut ids = Vec::new();
        let mut before = None;
        loop {
            let runs = self
                .services
                .events
                .job_runs
                .list_job_runs_by_operation_type_before(
                    crate::JobKey::LocationOperation,
                    "pending_import_conflicts",
                    before,
                    16,
                )
                .await?;
            let complete = runs.len() < 16;
            before = runs.last().map(|run| (run.started_at, run.id.clone()));
            for run in runs {
                match run
                    .progress_json
                    .as_deref()
                    .map(serde_json::from_str::<ConflictTaskJournal>)
                {
                    Some(Ok(journal)) if !journal.task.completed => ids.push(run.id),
                    Some(Ok(_)) => {}
                    _ => {
                        tracing::warn!(job_run_id = %run.id, "A conflict task has an unreadable journal; preserving its recovery evidence");
                        ids.push(run.id);
                    }
                }
            }
            if complete {
                break;
            }
        }
        Ok(ids)
    }

    pub(super) async fn resume_pending_import_conflict_tasks(&self) -> AppResult<()> {
        for id in self.resumable_conflict_task_job_run_ids().await? {
            let run = self.services.events.job_runs.get_job_run(&id).await?;
            if run
                .and_then(|run| run.progress_json)
                .as_deref()
                .is_some_and(|json| serde_json::from_str::<ConflictTaskJournal>(json).is_ok())
            {
                self.spawn_conflict_task(id);
            } else {
                tracing::warn!(job_run_id = %id, "An unreadable conflict journal stays interrupted; no files were touched");
            }
        }
        Ok(())
    }

    async fn run_conflict_task(&self, id: &str) -> AppResult<()> {
        let mut run = self
            .services
            .events
            .job_runs
            .get_job_run(id)
            .await?
            .ok_or_else(|| refused(refusal::NOT_FOUND, "Conflict task no longer exists"))?;
        let mut journal: ConflictTaskJournal =
            serde_json::from_str(run.progress_json.as_deref().unwrap_or(""))
                .map_err(|e| AppError::Repository(e.to_string()))?;
        if journal.task.completed {
            return Ok(());
        }
        if run.status != crate::JobRunStatus::Running {
            // This row is the durable selection journal. Individual location
            // executions still receive their own job run on every resume.
            run.status = crate::JobRunStatus::Running;
            run.completed_at = None;
            run.error_text = None;
            run.updated_at = chrono::Utc::now();
            run = self.services.events.job_runs.update_job_run(&run).await?;
            self.runtime
                .jobs
                .job_run_tracker
                .upsert_active_run(crate::JobRun::from_record(&run, None))
                .await;
        }
        while journal.next < journal.plans.len() {
            let planned = journal.plans[journal.next].clone();
            let operation_id = format!("conflict:{}:{}", id, journal.next);
            let actor = self
                .services
                .identity
                .users
                .get_by_id(&journal.actor_id)
                .await?
                .ok_or_else(|| {
                    refused(
                        refusal::NOT_FOUND,
                        "The confirming account no longer exists",
                    )
                })?;
            let attempt: AppResult<super::model::LocationOperationState> = async {
                let library = planned
                    .plan
                    .header
                    .source_library_id
                    .as_deref()
                    .expect("conflict library");
                self.require_library_permission(&actor, library, LibraryPermission::ResolveImports)
                    .await?;
                self.require_library_permission(&actor, library, LibraryPermission::ManageTitles)
                    .await?;
                let mut changes = self.subscribe_location_transfers();
                // Reserve the runner before enrollment, so another FIFO walker
                // can never classify this task's queued row as stranded.
                let guard = self.runtime.library.location_runners.begin(&operation_id);
                if self.location_operation(&operation_id).await?.is_none() {
                    self.validate_folder_conflict(&planned.execution, None)
                        .await?;
                    self.admit_location_operation_dispatch(
                        LocationOperationAdmission {
                            actor: &actor,
                            plan: planned.plan.clone(),
                            execution: planned.execution.clone(),
                            titles_total: 1,
                            failure_message: "Folder conflict resolution could not be started.",
                        },
                        Some(operation_id.clone()),
                        false,
                    )
                    .await?;
                }
                if !journal.task.operation_ids.contains(&operation_id) {
                    journal.task.operation_ids.push(operation_id.clone());
                }
                journal.current_operation_id = Some(operation_id.clone());
                run.progress_json = Some(
                    serde_json::to_string(&journal)
                        .map_err(|e| AppError::Repository(e.to_string()))?,
                );
                run = self.services.events.job_runs.update_job_run(&run).await?;
                // The deterministic ID joins enrollment to the task journal across a crash.
                let operation = self
                    .location_operation(&operation_id)
                    .await?
                    .expect("enrolled operation");
                if operation.state.is_terminal() {
                    return Ok(operation.state);
                }
                let Some(guard) = guard else {
                    loop {
                        let current =
                            self.location_operation(&operation_id)
                                .await?
                                .ok_or_else(|| {
                                    refused(
                                        refusal::NOT_FOUND,
                                        "The conflict operation no longer exists",
                                    )
                                })?;
                        if current.state.is_terminal() {
                            return Ok(current.state);
                        }
                        changes
                            .changed()
                            .await
                            .map_err(|e| AppError::Repository(e.to_string()))?;
                    }
                };
                let outcome = self.run_root_move(&operation_id, &planned.execution).await;
                drop(guard);
                let state = outcome?.state;
                if state.is_terminal() {
                    return Ok(state);
                }
                loop {
                    let current =
                        self.location_operation(&operation_id)
                            .await?
                            .ok_or_else(|| {
                                refused(
                                    refusal::NOT_FOUND,
                                    "The conflict operation no longer exists",
                                )
                            })?;
                    if current.state.is_terminal() {
                        return Ok(current.state);
                    }
                    changes
                        .changed()
                        .await
                        .map_err(|e| AppError::Repository(e.to_string()))?;
                }
            }
            .await;
            if !journal.task.operation_ids.contains(&operation_id)
                && self.location_operation(&operation_id).await?.is_some()
            {
                journal.task.operation_ids.push(operation_id);
            }
            let count = planned.selected_count;
            match attempt {
                Ok(
                    super::model::LocationOperationState::Completed
                    | super::model::LocationOperationState::CompletedWithWarnings,
                ) => journal.task.settled_count += count,
                Ok(state) if state.is_active() => return Ok(()),
                Ok(_) => journal.task.failed_count += count,
                Err(error) => {
                    tracing::warn!(job_run_id = %id, pending_import_id = %planned.row.pending_import_id, %error, "A conflict failed; continuing the selection");
                    journal.task.failed_count += count;
                }
            }
            journal.next += 1;
            journal.current_operation_id = None;
            run.progress_json = Some(
                serde_json::to_string(&journal).map_err(|e| AppError::Repository(e.to_string()))?,
            );
            run.updated_at = chrono::Utc::now();
            run = self.services.events.job_runs.update_job_run(&run).await?;
            self.runtime
                .jobs
                .job_run_tracker
                .upsert_active_run(crate::JobRun::from_record(&run, None))
                .await;
            self.runtime.library.location_runners.transfers.changed();
        }
        journal.task.completed = true;
        run.progress_json =
            Some(serde_json::to_string(&journal).map_err(|e| AppError::Repository(e.to_string()))?);
        run = self.services.events.job_runs.update_job_run(&run).await?;
        let status = if journal.task.failed_count > 0 {
            crate::JobRunStatus::Warning
        } else {
            crate::JobRunStatus::Completed
        };
        self.close_location_operation_job_run(
            &run,
            status,
            format!(
                "Settled {} of {} folder conflicts.",
                journal.task.settled_count, journal.task.total_count
            ),
            None,
            None,
        )
        .await;
        self.runtime.library.location_runners.transfers.changed();
        Ok(())
    }

    pub(super) async fn validate_folder_conflict(
        &self,
        plan: &RootMoveExecutionPlan,
        operation_id: Option<&str>,
    ) -> AppResult<()> {
        let conflict = plan
            .conflict
            .as_ref()
            .ok_or_else(|| refused(refusal::UNSUPPORTED, "Missing confirmed folder conflict"))?;
        let planned = plan
            .titles
            .first()
            .ok_or_else(|| refused(refusal::UNSUPPORTED, "Missing confirmed title"))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&planned.title_id)
            .await?
            .ok_or_else(|| refused(refusal::OWNERSHIP_CHANGED, "The title no longer exists"))?;
        if title.library_id != planned.source_library_id
            || (title.root_folder_id != planned.source_root_id
                && title.root_folder_id != planned.destination_root_id)
            || ![
                planned.source_folder_path.as_deref(),
                planned.destination_folder_path.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|folder| {
                title
                    .folder_path
                    .as_deref()
                    .is_some_and(|actual| folder_paths_match(folder, actual))
            })
        {
            return Err(refused(
                refusal::OWNERSHIP_CHANGED,
                "The title's placement changed after confirmation",
            ));
        }
        let catalog = self
            .services
            .library
            .location_operations
            .recovery_catalog()
            .await?;
        for root in conflict.roots.iter().filter(|root| {
            conflict.folders.iter().any(|folder| {
                crate::title_folder_rules::path_is_strictly_within(folder, &root.path)
            })
        }) {
            if !catalog.roots.iter().any(|actual| {
                actual.id == root.id
                    && actual.library_id == root.library_id
                    && actual.path == root.path
            }) {
                return Err(refused(
                    refusal::OWNERSHIP_CHANGED,
                    "A confirmed root changed identity or location",
                ));
            }
        }
        let resolutions = match operation_id {
            Some(id) => {
                self.services
                    .library
                    .location_operations
                    .file_resolutions(id, &planned.title_id)
                    .await?
            }
            None => Vec::new(),
        };
        for folder in &conflict.folders {
            super::title_folder_recovery::inspect_path(&stored_path_to_path_buf(folder), true)
                .await?;
            revalidate_conflict_folder(folder, &title, &catalog)?;
        }
        let expected_paths: BTreeSet<_> = conflict
            .files
            .iter()
            .map(|file| file.path.as_str())
            .chain(
                planned
                    .files
                    .iter()
                    .map(|file| file.destination_path.as_str()),
            )
            .chain(
                resolutions
                    .iter()
                    .filter(|row| row.completed && !row.is_disposal())
                    .map(|row| row.destination_path.as_str()),
            )
            .collect();
        if catalog.media.iter().any(|media| {
            media.title_id == title.id && !expected_paths.contains(media.file_path.as_str())
        }) {
            return Err(refused(
                refusal::OWNERSHIP_CHANGED,
                "The title gained media outside the confirmed plan",
            ));
        }
        for file in &conflict.files {
            if let Some(resolution) = resolutions.iter().find(|row| row.source_path == file.path)
                && conflict_resolution_current(resolution, &resolutions).await?
            {
                continue;
            }
            let path = stored_path_to_path_buf(&file.path);
            match tokio::fs::symlink_metadata(&path).await {
                Ok(_) => {
                    super::title_folder_recovery::inspect_path(&path, false).await?;
                    if !preview_version_current(file, &resolutions, conflict).await? {
                        return Err(refused(
                            refusal::FILE_CHANGED,
                            "A previewed file changed; all unprocessed files were preserved",
                        ));
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && operation_id.is_some()
                        && conflict.dispose_paths.contains(&file.path) => {}
                Err(error) => return Err(AppError::Repository(error.to_string())),
            }
        }
        Ok(())
    }

    pub(super) async fn reconcile_folder_conflict(
        &self,
        operation: &LocationOperation,
        plan: &RootMoveExecutionPlan,
    ) -> AppResult<()> {
        self.validate_folder_conflict(plan, Some(&operation.id))
            .await?;
        let conflict = plan.conflict.as_ref().expect("conflict plan");
        let planned = &plan.titles[0];
        let repo = &self.services.library.media_files;
        let resolutions = self
            .services
            .library
            .location_operations
            .file_resolutions(&operation.id, &planned.title_id)
            .await?;
        for row in resolutions.iter().filter(|row| !row.is_disposal()) {
            if !conflict_resolution_current(row, &resolutions).await? {
                return Err(refused(
                    refusal::FILE_CHANGED,
                    "A destination changed before catalog reconciliation",
                ));
            }
        }
        let verifications = self
            .services
            .library
            .location_operations
            .list_location_file_verifications(&operation.id, Some(&planned.title_id))
            .await?;
        let mut primary = None;
        let mut primary_path = None;
        let mut additional = Vec::new();
        let mut retained_paths = BTreeMap::new();
        for file in conflict
            .files
            .iter()
            .filter(|file| file.media && !conflict.dispose_paths.contains(&file.path))
        {
            let moved = planned
                .files
                .iter()
                .find(|moved| moved.source_path == file.path);
            let destination = resolutions
                .iter()
                .find(|row| row.source_path == file.path)
                .map(|row| row.destination_path.as_str())
                .or_else(|| moved.map(|file| file.destination_path.as_str()))
                .unwrap_or(&file.path);
            retained_paths.insert(file.path.clone(), destination.to_owned());
            if let Some(moved) = moved
                && !verifications.iter().any(|receipt| {
                    receipt.source_path == moved.source_path
                        && receipt.destination_path == destination
                        && receipt.outcome.permits_source_removal()
                })
            {
                return Err(refused(
                    refusal::FILE_CHANGED,
                    "Every adopted media file must have a durable verified destination",
                ));
            }
            let existing = repo.list_media_files_for_title(&planned.title_id).await?;
            let file_id = if let Some(existing) =
                existing.iter().find(|media| media.file_path == destination)
            {
                if let Some(id) = file.media_file_id.as_deref()
                    && id != existing.id
                {
                    repo.delete_media_file(id).await?;
                }
                existing.id.clone()
            } else if let Some(id) = file.media_file_id.as_deref() {
                repo.update_media_file_path(id, destination).await?;
                id.to_owned()
            } else {
                let candidate = candidate_file(&planned.title_id, file);
                repo.insert_media_file(&InsertMediaFileInput {
                    title_id: planned.title_id.clone(),
                    file_path: destination.to_owned(),
                    size_bytes: file.size_bytes.min(i64::MAX as u64) as i64,
                    role: MediaFileRole::Additional,
                    quality_label: candidate.quality_label,
                    scene_name: candidate.scene_name,
                    release_group: candidate.release_group,
                    source_type: candidate.source_type,
                    resolution: candidate.resolution,
                    video_codec_parsed: candidate.video_codec_parsed,
                    edition: candidate.edition,
                    ..Default::default()
                })
                .await?
            };
            if let Some(receipt) = verifications
                .iter()
                .find(|receipt| receipt.destination_path == destination)
                && let Some(hashes) = &receipt.hashes
            {
                repo.update_media_file_content_hashes(
                    &file_id,
                    &super::model::PersistedContentHashes::from_streamed(
                        hashes,
                        chrono::Utc::now(),
                    ),
                )
                .await?;
            }
            if file.path == conflict.primary_source {
                primary = Some(file_id);
                primary_path = Some(destination.to_owned());
            } else {
                additional.push(file_id);
            }
        }
        let primary = primary.ok_or_else(|| {
            refused(
                refusal::MEDIA_MISSING,
                "The chosen Primary could not be registered",
            )
        })?;
        // A disposal target keeps its row until cleanup removes it. Demote it
        // with the same write, so a failed cleanup never leaves two Primaries.
        let current = repo.list_media_files_for_title(&planned.title_id).await?;
        additional.extend(
            conflict
                .files
                .iter()
                .filter(|file| conflict.dispose_paths.contains(&file.path))
                .filter_map(|file| file.media_file_id.clone())
                .filter(|id| current.iter().any(|media| &media.id == id)),
        );
        additional.retain(|id| id != &primary);
        additional.sort();
        additional.dedup();
        repo.set_media_file_roles_for_title(&planned.title_id, &primary, &additional)
            .await?;
        let primary_path = primary_path.expect("registered Primary has a path");
        let mut collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&planned.title_id)
            .await?;
        // Collections describe individual physical variants, not the title's elected role.
        // Prefer an existing destination collection when two sources deduplicated there.
        collections.sort_by_key(|collection| {
            let already_at_destination = collection.ordered_path.as_ref().is_some_and(|path| {
                retained_paths
                    .values()
                    .any(|destination| destination == path)
            });
            (!already_at_destination, collection.id.clone())
        });
        let next_index = collections
            .iter()
            .filter_map(|collection| collection.collection_index.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut collection_paths = BTreeSet::new();
        for collection in &collections {
            if collection.collection_type != scryer_domain::CollectionType::Movie {
                continue;
            }
            let Some(source) = collection.ordered_path.as_deref() else {
                continue;
            };
            let destination = retained_paths.get(source).map(String::as_str).or_else(|| {
                retained_paths
                    .values()
                    .any(|destination| destination == source)
                    .then_some(source)
            });
            if let Some(destination) = destination {
                if !collection_paths.insert(destination.to_owned()) {
                    self.services
                        .catalog
                        .shows
                        .delete_collection(&collection.id)
                        .await?;
                    continue;
                }
                self.services
                    .catalog
                    .shows
                    .update_collection(
                        &collection.id,
                        crate::CollectionUpdate {
                            ordered_path: Some(destination.to_owned()),
                            ..Default::default()
                        },
                    )
                    .await?;
            } else if conflict.files.iter().any(|file| file.path == source) {
                self.services
                    .catalog
                    .shows
                    .delete_collection(&collection.id)
                    .await?;
            }
        }
        if !collection_paths.contains(&primary_path) {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&planned.title_id)
                .await?
                .expect("validated title remains present");
            self.services
                .catalog
                .shows
                .create_collection(scryer_domain::Collection {
                    id: scryer_domain::Id::new().0,
                    title_id: planned.title_id.clone(),
                    collection_type: scryer_domain::CollectionType::Movie,
                    collection_index: next_index.to_string(),
                    label: None,
                    ordered_path: Some(primary_path),
                    narrative_order: None,
                    first_episode_number: None,
                    last_episode_number: None,
                    monitored: title.monitored,
                    created_at: chrono::Utc::now(),
                })
                .await?;
        }
        self.services
            .catalog
            .titles
            .assign_title_folders(&[crate::TitleFolderWrite::assign(
                &planned.title_id,
                planned
                    .destination_folder_path
                    .as_deref()
                    .expect("survivor folder"),
            )])
            .await?;
        self.services
            .catalog
            .titles
            .update_metadata(
                &planned.title_id,
                None,
                None,
                None,
                Some(planned.destination_root_id.clone()),
            )
            .await?;
        Ok(())
    }

    async fn record_conflict_disposal(
        &self,
        operation: &LocationOperation,
        title_id: &str,
        conflict: &ConflictExecution,
        file: &ConflictFile,
        recycle: Option<crate::recycle_bin::PreparedRecycleFile>,
        resolutions: &mut Vec<super::resolution::FileResolution>,
    ) -> AppResult<usize> {
        if let Some(index) = resolutions
            .iter()
            .position(|row| row.is_disposal() && row.source_path == file.path)
        {
            if resolutions[index].completed {
                return Err(refused(
                    refusal::FILE_CHANGED,
                    "A disposed pathname became occupied again; it was preserved",
                ));
            }
            return Ok(index);
        }
        let native = stored_path_to_path_buf(&file.path);
        let version = super::resolution::file_version(&native).await?;
        if !preview_version_current(file, resolutions, conflict).await?
            || super::resolution::file_version(&native).await? != version
        {
            return Err(refused(
                refusal::FILE_CHANGED,
                "A hardlink changed before its disposal intent was recorded; it was preserved",
            ));
        }
        let proof = conflict
            .hardlink_proofs
            .iter()
            .find(|proof| same_inode_version(&proof.version, &version))
            .ok_or_else(|| {
                refused(
                    refusal::FILE_CHANGED,
                    "A hardlink disposal has no confirmed content proof",
                )
            })?;
        let before_link_count =
            super::hardlinks::link_count(&native)?
                .count()
                .ok_or_else(|| {
                    refused(
                        refusal::FILE_CHANGED,
                        "The hardlink count cannot be verified",
                    )
                })?;
        let mut before_versions = BTreeMap::new();
        let candidates: BTreeSet<_> = conflict
            .files
            .iter()
            .map(|file| file.path.clone())
            .chain(
                resolutions
                    .iter()
                    .filter(|row| !row.is_disposal())
                    .map(|row| row.destination_path.clone()),
            )
            .collect();
        for path in candidates {
            let native = stored_path_to_path_buf(&path);
            if tokio::fs::symlink_metadata(&native)
                .await
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                let current = super::resolution::file_version(&native).await?;
                if same_inode_version(&version, &current) {
                    before_versions.insert(path, current);
                }
            }
        }
        let row = super::resolution::FileResolution {
            operation_id: operation.id.clone(),
            title_id: title_id.to_owned(),
            source_path: file.path.clone(),
            original_destination: file.path.clone(),
            destination_path: recycle
                .as_ref()
                .map(|entry| entry.recycled_path.clone())
                .unwrap_or_else(|| file.path.clone()),
            disposition: super::resolution::ResolutionDisposition::Disposal(DisposalProof {
                sequence: resolutions
                    .iter()
                    .filter_map(disposal_proof)
                    .map(|proof| proof.sequence)
                    .max()
                    .unwrap_or(0)
                    + 1,
                before_link_count,
                after_link_count: None,
                after_versions: before_versions,
                recycle,
            }),
            completed: false,
            source_version: Some(version),
            destination_version: None,
            identity_hash: Some(proof.full_blake3.clone()),
            warning: None,
            reason_code: None,
        };
        self.services
            .library
            .location_operations
            .save_file_resolution(&row)
            .await?;
        resolutions.push(row);
        Ok(resolutions.len() - 1)
    }

    async fn finish_conflict_disposal(
        &self,
        conflict: &ConflictExecution,
        resolutions: &mut [super::resolution::FileResolution],
        index: usize,
    ) -> AppResult<()> {
        let mut row = resolutions[index].clone();
        let mut proof = disposal_proof(&row).expect("disposal record").clone();
        if row.completed {
            return Ok(());
        }
        let source_version = row.source_version.as_deref().expect("disposal version");
        if !tokio::fs::symlink_metadata(stored_path_to_path_buf(&row.source_path))
            .await
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            return Err(refused(
                refusal::FILE_CHANGED,
                "A disposal pathname is still occupied; its files were preserved",
            ));
        }
        let mut expected_count = proof.before_link_count.saturating_sub(1);
        if let Some(recycle) = &proof.recycle {
            let payload = stored_path_to_path_buf(&recycle.recycled_path);
            super::title_folder_recovery::inspect_path(&payload, false).await?;
            let version = super::resolution::file_version(&payload).await?;
            if conflict_hash(&recycle.recycled_path, &version)
                .await?
                .as_str()
                != row.identity_hash.as_deref().unwrap_or("")
            {
                return Err(refused(
                    refusal::FILE_CHANGED,
                    "The reserved recycled copy changed; remaining files were preserved",
                ));
            }
            if same_inode_version(&version, source_version) {
                expected_count = proof.before_link_count;
            }
            row.destination_version = Some(version);
        }
        proof.after_versions.clear();
        let candidates: BTreeSet<_> = conflict
            .files
            .iter()
            .map(|file| file.path.clone())
            .chain(
                resolutions
                    .iter()
                    .filter(|row| !row.is_disposal())
                    .map(|row| row.destination_path.clone()),
            )
            .chain(
                proof
                    .recycle
                    .iter()
                    .map(|recycle| recycle.recycled_path.clone()),
            )
            .collect();
        let mut verified_content = false;
        for path in candidates {
            let native = stored_path_to_path_buf(&path);
            match tokio::fs::symlink_metadata(&native).await {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(AppError::Repository(error.to_string())),
                Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                    continue;
                }
                Ok(_) => {}
            }
            let version = super::resolution::file_version(&native).await?;
            if !same_inode_version(&version, source_version) {
                continue;
            }
            if !same_content_version(&version, source_version)
                || super::hardlinks::link_count(&native)?.count() != Some(expected_count)
                || (!verified_content
                    && conflict_hash(&path, &version).await?.as_str()
                        != row.identity_hash.as_deref().unwrap_or(""))
            {
                return Err(refused(
                    refusal::FILE_CHANGED,
                    "A hardlinked file changed during disposal; remaining files were preserved",
                ));
            }
            verified_content = true;
            proof.after_versions.insert(path, version);
        }
        if let Some(recycle) = &proof.recycle {
            let file = conflict
                .files
                .iter()
                .find(|file| file.path == row.source_path)
                .expect("confirmed disposal");
            let config = self
                .recycle_bin_config_for_media_root(Some(&file.root_path))
                .await;
            crate::recycle_bin::finish_conflict_recycle(&config, recycle).await?;
        }
        proof.after_link_count = Some(expected_count);
        row.disposition = super::resolution::ResolutionDisposition::Disposal(proof);
        row.completed = true;
        self.services
            .library
            .location_operations
            .save_file_resolution(&row)
            .await?;
        resolutions[index] = row;
        Ok(())
    }

    pub(super) async fn clean_up_folder_conflict(
        &self,
        operation: &LocationOperation,
        plan: &RootMoveExecutionPlan,
        recycler: &dyn SourceRecycler,
    ) -> AppResult<()> {
        let conflict = plan.conflict.as_ref().expect("conflict plan");
        let planned = &plan.titles[0];
        // Validate the survivor afresh even on cleanup-only retries. No disposal can
        // proceed if the best copy or any adopted file ceased to match its receipt.
        let mut resolutions = self
            .services
            .library
            .location_operations
            .file_resolutions(&operation.id, &planned.title_id)
            .await?;
        // Deduplication can retain a path that the preview initially classified
        // as inferior. The durable resolution, including its catalog survivor,
        // always wins over the disposal allowlist.
        let retained: BTreeSet<String> = resolutions
            .iter()
            .filter(|row| row.completed && !row.is_disposal())
            .map(|row| row.destination_path.clone())
            .chain(
                conflict
                    .files
                    .iter()
                    .filter(|file| {
                        !conflict.dispose_paths.contains(&file.path)
                            && !planned
                                .files
                                .iter()
                                .any(|moved| moved.source_path == file.path)
                    })
                    .map(|file| file.path.clone()),
            )
            .collect();
        let mut disposal_remaining = false;
        for path in conflict
            .dispose_paths
            .iter()
            .filter(|path| !retained.contains(*path))
        {
            if tokio::fs::symlink_metadata(stored_path_to_path_buf(path))
                .await
                .is_ok()
            {
                disposal_remaining = true;
                break;
            }
        }
        if disposal_remaining {
            for proof in &conflict.hardlink_proofs {
                let candidates: BTreeSet<_> = conflict
                    .files
                    .iter()
                    .map(|file| file.path.clone())
                    .chain(
                        resolutions
                            .iter()
                            .filter(|row| !row.is_disposal())
                            .map(|row| row.destination_path.clone()),
                    )
                    .collect();
                for path in candidates {
                    let native = stored_path_to_path_buf(&path);
                    if !tokio::fs::symlink_metadata(&native)
                        .await
                        .is_ok_and(|metadata| {
                            metadata.is_file() && !metadata.file_type().is_symlink()
                        })
                    {
                        continue;
                    }
                    let version = super::resolution::file_version(&native).await?;
                    if same_inode_version(&version, &proof.version) {
                        if !same_content_version(&version, &proof.version)
                            || conflict_hash(&path, &version).await? != proof.full_blake3
                        {
                            return Err(refused(
                                refusal::FILE_CHANGED,
                                "Hardlinked content changed after confirmation; remaining files were preserved",
                            ));
                        }
                        break;
                    }
                }
            }
        }
        for row in resolutions.iter().filter(|row| !row.is_disposal()) {
            let linked_proof = row.destination_version.as_deref().and_then(|version| {
                conflict
                    .hardlink_proofs
                    .iter()
                    .find(|proof| same_inode_version(&proof.version, version))
            });
            if !conflict_resolution_current(row, &resolutions).await?
                || (disposal_remaining
                    && linked_proof.is_none()
                    && !row.proof_supports_disposal().await?)
            {
                return Err(refused(
                    refusal::FILE_CHANGED,
                    "An adopted destination changed; remaining originals were preserved",
                ));
            }
        }
        for file in conflict
            .files
            .iter()
            .filter(|file| file.media && !conflict.dispose_paths.contains(&file.path))
        {
            let destination = resolutions
                .iter()
                .find(|row| row.source_path == file.path)
                .map(|row| row.destination_path.as_str())
                .unwrap_or(&file.path);
            let path = stored_path_to_path_buf(destination);
            super::title_folder_recovery::inspect_path(&path, false).await?;
            if destination == file.path
                && !preview_version_current(file, &resolutions, conflict).await?
            {
                return Err(refused(
                    refusal::FILE_CHANGED,
                    "Surviving media changed; inferior files were preserved",
                ));
            }
        }
        for path in &conflict.dispose_paths {
            if retained.contains(path) {
                continue;
            }
            let file = conflict
                .files
                .iter()
                .find(|file| &file.path == path)
                .ok_or_else(|| {
                    refused(
                        refusal::FOLDER_UNSAFE,
                        "Disposal target is outside the confirmed allowlist",
                    )
                })?;
            let source = stored_path_to_path_buf(path);
            let linked = conflict
                .hardlink_proofs
                .iter()
                .any(|proof| same_inode_version(&proof.version, &file.version));
            let mut disposal_index = resolutions
                .iter()
                .position(|row| row.is_disposal() && row.source_path == *path);
            match tokio::fs::symlink_metadata(&source).await {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(AppError::Repository(error.to_string())),
                Ok(metadata) => {
                    super::title_folder_recovery::inspect_path(&source, false).await?;
                    if !metadata.is_file()
                        || disposal_index.is_some_and(|index| resolutions[index].completed)
                        || !preview_version_current(file, &resolutions, conflict).await?
                    {
                        return Err(refused(
                            refusal::FILE_CHANGED,
                            "A disposal target changed; it was preserved",
                        ));
                    }
                    let config = self
                        .recycle_bin_config_for_media_root(Some(&file.root_path))
                        .await;
                    if !config.enabled
                        && (!conflict.permanent_delete_confirmed
                            || !conflict.permanent_delete_paths.contains(path))
                    {
                        return Err(refused(
                            refusal::PERMANENT_CONFIRMATION_REQUIRED,
                            "Permanent disposal was not confirmed for this file",
                        ));
                    }
                    if config.enabled {
                        let media_row = self
                            .services
                            .library
                            .media_files
                            .list_media_files_for_title(&planned.title_id)
                            .await?
                            .iter()
                            .find(|media| file.media_file_id.as_deref() == Some(media.id.as_str()))
                            .map(crate::recycle_bin::RecycledMediaRowSnapshot::from_media_file);
                        let manifest = crate::recycle_bin::RecycleManifest {
                            schema: None,
                            entry_id: None,
                            source_operation_id: Some(operation.id.clone()),
                            recycled_at: chrono::Utc::now().to_rfc3339(),
                            original_path: path.clone(),
                            original_file_id: file.media_file_id.clone(),
                            size_bytes: file.size_bytes,
                            title_id: Some(planned.title_id.clone()),
                            media_root: Some(file.root_path.clone()),
                            reason: "folder_conflict_resolution".into(),
                            status: None,
                            replacement_file_id: None,
                            replacement_path: None,
                            media_row,
                        };
                        if linked {
                            let prepared = match disposal_index
                                .and_then(|index| disposal_proof(&resolutions[index]))
                                .and_then(|proof| proof.recycle.clone())
                            {
                                Some(prepared) => prepared,
                                None if disposal_index.is_some() => {
                                    return Err(refused(
                                        refusal::RECYCLE_UNAVAILABLE,
                                        "The confirmed disposal mode changed; the file was preserved",
                                    ));
                                }
                                None => {
                                    crate::recycle_bin::prepare_conflict_recycle(
                                        &config, &source, manifest,
                                    )
                                    .await?
                                }
                            };
                            let prepared = crate::recycle_bin::renew_conflict_recycle(
                                &config, &source, &prepared,
                            )
                            .await?;
                            if let Some(index) = disposal_index {
                                let row = &mut resolutions[index];
                                let super::resolution::ResolutionDisposition::Disposal(proof) =
                                    &mut row.disposition
                                else {
                                    unreachable!()
                                };
                                if proof.recycle.as_ref() != Some(&prepared) {
                                    proof.recycle = Some(prepared.clone());
                                    row.destination_path = prepared.recycled_path.clone();
                                    self.services
                                        .library
                                        .location_operations
                                        .save_file_resolution(row)
                                        .await?;
                                }
                            }
                            let index = self
                                .record_conflict_disposal(
                                    operation,
                                    &planned.title_id,
                                    conflict,
                                    file,
                                    Some(prepared.clone()),
                                    &mut resolutions,
                                )
                                .await?;
                            disposal_index = Some(index);
                            crate::recycle_bin::execute_conflict_recycle(
                                &config,
                                &source,
                                &prepared,
                                resolutions[index]
                                    .source_version
                                    .as_deref()
                                    .expect("disposal version"),
                                resolutions[index]
                                    .identity_hash
                                    .as_deref()
                                    .expect("disposal hash"),
                            )
                            .await
                            .map_err(|error| {
                                refused(refusal::RECYCLE_UNAVAILABLE, error.to_string())
                            })?;
                        } else {
                            crate::recycle_bin::recycle_file(&config, &source, manifest)
                                .await
                                .map_err(|error| {
                                    refused(refusal::RECYCLE_UNAVAILABLE, error.to_string())
                                })?;
                        }
                    } else {
                        if linked {
                            if disposal_index
                                .and_then(|index| disposal_proof(&resolutions[index]))
                                .is_some_and(|proof| proof.recycle.is_some())
                            {
                                return Err(refused(
                                    refusal::RECYCLE_UNAVAILABLE,
                                    "The confirmed disposal mode changed; the file was preserved",
                                ));
                            }
                            disposal_index = Some(
                                self.record_conflict_disposal(
                                    operation,
                                    &planned.title_id,
                                    conflict,
                                    file,
                                    None,
                                    &mut resolutions,
                                )
                                .await?,
                            );
                            let index = disposal_index.expect("recorded disposal");
                            if super::resolution::file_version(&source).await?.as_str()
                                != resolutions[index]
                                    .source_version
                                    .as_deref()
                                    .expect("disposal version")
                            {
                                return Err(refused(
                                    refusal::FILE_CHANGED,
                                    "A disposal target changed after its intent was recorded; it was preserved",
                                ));
                            }
                        }
                        tokio::fs::remove_file(&source)
                            .await
                            .map_err(|error| AppError::Repository(error.to_string()))?;
                    }
                }
            }
            if let Some(index) = disposal_index {
                self.finish_conflict_disposal(conflict, &mut resolutions, index)
                    .await?;
            }
            if let Some(id) = &file.media_file_id
                && self
                    .services
                    .library
                    .media_files
                    .list_media_files_for_title(&planned.title_id)
                    .await?
                    .iter()
                    .any(|row| &row.id == id)
            {
                self.services
                    .library
                    .media_files
                    .delete_media_file(id)
                    .await?;
            }
        }
        for id in &conflict.pending_import_ids {
            if let Some(item) = self
                .services
                .library
                .library_scan_unmatched_items
                .get_library_scan_unmatched_item(id)
                .await?
            {
                if item.title_id.as_deref() != Some(&planned.title_id) {
                    return Err(refused(
                        refusal::OWNERSHIP_CHANGED,
                        "The pending conflict changed owner",
                    ));
                }
                self.services
                    .library
                    .library_scan_unmatched_items
                    .delete_library_scan_unmatched_item(
                        &item.library_id,
                        item.facet,
                        &item.item_path,
                    )
                    .await?;
            }
        }
        let _ = recycler;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod hardlink_disposal_tests {
    use super::super::resolution::{FileResolution, ResolutionDisposition};
    use super::*;

    #[tokio::test]
    async fn disposal_intent_accepts_only_its_own_unlink_and_historical_content() {
        for mutate_content in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("selected.mkv");
            let retained = temp.path().join("retained.mkv");
            let external = temp.path().join("external.mkv");
            std::fs::write(&source, b"historical movie").unwrap();
            std::fs::hard_link(&source, &retained).unwrap();
            std::fs::hard_link(&source, &external).unwrap();
            let source_path = path_to_stored_string(&source);
            let retained_path = path_to_stored_string(&retained);
            let version = super::super::resolution::file_version(&source)
                .await
                .unwrap();
            let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
            let mut row = FileResolution {
                operation_id: "synthetic-operation".into(),
                title_id: "synthetic-title".into(),
                source_path: source_path.clone(),
                original_destination: source_path.clone(),
                destination_path: source_path,
                disposition: ResolutionDisposition::Disposal(DisposalProof {
                    sequence: 1,
                    before_link_count: 3,
                    after_link_count: None,
                    after_versions: BTreeMap::from([(retained_path.clone(), version.clone())]),
                    recycle: None,
                }),
                completed: false,
                source_version: Some(version.clone()),
                destination_version: None,
                identity_hash: Some(conflict_hash(&retained_path, &version).await.unwrap()),
                warning: None,
                reason_code: None,
            };
            std::fs::remove_file(&source).unwrap();
            assert!(
                disposal_version_current(&retained_path, &version, &[row.clone()])
                    .await
                    .unwrap()
            );
            if mutate_content {
                // Same inode, length and restored mtime are insufficient: the
                // durable content hash must still match after the own unlink.
                std::fs::write(&external, b"modified content").unwrap();
                std::fs::File::options()
                    .write(true)
                    .open(&external)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(modified))
                    .unwrap();
                assert!(
                    !disposal_version_current(&retained_path, &version, &[row.clone()])
                        .await
                        .unwrap()
                );
            } else {
                let current = super::super::resolution::file_version(&retained)
                    .await
                    .unwrap();
                let ResolutionDisposition::Disposal(proof) = &mut row.disposition else {
                    unreachable!()
                };
                proof.after_versions = BTreeMap::from([(retained_path.clone(), current)]);
                proof.after_link_count = Some(2);
                row.completed = true;
                assert!(
                    disposal_version_current(&retained_path, &version, &[row.clone()])
                        .await
                        .unwrap()
                );
                // Reoccupation of the selected pathname is never another
                // disposal opportunity, even when the old inode is retained.
                std::fs::write(&source, b"replacement file").unwrap();
                assert!(
                    !disposal_version_current(&retained_path, &version, &[row])
                        .await
                        .unwrap()
                );
                assert_eq!(std::fs::read(&source).unwrap(), b"replacement file");
            }
            assert!(retained.exists());
            assert!(external.exists());
        }
    }
}
