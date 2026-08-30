use super::lookup::find_completed_download;
use super::*;

pub(super) enum ExpectedEpisodeResolution {
    NotApplicable,
    Unresolved,
    Resolved(HashSet<String>),
    AtLeastOne(HashSet<String>),
}

pub(super) enum SourceVideoEpisodeResolution {
    Unavailable,
    NoVisibleVideos,
    Unmapped,
    Resolved(HashSet<String>),
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ArtifactSourceDisposition {
    Successful,
    Ignored,
    Rejected,
    Undisposed,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ArtifactMemberCompletion {
    Incomplete,
    Terminal,
    AllIntentionallyIgnored,
}

#[derive(Clone, Copy)]
enum ImportVerificationMode {
    Automatic,
    Manual {
        expected_mapping_count: Option<usize>,
    },
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
enum ImportArtifactSourceKey {
    RelativePath(String),
    NormalizedFileName(String),
}

/// Phase 1: evaluate a tracked download whose client reports completion.
///
/// Called every poll cycle for downloads in Downloading or ImportBlocked state.
/// Transitions to ImportPending if all validations pass, or ImportBlocked with
/// warnings if auto-import is not safe.
pub async fn verify_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
) -> AppResult<bool> {
    verify_import_inner(app, td, files_imported_this_pass, None).await
}

pub async fn verify_manual_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
    expected_mapping_count: Option<usize>,
) -> AppResult<bool> {
    verify_import_with_mode(
        app,
        td,
        files_imported_this_pass,
        None,
        None,
        ImportVerificationMode::Manual {
            expected_mapping_count,
        },
        false,
    )
    .await
}

pub(super) async fn verify_import_inner(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
    completed: Option<&CompletedDownload>,
) -> AppResult<bool> {
    verify_import_inner_with_release_evidence(app, td, files_imported_this_pass, completed, None)
        .await
}

pub(super) async fn verify_import_inner_with_release_evidence(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
    completed: Option<&CompletedDownload>,
    release_evidence: Option<&crate::import_workflow::ReleaseEvidence>,
) -> AppResult<bool> {
    verify_import_with_mode(
        app,
        td,
        files_imported_this_pass,
        completed,
        release_evidence,
        ImportVerificationMode::Automatic,
        false,
    )
    .await
}

pub(super) async fn verify_skipped_import_with_release_evidence(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
    completed: Option<&CompletedDownload>,
    release_evidence: Option<&crate::import_workflow::ReleaseEvidence>,
) -> AppResult<bool> {
    verify_import_with_mode(
        app,
        td,
        files_imported_this_pass,
        completed,
        release_evidence,
        ImportVerificationMode::Automatic,
        true,
    )
    .await
}

async fn verify_import_with_mode(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
    completed: Option<&CompletedDownload>,
    release_evidence: Option<&crate::import_workflow::ReleaseEvidence>,
    mode: ImportVerificationMode,
    require_terminal_artifact_members: bool,
) -> AppResult<bool> {
    let artifacts = import_artifacts_for_completed_download(app, td, completed).await?;

    if artifacts.is_empty() {
        return Ok(false);
    }

    let artifact_members = artifact_member_completion(&artifacts, td);
    if require_terminal_artifact_members && artifact_members == ArtifactMemberCompletion::Incomplete
    {
        return Ok(false);
    }

    let current_visible_files = match mode {
        ImportVerificationMode::Manual {
            expected_mapping_count: Some(count),
        } => count,
        _ => current_visible_video_file_count(app, td, completed).await,
    };
    let source_video_units = visible_source_episode_units(app, td, &artifacts, completed).await;
    let visible_sources_terminal =
        visible_source_files_have_terminal_dispositions(app, td, &artifacts, completed).await;
    if visible_sources_terminal == Some(false) {
        return Ok(false);
    }
    let mut successful_units = HashSet::new();
    let mut successful_source_files = HashSet::new();
    let mut rejected_units = HashSet::new();

    for artifact in &artifacts {
        let logical_unit = artifact.episode_id.clone().unwrap_or_else(|| {
            format!("{}:{}", artifact.media_kind, artifact.normalized_file_name)
        });

        match artifact.result.as_str() {
            "imported" | "already_present" => {
                successful_units.insert(logical_unit);
                if let Some(source_key) = import_artifact_source_key(artifact) {
                    successful_source_files.insert(source_key);
                }
            }
            "rejected" => {
                rejected_units.insert(logical_unit);
            }
            _ => {}
        }
    }

    let all_sources_intentionally_ignored = matches!(mode, ImportVerificationMode::Automatic)
        && artifact_members == ArtifactMemberCompletion::AllIntentionallyIgnored;
    if successful_units.is_empty() && !all_sources_intentionally_ignored {
        return Ok(false);
    }

    if td.facet.as_deref() == Some("movie") {
        return Ok(!successful_units.is_empty());
    }

    let manual_source_coverage = match mode {
        ImportVerificationMode::Automatic => None,
        ImportVerificationMode::Manual {
            expected_mapping_count,
        } => expected_mapping_count
            .map(|expected| expected > 0 && successful_source_files.len() >= expected),
    };
    if manual_source_coverage == Some(false) {
        return Ok(false);
    }

    match expected_episode_units_with_release_evidence(app, td, release_evidence).await {
        ExpectedEpisodeResolution::Resolved(expected_episode_units) => {
            let expected_episode_units = if matches!(mode, ImportVerificationMode::Automatic) {
                expected_episode_units_after_ignored_unmonitored(
                    app,
                    td,
                    &artifacts,
                    expected_episode_units,
                )
                .await?
            } else {
                expected_episode_units
            };
            if expected_episode_units.is_empty() {
                return Ok(all_sources_intentionally_ignored);
            }
            if successful_units.is_empty() {
                return Ok(false);
            }

            if let Some(source_units_complete) = source_video_expected_units_are_complete(
                &source_video_units,
                &successful_units,
                &expected_episode_units,
            ) {
                return Ok(source_units_complete);
            }

            return Ok(expected_episode_units
                .iter()
                .all(|unit| successful_units.contains(unit)));
        }
        ExpectedEpisodeResolution::AtLeastOne(expected_episode_units) => {
            let expected_episode_units = if matches!(mode, ImportVerificationMode::Automatic) {
                expected_episode_units_after_ignored_unmonitored(
                    app,
                    td,
                    &artifacts,
                    expected_episode_units,
                )
                .await?
            } else {
                expected_episode_units
            };
            if expected_episode_units.is_empty() {
                return Ok(all_sources_intentionally_ignored);
            }
            if successful_units.is_empty() {
                return Ok(false);
            }

            return Ok(expected_episode_units
                .iter()
                .any(|unit| successful_units.contains(unit)));
        }
        ExpectedEpisodeResolution::Unresolved => {
            if successful_units.is_empty() {
                return Ok(false);
            }
            if matches!(mode, ImportVerificationMode::Automatic)
                && successful_units_cover_visible_files(
                    successful_units.len(),
                    current_visible_files,
                )
            {
                return Ok(true);
            }

            return Ok(match mode {
                ImportVerificationMode::Automatic => {
                    files_imported_this_pass > 0 && rejected_units.is_empty()
                }
                ImportVerificationMode::Manual { .. } => manual_source_coverage.unwrap_or(false),
            });
        }
        ExpectedEpisodeResolution::NotApplicable => {}
    }

    if successful_units.is_empty() {
        return Ok(false);
    }

    Ok(match mode {
        ImportVerificationMode::Automatic => {
            if successful_units_cover_visible_files(successful_units.len(), current_visible_files) {
                return Ok(true);
            }
            !successful_units.is_empty()
        }
        ImportVerificationMode::Manual { .. } => manual_source_coverage.unwrap_or(false),
    })
}

fn source_video_expected_units_are_complete(
    source_video_units: &SourceVideoEpisodeResolution,
    successful_units: &HashSet<String>,
    expected_episode_units: &HashSet<String>,
) -> Option<bool> {
    match source_video_units {
        SourceVideoEpisodeResolution::Resolved(units) if !units.is_empty() => {
            let expected_source_units = units
                .iter()
                .filter(|unit| expected_episode_units.contains(*unit))
                .collect::<Vec<_>>();
            (!expected_source_units.is_empty()).then(|| {
                expected_source_units
                    .into_iter()
                    .all(|unit| successful_units.contains(unit))
            })
        }
        SourceVideoEpisodeResolution::Unmapped
        | SourceVideoEpisodeResolution::Resolved(_)
        | SourceVideoEpisodeResolution::NoVisibleVideos
        | SourceVideoEpisodeResolution::Unavailable => None,
    }
}

fn successful_units_cover_visible_files(
    successful_unit_count: usize,
    current_visible_files: usize,
) -> bool {
    current_visible_files > 0 && successful_unit_count >= current_visible_files
}

async fn expected_episode_units_after_ignored_unmonitored(
    app: &AppUseCase,
    td: &TrackedDownload,
    artifacts: &[crate::ImportArtifact],
    expected_episode_units: HashSet<String>,
) -> AppResult<HashSet<String>> {
    let ignored_unmonitored =
        ignored_unmonitored_expected_episode_ids(app, td, artifacts, &expected_episode_units)
            .await?;
    Ok(expected_episode_units
        .difference(&ignored_unmonitored)
        .cloned()
        .collect())
}

async fn ignored_unmonitored_expected_episode_ids(
    app: &AppUseCase,
    td: &TrackedDownload,
    artifacts: &[crate::ImportArtifact],
    expected_episode_units: &HashSet<String>,
) -> AppResult<HashSet<String>> {
    let Some(title_id) = tracked_title_id(td) else {
        return Ok(HashSet::new());
    };
    let ignored_episode_ids = artifacts
        .iter()
        .filter(|artifact| artifact.result == "ignored")
        .filter(|artifact| artifact_matches_tracked_title(artifact, title_id))
        .filter_map(|artifact| artifact.episode_id.as_deref())
        .map(str::trim)
        .filter(|episode_id| expected_episode_units.contains(*episode_id))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if ignored_episode_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let ignored_episode_ids = ignored_episode_ids.into_iter().collect::<Vec<_>>();
    Ok(app
        .services
        .catalog
        .shows
        .get_episodes_by_ids(&ignored_episode_ids)
        .await?
        .into_iter()
        .filter(|episode| episode.title_id == title_id && !episode.monitored)
        .map(|episode| episode.id)
        .collect())
}

#[cfg(test)]
pub(super) async fn expected_episode_units(
    app: &AppUseCase,
    td: &TrackedDownload,
) -> ExpectedEpisodeResolution {
    expected_episode_units_with_release_evidence(app, td, None).await
}

async fn expected_episode_units_with_release_evidence(
    app: &AppUseCase,
    td: &TrackedDownload,
    release_evidence: Option<&crate::import_workflow::ReleaseEvidence>,
) -> ExpectedEpisodeResolution {
    let Some(title_id) = td.title_id.as_deref() else {
        return ExpectedEpisodeResolution::Unresolved;
    };
    let Some(title) = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await
        .ok()
        .flatten()
    else {
        return ExpectedEpisodeResolution::Unresolved;
    };

    if let Some(scope) = release_evidence.and_then(crate::import_workflow::ReleaseEvidence::scope)
        && let Some(expected) = crate::import_workflow::expected_episode_ids_from_submission_scope(
            app, &title, scope, true,
        )
        .await
    {
        return ExpectedEpisodeResolution::Resolved(expected);
    }

    let Some(release_title) = expected_episode_release_title(td, release_evidence) else {
        return ExpectedEpisodeResolution::NotApplicable;
    };
    let parse_context = crate::build_release_parse_context(&title, None, None, td.facet.as_deref());
    let parsed = crate::parse_release_metadata_for_target(&release_title, &parse_context);
    let Some(ep_meta) = parsed.episode.as_ref() else {
        return ExpectedEpisodeResolution::NotApplicable;
    };
    let season_str = ep_meta.season.unwrap_or(1).to_string();
    let episodes =
        crate::import_workflow::resolve_target_episodes(app, &title, ep_meta, &season_str).await;

    if episodes.is_empty() {
        return ExpectedEpisodeResolution::Unresolved;
    }

    let expected_lookup_count = if ep_meta.season.is_some() && !ep_meta.episode_numbers.is_empty() {
        ep_meta
            .episode_numbers
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
    } else if !ep_meta.absolute_episode_numbers.is_empty() {
        ep_meta
            .absolute_episode_numbers
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
    } else if ep_meta.absolute_episode.is_some() {
        if ep_meta.episode_numbers.is_empty() {
            1
        } else {
            ep_meta
                .episode_numbers
                .iter()
                .copied()
                .collect::<HashSet<_>>()
                .len()
        }
    } else {
        0
    };

    if expected_lookup_count > 0 && episodes.len() < expected_lookup_count {
        return ExpectedEpisodeResolution::Unresolved;
    }

    let all_expected_episode_ids = episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect::<HashSet<_>>();
    let monitored_expected_episode_ids = episodes
        .into_iter()
        .filter(|episode| episode.monitored)
        .map(|episode| episode.id)
        .collect::<HashSet<_>>();
    let expected_episode_ids = if monitored_expected_episode_ids.is_empty() {
        all_expected_episode_ids
    } else {
        monitored_expected_episode_ids
    };

    if ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
        && ep_meta.is_partial_season
        && ep_meta.episode_numbers.is_empty()
        && ep_meta.absolute_episode_numbers.is_empty()
        && ep_meta.special_absolute_episode_numbers.is_empty()
    {
        return ExpectedEpisodeResolution::AtLeastOne(expected_episode_ids);
    }

    ExpectedEpisodeResolution::Resolved(expected_episode_ids)
}

/// The release name used to derive the expected episode units when the
/// submission scope does not name them outright: the grab-history name on the
/// tracked download, then the durable release evidence (the indexer release
/// title for a Scryer grab, else the client's release name). Never the download
/// client's mutable display label — verification must not expect episodes a
/// relabelled item merely appears to hold; with no real name it is simply not
/// applicable.
fn expected_episode_release_title(
    td: &TrackedDownload,
    release_evidence: Option<&crate::import_workflow::ReleaseEvidence>,
) -> Option<String> {
    td.source_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            release_evidence
                .and_then(|evidence| evidence.release_title(None))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

pub(super) async fn current_visible_video_file_count(
    app: &AppUseCase,
    td: &TrackedDownload,
    completed: Option<&CompletedDownload>,
) -> usize {
    let completed_lookup;
    let completed = match completed {
        Some(completed) => completed,
        None => {
            let Some(found) = find_completed_download(app, td, None).await else {
                return 0;
            };
            completed_lookup = found;
            &completed_lookup
        }
    };

    let path = std::path::Path::new(&completed.dest_dir);
    let filter_samples = td.facet.as_deref() != Some("movie");
    crate::import_workflow::find_video_files(path, filter_samples)
        .map(|files| files.len())
        .unwrap_or(0)
}

pub(super) async fn visible_source_files_have_terminal_dispositions(
    app: &AppUseCase,
    td: &TrackedDownload,
    artifacts: &[crate::ImportArtifact],
    completed: Option<&CompletedDownload>,
) -> Option<bool> {
    let completed_lookup;
    let completed = match completed {
        Some(completed) => completed,
        None => {
            let found = find_completed_download(app, td, None).await?;
            completed_lookup = found;
            &completed_lookup
        }
    };
    let filter_samples = td.facet.as_deref() != Some("movie");
    let files = crate::import_workflow::find_video_files(
        std::path::Path::new(&completed.dest_dir),
        filter_samples,
    )
    .ok()?;
    if files.is_empty() {
        return Some(
            artifact_member_completion(artifacts, td) != ArtifactMemberCompletion::Incomplete,
        );
    }

    let mut visible_file_name_counts: HashMap<String, usize> = HashMap::new();
    for file in &files {
        *visible_file_name_counts
            .entry(normalized_source_file_name(file))
            .or_default() += 1;
    }
    for file in files {
        let normalized_file_name = normalized_source_file_name(&file);
        let allow_filename_fallback = visible_file_name_counts
            .get(&normalized_file_name)
            .copied()
            .unwrap_or_default()
            == 1;
        let Some(rows) = import_artifact_rows_for_source_file(
            &file,
            completed,
            artifacts,
            allow_filename_fallback,
        ) else {
            return Some(false);
        };
        if !artifact_source_is_terminal(&rows, td) {
            return Some(false);
        }
    }

    Some(true)
}

pub(super) async fn visible_source_episode_units(
    app: &AppUseCase,
    td: &TrackedDownload,
    artifacts: &[crate::ImportArtifact],
    completed: Option<&CompletedDownload>,
) -> SourceVideoEpisodeResolution {
    let completed_lookup;
    let completed = match completed {
        Some(completed) => completed,
        None => {
            let Some(found) = find_completed_download(app, td, None).await else {
                return SourceVideoEpisodeResolution::Unavailable;
            };
            completed_lookup = found;
            &completed_lookup
        }
    };

    let Some(title_id) = td.title_id.as_deref() else {
        return SourceVideoEpisodeResolution::Unavailable;
    };
    let Some(title) = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await
        .ok()
        .flatten()
    else {
        return SourceVideoEpisodeResolution::Unavailable;
    };

    let path = std::path::Path::new(&completed.dest_dir);
    let filter_samples = td.facet.as_deref() != Some("movie");
    let files = match crate::import_workflow::find_video_files(path, filter_samples) {
        Ok(files) => files,
        Err(_) => {
            return artifact_source_episode_units(artifacts)
                .unwrap_or(SourceVideoEpisodeResolution::Unavailable);
        }
    };
    if files.is_empty() {
        return artifact_source_episode_units(artifacts)
            .unwrap_or(SourceVideoEpisodeResolution::NoVisibleVideos);
    }

    let parse_context = crate::build_release_parse_context(&title, None, None, td.facet.as_deref());
    let mut visible_file_name_counts: HashMap<String, usize> = HashMap::new();
    for file in &files {
        *visible_file_name_counts
            .entry(normalized_source_file_name(file))
            .or_default() += 1;
    }
    let mut units = HashSet::new();
    for file in files {
        let normalized_file_name = normalized_source_file_name(&file);
        let allow_filename_fallback = visible_file_name_counts
            .get(&normalized_file_name)
            .copied()
            .unwrap_or_default()
            == 1;
        if let Some(artifact_units) = episode_units_from_import_artifacts_for_source_file(
            &file,
            completed,
            artifacts,
            allow_filename_fallback,
        ) {
            units.extend(artifact_units);
            continue;
        }

        let file_name = file
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| file.to_string_lossy().into_owned());
        let parsed = crate::parse_release_metadata_for_target(&file_name, &parse_context);
        let Some(ep_meta) = parsed.episode.as_ref() else {
            tracing::debug!(
                file = %file.display(),
                source = %completed.name,
                "verify_import: visible source video did not parse to an episode"
            );
            return SourceVideoEpisodeResolution::Unmapped;
        };
        let season_str = ep_meta.season.unwrap_or(1).to_string();
        let episodes =
            crate::import_workflow::resolve_target_episodes(app, &title, ep_meta, &season_str)
                .await;
        if episodes.is_empty() {
            tracing::debug!(
                file = %file.display(),
                title_id = %title.id,
                source = %completed.name,
                "verify_import: visible source video did not resolve to catalog episodes"
            );
            return SourceVideoEpisodeResolution::Unmapped;
        }
        units.extend(episodes.into_iter().map(|episode| episode.id));
    }

    if units.is_empty() {
        SourceVideoEpisodeResolution::NoVisibleVideos
    } else {
        SourceVideoEpisodeResolution::Resolved(units)
    }
}

fn artifact_source_episode_units(
    artifacts: &[crate::ImportArtifact],
) -> Option<SourceVideoEpisodeResolution> {
    let mut grouped_units: HashMap<ImportArtifactSourceKey, HashSet<String>> = HashMap::new();
    for artifact in artifacts {
        let Some(source_key) = import_artifact_source_key(artifact) else {
            return Some(SourceVideoEpisodeResolution::Unmapped);
        };
        let group = grouped_units.entry(source_key).or_default();
        if let Some(episode_id) = artifact
            .episode_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            group.insert(episode_id.to_string());
        }
    }

    if grouped_units.is_empty() {
        return None;
    }

    let mut units = HashSet::new();
    for group_units in grouped_units.into_values() {
        if group_units.is_empty() {
            return Some(SourceVideoEpisodeResolution::Unmapped);
        }
        units.extend(group_units);
    }

    if units.is_empty() {
        Some(SourceVideoEpisodeResolution::Unmapped)
    } else {
        Some(SourceVideoEpisodeResolution::Resolved(units))
    }
}

fn artifact_member_completion(
    artifacts: &[crate::ImportArtifact],
    td: &TrackedDownload,
) -> ArtifactMemberCompletion {
    let Some(groups) = import_artifact_source_groups(artifacts) else {
        return ArtifactMemberCompletion::Incomplete;
    };
    if groups.is_empty() {
        return ArtifactMemberCompletion::Incomplete;
    }

    let mut all_ignored = true;
    for rows in groups.values() {
        match artifact_source_disposition(rows) {
            ArtifactSourceDisposition::Successful => all_ignored = false,
            ArtifactSourceDisposition::Ignored
                if ignored_artifact_rows_match_tracked_title(rows, td) => {}
            ArtifactSourceDisposition::Ignored
            | ArtifactSourceDisposition::Rejected
            | ArtifactSourceDisposition::Undisposed => {
                return ArtifactMemberCompletion::Incomplete;
            }
        }
    }

    if all_ignored {
        ArtifactMemberCompletion::AllIntentionallyIgnored
    } else {
        ArtifactMemberCompletion::Terminal
    }
}

fn artifact_source_is_terminal(artifacts: &[&crate::ImportArtifact], td: &TrackedDownload) -> bool {
    match artifact_source_disposition(artifacts) {
        ArtifactSourceDisposition::Successful => true,
        ArtifactSourceDisposition::Ignored => {
            ignored_artifact_rows_match_tracked_title(artifacts, td)
        }
        ArtifactSourceDisposition::Rejected | ArtifactSourceDisposition::Undisposed => false,
    }
}

fn import_artifact_source_groups(
    artifacts: &[crate::ImportArtifact],
) -> Option<HashMap<ImportArtifactSourceKey, Vec<&crate::ImportArtifact>>> {
    let mut groups: HashMap<ImportArtifactSourceKey, Vec<&crate::ImportArtifact>> = HashMap::new();
    for artifact in artifacts {
        let source_key = import_artifact_source_key(artifact)?;
        groups.entry(source_key).or_default().push(artifact);
    }

    Some(groups)
}

fn tracked_title_id(td: &TrackedDownload) -> Option<&str> {
    td.title_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

fn artifact_matches_tracked_title(artifact: &crate::ImportArtifact, title_id: &str) -> bool {
    artifact.title_id.as_deref().map(str::trim) == Some(title_id)
}

fn ignored_artifact_rows_match_tracked_title(
    artifacts: &[&crate::ImportArtifact],
    td: &TrackedDownload,
) -> bool {
    let Some(title_id) = tracked_title_id(td) else {
        return false;
    };
    artifacts
        .iter()
        .filter(|artifact| artifact.result == "ignored")
        .all(|artifact| artifact_matches_tracked_title(artifact, title_id))
}

fn artifact_source_disposition(artifacts: &[&crate::ImportArtifact]) -> ArtifactSourceDisposition {
    let mut has_ignored = false;
    let mut has_rejected = false;
    let mut has_undisposed = false;
    for artifact in artifacts {
        match artifact.result.as_str() {
            "imported" | "already_present" => return ArtifactSourceDisposition::Successful,
            "ignored" => has_ignored = true,
            "rejected" => has_rejected = true,
            _ => has_undisposed = true,
        }
    }

    if has_rejected {
        ArtifactSourceDisposition::Rejected
    } else if has_ignored && !has_undisposed {
        ArtifactSourceDisposition::Ignored
    } else {
        ArtifactSourceDisposition::Undisposed
    }
}

fn episode_units_from_import_artifacts_for_source_file(
    file: &Path,
    completed: &CompletedDownload,
    artifacts: &[crate::ImportArtifact],
    allow_filename_fallback: bool,
) -> Option<HashSet<String>> {
    let relative_path = file
        .strip_prefix(&completed.dest_dir)
        .ok()
        .map(path_to_stored_string)
        .filter(|path| !path.is_empty());
    if let Some(relative_path) = relative_path.as_deref() {
        let relative_matches = artifacts.iter().filter(|artifact| {
            artifact.relative_path.as_deref().map(str::trim) == Some(relative_path)
        });
        if let Some(units) = episode_units_from_artifact_rows(relative_matches) {
            return Some(units);
        }
    }

    if !allow_filename_fallback {
        return None;
    }

    let normalized_file_name = normalized_source_file_name(file);
    let file_name_matches = artifacts.iter().filter(|artifact| {
        artifact
            .normalized_file_name
            .trim()
            .eq_ignore_ascii_case(&normalized_file_name)
    });
    let file_name_matches = file_name_matches.collect::<Vec<_>>();
    if !artifact_rows_have_unique_source_key(&file_name_matches) {
        return None;
    }

    episode_units_from_artifact_rows(file_name_matches.into_iter())
}

fn import_artifact_rows_for_source_file<'a>(
    file: &Path,
    completed: &CompletedDownload,
    artifacts: &'a [crate::ImportArtifact],
    allow_filename_fallback: bool,
) -> Option<Vec<&'a crate::ImportArtifact>> {
    let relative_path = file
        .strip_prefix(&completed.dest_dir)
        .ok()
        .map(path_to_stored_string)
        .filter(|path| !path.is_empty());
    if let Some(relative_path) = relative_path.as_deref() {
        let rows = artifacts
            .iter()
            .filter(|artifact| {
                artifact.relative_path.as_deref().map(str::trim) == Some(relative_path)
            })
            .collect::<Vec<_>>();
        if !rows.is_empty() {
            return Some(rows);
        }
    }

    if !allow_filename_fallback {
        return None;
    }

    let normalized_file_name = normalized_source_file_name(file);
    let rows = artifacts
        .iter()
        .filter(|artifact| {
            artifact
                .normalized_file_name
                .trim()
                .eq_ignore_ascii_case(&normalized_file_name)
        })
        .collect::<Vec<_>>();
    (!rows.is_empty() && artifact_rows_have_unique_source_key(&rows)).then_some(rows)
}

fn episode_units_from_artifact_rows<'a>(
    artifacts: impl Iterator<Item = &'a crate::ImportArtifact>,
) -> Option<HashSet<String>> {
    let mut units = HashSet::new();
    for artifact in artifacts {
        if let Some(episode_id) = artifact
            .episode_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            units.insert(episode_id.to_string());
        }
    }

    (!units.is_empty()).then_some(units)
}

fn import_artifact_source_key(artifact: &crate::ImportArtifact) -> Option<ImportArtifactSourceKey> {
    if let Some(relative_path) = artifact
        .relative_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(ImportArtifactSourceKey::RelativePath(
            relative_path.to_string(),
        ));
    }

    let normalized_file_name = artifact.normalized_file_name.trim();
    if normalized_file_name.is_empty() {
        None
    } else {
        Some(ImportArtifactSourceKey::NormalizedFileName(
            normalized_file_name.to_ascii_lowercase(),
        ))
    }
}

fn artifact_rows_have_unique_source_key(artifacts: &[&crate::ImportArtifact]) -> bool {
    let mut source_keys = artifacts
        .iter()
        .filter_map(|artifact| import_artifact_source_key(artifact))
        .collect::<HashSet<_>>();
    if source_keys.len() <= 1 {
        return true;
    }

    source_keys.retain(|key| matches!(key, ImportArtifactSourceKey::RelativePath(_)));
    source_keys.len() <= 1
}

fn normalized_source_file_name(file: &Path) -> String {
    file.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
        .unwrap_or_else(|| file.to_string_lossy().to_ascii_lowercase())
}

#[cfg(test)]
mod expected_episode_release_title_tests {
    use super::*;
    use scryer_domain::Id;

    fn tracked_download(source_title: Option<&str>, display_label: &str) -> TrackedDownload {
        TrackedDownload {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            id: "client-1:dl-1".to_string(),
            client_id: "client-1".to_string(),
            client_type: "qbittorrent".to_string(),
            client_item: DownloadQueueItem {
                id: Id::new().0,
                title_id: Some("title-1".to_string()),
                episode_id: None,
                title_name: display_label.to_string(),
                facet: Some("series".to_string()),
                category: None,
                client_id: "client-1".to_string(),
                client_name: "qBittorrent".to_string(),
                client_type: "qbittorrent".to_string(),
                state: DownloadQueueState::Completed,
                progress_percent: 100,
                import_transfer_phase: None,
                import_transfer_bytes: None,
                import_transfer_total_bytes: None,
                import_transfer_started_at: None,
                import_transfer_updated_at: None,
                size_bytes: None,
                remaining_seconds: None,
                queued_at: None,
                last_updated_at: None,
                attention_required: false,
                attention_reason: None,
                download_client_item_id: "dl-1".to_string(),
                download_id: None,
                import_status: None,
                import_error_code: None,
                import_error_message: None,
                imported_at: None,
                delete_status: None,
                delete_error_message: None,
                source_provider: None,
                is_scryer_origin: false,
                tracked_state: None,
                tracked_status: None,
                tracked_status_messages: vec![],
                tracked_match_type: None,
                seeding: None,
            },
            completed_source: None,
            state: TrackedDownloadState::ImportPending,
            status: TrackedDownloadStatus::Ok,
            status_messages: vec![],
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: source_title.map(str::to_string),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::TitleParse,
            is_trackable: true,
            import_attempted: false,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            import_execution_retry: None,
            import_hold: None,
            skip_reacquire_on_failure: false,
            burned_by_import_gate: false,
            snapshot_missing_since: None,
        }
    }

    #[test]
    fn expected_episode_release_title_prefers_release_evidence_over_display_label() {
        let td = tracked_download(None, "Renamed by the user in the client");
        let evidence = crate::import_workflow::ReleaseEvidence::DownloaderObservation {
            release_name: Some("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string()),
        };

        assert_eq!(
            expected_episode_release_title(&td, Some(&evidence)).as_deref(),
            Some("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb")
        );
    }

    #[test]
    fn expected_episode_release_title_keeps_grab_history_first_and_never_uses_display_label() {
        let td = tracked_download(
            Some("Harbor.Pals.S01E02.1080p.WEB-DL.H264-GRP"),
            "Renamed by the user in the client",
        );
        let evidence = crate::import_workflow::ReleaseEvidence::DownloaderObservation {
            release_name: Some("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string()),
        };
        assert_eq!(
            expected_episode_release_title(&td, Some(&evidence)).as_deref(),
            Some("Harbor.Pals.S01E02.1080p.WEB-DL.H264-GRP")
        );

        // No grab history and no release evidence: the display label is not a
        // release name, so the expectation is simply not applicable.
        let no_evidence = tracked_download(Some("  "), "Renamed by the user in the client");
        let empty_evidence =
            crate::import_workflow::ReleaseEvidence::DownloaderObservation { release_name: None };
        assert_eq!(
            expected_episode_release_title(&no_evidence, Some(&empty_evidence)),
            None
        );
        assert_eq!(expected_episode_release_title(&no_evidence, None), None);
    }
}
