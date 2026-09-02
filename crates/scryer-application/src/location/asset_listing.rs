//! Per-file asset listing for one operation's Activity detail (T090, FR-091,
//! US8.1/US8.4).
//!
//! Activity's counters answer "how many files were renamed or deduplicated".
//! This module answers "which ones", which is the half FR-091's "nothing
//! silent" actually needs: the preview named every collision rename and every
//! proven duplicate before the user confirmed, and the operation detail has to
//! be able to name the same files afterwards.
//!
//! # Where the identities live
//!
//! Nothing new is persisted for this. The confirmed
//! [`RootMoveExecutionPlan`] already carries, per title, the destination paths
//! the collision planner renamed ([`RootMoveTitleExecution::renamed_destinations`])
//! and the source paths it proved redundant
//! ([`RootMoveTitleExecution::deduplicated_sources`]); the plan's file list
//! carries the source path each renamed destination came from. So the listing
//! is a pure read of the operation's stored plan, joined against its
//! checkpoints.
//!
//! # Planned is not done
//!
//! A plan states what *will* happen. A canceled, failed, or still-running title
//! has plan facts that never became outcomes, and presenting them as history
//! would be a quieter version of the silence FR-091 forbids: the user would
//! read "renamed to X" about a file that was never touched.
//!
//! So every entry is tagged with whether its title settled, using exactly the
//! rule [`crate::location::executor`] applies when it recomputes the FR-091
//! counters: only `Completed` and `CompletedWithWarnings` count. Everything
//! else is listed as planned. The listing and the counters therefore agree by
//! construction rather than by coincidence, and a resume that re-settles a
//! title flips the same entries the counters move.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::AppResult;
use crate::location::model::{TitleCheckpoint, TitleCheckpointState};
use crate::location::root_move::{RootMoveExecutionPlan, RootMoveTitleExecution};
use crate::services::AppUseCase;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};

/// One file the collision planner landed under a different name so destination
/// content could keep its own (FR-074/075).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedAsset {
    /// Stored path of the source file, or `None` when the stored plan no longer
    /// carries the file this destination came from.
    pub source_path: Option<String>,
    /// File name of that source path.
    pub source_name: Option<String>,
    /// Stored path the file lands under.
    pub destination_path: String,
    /// File name it lands under.
    pub destination_name: String,
    /// The label inside the `(from <Label>)` suffix, when the rename used one.
    /// Numeric-only disambiguation carries no label.
    pub provenance_label: Option<String>,
    /// Tracked media file, or `None` for a companion asset.
    pub media_file_id: Option<String>,
    pub size_bytes: u64,
    /// Whether the title carrying this rename actually settled.
    pub done: bool,
}

/// One source file proven redundant against identical destination content, so
/// it is recycled rather than copied (FR-073).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeduplicatedAsset {
    /// Stored path of the redundant source copy.
    pub source_path: String,
    /// Its file name.
    pub source_name: String,
    /// Stored path of the destination copy that survives, when the plan carries
    /// enough placement to name it.
    ///
    /// The plan records the redundant source and drops the file from its work
    /// list, so the survivor is reconstructed by relocating the source's
    /// folder-relative path onto the destination folder, which is how the
    /// planner built it in the first place.
    pub surviving_path: Option<String>,
    /// File name of that survivor.
    pub surviving_name: Option<String>,
    /// Whether the title carrying this dedup actually settled.
    pub done: bool,
}

/// One title's renames and dedups, with the settled fact that separates history
/// from intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleAssetListing {
    pub title_id: String,
    pub title_name: String,
    /// Position in the confirmed plan.
    pub sequence: i64,
    /// True only for `Completed` / `CompletedWithWarnings`, the same rule the
    /// executor's counters use.
    pub settled: bool,
    /// The title's checkpoint state, or `None` when it has not entered the run.
    pub checkpoint_state: Option<TitleCheckpointState>,
    pub renames: Vec<RenamedAsset>,
    pub dedups: Vec<DeduplicatedAsset>,
}

impl TitleAssetListing {
    fn is_empty(&self) -> bool {
        self.renames.is_empty() && self.dedups.is_empty()
    }
}

/// Every per-file rename and dedup one operation's confirmed plan carries,
/// split by title and by done-versus-planned.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocationOperationAssetListing {
    pub operation_id: String,
    /// Titles carrying at least one rename or dedup, in confirmed-plan order.
    /// A title with neither has nothing to list and is left out.
    pub titles: Vec<TitleAssetListing>,
    /// Renames the plan carries, across every title.
    pub renames_total: i64,
    /// How many of those belong to a title that settled.
    pub renames_done: i64,
    /// Dedups the plan carries, across every title.
    pub dedups_total: i64,
    /// How many of those belong to a title that settled.
    pub dedups_done: i64,
}

impl LocationOperationAssetListing {
    /// An operation with no readable plan lists nothing rather than failing:
    /// the counters beside it are still true, and a listing that cannot be
    /// built is not a reason to refuse the whole Activity detail.
    pub fn empty(operation_id: &str) -> Self {
        Self {
            operation_id: operation_id.to_string(),
            ..Self::default()
        }
    }
}

/// Whether a checkpoint state means the title's plan facts became outcomes.
///
/// Mirrors the executor's counter rule exactly (see
/// `OperationProgress::counters`): a blocked, skipped, failed, or still-moving
/// title has done none of its plan's dedup and rename work.
fn settled(state: Option<TitleCheckpointState>) -> bool {
    matches!(
        state,
        Some(TitleCheckpointState::Completed) | Some(TitleCheckpointState::CompletedWithWarnings)
    )
}

/// The asset listing for one confirmed plan, given the checkpoints as they
/// stand right now.
///
/// Pure: no IO, no clock, no catalog. The caller supplies the plan it read back
/// and the checkpoints it read back, and the same two inputs always produce the
/// same listing.
pub fn build_asset_listing(
    operation_id: &str,
    plan: &RootMoveExecutionPlan,
    checkpoints: &[TitleCheckpoint],
) -> LocationOperationAssetListing {
    let states: BTreeMap<&str, TitleCheckpointState> = checkpoints
        .iter()
        .map(|checkpoint| (checkpoint.title_id.as_str(), checkpoint.state))
        .collect();

    let mut listing = LocationOperationAssetListing::empty(operation_id);
    let mut titles: Vec<&RootMoveTitleExecution> = plan.titles.iter().collect();
    // The runner walks ascending sequence; the listing reads in the same order
    // so a row's position means the same thing in both places.
    titles.sort_by_key(|title| title.sequence);

    for title in titles {
        let state = states.get(title.title_id.as_str()).copied();
        let done = settled(state);
        let renames = renames_for(title, done);
        let dedups = dedups_for(title, done);

        listing.renames_total += renames.len() as i64;
        listing.dedups_total += dedups.len() as i64;
        if done {
            listing.renames_done += renames.len() as i64;
            listing.dedups_done += dedups.len() as i64;
        }

        let entry = TitleAssetListing {
            title_id: title.title_id.clone(),
            title_name: title.title_name.clone(),
            sequence: title.sequence,
            settled: done,
            checkpoint_state: state,
            renames,
            dedups,
        };
        if !entry.is_empty() {
            listing.titles.push(entry);
        }
    }

    listing
}

fn renames_for(title: &RootMoveTitleExecution, done: bool) -> Vec<RenamedAsset> {
    title
        .renamed_destinations
        .iter()
        .map(|destination_path| {
            // The rename branch of the planner keeps the file in the work list,
            // so its source is normally right here. A plan that lost the file
            // still names the destination rather than dropping the row.
            let file = title
                .files
                .iter()
                .find(|file| &file.destination_path == destination_path);
            let destination_name = crate::stored_paths::stored_file_name(destination_path);
            let source_name = file.map(|file| crate::stored_paths::stored_file_name(&file.source_path));
            RenamedAsset {
                provenance_label: collision_provenance_label(
                    source_name.as_deref(),
                    &destination_name,
                ),
                source_path: file.map(|file| file.source_path.clone()),
                source_name,
                destination_path: destination_path.clone(),
                destination_name,
                media_file_id: file.and_then(|file| file.media_file_id.clone()),
                size_bytes: file.map(|file| file.size_bytes).unwrap_or(0),
                done,
            }
        })
        .collect()
}

fn dedups_for(title: &RootMoveTitleExecution, done: bool) -> Vec<DeduplicatedAsset> {
    title
        .deduplicated_sources
        .iter()
        .map(|source_path| {
            let surviving_path = surviving_destination(title, source_path);
            DeduplicatedAsset {
                source_name: crate::stored_paths::stored_file_name(source_path),
                source_path: source_path.clone(),
                surviving_name: surviving_path.as_deref().map(crate::stored_paths::stored_file_name),
                surviving_path,
                done,
            }
        })
        .collect()
}

/// Where the destination copy that survives a dedup lives.
///
/// The planner placed every incoming file at
/// `<destination folder>/<path relative to the title's folder>`, and a dedup is
/// by definition a file whose name already existed there, so relocating the
/// source's folder-relative path is the same construction the plan used. A file
/// tracked outside its title's folder lands in the destination folder root,
/// which is the fallback here too.
fn surviving_destination(title: &RootMoveTitleExecution, source_path: &str) -> Option<String> {
    let destination_folder = stored_path_to_path_buf(title.destination_folder_path.as_deref()?);
    let source = stored_path_to_path_buf(source_path);
    let relative: Option<PathBuf> = title
        .source_folder_path
        .as_deref()
        .map(stored_path_to_path_buf)
        .and_then(|folder| {
            source
                .strip_prefix(&folder)
                .ok()
                .map(|relative| relative.to_path_buf())
        })
        .filter(|relative| !relative.as_os_str().is_empty());
    let destination = match relative {
        Some(relative) => destination_folder.join(relative),
        None => destination_folder.join(source.file_name()?),
    };
    Some(path_to_stored_string(destination))
}

/// Split a filename into (stem, extension-with-dot), the way the collision
/// planner does: a leading dot belongs to the stem, so `.plexmatch` has no
/// extension.
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    }
}

const PROVENANCE_MARKER: &str = " (from ";

/// The label inside the `"<stem> (from <Label>)<.ext>"` suffix FR-074 appends,
/// or `None` when the rename carried no such suffix (numeric-only
/// disambiguation, or a case-only correction).
///
/// Read off the destination name rather than stored: the label is the source
/// library's sanitized name at planning time, and the name the file actually
/// landed under is the authority on what the user will see on disk.
fn collision_provenance_label(source_name: Option<&str>, destination_name: &str) -> Option<String> {
    if let Some(source_name) = source_name {
        let (stem, _) = split_name(source_name);
        let prefix = format!("{stem}{PROVENANCE_MARKER}");
        if let Some(rest) = destination_name.strip_prefix(prefix.as_str()) {
            return label_before_close(rest);
        }
    }
    // A companion asset follows its media file's renamed stem rather than its
    // own (FR-075), so the suffix sits somewhere other than after this file's
    // stem. Fall back to reading the first suffix in the name; this only runs
    // for files the plan already recorded as renamed.
    let index = destination_name.find(PROVENANCE_MARKER)?;
    label_before_close(&destination_name[index + PROVENANCE_MARKER.len()..])
}

fn label_before_close(rest: &str) -> Option<String> {
    let end = rest.find(')')?;
    let label = rest[..end].trim();
    (!label.is_empty()).then(|| label.to_string())
}

impl AppUseCase {
    /// The rename and dedup asset listing for one operation, read from the plan
    /// it confirmed and the checkpoints it has written so far (FR-091).
    ///
    /// The plan JSON is the only place the per-file identities live, so this
    /// reads it through the operation repository rather than rebuilding a
    /// preview: a rebuilt preview would describe the filesystem as it is now,
    /// not the decisions the user confirmed and the runner carried out.
    ///
    /// An operation stored without a plan, or with one this build cannot read,
    /// lists nothing rather than failing. That is the same tolerance resume
    /// applies: an unreadable plan is a missing detail, not a broken operation.
    pub async fn location_operation_asset_listing(
        &self,
        operation_id: &str,
    ) -> AppResult<LocationOperationAssetListing> {
        let Some(plan_json) = self
            .services
            .library
            .location_operations
            .get_location_operation_plan_json(operation_id)
            .await?
        else {
            return Ok(LocationOperationAssetListing::empty(operation_id));
        };
        let plan: RootMoveExecutionPlan = match serde_json::from_str(&plan_json) {
            Ok(plan) => plan,
            Err(error) => {
                tracing::warn!(
                    operation_id = %operation_id,
                    error = %error,
                    "a location operation's stored plan could not be read; its asset listing is empty"
                );
                return Ok(LocationOperationAssetListing::empty(operation_id));
            }
        };
        let checkpoints = self
            .services
            .library
            .location_operations
            .list_location_title_checkpoints(operation_id)
            .await?;
        Ok(build_asset_listing(operation_id, &plan, &checkpoints))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;

    use crate::location::model::{TitleCheckpoint, TitleCheckpointPlacement};
    use crate::location::root_move::{RootMoveFileExecution, RootMoveTitleExecution};

    fn file(
        media_file_id: Option<&str>,
        source: &str,
        destination: &str,
        size_bytes: u64,
    ) -> RootMoveFileExecution {
        RootMoveFileExecution {
            media_file_id: media_file_id.map(str::to_string),
            source_path: source.to_string(),
            destination_path: destination.to_string(),
            size_bytes,
        }
    }

    fn title(title_id: &str, sequence: i64) -> RootMoveTitleExecution {
        RootMoveTitleExecution {
            title_id: title_id.to_string(),
            title_name: format!("Title {title_id}"),
            sequence,
            class: crate::location::classify::TitleLocationClass::RootMove,
            source_library_id: "library-1".to_string(),
            source_root_id: "root-1".to_string(),
            source_folder_path: Some("/source/Film (2020)".to_string()),
            destination_library_id: "library-1".to_string(),
            destination_root_id: "root-2".to_string(),
            destination_folder_path: Some("/destination/Film (2020)".to_string()),
            destination_root_path: Some("/destination".to_string()),
            source_root_path: Some("/source".to_string()),
            same_volume: Some(false),
            files: Vec::new(),
            deduplicated_sources: Vec::new(),
            deduplicated_media_file_ids: Vec::new(),
            renamed_destinations: Vec::new(),
            prune_directories: Vec::new(),
            warnings: Vec::new(),
            converted_facet: None,
            dropped_tag_prefixes: Vec::new(),
            merge_target_title_id: None,
        }
    }

    fn checkpoint(title_id: &str, sequence: i64, state: TitleCheckpointState) -> TitleCheckpoint {
        let now = Utc::now();
        TitleCheckpoint {
            operation_id: "operation-1".to_string(),
            title_id: title_id.to_string(),
            sequence,
            state,
            classification: None,
            placement: TitleCheckpointPlacement::default(),
            files_total: 0,
            files_verified: 0,
            bytes_total: 0,
            bytes_verified: 0,
            detail: None,
            started_at: Some(now),
            updated_at: now,
            completed_at: None,
        }
    }

    fn renaming_title(title_id: &str, sequence: i64) -> RootMoveTitleExecution {
        let mut title = title(title_id, sequence);
        title.files = vec![file(
            Some("media-1"),
            "/source/Film (2020)/Film.mkv",
            "/destination/Film (2020)/Film (from Movies 4K).mkv",
            2_048,
        )];
        title.renamed_destinations =
            vec!["/destination/Film (2020)/Film (from Movies 4K).mkv".to_string()];
        title
    }

    #[test]
    fn a_settled_title_reports_its_rename_as_done_with_its_provenance() {
        let plan = RootMoveExecutionPlan {
            titles: vec![renaming_title("title-1", 1)],
            ..RootMoveExecutionPlan::default()
        };
        let checkpoints = vec![checkpoint("title-1", 1, TitleCheckpointState::Completed)];

        let listing = build_asset_listing("operation-1", &plan, &checkpoints);

        assert_eq!(listing.renames_total, 1);
        assert_eq!(listing.renames_done, 1);
        let title = &listing.titles[0];
        assert!(title.settled);
        let rename = &title.renames[0];
        assert!(rename.done);
        assert_eq!(rename.source_name.as_deref(), Some("Film.mkv"));
        assert_eq!(rename.destination_name, "Film (from Movies 4K).mkv");
        assert_eq!(rename.provenance_label.as_deref(), Some("Movies 4K"));
        assert_eq!(rename.media_file_id.as_deref(), Some("media-1"));
        assert_eq!(rename.size_bytes, 2_048);
    }

    /// FR-091's "nothing silent" cuts both ways: a canceled title's planned
    /// rename must not read as something that happened.
    #[test]
    fn an_unsettled_title_reports_the_same_rename_as_planned() {
        let plan = RootMoveExecutionPlan {
            titles: vec![renaming_title("title-1", 1)],
            ..RootMoveExecutionPlan::default()
        };

        for state in [
            None,
            Some(TitleCheckpointState::Moving),
            Some(TitleCheckpointState::Failed),
            Some(TitleCheckpointState::Blocked),
            Some(TitleCheckpointState::Skipped),
        ] {
            let checkpoints = state
                .map(|state| vec![checkpoint("title-1", 1, state)])
                .unwrap_or_default();

            let listing = build_asset_listing("operation-1", &plan, &checkpoints);

            assert_eq!(listing.renames_total, 1, "state {state:?}");
            assert_eq!(listing.renames_done, 0, "state {state:?}");
            assert!(!listing.titles[0].settled, "state {state:?}");
            assert!(!listing.titles[0].renames[0].done, "state {state:?}");
            assert_eq!(listing.titles[0].checkpoint_state, state);
        }
    }

    #[test]
    fn a_warned_completion_counts_as_settled_like_the_executor_counters_do() {
        let plan = RootMoveExecutionPlan {
            titles: vec![renaming_title("title-1", 1)],
            ..RootMoveExecutionPlan::default()
        };
        let checkpoints = vec![checkpoint(
            "title-1",
            1,
            TitleCheckpointState::CompletedWithWarnings,
        )];

        let listing = build_asset_listing("operation-1", &plan, &checkpoints);

        assert_eq!(listing.renames_done, 1);
        assert!(listing.titles[0].renames[0].done);
    }

    #[test]
    fn a_dedup_names_the_destination_copy_that_survives() {
        let mut title = title("title-1", 1);
        title.deduplicated_sources = vec!["/source/Film (2020)/Extras/Film.mkv".to_string()];
        title.deduplicated_media_file_ids = vec!["media-1".to_string()];
        let plan = RootMoveExecutionPlan {
            titles: vec![title],
            ..RootMoveExecutionPlan::default()
        };
        let checkpoints = vec![checkpoint(
            "title-1",
            1,
            TitleCheckpointState::CompletedWithWarnings,
        )];

        let listing = build_asset_listing("operation-1", &plan, &checkpoints);

        assert_eq!(listing.dedups_total, 1);
        assert_eq!(listing.dedups_done, 1);
        let dedup = &listing.titles[0].dedups[0];
        assert_eq!(dedup.source_path, "/source/Film (2020)/Extras/Film.mkv");
        assert_eq!(dedup.source_name, "Film.mkv");
        assert_eq!(
            dedup.surviving_path.as_deref(),
            Some("/destination/Film (2020)/Extras/Film.mkv"),
            "the survivor keeps the source's folder-relative position"
        );
        assert_eq!(dedup.surviving_name.as_deref(), Some("Film.mkv"));
        assert!(dedup.done);
    }

    /// A file tracked outside its title's folder lands in the destination
    /// folder root, so its survivor does too.
    #[test]
    fn a_dedup_outside_the_title_folder_falls_back_to_the_destination_root() {
        let mut title = title("title-1", 1);
        title.deduplicated_sources = vec!["/elsewhere/Stray.mkv".to_string()];
        let plan = RootMoveExecutionPlan {
            titles: vec![title],
            ..RootMoveExecutionPlan::default()
        };

        let listing = build_asset_listing("operation-1", &plan, &[]);

        assert_eq!(
            listing.titles[0].dedups[0].surviving_path.as_deref(),
            Some("/destination/Film (2020)/Stray.mkv")
        );
    }

    #[test]
    fn a_dedup_without_a_destination_folder_states_no_survivor_rather_than_guessing() {
        let mut title = title("title-1", 1);
        title.destination_folder_path = None;
        title.deduplicated_sources = vec!["/source/Film (2020)/Film.mkv".to_string()];
        let plan = RootMoveExecutionPlan {
            titles: vec![title],
            ..RootMoveExecutionPlan::default()
        };

        let listing = build_asset_listing("operation-1", &plan, &[]);

        assert_eq!(listing.dedups_total, 1);
        assert!(listing.titles[0].dedups[0].surviving_path.is_none());
        assert!(listing.titles[0].dedups[0].surviving_name.is_none());
    }

    /// A sidecar follows its media file's renamed stem (FR-075), so the suffix
    /// is not after the sidecar's own stem.
    #[test]
    fn a_companion_asset_rename_still_reports_its_provenance() {
        let mut title = title("title-1", 1);
        title.files = vec![file(
            None,
            "/source/Film (2020)/Film.en.srt",
            "/destination/Film (2020)/Film (from Movies 4K).en.srt",
            120,
        )];
        title.renamed_destinations =
            vec!["/destination/Film (2020)/Film (from Movies 4K).en.srt".to_string()];
        let plan = RootMoveExecutionPlan {
            titles: vec![title],
            ..RootMoveExecutionPlan::default()
        };

        let listing = build_asset_listing("operation-1", &plan, &[]);

        let rename = &listing.titles[0].renames[0];
        assert_eq!(rename.provenance_label.as_deref(), Some("Movies 4K"));
        assert!(rename.media_file_id.is_none());
    }

    /// Numeric-only disambiguation carries no source label, and inventing one
    /// would put a library name in the UI that is not in the file name.
    #[test]
    fn a_rename_without_a_source_suffix_reports_no_provenance() {
        let mut title = title("title-1", 1);
        title.files = vec![file(
            Some("media-1"),
            "/source/Film (2020)/Film.mkv",
            "/destination/Film (2020)/Film (2).mkv",
            10,
        )];
        title.renamed_destinations = vec!["/destination/Film (2020)/Film (2).mkv".to_string()];
        let plan = RootMoveExecutionPlan {
            titles: vec![title],
            ..RootMoveExecutionPlan::default()
        };

        let listing = build_asset_listing("operation-1", &plan, &[]);

        assert!(listing.titles[0].renames[0].provenance_label.is_none());
    }

    #[test]
    fn a_plan_with_no_collisions_lists_no_titles_at_all() {
        let mut title = title("title-1", 1);
        title.files = vec![file(
            Some("media-1"),
            "/source/Film (2020)/Film.mkv",
            "/destination/Film (2020)/Film.mkv",
            10,
        )];
        let plan = RootMoveExecutionPlan {
            titles: vec![title],
            ..RootMoveExecutionPlan::default()
        };
        let checkpoints = vec![checkpoint("title-1", 1, TitleCheckpointState::Completed)];

        let listing = build_asset_listing("operation-1", &plan, &checkpoints);

        assert_eq!(listing.operation_id, "operation-1");
        assert!(listing.titles.is_empty());
        assert_eq!(listing.renames_total, 0);
        assert_eq!(listing.dedups_total, 0);
    }

    #[test]
    fn titles_read_in_confirmed_plan_order_however_the_plan_was_stored() {
        let plan = RootMoveExecutionPlan {
            titles: vec![renaming_title("title-2", 7), renaming_title("title-1", 3)],
            ..RootMoveExecutionPlan::default()
        };
        let checkpoints = vec![checkpoint("title-2", 7, TitleCheckpointState::Completed)];

        let listing = build_asset_listing("operation-1", &plan, &checkpoints);

        let order: Vec<&str> = listing
            .titles
            .iter()
            .map(|title| title.title_id.as_str())
            .collect();
        assert_eq!(order, vec!["title-1", "title-2"]);
        // Only the settled title's rename is history; the other is still intent.
        assert_eq!(listing.renames_total, 2);
        assert_eq!(listing.renames_done, 1);
        assert!(!listing.titles[0].renames[0].done);
        assert!(listing.titles[1].renames[0].done);
    }

    /// A stored plan whose file list no longer carries the renamed destination
    /// still names the destination rather than dropping the row.
    #[test]
    fn a_rename_with_no_matching_file_still_names_its_destination() {
        let mut title = title("title-1", 1);
        title.renamed_destinations =
            vec!["/destination/Film (2020)/Film (from Movies 4K).mkv".to_string()];
        let plan = RootMoveExecutionPlan {
            titles: vec![title],
            ..RootMoveExecutionPlan::default()
        };

        let listing = build_asset_listing("operation-1", &plan, &[]);

        let rename = &listing.titles[0].renames[0];
        assert!(rename.source_path.is_none());
        assert!(rename.source_name.is_none());
        assert_eq!(rename.destination_name, "Film (from Movies 4K).mkv");
        assert_eq!(rename.provenance_label.as_deref(), Some("Movies 4K"));
        assert_eq!(rename.size_bytes, 0);
    }
}
