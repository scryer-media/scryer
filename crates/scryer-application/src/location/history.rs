use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use scryer_domain::{DomainEventPayload, MediaPathUpdate, MediaUpdateType, TitleMovedEventData};

use super::executor::{PlannedTitle, TitleCompletionRecorder};
use super::model::{LocationOperation, TitleCheckpointState};
use super::root_move::{RootMoveExecutionPlan, RootMoveTitleExecution};
use crate::domain_events::{DomainEventActor, new_title_domain_event, title_context_snapshot};
use crate::{AppError, AppResult, AppUseCase};

pub(crate) struct LocationTitleHistory<'a> {
    app: &'a AppUseCase,
    titles: BTreeMap<&'a str, &'a RootMoveTitleExecution>,
    libraries: BTreeMap<String, String>,
    actor: tokio::sync::OnceCell<DomainEventActor>,
}

impl<'a> LocationTitleHistory<'a> {
    pub async fn new(app: &'a AppUseCase, plan: &'a RootMoveExecutionPlan) -> AppResult<Self> {
        let ids: BTreeSet<_> = plan
            .titles
            .iter()
            .flat_map(|title| [&title.source_library_id, &title.destination_library_id])
            .collect();
        let mut libraries = BTreeMap::new();
        for id in ids {
            let name = app
                .services
                .catalog
                .libraries
                .get_by_id(id)
                .await?
                .map(|library| library.name)
                .unwrap_or_else(|| id.clone());
            libraries.insert(id.clone(), name);
        }
        Ok(Self {
            app,
            titles: plan
                .titles
                .iter()
                .map(|title| (title.title_id.as_str(), title))
                .collect(),
            libraries,
            actor: tokio::sync::OnceCell::new(),
        })
    }
}

#[async_trait]
impl TitleCompletionRecorder for LocationTitleHistory<'_> {
    async fn record(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
        state: TitleCheckpointState,
        detail: Option<&str>,
    ) -> AppResult<()> {
        let planned = self
            .titles
            .get(title.title_id.as_str())
            .ok_or_else(|| AppError::Repository("missing title move history plan".into()))?;
        let surviving_id = planned
            .merge_target_title_id
            .as_deref()
            .unwrap_or(&planned.title_id);
        let surviving = self
            .app
            .services
            .catalog
            .titles
            .get_by_id(surviving_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {surviving_id}")))?;
        let actor = self
            .actor
            .get_or_try_init(|| async {
                let actor = if let Some(id) = operation.initiated_by_user_id.as_deref() {
                    self.app
                        .services
                        .identity
                        .users
                        .get_by_id(id)
                        .await?
                        .as_ref()
                        .map(DomainEventActor::user)
                        .unwrap_or_else(|| DomainEventActor::from(Some(id.to_owned())))
                } else {
                    DomainEventActor::system()
                };
                Ok::<_, AppError>(actor)
            })
            .await?;
        // The catalog row is authoritative for where a file landed: a transfer
        // may keep a file under a different name than the plan proposed. A
        // failed read falls back to the planned destinations.
        let current_paths = self
            .app
            .services
            .library
            .media_files
            .list_media_files_for_title(surviving_id)
            .await
            .map(|files| {
                files
                    .into_iter()
                    .map(|file| (file.id, file.file_path))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_else(|error| {
                tracing::warn!(
                    title_id = surviving_id,
                    error = %error,
                    "could not list media files for the title-moved event; using planned destinations"
                );
                BTreeMap::new()
            });
        let media_updates = moved_media_updates(planned, &current_paths);
        let mut event = new_title_domain_event(
            actor.clone(),
            &surviving,
            DomainEventPayload::TitleMoved(TitleMovedEventData {
                title: title_context_snapshot(&surviving),
                operation_id: operation.id.clone(),
                operation_type: operation.operation_type.as_str().into(),
                mode: operation.mode.as_str().into(),
                source_title_id: planned.title_id.clone(),
                source_title_name: planned.title_name.clone(),
                source_library_id: planned.source_library_id.clone(),
                source_library_name: self.libraries[&planned.source_library_id].clone(),
                destination_library_id: planned.destination_library_id.clone(),
                destination_library_name: self.libraries[&planned.destination_library_id].clone(),
                source_root_id: planned.source_root_id.clone(),
                destination_root_id: planned.destination_root_id.clone(),
                source_path: planned.source_folder_path.clone(),
                destination_path: surviving
                    .folder_path
                    .clone()
                    .or_else(|| planned.destination_folder_path.clone()),
                completed_with_warnings: state == TitleCheckpointState::CompletedWithWarnings,
                detail: detail.map(str::to_owned),
                media_updates,
            }),
        );
        // Replaying cleanup after a crash must not create another history row.
        event.event_id = format!("location:{}:{}", operation.id, planned.title_id);
        event.correlation_id = Some(operation.id.clone());
        let stored = self
            .app
            .services
            .events
            .domain_events
            .append_once(event)
            .await?;
        self.app.publish_stored_domain_event(&stored).await;
        Ok(())
    }
}

/// Each moved media file as its old path (`deleted`) followed by its new path
/// (`created`). Companion assets are not media files and are left out. The new
/// path is the file's current catalog path when the catalog still tracks it,
/// otherwise the planned destination (a merged duplicate's surviving copy).
fn moved_media_updates(
    planned: &RootMoveTitleExecution,
    current_paths: &BTreeMap<String, String>,
) -> Vec<MediaPathUpdate> {
    let mut updates = Vec::new();
    for file in &planned.files {
        let Some(media_file_id) = file.media_file_id.as_deref() else {
            continue;
        };
        let destination = current_paths
            .get(media_file_id)
            .cloned()
            .unwrap_or_else(|| file.destination_path.clone());
        if destination == file.source_path {
            continue;
        }
        updates.push(MediaPathUpdate {
            path: file.source_path.clone(),
            update_type: MediaUpdateType::Deleted,
        });
        updates.push(MediaPathUpdate {
            path: destination,
            update_type: MediaUpdateType::Created,
        });
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::root_move::RootMoveFileExecution;

    fn file(media_file_id: Option<&str>, source: &str, destination: &str) -> RootMoveFileExecution {
        RootMoveFileExecution {
            media_file_id: media_file_id.map(str::to_owned),
            source_path: source.to_owned(),
            destination_path: destination.to_owned(),
            size_bytes: 1,
            source_content: None,
        }
    }

    fn planned(files: Vec<RootMoveFileExecution>) -> RootMoveTitleExecution {
        let mut title: RootMoveTitleExecution = serde_json::from_value(serde_json::json!({
            "title_id": "title-1",
            "title_name": "Synthetic Film",
            "sequence": 0,
            "class": "cross_library_transfer",
            "source_library_id": "library-a",
            "source_root_id": "root-a",
            "source_folder_path": "/root-a/Synthetic Film (2020)",
            "destination_library_id": "library-b",
            "destination_root_id": "root-b",
            "destination_folder_path": "/root-b/Synthetic Film (2020)",
            "destination_root_path": "/root-b",
            "source_root_path": "/root-a",
            "same_volume": false,
            "files": [],
            "prune_directories": [],
            "warnings": [],
        }))
        .expect("synthetic title execution");
        title.files = files;
        title
    }

    #[test]
    fn moved_media_updates_pair_each_media_file_with_its_current_path() {
        let plan = planned(vec![
            file(
                Some("media-1"),
                "/root-a/Synthetic Film (2020)/film.mkv",
                "/root-b/Synthetic Film (2020)/film.mkv",
            ),
            file(
                None,
                "/root-a/Synthetic Film (2020)/film.en.srt",
                "/root-b/Synthetic Film (2020)/film.en.srt",
            ),
            file(
                Some("media-2"),
                "/root-a/Synthetic Film (2020)/extra.mkv",
                "/root-b/Synthetic Film (2020)/extra.mkv",
            ),
        ]);
        // The catalog kept the second file under another name; the first is
        // no longer tracked (a merged duplicate) and uses the planned path.
        let current = BTreeMap::from([(
            "media-2".to_string(),
            "/root-b/Synthetic Film (2020)/extra (2).mkv".to_string(),
        )]);

        let updates = moved_media_updates(&plan, &current);

        assert_eq!(
            updates,
            vec![
                MediaPathUpdate {
                    path: "/root-a/Synthetic Film (2020)/film.mkv".into(),
                    update_type: MediaUpdateType::Deleted,
                },
                MediaPathUpdate {
                    path: "/root-b/Synthetic Film (2020)/film.mkv".into(),
                    update_type: MediaUpdateType::Created,
                },
                MediaPathUpdate {
                    path: "/root-a/Synthetic Film (2020)/extra.mkv".into(),
                    update_type: MediaUpdateType::Deleted,
                },
                MediaPathUpdate {
                    path: "/root-b/Synthetic Film (2020)/extra (2).mkv".into(),
                    update_type: MediaUpdateType::Created,
                },
            ]
        );
    }

    #[test]
    fn moved_media_updates_skip_files_that_did_not_change_path() {
        let plan = planned(vec![file(
            Some("media-1"),
            "/root-a/Synthetic Film (2020)/film.mkv",
            "/root-a/Synthetic Film (2020)/film.mkv",
        )]);
        assert!(moved_media_updates(&plan, &BTreeMap::new()).is_empty());
    }
}
