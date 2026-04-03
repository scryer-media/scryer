use std::path::Path;

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, LibraryRenamer, RenameApplyItemResult, RenameApplyStatus, RenamePlan,
    RenameWriteAction,
};
use tokio::fs;
use tokio::io::AsyncWriteExt;

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
    async fn validate_targets(&self, plan: &RenamePlan) -> AppResult<()> {
        for item in &plan.items {
            if matches!(item.write_action, RenameWriteAction::Replace) {
                return Err(AppError::Validation(
                    "rename replace action is not supported".into(),
                ));
            }

            if !matches!(item.write_action, RenameWriteAction::Move) {
                continue;
            }

            let source = Path::new(&item.current_path);
            let source_meta = fs::metadata(source)
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;
            if !source_meta.is_file() {
                return Err(AppError::Validation(format!(
                    "rename source is not a file: {}",
                    item.current_path
                )));
            }

            let Some(target_path) = item.proposed_path.as_deref() else {
                return Err(AppError::Validation(
                    "rename target path is required for move/replace actions".into(),
                ));
            };

            if let Some(parent) = Path::new(target_path).parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|err| AppError::Repository(err.to_string()))?;
            }

            if target_path != item.current_path && fs::metadata(target_path).await.is_ok() {
                return Err(AppError::Validation(format!(
                    "rename target already exists: {target_path}"
                )));
            }
        }

        Ok(())
    }

    async fn apply_plan(&self, plan: &RenamePlan) -> AppResult<Vec<RenameApplyItemResult>> {
        let mut out = Vec::with_capacity(plan.items.len());

        for item in &plan.items {
            let mut result = RenameApplyItemResult {
                collection_id: item.collection_id.clone(),
                media_file_id: item.media_file_id.clone(),
                current_path: item.current_path.clone(),
                proposed_path: item.proposed_path.clone(),
                final_path: None,
                write_action: item.write_action.clone(),
                status: RenameApplyStatus::Skipped,
                reason_code: item.reason_code.clone(),
                error_message: None,
            };

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
                        out.push(result);
                        continue;
                    };

                    match move_file(&item.current_path, target, false).await {
                        Ok(()) => {
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

            out.push(result);
        }

        Ok(out)
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

            move_file(final_path, &item.current_path, false)
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;
        }

        Ok(applied_items.to_vec())
    }
}

async fn move_file(source: &str, target: &str, replace: bool) -> std::io::Result<()> {
    if replace && target != source && fs::metadata(target).await.is_ok() {
        fs::remove_file(target).await?;
    }

    if let Some(parent) = Path::new(target).parent() {
        fs::create_dir_all(parent).await?;
    }

    match fs::rename(source, target).await {
        Ok(()) => Ok(()),
        Err(err) if is_cross_device_error(&err) => {
            fs::copy(source, target).await?;
            let mut file = fs::OpenOptions::new().write(true).open(target).await?;
            file.flush().await?;
            file.sync_all().await?;
            fs::remove_file(source).await?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn is_cross_device_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(18)
}
