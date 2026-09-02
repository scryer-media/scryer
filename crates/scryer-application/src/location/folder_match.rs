//! Folder-match correction: change which existing folder a title owns (US1).
//!
//! This is the one location workflow that never touches file content. It moves
//! catalog ownership of a directory from one title to another (or from nobody to
//! a title) and rebuilds the media associations derived from that ownership by
//! rescanning. Nothing on disk is created, renamed, moved, or deleted
//! (FR-002, FR-014, SC-001).
//!
//! The workflow is built on the folder-ownership seams in
//! [`crate::folder_ownership`]: a title's owned folder is a single stored path on
//! the title row, and every media association is derived from what a scan finds
//! under it. That makes the correction two commits and a rebuild rather than a
//! file operation.
//!
//! ## Atomicity (FR-008)
//!
//! The repositories are per-entity and expose no cross-entity transaction, so
//! swap and takeover are made atomic *from the user's perspective* by a
//! compensating transaction rather than a database one:
//!
//! 1. **Validate** everything that can be checked before any write.
//! 2. **Commit ownership** — one `folder_path` write per title. If the second
//!    write fails, the first is reverted, so no title is left holding a folder
//!    the other still claims.
//! 3. **Rebuild** — detach the associations that came from each title's former
//!    folder, then rescan. A failure here restores both `folder_path` values and
//!    rescans the original folders, which rebuilds the detached associations
//!    from the files still sitting untouched on disk.
//!
//! The rebuild is reversible precisely because associations are derived: the
//! files never moved, so rescanning the original folder reproduces the rows the
//! detach removed.

use std::path::Path;

use scryer_domain::{Library, LibraryRoot, MediaFacet, Title, User};
use serde::{Deserialize, Serialize};

use crate::catalog_workflow::library_path_is_under_root;
use crate::folder_ownership::{detach_title_media_in_folder, title_folder_path, title_owns_folder};
use crate::library_scan_unmatched::{
    LIBRARY_SCAN_FOLDER_OWNERSHIP_CHANGED_BY_USER, build_title_bound_unmatched_scan_item,
    clear_library_scan_unmatched_item, persist_library_scan_unmatched_item,
};
use crate::stored_paths::{
    path_to_stored_string, stored_path_is_within_folder, stored_path_to_path_buf,
};
use crate::{AppError, AppResult, AppUseCase, LibraryScanSummary};

/// How the selected folder relates to the title being edited (FR-002).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FolderMatchOwnership {
    /// No title in the library claims the folder.
    Unowned,
    /// The title being edited already owns it; selecting it is a no-op (FR-005).
    OwnedByThisTitle,
    /// Another title owns it; it is never taken silently (FR-006).
    OwnedByAnotherTitle,
}

impl FolderMatchOwnership {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unowned => "unowned",
            Self::OwnedByThisTitle => "owned_by_this_title",
            Self::OwnedByAnotherTitle => "owned_by_another_title",
        }
    }
}

/// How the user chose to settle the selected folder's ownership.
///
/// Modeled as one input rather than separate mutations so the caller sends the
/// same request shape in every case and the backend, not the client, decides
/// which resolutions a given folder actually admits. [`Self::Assign`] is the
/// default and is *rejected* against an owned folder, which is what makes
/// "reject" the default outcome of a conflict (FR-006).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FolderMatchResolution {
    /// Claim an unowned folder.
    #[default]
    Assign,
    /// Trade folders with the current owner; each title ends up with the other's
    /// former folder (FR-006).
    Swap,
    /// Take the folder; the former owner becomes unmatched and needs repair
    /// (FR-007).
    TakeOver,
}

impl FolderMatchResolution {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Assign => "assign",
            Self::Swap => "swap",
            Self::TakeOver => "take_over",
        }
    }
}

/// What actually happened when the change was applied.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FolderMatchOutcome {
    /// The title already owned the folder; nothing was submitted (FR-005).
    AlreadyOwned,
    /// An unowned folder became the title's folder (FR-003).
    Assigned,
    /// Two titles traded folders (FR-006).
    Swapped,
    /// The folder changed hands and the former owner is now unmatched (FR-007).
    TakenOver,
}

impl FolderMatchOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AlreadyOwned => "already_owned",
            Self::Assigned => "assigned",
            Self::Swapped => "swapped",
            Self::TakenOver => "taken_over",
        }
    }
}

/// Minimal identification of a title involved in the change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderMatchTitleRef {
    pub title_id: String,
    pub title_name: String,
    /// The folder this title owns, or `None` when it owns none.
    pub folder_path: Option<String>,
}

impl FolderMatchTitleRef {
    fn from_title(title: &Title) -> Self {
        Self {
            title_id: title.id.clone(),
            title_name: title.name.clone(),
            folder_path: title_folder_path(title).map(str::to_string),
        }
    }
}

/// The title left without a folder by a takeover, and how it surfaces for repair
/// (FR-007, SC-008).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisplacedTitleRepair {
    pub title_id: String,
    pub title_name: String,
    /// The folder it no longer owns.
    pub previous_folder_path: String,
    /// Reason code recorded on its unmatched-discovery item.
    pub repair_reason_code: String,
}

/// Everything the **Change folder** dialog must state before the user confirms
/// (FR-002).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeTitleFolderPreview {
    pub title: FolderMatchTitleRef,
    pub facet: MediaFacet,
    pub library_id: String,
    pub library_name: String,
    /// Root containing the title's current folder, when it has one.
    pub current_root_id: Option<String>,
    pub current_root_path: Option<String>,
    pub selected_folder_path: String,
    /// Root containing the selected folder. Always one of the title's current
    /// library roots — candidates outside them are rejected (FR-001).
    pub selected_root_id: String,
    pub selected_root_path: String,
    pub ownership: FolderMatchOwnership,
    /// The other title holding the selected folder, when there is one.
    pub current_owner: Option<FolderMatchTitleRef>,
    /// Tracked media rows the title currently has inside its existing folder.
    pub current_folder_tracked_media_count: u32,
    /// Tracked media rows inside the selected folder, counted across the title
    /// being edited and the selected folder's owner.
    pub selected_folder_tracked_media_count: u32,
    /// Always `false`. Stated explicitly because the dialog must say so
    /// (FR-002) and because every other location workflow can move files.
    pub files_will_move: bool,
    /// True when the selected folder is the one the title already owns; the UI
    /// explains and submits nothing (FR-005).
    pub no_op: bool,
    /// The resolutions this exact selection admits.
    pub available_resolutions: Vec<FolderMatchResolution>,
}

/// The applied result, including anything the user must now go repair.
///
/// Not serialized: this is a response, never a persisted record, and it carries
/// [`LibraryScanSummary`] straight through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeTitleFolderResult {
    pub outcome: FolderMatchOutcome,
    /// The edited title after the change.
    pub title: FolderMatchTitleRef,
    /// The folder the edited title owned before, when it owned one.
    pub previous_folder_path: Option<String>,
    /// Media associations detached because they came from a folder a title gave
    /// up. Counts both titles in a swap.
    pub detached_media_file_count: u32,
    /// Rescan of the edited title's new folder; `None` for a no-op.
    pub scan: Option<LibraryScanSummary>,
    /// The other title after a swap, with the folder it received.
    pub swapped_title: Option<FolderMatchTitleRef>,
    /// Rescan of the swapped title's new folder.
    pub swapped_title_scan: Option<LibraryScanSummary>,
    /// The title left unmatched by a takeover.
    pub displaced_title: Option<DisplacedTitleRepair>,
}

/// Everything both the preview and the apply need, resolved and validated once.
struct FolderMatchContext {
    title: Title,
    library: Library,
    selected_folder: String,
    selected_root: LibraryRoot,
    current_root: Option<LibraryRoot>,
    /// The other title owning `selected_folder`, when there is one.
    owner: Option<Title>,
}

impl FolderMatchContext {
    fn ownership(&self) -> FolderMatchOwnership {
        if title_owns_folder(&self.title, &stored_path_to_path_buf(&self.selected_folder)) {
            FolderMatchOwnership::OwnedByThisTitle
        } else if self.owner.is_some() {
            FolderMatchOwnership::OwnedByAnotherTitle
        } else {
            FolderMatchOwnership::Unowned
        }
    }

    fn current_folder(&self) -> Option<&str> {
        title_folder_path(&self.title)
    }

    fn available_resolutions(&self) -> Vec<FolderMatchResolution> {
        match self.ownership() {
            // Nothing to submit, so nothing to offer (FR-005).
            FolderMatchOwnership::OwnedByThisTitle => Vec::new(),
            FolderMatchOwnership::Unowned => vec![FolderMatchResolution::Assign],
            // Cancel is the third option and lives in the client: it is the
            // absence of a request, not a resolution the backend performs.
            FolderMatchOwnership::OwnedByAnotherTitle => {
                let mut resolutions = Vec::new();
                // Trading folders needs a folder to trade.
                if self.current_folder().is_some() {
                    resolutions.push(FolderMatchResolution::Swap);
                }
                resolutions.push(FolderMatchResolution::TakeOver);
                resolutions
            }
        }
    }
}

/// Count the media rows of `title_id` that live inside `folder`.
async fn tracked_media_count_in_folder(
    app: &AppUseCase,
    title_id: &str,
    folder: &str,
) -> AppResult<u32> {
    let count = app
        .services
        .library
        .media_files
        .list_media_files_for_title(title_id)
        .await?
        .into_iter()
        .filter(|media_file| stored_path_is_within_folder(folder, &media_file.file_path))
        .count();
    Ok(count as u32)
}

impl AppUseCase {
    async fn resolve_folder_match_context(
        &self,
        actor: &User,
        title_id: &str,
        folder_path: &str,
    ) -> AppResult<FolderMatchContext> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        // FR-083: management permission on the library that owns the title. Both
        // titles in a swap or takeover live in that same library, so one check
        // covers the whole operation.
        self.require_library_management_permission(actor, &title.library_id)
            .await?;

        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(&title.library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {}", title.library_id)))?;

        let selected_folder = folder_path.trim();
        if selected_folder.is_empty() {
            return Err(AppError::Validation("folder path is required".into()));
        }
        let selected_folder = path_to_stored_string(stored_path_to_path_buf(selected_folder));

        // FR-001: candidates are restricted to the title's current library roots.
        let selected_root = library
            .roots
            .iter()
            .find(|root| library_path_is_under_root(&selected_folder, &root.path))
            .cloned()
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "folder {} is not inside a root of library {}",
                    crate::stored_paths::stored_path_to_display_string(&selected_folder),
                    library.name
                ))
            })?;

        let current_root = title_folder_path(&title).and_then(|current| {
            library
                .roots
                .iter()
                .find(|root| library_path_is_under_root(current, &root.path))
                .cloned()
        });

        let owner =
            crate::folder_ownership::find_other_folder_owner(self, &title, &selected_folder)
                .await?;

        Ok(FolderMatchContext {
            title,
            library,
            selected_folder,
            selected_root,
            current_root,
            owner,
        })
    }

    /// Describe what changing this title's folder match would do (FR-002).
    ///
    /// Reads only; no ownership is changed and no work is submitted.
    pub async fn change_title_folder_preview(
        &self,
        actor: &User,
        title_id: &str,
        folder_path: &str,
    ) -> AppResult<ChangeTitleFolderPreview> {
        let context = self
            .resolve_folder_match_context(actor, title_id, folder_path)
            .await?;
        let ownership = context.ownership();

        let current_folder_tracked_media_count = match context.current_folder() {
            Some(current) => {
                tracked_media_count_in_folder(self, &context.title.id, current).await?
            }
            None => 0,
        };

        // Counted over the edited title and the selected folder's owner. Those
        // are the only two titles whose associations this workflow can touch;
        // sweeping the whole library would mean loading every media row in it to
        // fill in a dialog.
        let mut selected_folder_tracked_media_count =
            tracked_media_count_in_folder(self, &context.title.id, &context.selected_folder)
                .await?;
        if let Some(owner) = context.owner.as_ref() {
            selected_folder_tracked_media_count +=
                tracked_media_count_in_folder(self, &owner.id, &context.selected_folder).await?;
        }

        Ok(ChangeTitleFolderPreview {
            title: FolderMatchTitleRef::from_title(&context.title),
            facet: context.title.facet.clone(),
            library_id: context.library.id.clone(),
            library_name: context.library.name.clone(),
            current_root_id: context.current_root.as_ref().map(|root| root.id.clone()),
            current_root_path: context.current_root.as_ref().map(|root| root.path.clone()),
            selected_folder_path: context.selected_folder.clone(),
            selected_root_id: context.selected_root.id.clone(),
            selected_root_path: context.selected_root.path.clone(),
            ownership,
            current_owner: context.owner.as_ref().map(FolderMatchTitleRef::from_title),
            current_folder_tracked_media_count,
            selected_folder_tracked_media_count,
            // FR-002/FR-014: folder-match correction adopts an existing folder.
            files_will_move: false,
            no_op: ownership == FolderMatchOwnership::OwnedByThisTitle,
            available_resolutions: context.available_resolutions(),
        })
    }

    /// Apply a folder-match correction (FR-003, FR-005–FR-008).
    ///
    /// Only folder ownership and the media associations derived from it change:
    /// metadata identity, monitoring, quality settings, tags, history, and
    /// requests are never read or written here (FR-004).
    pub async fn apply_title_folder_change(
        &self,
        actor: &User,
        title_id: &str,
        folder_path: &str,
        resolution: FolderMatchResolution,
    ) -> AppResult<ChangeTitleFolderResult> {
        let context = self
            .resolve_folder_match_context(actor, title_id, folder_path)
            .await?;

        // FR-084: a running location operation's plan is built against the very
        // folder ownership this workflow rewrites. Both the edited title and,
        // in a conflict, the candidate folder's owner must be unowned.
        let mut guarded_titles = vec![crate::location::ownership_guard::OwnedEntity::Title(
            title_id.to_string(),
        )];
        if let Some(owner) = context.owner.as_ref() {
            guarded_titles.push(crate::location::ownership_guard::OwnedEntity::Title(
                owner.id.clone(),
            ));
        }
        self.ensure_location_ownership_allows(
            &crate::location::ownership_guard::FOLDER_MATCH_ENTRY,
            &guarded_titles,
        )
        .await?;

        match (context.ownership(), resolution) {
            // FR-005: an explicit no-op with an explanation, whatever the caller
            // asked for. Nothing is submitted.
            (FolderMatchOwnership::OwnedByThisTitle, _) => Ok(ChangeTitleFolderResult {
                outcome: FolderMatchOutcome::AlreadyOwned,
                title: FolderMatchTitleRef::from_title(&context.title),
                previous_folder_path: context.current_folder().map(str::to_string),
                detached_media_file_count: 0,
                scan: None,
                swapped_title: None,
                swapped_title_scan: None,
                displaced_title: None,
            }),
            (FolderMatchOwnership::Unowned, FolderMatchResolution::Assign) => {
                self.apply_folder_assignment(actor, context).await
            }
            (FolderMatchOwnership::Unowned, resolution) => Err(AppError::Validation(format!(
                "folder {} is not owned by another title; resolution {} does not apply",
                crate::stored_paths::stored_path_to_display_string(&context.selected_folder),
                resolution.as_str()
            ))),
            // FR-006: never silently stolen. The default resolution refuses and
            // names the owner so the caller can offer swap or take over.
            (FolderMatchOwnership::OwnedByAnotherTitle, FolderMatchResolution::Assign) => {
                let owner = context.owner.as_ref().expect("owner present");
                Err(AppError::Validation(format!(
                    "folder {} is already owned by title {}; choose swap or take over",
                    crate::stored_paths::stored_path_to_display_string(&context.selected_folder),
                    owner.name
                )))
            }
            (FolderMatchOwnership::OwnedByAnotherTitle, FolderMatchResolution::Swap) => {
                self.apply_folder_swap(actor, context).await
            }
            (FolderMatchOwnership::OwnedByAnotherTitle, FolderMatchResolution::TakeOver) => {
                self.apply_folder_takeover(actor, context).await
            }
        }
    }

    /// Write (or clear) a title's owned folder. The single commit primitive every
    /// path below uses, so compensation is always the same call in reverse.
    async fn commit_title_folder(&self, title_id: &str, folder: Option<&str>) -> AppResult<()> {
        match folder {
            Some(folder) => {
                self.services
                    .catalog
                    .titles
                    .set_folder_path(title_id, folder)
                    .await
            }
            None => {
                self.services
                    .catalog
                    .titles
                    .clear_folder_path(title_id)
                    .await
            }
        }
    }

    /// Put a title's folder back and rebuild what the failed attempt detached.
    ///
    /// Best-effort by construction: it runs while another error is already on
    /// its way to the caller, so a second failure is logged rather than
    /// replacing the original cause.
    async fn restore_title_folder(&self, actor: &User, title_id: &str, folder: Option<&str>) {
        if let Err(error) = self.commit_title_folder(title_id, folder).await {
            tracing::error!(
                title_id = %title_id,
                %error,
                "failed to restore title folder ownership after a folder-match failure"
            );
            return;
        }
        if folder.is_none() {
            return;
        }
        if let Err(error) = self.scan_title_library(actor, title_id).await {
            tracing::error!(
                title_id = %title_id,
                %error,
                "restored title folder ownership but could not rebuild its media associations"
            );
        }
    }

    /// FR-003: an unowned destination.
    async fn apply_folder_assignment(
        &self,
        actor: &User,
        context: FolderMatchContext,
    ) -> AppResult<ChangeTitleFolderResult> {
        let previous_folder = context.current_folder().map(str::to_string);
        let title_id = context.title.id.clone();

        self.commit_title_folder(&title_id, Some(&context.selected_folder))
            .await?;

        let mut detached = 0;
        if let Some(previous) = previous_folder.as_deref() {
            detached =
                detach_title_media_in_folder(self, &title_id, &stored_path_to_path_buf(previous))
                    .await?;
        }

        // The folder now has an owner, so it is no longer awaiting a match. The
        // old folder is left unowned, which is what puts it back in front of
        // unmatched discovery on the next scan (FR-003).
        self.clear_folder_match_unmatched_item(&context, &context.selected_folder)
            .await?;

        let scan = match self.scan_title_library(actor, &title_id).await {
            Ok(scan) => scan,
            Err(error) => {
                self.restore_title_folder(actor, &title_id, previous_folder.as_deref())
                    .await;
                return Err(error);
            }
        };

        let title = self.reload_folder_match_title(&title_id).await?;
        Ok(ChangeTitleFolderResult {
            outcome: FolderMatchOutcome::Assigned,
            title: FolderMatchTitleRef::from_title(&title),
            previous_folder_path: previous_folder,
            detached_media_file_count: detached,
            scan: Some(scan),
            swapped_title: None,
            swapped_title_scan: None,
            displaced_title: None,
        })
    }

    /// FR-006 + FR-008: both titles end up owning the other's former folder, or
    /// neither changes.
    async fn apply_folder_swap(
        &self,
        actor: &User,
        context: FolderMatchContext,
    ) -> AppResult<ChangeTitleFolderResult> {
        let owner = context.owner.as_ref().expect("owner present").clone();
        let title_folder = context
            .current_folder()
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "title {} owns no folder to swap; take over the folder instead",
                    context.title.name
                ))
            })?
            .to_string();
        let owner_folder = title_folder_path(&owner)
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "title {} was reported as the owner of {} but owns no folder",
                    owner.name, context.selected_folder
                ))
            })?
            .to_string();

        let title_id = context.title.id.clone();
        let owner_id = owner.id.clone();

        // Commit both sides. A failure on the second write rolls the first back,
        // so the two titles never both claim one folder.
        self.commit_title_folder(&title_id, Some(&owner_folder))
            .await?;
        if let Err(error) = self
            .commit_title_folder(&owner_id, Some(&title_folder))
            .await
        {
            self.restore_title_folder(actor, &title_id, Some(&title_folder))
                .await;
            return Err(error);
        }

        let rebuild = self
            .rebuild_swapped_titles(actor, &title_id, &title_folder, &owner_id, &owner_folder)
            .await;
        let (detached, scan, owner_scan) = match rebuild {
            Ok(rebuilt) => rebuilt,
            Err(error) => {
                self.restore_title_folder(actor, &title_id, Some(&title_folder))
                    .await;
                self.restore_title_folder(actor, &owner_id, Some(&owner_folder))
                    .await;
                return Err(error);
            }
        };

        let title = self.reload_folder_match_title(&title_id).await?;
        let owner = self.reload_folder_match_title(&owner_id).await?;
        Ok(ChangeTitleFolderResult {
            outcome: FolderMatchOutcome::Swapped,
            title: FolderMatchTitleRef::from_title(&title),
            previous_folder_path: Some(title_folder),
            detached_media_file_count: detached,
            scan: Some(scan),
            swapped_title: Some(FolderMatchTitleRef::from_title(&owner)),
            swapped_title_scan: Some(owner_scan),
            displaced_title: None,
        })
    }

    /// Detach both titles' associations to the folders they gave up, then rescan
    /// both. Any error aborts before the caller commits to the new state.
    async fn rebuild_swapped_titles(
        &self,
        actor: &User,
        title_id: &str,
        title_folder: &str,
        owner_id: &str,
        owner_folder: &str,
    ) -> AppResult<(u32, LibraryScanSummary, LibraryScanSummary)> {
        let mut detached =
            detach_title_media_in_folder(self, title_id, &stored_path_to_path_buf(title_folder))
                .await?;
        detached +=
            detach_title_media_in_folder(self, owner_id, &stored_path_to_path_buf(owner_folder))
                .await?;
        let scan = self.scan_title_library(actor, title_id).await?;
        let owner_scan = self.scan_title_library(actor, owner_id).await?;
        Ok((detached, scan, owner_scan))
    }

    /// FR-007 + FR-008: the edited title takes the folder; the former owner is
    /// left unmatched and surfaced for repair.
    async fn apply_folder_takeover(
        &self,
        actor: &User,
        context: FolderMatchContext,
    ) -> AppResult<ChangeTitleFolderResult> {
        let owner = context.owner.as_ref().expect("owner present").clone();
        let owner_folder = title_folder_path(&owner)
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "title {} was reported as the owner of {} but owns no folder",
                    owner.name, context.selected_folder
                ))
            })?
            .to_string();
        let previous_folder = context.current_folder().map(str::to_string);
        let title_id = context.title.id.clone();
        let owner_id = owner.id.clone();

        // Release before claiming: the intermediate state is "nobody owns it",
        // never "two titles own it".
        self.commit_title_folder(&owner_id, None).await?;
        if let Err(error) = self
            .commit_title_folder(&title_id, Some(&context.selected_folder))
            .await
        {
            self.restore_title_folder(actor, &owner_id, Some(&owner_folder))
                .await;
            return Err(error);
        }

        let rebuild = self
            .rebuild_taken_over_title(
                actor,
                &title_id,
                previous_folder.as_deref(),
                &owner_id,
                &owner_folder,
            )
            .await;
        let (detached, scan) = match rebuild {
            Ok(rebuilt) => rebuilt,
            Err(error) => {
                self.restore_title_folder(actor, &title_id, previous_folder.as_deref())
                    .await;
                self.restore_title_folder(actor, &owner_id, Some(&owner_folder))
                    .await;
                return Err(error);
            }
        };

        // Clearing first is load-bearing, not tidying: the unmatched store keeps
        // an existing row's `ignored` status when a pending row is upserted over
        // it, so a folder someone had previously ignored would swallow the
        // repair item the displaced title needs.
        self.clear_folder_match_unmatched_item(&context, &context.selected_folder)
            .await?;
        self.record_displaced_title_repair(&context, &owner, &owner_folder)
            .await?;

        let title = self.reload_folder_match_title(&title_id).await?;
        Ok(ChangeTitleFolderResult {
            outcome: FolderMatchOutcome::TakenOver,
            title: FolderMatchTitleRef::from_title(&title),
            previous_folder_path: previous_folder,
            detached_media_file_count: detached,
            scan: Some(scan),
            swapped_title: None,
            swapped_title_scan: None,
            displaced_title: Some(DisplacedTitleRepair {
                title_id: owner.id,
                title_name: owner.name,
                previous_folder_path: owner_folder,
                repair_reason_code: LIBRARY_SCAN_FOLDER_OWNERSHIP_CHANGED_BY_USER.to_string(),
            }),
        })
    }

    async fn rebuild_taken_over_title(
        &self,
        actor: &User,
        title_id: &str,
        previous_folder: Option<&str>,
        displaced_id: &str,
        taken_folder: &str,
    ) -> AppResult<(u32, LibraryScanSummary)> {
        // The displaced title's rows point into a folder it no longer owns.
        let mut detached = detach_title_media_in_folder(
            self,
            displaced_id,
            &stored_path_to_path_buf(taken_folder),
        )
        .await?;
        if let Some(previous) = previous_folder {
            detached +=
                detach_title_media_in_folder(self, title_id, &stored_path_to_path_buf(previous))
                    .await?;
        }
        let scan = self.scan_title_library(actor, title_id).await?;
        Ok((detached, scan))
    }

    async fn reload_folder_match_title(&self, title_id: &str) -> AppResult<Title> {
        self.services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))
    }

    async fn clear_folder_match_unmatched_item(
        &self,
        context: &FolderMatchContext,
        folder: &str,
    ) -> AppResult<()> {
        clear_library_scan_unmatched_item(self, &context.title.facet, &context.library.id, folder)
            .await
    }

    /// Put the displaced title into the unmatched/repair experience with the
    /// documented reason (FR-007, SC-008).
    async fn record_displaced_title_repair(
        &self,
        context: &FolderMatchContext,
        displaced: &Title,
        displaced_folder: &str,
    ) -> AppResult<()> {
        let display_name =
            folder_display_name(displaced_folder).unwrap_or_else(|| displaced.name.clone());
        let item = build_title_bound_unmatched_scan_item(
            &displaced.facet,
            &context.library.id,
            &displaced.id,
            None,
            &context.selected_root.path,
            displaced_folder,
            &display_name,
            &displaced.name,
            displaced.year.map(|year| year as u32),
            LIBRARY_SCAN_FOLDER_OWNERSHIP_CHANGED_BY_USER,
            // Folder-shaped, so there is no single file size to record.
            None,
        );
        persist_library_scan_unmatched_item(self, &item).await?;
        tracing::info!(
            title_id = %displaced.id,
            folder_path = %displaced_folder,
            reason_code = LIBRARY_SCAN_FOLDER_OWNERSHIP_CHANGED_BY_USER,
            "title displaced by a folder takeover now needs repair"
        );
        Ok(())
    }
}

fn folder_display_name(folder: &str) -> Option<String> {
    Path::new(&stored_path_to_path_buf(folder))
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assign_is_the_default_resolution_so_owned_folders_reject_by_default() {
        assert_eq!(
            FolderMatchResolution::default(),
            FolderMatchResolution::Assign
        );
    }

    #[test]
    fn resolution_and_outcome_wire_values_are_stable() {
        assert_eq!(FolderMatchResolution::Swap.as_str(), "swap");
        assert_eq!(FolderMatchResolution::TakeOver.as_str(), "take_over");
        assert_eq!(FolderMatchOutcome::AlreadyOwned.as_str(), "already_owned");
        assert_eq!(
            FolderMatchOwnership::OwnedByAnotherTitle.as_str(),
            "owned_by_another_title"
        );
    }

    #[test]
    fn folder_display_name_uses_the_leaf_directory() {
        assert_eq!(
            folder_display_name("/library/Movies/Some Movie (2024)"),
            Some("Some Movie (2024)".to_string())
        );
    }
}
