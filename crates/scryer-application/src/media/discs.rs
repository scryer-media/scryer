use std::path::PathBuf;

use crate::{AppError, AppResult, AppUseCase, MediaAnalysisOutcome, TitleMediaFile};
use scryer_domain::User;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MediaSourceVersion {
    length: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl MediaSourceVersion {
    pub(crate) async fn read(path: &std::path::Path) -> AppResult<Self> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            identity: (metadata.dev(), metadata.ino()),
        })
    }
}

pub(crate) struct CataloguedMediaAnalysis {
    pub(crate) outcome: MediaAnalysisOutcome,
    pub(crate) expected: Option<TitleMediaFile>,
    pub(crate) source: MediaSourceVersion,
}

impl AppUseCase {
    /// Diagnose only an authorized, registered physical file using bounded native reads.
    pub async fn diagnose_media_file(
        &self,
        actor: &User,
        file_id: &str,
    ) -> AppResult<scryer_media_types::StructuralDiagnostics> {
        let file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))?;
        self.get_title_for_management(actor, &file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", file.title_id)))?;
        let path = crate::stored_paths::stored_path_to_path_buf(&file.file_path);
        let selection = file
            .analysis_details
            .disc
            .as_ref()
            .map(|disc| disc.selection.clone())
            .unwrap_or_default();
        let result = self
            .services
            .library
            .media_analyzer
            .diagnose_file(path, selection)
            .await?;
        tracing::debug!(
            file_id,
            bytes_read = result.report.bytes_read,
            elapsed_ms = result.report.elapsed_ms,
            incomplete = result.report.status == scryer_media_types::ProbeStatus::Incomplete,
            warnings = result.report.warnings.len(),
            "bounded media diagnostics completed"
        );
        Ok(result)
    }

    /// Reuse the saved disc title choice during any refresh of a catalogued file.
    pub(crate) async fn analyze_catalogued_media_file(
        &self,
        file_id: Option<&str>,
        path: PathBuf,
    ) -> AppResult<CataloguedMediaAnalysis> {
        let expected = if let Some(id) = file_id {
            self.services
                .library
                .media_files
                .get_media_file_by_id(id)
                .await?
        } else {
            None
        };
        let selection = expected
            .as_ref()
            .and_then(|file| file.analysis_details.disc.as_ref())
            .map(|disc| disc.selection.clone())
            .unwrap_or_default();
        let source = MediaSourceVersion::read(&path).await?;
        let outcome = self
            .services
            .library
            .media_analyzer
            .analyze_file_with_selection(path.clone(), selection)
            .await?;
        if source != MediaSourceVersion::read(&path).await? {
            return Err(AppError::Validation(
                "media source changed during inspection".into(),
            ));
        }
        Ok(CataloguedMediaAnalysis {
            outcome,
            expected,
            source,
        })
    }

    pub async fn media_file_disc_episode_targets(
        &self,
        actor: &User,
        file_id: &str,
    ) -> AppResult<Vec<scryer_domain::Episode>> {
        let file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))?;
        let title = self
            .get_title_for_management(actor, &file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", file.title_id)))?;
        if title.facet == scryer_domain::MediaFacet::Movie {
            return Ok(Vec::new());
        }
        self.services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await
    }

    pub(crate) async fn validate_disc_episode_mappings(
        &self,
        title: &scryer_domain::Title,
        disc: &mut scryer_media_types::DiscMetadata,
    ) -> AppResult<()> {
        if disc.selection.episode_mappings.len() > 512 {
            return Err(AppError::Validation(
                "disc episode mapping limit exceeded".into(),
            ));
        }
        let mut episode_ids = std::collections::HashSet::new();
        let mut title_ids = std::collections::HashSet::new();
        let mut canonical_titles = Vec::with_capacity(disc.selection.episode_mappings.len());
        for mapping in &disc.selection.episode_mappings {
            let [episode_id] = mapping.episode_ids.as_slice() else {
                return Err(AppError::Validation("map each complete disc title to one episode; combined episodes require separate authored playback titles".into()));
            };
            let authored = disc
                .titles
                .iter()
                .find(|item| {
                    item.id == mapping.disc_title_id
                        || item.aliases.contains(&mapping.disc_title_id)
                })
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "mapped disc title {} no longer exists; review the mapping",
                        mapping.disc_title_id
                    ))
                })?;
            if authored.report.status != scryer_media_types::ProbeStatus::Complete {
                return Err(AppError::Validation(format!(
                    "mapped disc title {} has incomplete or unsupported analysis",
                    authored.id
                )));
            }
            if !episode_ids.insert(episode_id.clone()) || !title_ids.insert(authored.id.clone()) {
                return Err(AppError::Validation(
                    "each episode and distinct playback title can occur in only one mapping".into(),
                ));
            }
            let episode = self
                .services
                .catalog
                .shows
                .get_episode_by_id(episode_id)
                .await?
                .filter(|episode| episode.title_id == title.id)
                .ok_or_else(|| {
                    AppError::Validation("mapped episode does not belong to this title".into())
                })?;
            let special = episode
                .season_number
                .as_deref()
                .and_then(|season| season.trim().parse::<u32>().ok())
                == Some(0);
            let expected = episode
                .duration_seconds
                .filter(|value| *value > 0)
                .or_else(|| {
                    (!special)
                        .then_some(title.runtime_minutes)
                        .flatten()
                        .filter(|value| *value > 0)
                        .map(|value| i64::from(value) * 60)
                });
            let actual = authored
                .duration_seconds
                .filter(|value| value.is_finite() && *value > 0.0)
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "mapped disc title {} has no established runtime",
                        authored.id
                    ))
                })?;
            let plausible = expected.map_or(actual >= 60.0, |expected| {
                actual >= expected as f64 / 2.0 && actual <= expected as f64 * 2.0
            });
            if !plausible {
                return Err(AppError::Validation(format!(
                    "disc title {} runtime ({actual:.3} seconds) is implausible for episode {episode_id}",
                    authored.id
                )));
            }
            canonical_titles.push(authored.id.clone());
        }
        // Failed validation must retain the original saved selection so the
        // failed-attempt guard can identify the associations requiring review.
        for (mapping, canonical) in disc
            .selection
            .episode_mappings
            .iter_mut()
            .zip(canonical_titles)
        {
            mapping.disc_title_id = canonical;
        }
        Ok(())
    }

    async fn validate_disc_selection_update(
        &self,
        title: &scryer_domain::Title,
        expected: &TitleMediaFile,
        path: &std::path::Path,
        source: &MediaSourceVersion,
        analysis: &mut crate::MediaFileAnalysis,
    ) -> AppResult<()> {
        let disc =
            analysis.details.disc.as_mut().ok_or_else(|| {
                AppError::Validation("image has no recognized disc titles".into())
            })?;
        let validation = self.validate_disc_episode_mappings(title, disc).await;
        if *source != MediaSourceVersion::read(path).await? {
            return Err(AppError::Validation(
                "media source changed during inspection".into(),
            ));
        }
        if let Err(error) = validation {
            analysis.details.report.status = scryer_media_types::ProbeStatus::Incomplete;
            analysis
                .details
                .report
                .warnings
                .push(scryer_media_types::ProbeWarning {
                    code: "disc_episode_mapping_review".into(),
                    message: error.to_string(),
                    ..Default::default()
                });
            self.services
                .library
                .media_files
                .record_media_analysis_attempt(expected, analysis)
                .await?;
            return Err(error);
        }
        Ok(())
    }

    /// Replace explicit episode coverage while retaining a single physical ISO record.
    pub async fn map_media_file_disc_episodes(
        &self,
        actor: &User,
        file_id: &str,
        mappings: Vec<scryer_media_types::DiscEpisodeMapping>,
    ) -> AppResult<TitleMediaFile> {
        if mappings.len() > 512 {
            return Err(AppError::Validation(
                "disc episode mapping limit exceeded".into(),
            ));
        }
        let files = &self.services.library.media_files;
        let expected = files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))?;
        let title = self
            .get_title_for_management(actor, &expected.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", expected.title_id)))?;
        if title.facet == scryer_domain::MediaFacet::Movie {
            return Err(AppError::Validation(
                "episode mappings require a series title".into(),
            ));
        }
        let path = crate::stored_paths::stored_path_to_path_buf(&expected.file_path);
        if !scryer_domain::is_disc_image(&path) {
            return Err(AppError::Validation(
                "disc episode mappings require an ISO image".into(),
            ));
        }
        let mut selection = expected
            .analysis_details
            .disc
            .as_ref()
            .map(|disc| disc.selection.clone())
            .unwrap_or_default();
        selection.episode_mappings = mappings;
        let source = MediaSourceVersion::read(&path).await?;
        let outcome = self
            .services
            .library
            .media_analyzer
            .analyze_file_with_selection(path.clone(), selection)
            .await?;
        if source != MediaSourceVersion::read(&path).await? {
            return Err(AppError::Validation(
                "media source changed during inspection".into(),
            ));
        }
        let mut analysis = match outcome {
            MediaAnalysisOutcome::Valid(analysis) => analysis,
            MediaAnalysisOutcome::Inconclusive(analysis) => {
                files
                    .record_media_analysis_attempt(&expected, &analysis)
                    .await?;
                return Err(AppError::Validation(
                    "disc analysis requires review before episodes can be made available".into(),
                ));
            }
            MediaAnalysisOutcome::Invalid(error) => return Err(AppError::Validation(error)),
        };
        self.validate_disc_selection_update(&title, &expected, &path, &source, &mut analysis)
            .await?;
        if !files
            .update_media_file_analysis_if_unchanged(&expected, *analysis)
            .await?
        {
            return Err(AppError::Validation(
                "media source or selection changed during mapping; refresh and retry".into(),
            ));
        }
        files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))
    }

    /// Change a registered physical image's playback title without moving or copying it.
    pub async fn select_media_file_disc_title(
        &self,
        actor: &User,
        file_id: &str,
        title_id: Option<String>,
    ) -> AppResult<TitleMediaFile> {
        let files = &self.services.library.media_files;
        let expected = files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))?;
        let title = self
            .get_title_for_management(actor, &expected.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", expected.title_id)))?;
        let path = crate::stored_paths::stored_path_to_path_buf(&expected.file_path);
        if !scryer_domain::is_disc_image(&path) {
            return Err(AppError::Validation(
                "disc selection requires an ISO image".into(),
            ));
        }
        if title_id.as_deref().is_some_and(|id| {
            id.is_empty() || id.len() > 16 || !id.bytes().all(|byte| byte.is_ascii_digit())
        }) {
            return Err(AppError::Validation(
                "invalid numeric disc title identity".into(),
            ));
        }
        let mut selection = expected
            .analysis_details
            .disc
            .as_ref()
            .map(|disc| disc.selection.clone())
            .unwrap_or_default();
        selection.title_id = title_id;
        let source = MediaSourceVersion::read(&path).await?;
        let outcome = self
            .services
            .library
            .media_analyzer
            .analyze_file_with_selection(path.clone(), selection)
            .await?;
        if source != MediaSourceVersion::read(&path).await? {
            return Err(AppError::Validation(
                "media source changed during inspection".into(),
            ));
        }
        let mut analysis = match outcome {
            MediaAnalysisOutcome::Valid(analysis) => analysis,
            MediaAnalysisOutcome::Inconclusive(analysis) => {
                files
                    .record_media_analysis_attempt(&expected, &analysis)
                    .await?;
                return Err(AppError::Validation("the requested disc title could not be established; review its inspection status".into()));
            }
            MediaAnalysisOutcome::Invalid(error) => return Err(AppError::Validation(error)),
        };
        if analysis
            .details
            .disc
            .as_ref()
            .is_none_or(|disc| disc.selected_title_id.is_none())
        {
            return Err(AppError::Validation(
                "the requested disc title does not exist".into(),
            ));
        }
        self.validate_disc_selection_update(&title, &expected, &path, &source, &mut analysis)
            .await?;
        if !files
            .update_media_file_analysis_if_unchanged(&expected, *analysis)
            .await?
        {
            return Err(AppError::Validation(
                "media source or selection changed during inspection; refresh and retry".into(),
            ));
        }
        files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))
    }
}
