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
    pub const ALL: &[&str] = &[
        SELECTION_INVALID,
        NOT_FOUND,
        UNSUPPORTED,
        FOLDER_UNSAFE,
        OWNERSHIP_CHANGED,
        MEDIA_MISSING,
        PLAN_CHANGED,
        PERMANENT_CONFIRMATION_REQUIRED,
        FILE_CHANGED,
        RECYCLE_UNAVAILABLE,
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

fn overlaps(a: &str, b: &str) -> bool {
    folder_paths_match(a, b)
        || crate::title_folder_rules::path_is_strictly_within(a, b)
        || crate::title_folder_rules::path_is_strictly_within(b, a)
}

/// Reuse the release parser's title and metadata identities; explicit disagreement loses.
fn folder_name_rank(title: &Title, folder: &str) -> i8 {
    let path = stored_path_to_path_buf(folder);
    let raw = path.file_name().unwrap_or_default().to_string_lossy();
    let parsed = crate::release_parser::parse_release_metadata(&raw);
    let mut identity_match = false;
    for (source, value) in [
        ("tmdb", parsed.tmdb_id.as_deref()),
        ("imdb", parsed.imdb_id.as_deref()),
        ("tvdb", parsed.tvdb_id.as_deref()),
    ] {
        if let Some(value) = value {
            let expected = title
                .external_ids
                .iter()
                .find(|id| id.source.eq_ignore_ascii_case(source))
                .map(|id| id.value.as_str())
                .or_else(|| {
                    (source == "imdb")
                        .then_some(title.imdb_id.as_deref())
                        .flatten()
                });
            if let Some(expected) = expected {
                if expected != value {
                    return -1;
                }
                identity_match = true;
            } else {
                return -1;
            }
        }
    }
    if identity_match {
        return 1;
    }
    let normalize = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let target = normalize(&title.name);
    let matches = normalize(&parsed.normalized_title) == target
        || parsed
            .normalized_title_variants
            .iter()
            .any(|name| normalize(name) == target)
        || title
            .aliases
            .iter()
            .any(|name| normalize(name) == normalize(&parsed.normalized_title));
    if matches
        && parsed
            .year
            .is_none_or(|year| title.year.is_none_or(|expected| year == expected))
    {
        return 1;
    }
    // A recognizable release or year identifies a different work, never junk.
    if (parsed.year.is_some() || parsed.quality.is_some() || parsed.source.is_some())
        && !parsed.normalized_title.is_empty()
    {
        -1
    } else {
        0
    }
}

async fn enumerate(folder: &str, root: &str) -> AppResult<(Vec<ConflictFile>, Vec<String>)> {
    let base = stored_path_to_path_buf(folder);
    if base.to_str().is_none() {
        return Err(refused(
            refusal::FOLDER_UNSAFE,
            "The folder path cannot be safely represented by the collision planner; its files were preserved",
        ));
    }
    super::title_folder_recovery::inspect_path(&base, false).await?;
    let mut pending = vec![base.clone()];
    let mut files = Vec::new();
    let mut directories = Vec::new();
    while let Some(path) = pending.pop() {
        let mut entries = tokio::fs::read_dir(&path)
            .await
            .map_err(|e| AppError::Repository(e.to_string()))?;
        directories.push(path_to_stored_string(&path));
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AppError::Repository(e.to_string()))?
        {
            if files.len() + directories.len() + pending.len() >= MAX_FILES {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "The folder exceeds the safe preview bound",
                ));
            }
            let path = entry.path();
            if path.to_str().is_none() {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "This folder contains a filename that cannot be safely represented by the collision planner; its files were preserved",
                ));
            }
            let metadata = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|e| AppError::Repository(e.to_string()))?;
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
            item.item_path.clone(),
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
                folders.push(sibling.item_path);
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
        for folder in &folders {
            let root = library
                .roots
                .iter()
                .find(|root| crate::title_folder_rules::path_is_strictly_within(folder, &root.path))
                .ok_or_else(|| {
                    refused(
                        refusal::FOLDER_UNSAFE,
                        "Every conflict folder must be strictly within this library's roots",
                    )
                })?;
            if folders
                .iter()
                .any(|other| folder != other && overlaps(folder, other))
            {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "Conflict folders must not overlap",
                ));
            }
            if catalog.owners.iter().any(|owner| {
                owner.id != title.id
                    && owner
                        .folder_path
                        .as_deref()
                        .is_some_and(|owned| overlaps(folder, owned))
            }) || catalog.media.iter().any(|media| {
                media.title_id != title.id
                    && crate::title_folder_rules::path_is_strictly_within(&media.file_path, folder)
            }) {
                return Err(refused(
                    refusal::OWNERSHIP_CHANGED,
                    "Another title owns content in a conflict folder",
                ));
            }
            let (mut entries, mut dirs) = enumerate(folder, &root.path).await?;
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
        let destination_root = library
            .roots
            .iter()
            .find(|root| crate::title_folder_rules::path_is_strictly_within(&survivor, &root.path))
            .expect("validated folder root");
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
                if !file.media {
                    if let Some(media) = moving
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
                }
                item
            })
            .collect();
        let collision = plan_collisions(
            &CollisionPlanRequest::new(
                PathCaseRule::platform_default(),
                CollisionNaming::from_source_library("duplicate"),
            )
            .with_destination(destination)
            .with_incoming(incoming),
        );
        let execution_files = moving
            .iter()
            .map(|file| RootMoveFileExecution {
                media_file_id: file.media_file_id.clone().or_else(|| {
                    file.media.then(|| {
                        format!(
                            "conflict-media:{}",
                            blake3::hash(file.path.as_bytes()).to_hex()
                        )
                    })
                }),
                source_path: file.path.clone(),
                destination_path: path_to_stored_string(
                    stored_path_to_path_buf(&survivor).join(
                        &collision
                            .decision(&file.path)
                            .expect("planned incoming file")
                            .final_name,
                    ),
                ),
                size_bytes: file.size_bytes,
                source_content: tracked
                    .iter()
                    .find(|media| media.file_path == file.path)
                    .and_then(super::model::KnownSourceContent::from_media_file),
            })
            .collect::<Vec<_>>();
        let mut permanent_delete_paths = Vec::new();
        for path in &dispose {
            let file = files
                .iter()
                .find(|file| &file.path == path)
                .expect("allowlisted file");
            if super::hardlinks::link_count(&stored_path_to_path_buf(path))?.is_hardlinked() {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "A file selected for disposal has other hardlinks; preserve all files or resolve those links before disposing of duplicates",
                ));
            }
            if !self
                .recycle_bin_config_for_media_root(Some(&file.root_path))
                .await
                .enabled
            {
                permanent_delete_paths.push(path.clone());
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
                stored_path_to_path_buf(&file.source_path).file_name()
                    != stored_path_to_path_buf(&file.destination_path).file_name()
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
    stored_path_to_path_buf(path)
        .strip_prefix(stored_path_to_path_buf(folder))
        .expect("validated descendant")
        .to_string_lossy()
        .into_owned()
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

async fn preview_version_current(
    file: &ConflictFile,
    resolutions: &[super::resolution::FileResolution],
) -> AppResult<bool> {
    let actual = super::resolution::file_version(&stored_path_to_path_buf(&file.path)).await?;
    if actual == file.version {
        return Ok(true);
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
            match self
                .plan_pending_import_conflict(actor, id, request, &catalog)
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
                    let reason = error
                        .validation_reason()
                        .unwrap_or(refusal::UNSUPPORTED)
                        .to_owned();
                    fingerprints.push((id.clone(), reason.clone()));
                    rows.push(ConflictRow {
                        pending_import_id: id.clone(),
                        title_id: String::new(),
                        title_name: String::new(),
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
        let preview = ConflictPreview {
            plan_fingerprint: fingerprint,
            total_count: rows.len() as i32,
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
            if !catalog.roots.iter().any(|root| {
                root.library_id == title.library_id
                    && crate::title_folder_rules::path_is_strictly_within(folder, &root.path)
            }) || catalog.owners.iter().any(|owner| {
                owner.id != title.id
                    && owner
                        .folder_path
                        .as_deref()
                        .is_some_and(|other| overlaps(folder, other))
            }) || catalog.media.iter().any(|media| {
                media.title_id != title.id
                    && crate::title_folder_rules::path_is_strictly_within(&media.file_path, folder)
            }) {
                return Err(refused(
                    refusal::OWNERSHIP_CHANGED,
                    "Conflict folder ownership or roots changed",
                ));
            }
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
                    .filter(|row| row.completed)
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
                && resolution.proof_is_current().await
            {
                continue;
            }
            let path = stored_path_to_path_buf(&file.path);
            match tokio::fs::symlink_metadata(&path).await {
                Ok(_) => {
                    super::title_folder_recovery::inspect_path(&path, false).await?;
                    if !preview_version_current(file, &resolutions).await? {
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
        for row in &resolutions {
            if !row.proof_is_current().await {
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
            .set_folder_path(
                &planned.title_id,
                planned
                    .destination_folder_path
                    .as_deref()
                    .expect("survivor folder"),
            )
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
        let resolutions = self
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
            .filter(|row| row.completed)
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
        for row in &resolutions {
            if !row.proof_is_current().await
                || (disposal_remaining && !row.proof_supports_disposal().await?)
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
            if destination == file.path && !preview_version_current(file, &resolutions).await? {
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
            match tokio::fs::symlink_metadata(&source).await {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(AppError::Repository(error.to_string())),
                Ok(metadata) => {
                    super::title_folder_recovery::inspect_path(&source, false).await?;
                    if !metadata.is_file()
                        || super::resolution::file_version(&source).await? != file.version
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
                        crate::recycle_bin::recycle_file(&config, &source, manifest)
                            .await
                            .map_err(|error| {
                                refused(refusal::RECYCLE_UNAVAILABLE, error.to_string())
                            })?;
                    } else {
                        tokio::fs::remove_file(&source)
                            .await
                            .map_err(|error| AppError::Repository(error.to_string()))?;
                    }
                }
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
