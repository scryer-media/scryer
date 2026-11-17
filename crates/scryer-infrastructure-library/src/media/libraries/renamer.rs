use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use scryer_application::disk::{Disk, EntryKind, Parents, PlaceRefusal, ReadBefore, placement};
use scryer_application::stored_paths::stored_path_to_path_buf;
use scryer_application::{
    AppError, AppResult, ImportFilePermissions, LibraryRenamer, RenameApplyItemResult,
    RenameApplyStatus, RenamePlan, RenameWriteAction,
};

pub struct FileSystemLibraryRenamer;

impl Default for FileSystemLibraryRenamer {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystemLibraryRenamer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LibraryRenamer for FileSystemLibraryRenamer {
    /// Reports whether the plan is shaped correctly. Per-file conditions are
    /// deliberately not checked here: they are re-checked immediately before
    /// each move, so one unusable file fails on its own instead of cancelling
    /// every other rename in the plan.
    async fn validate_targets(&self, plan: &RenamePlan) -> AppResult<()> {
        for item in &plan.items {
            if matches!(item.write_action, RenameWriteAction::Move) && item.proposed_path.is_none()
            {
                return Err(AppError::validation_refused(
                    crate::error_reasons::MEDIA_RENAME_TARGET_PATH_REQUIRED,
                    "rename target path is required for move actions",
                ));
            }
        }

        Ok(())
    }

    async fn apply_plan(
        &self,
        plan: &RenamePlan,
        permissions: &ImportFilePermissions,
    ) -> AppResult<Vec<RenameApplyItemResult>> {
        // A companion only moves when the media file it belongs to actually
        // moved, so the plan is walked in two passes: primaries first, then
        // companions gated on their primary's outcome. Results are written back
        // into their original plan positions so callers can still pair a result
        // with its item by index (#224).
        let mut slots: Vec<Option<RenameApplyItemResult>> =
            (0..plan.items.len()).map(|_| None).collect();
        let mut primary_outcomes: HashMap<&str, RenameApplyStatus> = HashMap::new();
        let primary_indexes = plan
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.companion_of.is_none())
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let companion_indexes = plan
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.companion_of.is_some())
            .map(|(index, _)| index)
            .collect::<Vec<_>>();

        for index in primary_indexes.into_iter().chain(companion_indexes) {
            let item = &plan.items[index];
            let mut result = RenameApplyItemResult {
                collection_id: item.collection_id.clone(),
                media_file_id: item.media_file_id.clone(),
                series_movie_link_ids: item.series_movie_link_ids.clone(),
                current_path: item.current_path.clone(),
                proposed_path: item.proposed_path.clone(),
                final_path: None,
                write_action: item.write_action.clone(),
                status: RenameApplyStatus::Skipped,
                reason_code: item.reason_code.clone(),
                error_message: None,
            };

            // The primary's outcome is resolved from the recorded pass-one
            // results, so this holds whichever order the two appear in.
            if let Some(primary) = item.companion_of.as_deref()
                && primary_outcomes.get(primary) != Some(&RenameApplyStatus::Applied)
            {
                result.status = RenameApplyStatus::Skipped;
                result.reason_code = "companion_primary_not_applied".into();
                result.error_message =
                    Some("companion left in place because its media file did not move".to_string());
                slots[index] = Some(result);
                continue;
            }

            match item.write_action {
                RenameWriteAction::Noop => {
                    result.status = RenameApplyStatus::Skipped;
                    result.final_path = item.proposed_path.clone();
                }
                RenameWriteAction::Skip => {
                    result.status = RenameApplyStatus::Skipped;
                }
                RenameWriteAction::Error => {
                    result.status = RenameApplyStatus::Failed;
                }
                RenameWriteAction::Replace => {
                    result.status = RenameApplyStatus::Failed;
                    result.reason_code = "replace_not_supported".into();
                    result.error_message = Some("rename replace action is not supported".into());
                }
                RenameWriteAction::Move => {
                    let Some(target) = item.proposed_path.as_deref() else {
                        result.status = RenameApplyStatus::Failed;
                        result.reason_code = "missing_target".into();
                        result.error_message =
                            Some("rename target path missing for move action".into());
                        if item.companion_of.is_none() {
                            primary_outcomes.insert(&item.current_path, result.status.clone());
                        }
                        slots[index] = Some(result);
                        continue;
                    };

                    // Checked here rather than up front so the window between
                    // the check and the write stays as small as the filesystem
                    // allows, and so a file that cannot move does not cancel
                    // the rest of the plan.
                    let prepared =
                        prepare_move_target(&item.current_path, target, permissions).await;
                    let read = match prepared {
                        Ok(read) => read,
                        Err((reason_code, message)) => {
                            result.status = RenameApplyStatus::Failed;
                            result.reason_code = reason_code;
                            result.error_message = Some(message);
                            if item.companion_of.is_none() {
                                primary_outcomes.insert(&item.current_path, result.status.clone());
                            }
                            slots[index] = Some(result);
                            continue;
                        }
                    };

                    match move_file(&item.current_path, target, Some(&read)).await {
                        Ok(()) => {
                            // Configured permissions take precedence over
                            // whatever the file carried at its old path.
                            crate::workflow::file_importer::apply_file_permissions_best_effort(
                                &stored_path_to_path_buf(target),
                                permissions,
                            );
                            result.status = RenameApplyStatus::Applied;
                            result.final_path = Some(target.to_string());
                        }
                        Err(err) => {
                            result.status = RenameApplyStatus::Failed;
                            result.reason_code = "rename_io_failed".into();
                            result.error_message = Some(err.to_string());
                        }
                    }
                }
            }

            if item.companion_of.is_none() {
                primary_outcomes.insert(&item.current_path, result.status.clone());
            }
            slots[index] = Some(result);
        }

        Ok(slots.into_iter().flatten().collect())
    }

    async fn rollback(
        &self,
        applied_items: &[RenameApplyItemResult],
    ) -> AppResult<Vec<RenameApplyItemResult>> {
        for item in applied_items.iter().rev() {
            if !matches!(item.write_action, RenameWriteAction::Move) {
                continue;
            }

            let Some(final_path) = item.final_path.as_deref() else {
                continue;
            };

            if final_path == item.current_path {
                continue;
            }

            move_file(final_path, &item.current_path, None)
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;
        }

        Ok(applied_items.to_vec())
    }
}

/// Verifies one file can move and prepares its destination directory,
/// returning a per-item reason code and message when it cannot. What it read
/// of the source, and the folder it made, are handed to the move, which then
/// reads neither again.
async fn prepare_move_target(
    current_path: &str,
    target_path: &str,
    permissions: &ImportFilePermissions,
) -> Result<ReadBefore, (String, String)> {
    let source = stored_path_to_path_buf(current_path);
    let reading = Disk::process().uncached(&source).await;
    if reading.exists() != Some(true) {
        let absence = reading.describe_absence();
        return Err((
            "source_missing".to_string(),
            format!("rename source is unreadable: {current_path}: {absence}"),
        ));
    }
    if reading.is(EntryKind::Directory) {
        return Err((
            "source_not_file".to_string(),
            format!("rename source is not a file: {current_path}"),
        ));
    }

    let target = stored_path_to_path_buf(target_path);
    if let Some(parent) = target.parent() {
        // Season and title folders created by a rename get the configured
        // permissions, the way the importer already treats folders it creates
        // and Sonarr treats every folder it makes during a move.
        // The disk service looks at each ancestor off the runtime. A look
        // that does not answer means no folder gets the permissions.
        let created =
            crate::workflow::file_importer::missing_destination_dirs_off_runtime(parent).await;
        let made = Disk::process().create_dir(parent, Parents::Create).await;
        made.map_err(std::io::Error::from).map_err(|err| {
            (
                "target_parent_unwritable".to_string(),
                format!("could not create {}: {err}", parent.display()),
            )
        })?;
        for directory in &created {
            crate::workflow::file_importer::apply_directory_permissions_best_effort(
                directory,
                permissions,
            );
        }
    }

    if !rename_paths_equivalent(current_path, target_path)
        && !Disk::process()
            .destination_is_free_for(&source, &target)
            .await
    {
        return Err((
            "target_exists".to_string(),
            format!("rename target already exists: {target_path}"),
        ));
    }

    Ok(ReadBefore::made_folder(reading, &target))
}

async fn move_file(
    source: &str,
    target: &str,
    read: Option<&ReadBefore>,
) -> Result<(), PlaceRefusal> {
    let (source, target) = (
        stored_path_to_path_buf(source),
        stored_path_to_path_buf(target),
    );
    let disk = Disk::process();
    let policy = placement::library_rename();
    match read {
        Some(read) => disk.place_after_reading(source, target, policy, read).await,
        None => disk.place(source, target, policy).await,
    }
    .map(|_| ())
}

fn rename_paths_equivalent(source: &str, target: &str) -> bool {
    rename_path_key(source) == rename_path_key(target)
}

#[cfg(windows)]
fn rename_path_key(stored_path: &str) -> String {
    lexically_normalize(&stored_path_to_path_buf(stored_path))
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase()
}

#[cfg(not(windows))]
fn rename_path_key(stored_path: &str) -> String {
    lexically_normalize(&stored_path_to_path_buf(stored_path))
        .to_string_lossy()
        .into_owned()
}

fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `rename(2)` replaces the destination silently, so this is the guarantee
    /// that stops a rename from destroying a file that appeared after planning.
    #[tokio::test]
    async fn move_file_refuses_to_replace_an_existing_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let target = dir.path().join("target.mkv");
        std::fs::write(&source, b"source-payload").expect("write source");
        std::fs::write(&target, b"target-payload").expect("write target");

        let error = move_file(&source.to_string_lossy(), &target.to_string_lossy(), None)
            .await
            .expect_err("an occupied destination must not be replaced");
        assert_eq!(
            error.refused,
            scryer_application::disk::Refused::DestinationExists
        );

        assert_eq!(
            std::fs::read(&target).expect("target still present"),
            b"target-payload",
            "the destination must keep its bytes"
        );
        assert_eq!(
            std::fs::read(&source).expect("source still present"),
            b"source-payload",
            "the source must stay put when the move is refused"
        );
    }

    /// On a case-insensitive volume the destination name resolves to the source
    /// file, which is a rename to perform rather than a collision to reject.
    #[tokio::test]
    async fn move_file_performs_a_case_only_rename() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("one piece.mkv");
        let target = dir.path().join("ONE PIECE.mkv");
        std::fs::write(&source, b"payload").expect("write source");

        move_file(&source.to_string_lossy(), &target.to_string_lossy(), None)
            .await
            .expect("case-only rename should succeed");

        assert_eq!(
            std::fs::read(&target).expect("renamed file present"),
            b"payload"
        );
        let names = std::fs::read_dir(dir.path())
            .expect("list dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["ONE PIECE.mkv".to_string()]);
    }

    /// One unusable file used to abort the whole plan, so a single squatting
    /// destination cancelled every other rename in a bulk operation.
    #[tokio::test]
    async fn apply_plan_fails_only_the_item_it_cannot_move() {
        let dir = tempfile::tempdir().expect("tempdir");
        let blocked_source = dir.path().join("blocked.mkv");
        let blocked_target = dir.path().join("blocked-target.mkv");
        let movable_source = dir.path().join("movable.mkv");
        let movable_target = dir.path().join("movable-target.mkv");
        std::fs::write(&blocked_source, b"blocked").expect("write");
        std::fs::write(&blocked_target, b"occupied").expect("write");
        std::fs::write(&movable_source, b"movable").expect("write");

        let item = |current: &std::path::Path, proposed: &std::path::Path| {
            scryer_application::RenamePlanItem {
                collection_id: None,
                media_file_id: None,
                series_movie_link_ids: Vec::new(),
                current_path: current.to_string_lossy().into_owned(),
                proposed_path: Some(proposed.to_string_lossy().into_owned()),
                normalized_filename: None,
                collision: false,
                reason_code: "rename_move".to_string(),
                write_action: RenameWriteAction::Move,
                source_size_bytes: None,
                source_mtime_unix_ms: None,
                companion_of: None,
            }
        };
        let plan = RenamePlan {
            facet: scryer_domain::MediaFacet::Movie,
            title_id: None,
            template: String::new(),
            collision_policy: scryer_application::RenameCollisionPolicy::Skip,
            missing_metadata_policy: scryer_application::RenameMissingMetadataPolicy::FallbackTitle,
            fingerprint: String::new(),
            total: 2,
            renamable: 2,
            noop: 0,
            conflicts: 0,
            errors: 0,
            items: vec![
                item(&blocked_source, &blocked_target),
                item(&movable_source, &movable_target),
            ],
            title_folders: Vec::new(),
            folder_repairs: Vec::new(),
            naming_context: Vec::new(),
        };

        let results = FileSystemLibraryRenamer::new()
            .apply_plan(&plan, &ImportFilePermissions::default())
            .await
            .expect("apply should report per-item outcomes");

        assert_eq!(results[0].status, RenameApplyStatus::Failed);
        assert_eq!(results[0].reason_code, "target_exists");
        assert_eq!(results[1].status, RenameApplyStatus::Applied);
        assert_eq!(
            std::fs::read(&blocked_target).expect("occupied target intact"),
            b"occupied"
        );
        assert_eq!(
            std::fs::read(&movable_target).expect("the movable file still moved"),
            b"movable"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn rename_path_key_preserves_case_on_non_windows() {
        assert_ne!(
            rename_path_key("/media/Movie.mkv"),
            rename_path_key("/media/movie.mkv")
        );
    }

    #[cfg(windows)]
    #[test]
    fn rename_path_key_folds_case_and_separators_on_windows() {
        assert_eq!(
            rename_path_key(r"C:\Media\Movie.mkv"),
            rename_path_key("C:/media/movie.mkv")
        );
    }
}
