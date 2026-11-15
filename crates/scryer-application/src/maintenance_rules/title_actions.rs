//! The maintenance actions that act through an existing title workflow:
//! monitoring a subject, fetching its subtitles, and moving its title.
//!
//! Each one goes through the operation an operator would use, with the
//! system execution actor: monitoring through the monitoring workflow,
//! subtitles through the per-file subtitle search and the operator's
//! subtitle settings, and moves through the managed location move, whose
//! verified copier places and verifies every file before the source is
//! released.

use scryer_domain::{LifecycleCandidate, MaintenanceRuleSubjectKind as Scope, Title, User};

use super::action_execution::execution_reason;
use crate::location::classify::DestinationRequest;
use crate::location::operations::{RootMovePreviewRequest, StartRootMoveRequest};
use crate::location::preview::PlanConfirmationRequest;
use crate::{AppError, AppResult, AppUseCase};

/// Where a move action sends a title, as the rule wrote it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MaintenanceMoveTarget<'a> {
    /// A library name. It must name exactly one library of the title's kind.
    Library(&'a str),
    /// A root path. It must be a root of the title's own library.
    Root(&'a str),
}

/// A move target resolved to the library and root it names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MaintenanceMovePlan {
    destination: DestinationRequest,
    pub(crate) library_id: String,
    pub(crate) root_id: Option<String>,
}

/// What asking for a move did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MaintenanceMoveOutcome {
    /// The title is already in the named library or on the named root.
    AlreadyThere,
    /// A location operation was started for the title.
    Started {
        operation_id: String,
        library_id: String,
        root_id: Option<String>,
    },
    /// Nothing was started, for the coded reason.
    Held(&'static str),
}

/// A root path compared the way an operator writes it: a trailing separator
/// does not make it another root.
pub(super) fn comparable_root_path(path: &str) -> &str {
    let trimmed = path.trim();
    match trimmed.trim_end_matches(['/', '\\']) {
        "" => trimmed,
        stripped => stripped,
    }
}

/// What a rule's `notify` action sends, taken before any step runs, so a rule
/// that also deletes the title can still name it.
#[derive(Clone, Debug)]
pub(super) struct MaintenanceNotifyContext {
    title: Title,
    subject_label: String,
    message: String,
}

impl AppUseCase {
    /// The notification a candidate's decided actions ask for, if any. One
    /// title read, and one subject read for a season or an episode, only for
    /// a candidate that notifies.
    pub(super) async fn maintenance_notify_context(
        &self,
        candidate: &LifecycleCandidate,
    ) -> AppResult<Option<MaintenanceNotifyContext>> {
        let Some(message) = super::declared_actions::decided_actions_from_json(
            candidate.decided_actions_json.as_deref(),
        )
        .and_then(|actions| actions.notify) else {
            return Ok(None);
        };
        let Some(title) = self
            .services
            .catalog
            .titles
            .get_by_id(&candidate.title_id)
            .await?
        else {
            return Ok(None);
        };
        let subject_label = if candidate.subject_kind == Scope::Title.as_storage_str() {
            title.name.clone()
        } else {
            let subject = self
                .maintenance_resolve_subject(
                    &candidate.subject_kind,
                    &candidate.subject_id,
                    &title,
                    &[],
                )
                .await?;
            format!("{} {}", title.name, subject.label)
        };
        Ok(Some(MaintenanceNotifyContext {
            title,
            subject_label,
            message,
        }))
    }

    /// Send a rule's notification for a candidate whose actions were applied.
    /// The event id is the candidate's match, so a completion that is
    /// recorded again never sends it twice. A failure to write it is logged;
    /// the actions it reports have already happened.
    pub(super) async fn send_maintenance_rule_notification(
        &self,
        candidate: &LifecycleCandidate,
        rule_name: &str,
        context: MaintenanceNotifyContext,
    ) {
        let actor = User::system_execution_actor();
        let mut event = crate::events::domain_events::new_title_domain_event(
            &actor,
            &context.title,
            scryer_domain::DomainEventPayload::MaintenanceRuleNotified(
                scryer_domain::MaintenanceRuleNotifiedEventData {
                    title: crate::events::domain_events::title_context_snapshot(&context.title),
                    rule_set_id: candidate.rule_set_id.clone(),
                    rule_name: rule_name.to_string(),
                    subject_kind: candidate.subject_kind.clone(),
                    subject_id: candidate.subject_id.clone(),
                    subject_label: context.subject_label,
                    message: context.message,
                },
            ),
        );
        event.event_id = format!(
            "maintenance-notify:{}:{}:{}",
            candidate.id, candidate.match_generation, candidate.revision_number
        );
        match self.services.events.domain_events.append_once(event).await {
            Ok(stored) => self.publish_stored_domain_event(&stored).await,
            Err(error) => tracing::warn!(
                candidate_id = candidate.id,
                rule_set_id = candidate.rule_set_id,
                error = %error,
                "could not record a maintenance rule notification"
            ),
        }
    }

    /// Monitor the candidate's subject: the title itself, a season, or an
    /// episode. Monitoring a title does not change its seasons or episodes,
    /// the same as the title's own monitor switch.
    pub(super) async fn monitor_maintenance_subject(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
    ) -> AppResult<()> {
        let actor = User::system_execution_actor();
        if candidate.subject_kind == Scope::Title.as_storage_str() {
            if !title.monitored {
                self.set_title_monitored(&actor, &title.id, true).await?;
            }
            return Ok(());
        }
        let subject = self
            .maintenance_resolve_subject(&candidate.subject_kind, &candidate.subject_id, title, &[])
            .await?;
        if subject.monitored {
            return Ok(());
        }
        match subject.kind {
            Scope::Episode => {
                self.set_episode_monitored(&actor, &subject.id, true)
                    .await?;
            }
            Scope::Season => {
                self.set_collection_monitored(&actor, &subject.id, true)
                    .await?;
            }
            Scope::Title => {
                return Err(AppError::Repository(
                    "maintenance monitor subject resolved to a title".into(),
                ));
            }
        }
        Ok(())
    }

    /// Whether the candidate's subject is monitored now, read fresh.
    pub(super) async fn maintenance_monitoring_is_confirmed(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
    ) -> AppResult<bool> {
        if candidate.subject_kind == Scope::Title.as_storage_str() {
            let Some(fresh) = self.services.catalog.titles.get_by_id(&title.id).await? else {
                return Err(AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {}", title.id),
                ));
            };
            return Ok(fresh.monitored);
        }
        let fresh = self
            .maintenance_resolve_subject(&candidate.subject_kind, &candidate.subject_id, title, &[])
            .await?;
        Ok(fresh.monitored)
    }

    /// Start a subtitle search for every file of the candidate's subject,
    /// through the same per-file search an import uses. Returns how many
    /// searches were started; none when subtitles are switched off or no
    /// language is configured.
    pub(super) async fn fetch_maintenance_subject_subtitles(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
    ) -> AppResult<usize> {
        let settings = self.subtitle_settings().await?;
        if !settings.enabled || settings.languages.is_empty() {
            return Ok(0);
        }
        let mut files_by_title = self
            .maintenance_files_for_titles(std::slice::from_ref(title))
            .await?;
        let title_files = files_by_title.remove(&title.id).unwrap_or_default();
        let files = if candidate.subject_kind == Scope::Title.as_storage_str() {
            title_files
        } else {
            self.maintenance_resolve_subject(
                &candidate.subject_kind,
                &candidate.subject_id,
                title,
                &title_files,
            )
            .await?
            .files
        };
        for file in &files {
            crate::subtitles::orchestration::spawn_subtitle_search_for_file(
                self.clone(),
                title.id.clone(),
                file.id.clone(),
            );
        }
        Ok(files.len())
    }

    /// Whether a completed move step of this candidate's sequence put the
    /// title where it is now. Read only when the title is outside the rule's
    /// libraries, so a pass that finds it in scope never reads the steps.
    pub(super) async fn maintenance_sequence_moved_title(
        &self,
        candidate: &LifecycleCandidate,
        sequence: &super::MaintenanceActionSequence,
        title: &Title,
    ) -> AppResult<bool> {
        if !sequence.steps.iter().any(|step| step.kind.is_move()) {
            return Ok(false);
        }
        let runs = self
            .services
            .customization
            .maintenance_evaluation
            .list_action_steps(
                &candidate.id,
                candidate.match_generation,
                candidate.revision_number,
            )
            .await?;
        // Landing on the target is not enough: someone else may have put the
        // title there. The step's own operation, by the id recorded before it
        // was started, must have completed.
        let operation_ids: Vec<String> = sequence
            .steps
            .iter()
            .filter(|step| step.kind.is_move())
            .filter_map(|step| runs.iter().find(|run| run.key.step_id == step.id))
            .filter(|run| run.state.is_completion_success())
            .filter_map(|run| serde_json::from_str::<serde_json::Value>(&run.provenance_json).ok())
            .filter(|provenance| super::sequence_execution::sequence_move_landed(provenance, title))
            .filter_map(|provenance| {
                provenance
                    .get("operation_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        self.maintenance_any_move_operation_completed(&operation_ids)
            .await
    }

    /// Whether any of these location operations exists and completed. An id
    /// recorded for a move that was never created, or one still running,
    /// failed or canceled, does not count.
    pub(crate) async fn maintenance_any_move_operation_completed(
        &self,
        operation_ids: &[String],
    ) -> AppResult<bool> {
        for operation_id in operation_ids {
            if self
                .location_operation(operation_id)
                .await?
                .is_some_and(|operation| {
                    matches!(
                        operation.state,
                        crate::location::model::LocationOperationState::Completed
                            | crate::location::model::LocationOperationState::CompletedWithWarnings
                    )
                })
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Move a title to the library or root a rule named, through the managed
    /// location move: preview, then start with that preview's fingerprint.
    /// The move runs in the background; its files are copied and verified
    /// before the source is released, as for a move an operator starts. The
    /// executor resolves and starts in two calls so it can checkpoint the
    /// operation id between them; this is the one-call form.
    #[cfg(test)]
    pub(crate) async fn maintenance_move_title(
        &self,
        title: &Title,
        target: MaintenanceMoveTarget<'_>,
    ) -> AppResult<MaintenanceMoveOutcome> {
        match self.maintenance_resolve_move(title, target).await? {
            Ok(planned) => self.maintenance_start_move(title, planned, None).await,
            Err(outcome) => Ok(outcome),
        }
    }

    /// Resolve where a move action sends a title, without starting anything.
    /// An `Err` is the outcome to report instead of a move: the title is
    /// already there, or the target does not resolve.
    pub(crate) async fn maintenance_resolve_move(
        &self,
        title: &Title,
        target: MaintenanceMoveTarget<'_>,
    ) -> AppResult<Result<MaintenanceMovePlan, MaintenanceMoveOutcome>> {
        let libraries = self.services.catalog.libraries.list(None).await?;
        let (destination, library_id, root_id) = match target {
            MaintenanceMoveTarget::Library(name) => {
                let named: Vec<_> = libraries
                    .iter()
                    .filter(|library| library.name == name)
                    .collect();
                let of_kind: Vec<_> = named
                    .iter()
                    .filter(|library| library.facet == title.facet)
                    .collect();
                let library = match of_kind.as_slice() {
                    [library] => library,
                    [] if !named.is_empty() => {
                        return Ok(Err(MaintenanceMoveOutcome::Held(
                            execution_reason::MOVE_LIBRARY_FACET_MISMATCH,
                        )));
                    }
                    _ => {
                        return Ok(Err(MaintenanceMoveOutcome::Held(
                            execution_reason::MOVE_LIBRARY_UNRESOLVED,
                        )));
                    }
                };
                if library.id == title.library_id {
                    return Ok(Err(MaintenanceMoveOutcome::AlreadyThere));
                }
                (
                    DestinationRequest::to_library(library.id.clone()),
                    library.id.clone(),
                    None,
                )
            }
            MaintenanceMoveTarget::Root(path) => {
                let wanted = comparable_root_path(path);
                let Some(root) = libraries
                    .iter()
                    .find(|library| library.id == title.library_id)
                    .and_then(|library| {
                        library
                            .roots
                            .iter()
                            .find(|root| comparable_root_path(&root.path) == wanted)
                    })
                else {
                    return Ok(Err(MaintenanceMoveOutcome::Held(
                        execution_reason::MOVE_ROOT_NOT_IN_LIBRARY,
                    )));
                };
                if root.id == title.root_folder_id {
                    return Ok(Err(MaintenanceMoveOutcome::AlreadyThere));
                }
                (
                    DestinationRequest::to_library_root(title.library_id.clone(), root.id.clone()),
                    title.library_id.clone(),
                    Some(root.id.clone()),
                )
            }
        };
        Ok(Ok(MaintenanceMovePlan {
            destination,
            library_id,
            root_id,
        }))
    }

    /// Start a resolved move. `operation_id`, when given, is the id the
    /// location operation is created with, so a caller that recorded it
    /// before this call can tell afterwards whether that operation exists.
    pub(crate) async fn maintenance_start_move(
        &self,
        title: &Title,
        planned: MaintenanceMovePlan,
        operation_id: Option<String>,
    ) -> AppResult<MaintenanceMoveOutcome> {
        let MaintenanceMovePlan {
            destination,
            library_id,
            root_id,
        } = planned;
        let actor = User::system_execution_actor();
        let preview = match self
            .preview_root_move(
                &actor,
                RootMovePreviewRequest {
                    title_ids: vec![title.id.clone()],
                    destination: destination.clone(),
                },
            )
            .await
        {
            Ok(preview) => preview,
            Err(error) if move_refusal(&error) => {
                return Ok(MaintenanceMoveOutcome::Held(execution_reason::MOVE_REFUSED));
            }
            Err(error) => return Err(error),
        };
        match self
            .start_root_move_as(
                &actor,
                StartRootMoveRequest {
                    title_ids: vec![title.id.clone()],
                    destination,
                    confirmation: PlanConfirmationRequest {
                        fingerprint: preview.plan.fingerprint,
                        typed_confirmation: None,
                    },
                },
                operation_id,
            )
            .await
        {
            Ok(accepted) => Ok(MaintenanceMoveOutcome::Started {
                operation_id: accepted.operation.id,
                library_id,
                root_id,
            }),
            // Nothing to move (a title the plan cannot place), a stale plan,
            // or a destination the move refuses: the files stay where they
            // are and the step retries on a later pass.
            Err(error) if move_refusal(&error) => {
                Ok(MaintenanceMoveOutcome::Held(execution_reason::MOVE_REFUSED))
            }
            Err(error) => Err(error),
        }
    }
}

/// Whether a move error is the move declining this plan, rather than a
/// failure to reach the catalog or the filesystem.
fn move_refusal(error: &AppError) -> bool {
    matches!(
        error,
        AppError::ValidationRefused { .. }
            | AppError::ValidationRefusedWithCount { .. }
            | AppError::LocationPlanRefused { .. }
            | AppError::LocationRootRefused { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::comparable_root_path;

    #[test]
    fn a_trailing_separator_names_the_same_root() {
        assert_eq!(comparable_root_path("/media/movies/"), "/media/movies");
        assert_eq!(comparable_root_path(" /media/movies "), "/media/movies");
        assert_eq!(comparable_root_path("/"), "/");
        assert_eq!(comparable_root_path(r"D:\media\"), r"D:\media");
    }
}
