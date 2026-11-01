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
            if !crate::stored_paths::stored_path_is_native(folder) {
                return Err(refused(
                    refusal::FOLDER_UNSAFE,
                    "The folder path uses another platform's native encoding; its files were preserved",
                ));
            }
            let root = library
                .roots
                .iter()
                .find(|root| {
                    crate::stored_paths::stored_path_is_native(&root.path)
                        && crate::title_folder_rules::path_is_strictly_within(folder, &root.path)
                })
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
        if !dispose.is_empty() {
            for file in &files {
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
