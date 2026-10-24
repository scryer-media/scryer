//! Episode-level Fix Match: an operator moves one media file to other
//! episodes of its title in a single re-match and rename.
//!
//! The links move in one store transaction, then the file is renamed on disk
//! to the name the new episodes render, through the library renamer. If the
//! rename cannot happen, the links go back exactly as they were, so a link
//! never names a path the file is not at.

use super::*;
use crate::episode_file_reassign_refusal as refusal;
use crate::library_rename::ReassignedFileRename;
use crate::{
    EpisodeFileReassignOutcome, EpisodeFileReassignRequest, EpisodeFileReassignment,
    ReassignEpisodeFileResult, RefusalFact,
};
use scryer_domain::MediaFileReassignedEventData;

impl AppUseCase {
    /// Make `episode_ids` the episodes one series or anime file serves, as
    /// their primary file, and rename the file to match.
    ///
    /// Another file that is primary for a target episode refuses the request
    /// unless `replace` is set; with it, that file stays linked as an
    /// additional file. No file is ever deleted.
    pub async fn reassign_episode_file(
        &self,
        actor: &User,
        file_id: &str,
        episode_ids: &[String],
        replace: bool,
    ) -> AppResult<ReassignEpisodeFileResult> {
        let mut episode_ids = episode_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        episode_ids.sort();
        episode_ids.dedup();
        if episode_ids.is_empty() {
            return Err(AppError::validation_refused(
                refusal::NO_EPISODES,
                "choose at least one episode to assign the file to",
            ));
        }

        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&media_file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", media_file.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        if !matches!(title.facet, MediaFacet::Series | MediaFacet::Anime) {
            return Err(AppError::validation_refused(
                refusal::NOT_EPISODIC_TITLE,
                "only series and anime files can be assigned to episodes",
            ));
        }
        if !media_file.series_movie_link_ids.is_empty()
            || scryer_domain::is_disc_image(std::path::Path::new(&media_file.file_path))
        {
            return Err(AppError::validation_refused(
                refusal::UNSUPPORTED_FILE,
                "this file cannot be assigned to episodes",
            ));
        }

        // The file's links and path change together; a location operation or
        // a deletion of the same file must not run underneath.
        let _location_guard = self
            .acquire_location_title_mutation(
                &crate::location::ownership_guard::EPISODE_FILE_REASSIGN_ENTRY,
                &title.id,
            )
            .await?;
        let _file_guard = self
            .runtime
            .jobs
            .interactive_operation_guards
            .try_acquire(&format!("media-file:{file_id}"))
            .await
            .ok_or_else(|| {
                AppError::LocationOperationBusy(format!(
                    "another operation is already changing media file {file_id}"
                ))
            })?;

        let outcome = self
            .services
            .library
            .media_files
            .reassign_file_episode_links(&EpisodeFileReassignRequest {
                file_id: media_file.id.clone(),
                title_id: title.id.clone(),
                episode_ids: episode_ids.clone(),
                replace,
            })
            .await?;
        let reassignment = match outcome {
            EpisodeFileReassignOutcome::Reassigned(reassignment) => reassignment,
            EpisodeFileReassignOutcome::FileNotFound => {
                return Err(AppError::NotFound(format!("media file {file_id}")));
            }
            EpisodeFileReassignOutcome::SeriesMovieFile => {
                return Err(AppError::validation_refused(
                    refusal::UNSUPPORTED_FILE,
                    "this file cannot be assigned to episodes",
                ));
            }
            EpisodeFileReassignOutcome::EpisodeOutsideTitle { episode_id } => {
                return Err(AppError::validation_refused_with_facts(
                    refusal::EPISODE_OUTSIDE_TITLE,
                    "a chosen episode does not belong to this file's title",
                    vec![RefusalFact::new(refusal::EPISODE_ID_FACT, episode_id)],
                ));
            }
            EpisodeFileReassignOutcome::PrimaryConflict {
                episode_id,
                file_id: conflicting_file_id,
            } => {
                return Err(AppError::validation_refused_with_facts(
                    refusal::PRIMARY_CONFLICT,
                    "a chosen episode already has a primary file",
                    vec![
                        RefusalFact::new(refusal::CONFLICTING_FILE_ID_FACT, conflicting_file_id),
                        RefusalFact::new(refusal::EPISODE_ID_FACT, episode_id),
                    ],
                ));
            }
        };

        let rename = match self
            .rename_reassigned_episode_file(actor, &title, &media_file.id)
            .await
        {
            Ok(rename) => rename,
            Err(error) => ReassignedFileRename::Failed {
                reason_code: "rename_plan_failed".into(),
                message: error.to_string(),
            },
        };
        let (previous_path, file_path) = match rename {
            ReassignedFileRename::Placed {
                previous_path,
                file_path,
            } => (previous_path, file_path),
            refused_or_failed => {
                self.restore_reassigned_episode_links(&reassignment).await?;
                return Err(reassignment_rename_refusal(refused_or_failed));
            }
        };

        let file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(&media_file.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))?;
        let previous_episode_ids = reassignment.previous_episode_ids();
        let demoted_file_ids = reassignment.demoted_file_ids();
        let mut promoted_file_ids = reassignment
            .promoted_links
            .iter()
            .map(|link| link.file_id.clone())
            .collect::<Vec<_>>();
        promoted_file_ids.sort();
        promoted_file_ids.dedup();

        self.append_domain_event(new_title_domain_event(
            actor,
            &title,
            DomainEventPayload::MediaFileReassigned(MediaFileReassignedEventData {
                title: title_context_snapshot(&title),
                file_id: file.id.clone(),
                previous_path: previous_path.clone(),
                file_path: file_path.clone(),
                previous_episode_ids: previous_episode_ids.clone(),
                episode_ids: reassignment.episode_ids.clone(),
                demoted_file_ids: demoted_file_ids.clone(),
            }),
        ))
        .await?;
        self.emit_title_updated_activity(actor, &title).await;
        // Episodes the file left may be missing now; the acquisition loop
        // re-reads availability from the links.
        self.sync_title_for_immediate_acquisition(&title).await;
        info!(
            file_id = %file.id,
            title_id = %title.id,
            previous_path = %previous_path,
            file_path = %file_path,
            "media file reassigned to other episodes"
        );

        Ok(ReassignEpisodeFileResult {
            file,
            previous_path,
            previous_episode_ids,
            episode_ids: reassignment.episode_ids,
            demoted_file_ids,
            promoted_file_ids,
        })
    }

    /// Put the links back after the rename could not place the file. A
    /// failure here leaves links that name episodes the file was not renamed
    /// for, so it is logged loudly and surfaced as the request's error.
    async fn restore_reassigned_episode_links(
        &self,
        reassignment: &EpisodeFileReassignment,
    ) -> AppResult<()> {
        if let Err(error) = self
            .services
            .library
            .media_files
            .restore_file_episode_links(reassignment)
            .await
        {
            tracing::error!(
                file_id = %reassignment.file_id,
                error = %error,
                "could not restore the episode links of a file whose reassignment rename failed"
            );
            return Err(error);
        }
        Ok(())
    }
}

fn reassignment_rename_refusal(rename: ReassignedFileRename) -> AppError {
    match rename {
        ReassignedFileRename::Placed { .. } => {
            AppError::Repository("a placed reassignment rename is not a refusal".into())
        }
        ReassignedFileRename::Refused { occupied: true, .. } => AppError::validation_refused(
            refusal::DESTINATION_OCCUPIED,
            "another file already has the name this episode renders; the file was left as it was",
        ),
        ReassignedFileRename::Refused { reason_code, .. } => {
            AppError::validation_refused_with_facts(
                refusal::RENAME_REFUSED,
                format!(
                    "the file could not be renamed for the new episode ({reason_code}); it was left as it was"
                ),
                vec![RefusalFact::new(refusal::RENAME_REASON_FACT, reason_code)],
            )
        }
        ReassignedFileRename::Failed {
            reason_code,
            message,
        } => AppError::validation_refused_with_facts(
            refusal::RENAME_FAILED,
            format!("moving the file to its new name failed: {message}; it was left as it was"),
            vec![RefusalFact::new(refusal::RENAME_REASON_FACT, reason_code)],
        ),
    }
}
