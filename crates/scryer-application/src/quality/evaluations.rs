//! Explicitly requested, display-only evaluations of stored files.

use crate::{AppError, AppResult, AppUseCase, TitleMediaFile};
use scryer_domain::{LibraryPermission, User};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::Mutex;
mod refresh_job;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ScoreRefreshSelection {
    Files(Vec<String>),
    Seasons(Vec<String>),
    Titles(Vec<String>),
}

impl ScoreRefreshSelection {
    pub fn ids(&self) -> &[String] {
        match self {
            Self::Files(ids) | Self::Seasons(ids) | Self::Titles(ids) => ids,
        }
    }
}

#[derive(Default)]
pub(crate) struct ScoreRefreshRuntime {
    pub worker: Mutex<()>,
    pub cancellations: Mutex<HashMap<String, (String, tokio_util::sync::CancellationToken)>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileScoreEvaluation {
    pub score: i32,
    pub evaluated_at: String,
    pub fingerprint: String,
    pub time_sensitive: bool,
    pub entries: Vec<ScoreEvaluationEntry>,
    pub rejection_codes: Vec<String>,
}

pub type ScoreEvaluationEntry = scryer_domain::ScoringSnapshotEntry;

impl From<&crate::quality_profile::ScoringEntry> for ScoreEvaluationEntry {
    fn from(entry: &crate::quality_profile::ScoringEntry) -> Self {
        use crate::quality_profile::ScoringSource;
        Self {
            code: entry.code.clone(),
            delta: entry.delta,
            kind: entry.kind.as_str().to_owned(),
            source: match &entry.source {
                ScoringSource::Builtin => None,
                ScoringSource::UserRule { name, .. } | ScoringSource::SystemRule { name, .. } => {
                    Some(name.clone())
                }
            },
            origin: match &entry.source {
                ScoringSource::Builtin => None,
                ScoringSource::UserRule { .. } => Some("user".to_owned()),
                ScoringSource::SystemRule { .. } => Some("system".to_owned()),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileScoreEvaluationView {
    pub evaluation: FileScoreEvaluation,
    pub stale: bool,
}

struct EvaluationInputs {
    file: TitleMediaFile,
    context: super::canonical_context::ResolvedScoringContext,
    basis: crate::quality_profile::CoverageSizeBasis,
    episode_ids: Vec<String>,
    fingerprint: String,
}

impl AppUseCase {
    async fn file_score_inputs(
        &self,
        actor: &User,
        file_id: &str,
        manage: bool,
    ) -> AppResult<EvaluationInputs> {
        // Jobs may outlive the request's authorization snapshot.
        let fresh_actor;
        let actor = if manage {
            let user = self
                .services
                .identity
                .users
                .get_by_id(&actor.id)
                .await?
                .ok_or_else(|| {
                    AppError::unauthorized_reason(
                        crate::error_reasons::quality::SCORE_REFRESH_USER_NO_LONGER_EXISTS,
                        "score refresh user no longer exists",
                    )
                })?;
            fresh_actor = self.attach_user_authorization(user).await?;
            &fresh_actor
        } else {
            actor
        };
        let file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::quality::MEDIA_FILE_NOT_FOUND,
                    "media file",
                )
            })?;
        let title = self
            .get_title(actor, &file.title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::quality::MEDIA_TITLE_NOT_FOUND,
                    "media title",
                )
            })?;
        if manage {
            self.require_library_permission(
                actor,
                &title.library_id,
                LibraryPermission::ManageTitles,
            )
            .await?;
        }
        let episode_ids = self
            .services
            .library
            .media_files
            .score_evaluation_episode_ids(&file)
            .await?;
        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;
        let basis = crate::acquisition_coverage::episode_span_size_basis(
            &episodes,
            &episode_ids,
            title.runtime_minutes,
        );
        let profile = self.resolve_quality_profile_for_title(&title).await?;
        let context = self
            .resolve_canonical_scoring_context(&title, &profile)
            .await;
        let fingerprint = context.evaluation_fingerprint(&file, basis, &episode_ids);
        Ok(EvaluationInputs {
            file,
            context,
            basis,
            episode_ids,
            fingerprint,
        })
    }

    /// Read the saved result and compare inputs, without evaluating rules.
    pub async fn media_file_score_evaluation(
        &self,
        actor: &User,
        file_id: &str,
    ) -> AppResult<Option<FileScoreEvaluationView>> {
        // Authorize before reading a persisted result, including legacy files.
        let file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::quality::MEDIA_FILE_NOT_FOUND,
                    "media file",
                )
            })?;
        self.get_title(actor, &file.title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::quality::MEDIA_TITLE_NOT_FOUND,
                    "media title",
                )
            })?;
        let Some(evaluation) = self
            .services
            .library
            .media_files
            .get_score_evaluation(file_id)
            .await?
        else {
            return Ok(None);
        };
        let inputs = self.file_score_inputs(actor, file_id, false).await?;
        let stale = evaluation.fingerprint != inputs.fingerprint;
        Ok(Some(FileScoreEvaluationView { evaluation, stale }))
    }

    pub(crate) async fn refresh_file_score(
        &self,
        actor: &User,
        file_id: &str,
        requested_at: &str,
    ) -> AppResult<()> {
        let inputs = self.file_score_inputs(actor, file_id, true).await?;
        if let Some(saved) = self
            .services
            .library
            .media_files
            .get_score_evaluation(file_id)
            .await?
            && !saved.time_sensitive
            && saved.fingerprint == inputs.fingerprint
            && chrono::DateTime::parse_from_rfc3339(&saved.evaluated_at)
                .ok()
                .zip(chrono::DateTime::parse_from_rfc3339(requested_at).ok())
                .is_some_and(|(saved, requested)| saved >= requested)
        {
            return Ok(());
        }
        let fingerprint = inputs.fingerprint.clone();
        let title_id = inputs.file.title_id.clone();
        let evaluated_at = self.runtime.environment.now().to_rfc3339();
        let result = tokio::task::spawn_blocking(move || {
            let view = inputs.context.view(inputs.basis, false);
            let preview = if inputs.episode_ids.is_empty() {
                super::canonical::score_media_file_preview(&inputs.file, &view)
            } else {
                super::canonical::score_media_file_for_episodes_preview(
                    &inputs.file,
                    &inputs.episode_ids,
                    &view,
                )
            };
            if preview.engine_error.is_some() || !preview.rule_errors.is_empty() {
                return Err(AppError::validation_refused(
                    crate::error_reasons::quality::FILE_SCORE_EVALUATION_FAILED,
                    "file score evaluation failed; previous evaluation retained",
                ));
            }
            let scored = preview.scored;
            let decision = scored
                .analyzed_decision
                .as_ref()
                .unwrap_or(&scored.announced_decision);
            Ok(FileScoreEvaluation {
                score: scored.total,
                evaluated_at,
                fingerprint: inputs.fingerprint,
                time_sensitive: inputs.context.evaluation_reads_clock(),
                entries: decision
                    .scoring_log
                    .iter()
                    .map(ScoreEvaluationEntry::from)
                    .collect(),
                rejection_codes: decision.block_codes.clone(),
            })
        })
        .await
        .map_err(|_| AppError::Repository("file score worker failed".into()))??;
        let latest = self.file_score_inputs(actor, file_id, true).await?;
        if latest.fingerprint != fingerprint || latest.file.title_id != title_id {
            return Err(AppError::validation_refused(
                crate::error_reasons::quality::FILE_SCORING_INPUTS_CHANGED,
                "file scoring inputs changed; refresh again",
            ));
        }
        self.services
            .library
            .media_files
            .save_score_evaluation(file_id, &title_id, &result)
            .await
    }
}
