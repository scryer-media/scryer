use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use scryer_domain::{DomainEventPayload, TitleMovedEventData};

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
