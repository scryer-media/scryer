fn base_completed_import_result(
    import_id: &str,
    completed: &CompletedDownload,
    started_at: DateTime<Utc>,
) -> ImportResult {
    ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: None,
        title_id: None,
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: Some(completed.name.clone()),
        source_path: completed.dest_dir.clone(),
        dest_path: None,
        quality: None,
        episode_ids: Vec::new(),
        file_size_bytes: None,
        link_type: None,
        error_message: None,
        started_at,
        completed_at: Utc::now(),
    }
}
fn facet_for_completed_download(completed: &CompletedDownload) -> Option<MediaFacet> {
    match extract_parameter(&completed.parameters, "*scryer_facet")
        .as_deref()
        .map(str::trim)
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("movie") => Some(MediaFacet::Movie),
        Some("series") => Some(MediaFacet::Series),
        Some("anime") => Some(MediaFacet::Anime),
        _ => None,
    }
}
fn facet_from_tracked_label(value: Option<&str>) -> Option<MediaFacet> {
    match value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("movie") => Some(MediaFacet::Movie),
        Some("series") => Some(MediaFacet::Series),
        Some("anime") => Some(MediaFacet::Anime),
        _ => None,
    }
}
// ---------------------------------------------------------------------------
// Series import: process ALL video files, link each to its episode
// ---------------------------------------------------------------------------

async fn import_series_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    let ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template,
    } = resolve_import_paths(app, title).await?;
    let full_folder_path = effective_title_folder_path(&media_root, title, &folder_template, None);

    let quality_profile = resolve_import_quality_profile(app, title).await;

    let nfo_enabled = app
        .resolve_nfo_write_on_import(Some(&title.library_id), &title.facet)
        .await?;
    let import_mode = app
        .resolve_import_mode(Some(&title.library_id), &title.facet)
        .await?;

    let mut imported_count: usize = 0;
    let mut skipped_count: usize = 0;
    let mut rejected_count: usize = 0;
    let mut failed_count: usize = 0;
    let mut last_error: Option<String> = None;
    let mut last_rejection_skip_reason: Option<ImportSkipReason> = None;
    let mut last_skipped_message: Option<String> = None;
    let mut last_skipped_skip_reason: Option<ImportSkipReason> = None;
    let mut imported_updates: Vec<NotificationMediaUpdate> = Vec::new();
    let mut imported_episode_ids: Vec<String> = Vec::new();
    let mut attributed_episode_ids: Vec<String> = Vec::new();
    let mut imported_link_type: Option<scryer_domain::ImportStrategy> = None;
    let expected_episode_ids =
        expected_episode_ids_for_completed_download(app, title, completed).await;

    for source_video in video_files {
        match import_single_episode_file(
            app,
            actor,
            title,
            import_id,
            rename_enabled,
            &rename_template,
            &season_folder_template,
            &full_folder_path,
            completed,
            source_video,
            video_files.len() > 1,
            &quality_profile,
            nfo_enabled,
            expected_episode_ids.as_ref(),
        )
        .await
        {
            Ok(EpisodeImportOutcome::Imported {
                dest_path,
                episode_ids,
                link_type,
                ..
            }) => {
                imported_count += 1;
                imported_updates.push(NotificationMediaUpdate::created(dest_path));
                append_unique_episode_ids(&mut imported_episode_ids, &episode_ids);
                append_unique_episode_ids(&mut attributed_episode_ids, &episode_ids);
                if link_type == Some(scryer_domain::ImportStrategy::Move) {
                    imported_link_type = link_type;
                }
            }
            Ok(EpisodeImportOutcome::Skipped {
                message,
                skip_reason,
                episode_ids,
                ..
            }) => {
                skipped_count += 1;
                append_unique_episode_ids(&mut attributed_episode_ids, &episode_ids);
                last_skipped_message = Some(message);
                last_skipped_skip_reason = skip_reason;
            }
            Ok(EpisodeImportOutcome::Rejected {
                rejection,
                episode_ids,
                ..
            }) => {
                rejected_count += 1;
                append_unique_episode_ids(&mut attributed_episode_ids, &episode_ids);
                last_error = Some(rejection.message.clone());
                last_rejection_skip_reason = rejection.skip_reason.clone();
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    file = %source_video.display(),
                    title = %title.name,
                    "failed to import episode file"
                );
                last_error = Some(err.to_string());
                failed_count += 1;
            }
        }
    }

    if imported_count > 0 {
        persist_title_folder_path_if_missing(app, title, &full_folder_path).await;
        write_series_sidecars(app, title, &full_folder_path, nfo_enabled).await;
    }

    let move_import_has_failure =
        import_mode == scryer_domain::ImportMode::Move && failed_count > 0;
    let (decision, status, skip_reason) = if move_import_has_failure {
        (ImportDecision::Failed, ImportStatus::Failed, None)
    } else if imported_count > 0 {
        (ImportDecision::Imported, ImportStatus::Completed, None)
    } else if failed_count > 0 {
        (ImportDecision::Failed, ImportStatus::Failed, None)
    } else if rejected_count > 0 {
        (
            ImportDecision::Rejected,
            ImportStatus::Failed,
            last_rejection_skip_reason,
        )
    } else {
        // All files skipped (no parseable episode info, already imported, etc.)
        // — this is a permanent condition, not worth retrying.
        (
            ImportDecision::Skipped,
            ImportStatus::Skipped,
            last_skipped_skip_reason,
        )
    };

    let error_message = if imported_count == 0
        && failed_count == 0
        && rejected_count == 0
        && skipped_count > 0
    {
        last_skipped_message
    } else if failed_count > 0 || skipped_count > 0 || rejected_count > 0 {
        Some(format!(
            "{imported_count} imported, {skipped_count} skipped, {rejected_count} rejected, {failed_count} failed{}",
            last_error
                .as_ref()
                .map(|e| format!(". Last error: {e}"))
                .unwrap_or_default()
        ))
    } else {
        None
    };

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision,
        skip_reason,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: Some(completed.name.clone()),
        source_path: completed.dest_dir.clone(),
        dest_path: None,
        quality: None,
        episode_ids: attributed_episode_ids,
        file_size_bytes: None,
        link_type: imported_link_type,
        error_message,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    let status = completed_import_status_for_result(&result, status);
    app.update_import_status_and_notify(import_id, status, result_json)
        .await?;

    if imported_count > 0 && !move_import_has_failure {
        app.append_domain_event(new_title_domain_event(
            actor,
            title,
            DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: title_context_snapshot(title),
                media_updates: imported_updates
                    .into_iter()
                    .map(|update| created_media_update(update.path))
                    .collect(),
                imported_count: imported_count as i32,
                import_id: Some(import_id.to_string()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: Some(completed.name.clone()),
                source_path: Some(completed.dest_dir.clone()),
                dest_path: None,
                quality: None,
                episode_ids: imported_episode_ids,
            }),
        ))
        .await?;
    }

    Ok(result)
}
enum EpisodeImportOutcome {
    Imported {
        dest_path: String,
        episode_ids: Vec<String>,
        imported_media_file_id: Option<String>,
        reason_code: Option<String>,
        link_type: Option<scryer_domain::ImportStrategy>,
    },
    Skipped {
        message: String,
        reason_code: Option<String>,
        skip_reason: Option<ImportSkipReason>,
        episode_ids: Vec<String>,
    },
    Rejected {
        rejection: crate::post_download_gate::ImportedFileRejection,
        finalize_before_import: bool,
        reason_code: Option<String>,
        episode_ids: Vec<String>,
    },
}
fn append_unique_episode_ids(target: &mut Vec<String>, source: &[String]) {
    for episode_id in source {
        if !target.contains(episode_id) {
            target.push(episode_id.clone());
        }
    }
}
#[derive(Clone, Debug)]
struct EpisodeUpgradePlan {
    primary_incumbent: crate::EpisodeScopedMediaFile,
    additional_superseded: Vec<crate::EpisodeScopedMediaFile>,
    previous_best_score: i32,
}
async fn expected_episode_ids_for_completed_download(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    completed: &CompletedDownload,
) -> Option<HashSet<String>> {
    let identity = completed_download_identity(completed);
    let submission = app
        .services
        .workflow
        .download_submissions
        .find_by_client_item_id(&identity)
        .await
        .ok()
        .flatten();

    if let Some(submission) = submission.as_ref()
        && let Some(ids) =
            expected_episode_ids_from_submission_scope(app, title, &submission.scope).await
        && !ids.is_empty()
    {
        return Some(ids);
    }

    let release_title = submission
        .as_ref()
        .and_then(|submission| submission.source_title.as_deref())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(completed.name.as_str());
    expected_episode_ids_from_release_title(app, title, release_title).await
}
async fn expected_episode_ids_from_submission_scope(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    scope: &SubmissionScope,
) -> Option<HashSet<String>> {
    match scope {
        SubmissionScope::Episode { episode_id } => Some(HashSet::from([episode_id.clone()])),
        SubmissionScope::EpisodeSet { episode_ids } => Some(episode_ids.iter().cloned().collect()),
        SubmissionScope::Collection { collection_id } => {
            episode_ids_for_collection(app, title, collection_id, true).await
        }
        SubmissionScope::Title | SubmissionScope::SeriesMovie { .. } | SubmissionScope::Orphan => None,
    }
}
async fn expected_episode_ids_from_release_title(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    release_title: &str,
) -> Option<HashSet<String>> {
    let parsed = normalize_release_title_signal(parse_release_metadata(release_title));
    let ep_meta = parsed.episode.as_ref()?;
    let season = ep_meta.season.unwrap_or(1).to_string();
    let mut episodes = resolve_target_episodes(app, title, ep_meta, &season).await;

    if ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack {
        let monitored: Vec<_> = episodes
            .iter()
            .filter(|episode| episode.monitored)
            .map(|episode| episode.id.clone())
            .collect();
        if !monitored.is_empty() {
            return Some(monitored.into_iter().collect());
        }
    }

    if episodes.is_empty() {
        None
    } else {
        Some(episodes.drain(..).map(|episode| episode.id).collect())
    }
}
fn resolved_episode_ids_are_within_expected(
    target_episode_ids: &[String],
    expected_episode_ids: &HashSet<String>,
) -> bool {
    target_episode_ids.is_empty()
        || target_episode_ids
            .iter()
            .all(|episode_id| expected_episode_ids.contains(episode_id))
}
async fn episode_ids_for_collection(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    collection_id: &str,
    monitored_only: bool,
) -> Option<HashSet<String>> {
    match app
        .services
        .catalog
        .shows
        .list_episodes_for_collection(collection_id)
        .await
    {
        Ok(episodes) => {
            let ids: HashSet<String> = episodes
                .into_iter()
                .filter(|episode| episode.title_id == title.id)
                .filter(|episode| !monitored_only || episode.monitored)
                .map(|episode| episode.id)
                .collect();
            (!ids.is_empty()).then_some(ids)
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                collection_id,
                title_id = %title.id,
                "failed to resolve expected grabbed-release episode set"
            );
            None
        }
    }
}
fn reject_broader_episode_incumbent(
    incumbent: &crate::EpisodeScopedMediaFile,
) -> crate::post_download_gate::ImportedFileRejection {
    crate::post_download_gate::ImportedFileRejection {
        message: format!(
            "existing episode file {} spans a broader episode set and cannot be replaced by this import",
            incumbent.media_file.file_path
        ),
        recycle_reason: "policy_mismatch",
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        blocking_rule_codes: Vec::new(),
    }
}
fn reject_non_upgrade_episode_incumbent(
    incumbent: &crate::EpisodeScopedMediaFile,
    new_score: i32,
) -> crate::post_download_gate::ImportedFileRejection {
    let old_score = media_file_score(&incumbent.media_file);
    crate::post_download_gate::ImportedFileRejection {
        message: format!(
            "existing episode file {} is equal or better (score {} >= {})",
            incumbent.media_file.file_path, old_score, new_score
        ),
        recycle_reason: "already_imported",
        skip_reason: Some(ImportSkipReason::AlreadyImported),
        blocking_rule_codes: Vec::new(),
    }
}
fn build_episode_upgrade_plan(
    incumbents: &[crate::EpisodeScopedMediaFile],
    target_episode_ids: &[String],
    new_score: i32,
) -> Result<EpisodeUpgradePlan, crate::post_download_gate::ImportedFileRejection> {
    let target_episode_ids = target_episode_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut sorted_incumbents = incumbents.to_vec();
    sorted_incumbents.sort_by(|left, right| {
        media_file_score(&right.media_file)
            .cmp(&media_file_score(&left.media_file))
            .then_with(|| right.media_file.created_at.cmp(&left.media_file.created_at))
            .then_with(|| right.media_file.id.cmp(&left.media_file.id))
    });

    for incumbent in &sorted_incumbents {
        let incumbent_episode_ids = incumbent
            .episode_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if !incumbent_episode_ids.is_subset(&target_episode_ids) {
            return Err(reject_broader_episode_incumbent(incumbent));
        }

        if new_score <= media_file_score(&incumbent.media_file) {
            return Err(reject_non_upgrade_episode_incumbent(incumbent, new_score));
        }
    }

    let previous_best_score = sorted_incumbents
        .iter()
        .map(|incumbent| media_file_score(&incumbent.media_file))
        .max()
        .unwrap_or(0);
    let primary_incumbent = sorted_incumbents.remove(0);

    Ok(EpisodeUpgradePlan {
        primary_incumbent,
        additional_superseded: sorted_incumbents,
        previous_best_score,
    })
}
async fn cleanup_superseded_episode_incumbents(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    superseded: &[crate::EpisodeScopedMediaFile],
    replacement_file_id: &str,
    replacement_path: &Path,
) {
    for incumbent in superseded {
        let mut recycle_result = None;
        let old_path = crate::stored_paths::stored_path_to_path_buf(&incumbent.media_file.file_path);
        if old_path.exists() {
            let old_file_recycle_context =
                match crate::upgrade::resolve_old_file_recycle_context(
                    app,
                    title,
                    &incumbent.media_file,
                )
                .await
                {
                    Ok(context) => context,
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            path = %old_path.display(),
                            file_id = %incumbent.media_file.id,
                            "failed to resolve recycle context for superseded episode incumbent; keeping its database record to avoid orphaning the on-disk file"
                        );
                        continue;
                    }
                };
            let metadata = crate::recycle_bin::ReplacedMediaRecycleMetadata {
                original_path: &incumbent.media_file.file_path,
                original_file_id: &incumbent.media_file.id,
                size_bytes: incumbent.media_file.size_bytes as u64,
                title_id: &title.id,
                media_root: Some(old_file_recycle_context.media_root.as_str()),
            };

            match crate::recycle_bin::recycle_replaced_media_file(
                &old_file_recycle_context.recycle_config,
                &old_path,
                metadata,
                true,
            )
            .await
            {
                Ok(result) => recycle_result = result,
                Err(error) => {
                    // Physical cleanup failed or was refused for safety. The file is
                    // still on disk, so keep its database record rather than orphaning
                    // the file; a later upgrade can retry cleanup.
                    tracing::warn!(
                        error = %error,
                        path = %old_path.display(),
                        file_id = %incumbent.media_file.id,
                        "failed to recycle superseded episode incumbent; keeping its database record to avoid orphaning the on-disk file"
                    );
                    continue;
                }
            }
        }

        if let Err(error) = app
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::MediaFileDeleted(scryer_domain::MediaFileDeletedEventData {
                    title: title_context_snapshot(title),
                    media_updates: vec![deleted_media_update(
                        incumbent.media_file.file_path.clone(),
                    )],
                    file_id: Some(incumbent.media_file.id.clone()),
                    reason: scryer_domain::MediaFileDeletedReason::UpgradeCleanup,
                    episode_ids: incumbent.episode_ids.clone(),
                }),
            ))
            .await
        {
            tracing::warn!(
                error = %error,
                file_id = %incumbent.media_file.id,
                "failed to emit superseded episode cleanup event"
            );
        }

        let deleted_record = match app
            .delete_media_file_record_with_dependents(&incumbent.media_file.id)
            .await
        {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    file_id = %incumbent.media_file.id,
                    "failed to delete superseded episode media file record"
                );
                false
            }
        };

        if deleted_record
            && let Err(error) = crate::recycle_bin::commit_recycle_entry(
                &recycle_result,
                replacement_file_id,
                replacement_path,
            )
            .await
        {
            tracing::warn!(
                error = %error,
                file_id = %incumbent.media_file.id,
                "superseded recycle entry could not be committed; it will not auto-purge"
            );
        }
    }
}
/// Import a single episode video file: parse, gate, import, and link.
#[expect(
    clippy::too_many_arguments,
    reason = "single-episode imports need the full source, rename, and persistence context together"
)]
async fn import_single_episode_file(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    rename_enabled: bool,
    rename_template: &str,
    season_folder_template: &str,
    title_folder_path: &Path,
    completed: &CompletedDownload,
    source_video: &Path,
    other_video_files: bool,
    quality_profile: &crate::QualityProfile,
    nfo_enabled: bool,
    expected_episode_ids: Option<&HashSet<String>>,
) -> AppResult<EpisodeImportOutcome> {
    let parsed =
        build_augmented_episode_import_metadata(source_video, completed, other_video_files);

    // Must have episode info to proceed
    let ep_meta = match parsed.episode.as_ref() {
        Some(ep) if !ep.episode_numbers.is_empty() => ep,
        Some(ep)
            if ep.absolute_episode.is_some() && title.facet == scryer_domain::MediaFacet::Anime =>
        {
            ep
        }
        Some(ep) if ep.air_date.is_some() => ep,
        Some(ep) if ep.release_type == crate::ParsedEpisodeReleaseType::SeasonPack => ep,
        _ => {
            tracing::debug!(
                file = %source_video.display(),
                "skipping file with no parseable episode info"
            );
            return Ok(EpisodeImportOutcome::Skipped {
                message: "file has no parseable episode info".to_string(),
                reason_code: None,
                skip_reason: Some(ImportSkipReason::UnparseableEpisode),
                episode_ids: Vec::new(),
            });
        }
    };

    let season = ep_meta.season.unwrap_or(1);
    let season_str = season.to_string();

    // Resolve target episodes early so we can enrich rename tokens with DB
    // metadata (e.g. absolute_number from TVDB).
    let target_episodes = resolve_target_episodes(app, title, ep_meta, &season_str).await;
    let target_episode_ids: Vec<String> = target_episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect();
    if let Some(expected_episode_ids) = expected_episode_ids
        && !resolved_episode_ids_are_within_expected(&target_episode_ids, expected_episode_ids)
    {
        return Ok(EpisodeImportOutcome::Rejected {
            rejection: crate::post_download_gate::ImportedFileRejection {
                message: "file resolves to episode(s) outside the grabbed release".to_string(),
                recycle_reason: "episode_outside_grabbed_release",
                skip_reason: Some(ImportSkipReason::PolicyMismatch),
                blocking_rule_codes: vec!["episode_outside_grabbed_release".to_string()],
            },
            finalize_before_import: false,
            reason_code: Some("episode_outside_grabbed_release".to_string()),
            episode_ids: target_episode_ids.clone(),
        });
    }
    let ep_num_str = ep_meta
        .episode_numbers
        .first()
        .map(|n| n.to_string())
        .unwrap_or_default();
    let abs_str = ep_meta.absolute_episode.map(|n| n.to_string()).or_else(|| {
        target_episodes
            .first()
            .and_then(|ep| ep.absolute_number.clone())
    });
    let episode_title = target_episodes.first().and_then(|ep| ep.title.as_deref());
    let additional_import = completed_import_purpose(app, completed)
        .await
        .is_additional_file();
    let outcome = execute_resolved_episode_import(
        app,
        actor,
        title,
        import_id,
        rename_enabled,
        rename_template,
        season_folder_template,
        title_folder_path,
        source_video,
        &parsed,
        &target_episodes,
        &target_episodes,
        season as u32,
        &ep_num_str,
        abs_str.as_deref(),
        episode_title,
        quality_profile,
        None,
        crate::post_download_gate::RuntimeSampleValidationMode::EnforceAutomatic,
        additional_import,
    )
    .await?;

    match &outcome {
        EpisodeImportOutcome::Imported {
            dest_path,
            imported_media_file_id,
            reason_code,
            ..
        } => {
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "imported",
                reason_code.as_deref(),
                imported_media_file_id.as_deref(),
                &target_episodes,
            )
            .await;

            if imported_media_file_id.is_some()
                && reason_code.as_deref() != Some("additional_file")
            {
                if nfo_enabled {
                    let nfo_path = std::path::Path::new(dest_path).with_extension("nfo");
                    if let Some(episode) = target_episodes.first() {
                        let nfo_content = render_episode_nfo(title, episode);
                        if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await
                        {
                            tracing::warn!(
                                error = %err,
                                path = %nfo_path.display(),
                                "failed to write episode NFO sidecar"
                            );
                        }
                    }
                }

                spawn_post_processing(PostProcessingContext {
                    app: app.clone(),
                    actor: crate::domain_events::DomainEventActor::from(actor),
                    title_id: title.id.clone(),
                    title_name: title.name.clone(),
                    facet: title.facet.clone(),
                    dest_path: PathBuf::from(dest_path),
                    year: title.year,
                    imdb_id: title
                        .external_ids
                        .iter()
                        .find(|e| e.source == "imdb")
                        .map(|e| e.value.clone()),
                    tvdb_id: title
                        .external_ids
                        .iter()
                        .find(|e| e.source == "tvdb")
                        .map(|e| e.value.clone()),
                    season: Some(season),
                    episode: ep_meta.episode_numbers.first().copied(),
                    quality: parsed.quality.clone(),
                });
            }
        }
        EpisodeImportOutcome::Skipped {
            reason_code,
            skip_reason,
            ..
        } => {
            let artifact_result = if reason_code.as_deref() == Some("duplicate_file")
                || matches!(
                    skip_reason.as_ref(),
                    Some(ImportSkipReason::AlreadyImported | ImportSkipReason::DuplicateFile)
                ) {
                "already_present"
            } else {
                "rejected"
            };
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                artifact_result,
                reason_code.as_deref(),
                None,
                &target_episodes,
            )
            .await;
        }
        EpisodeImportOutcome::Rejected {
            rejection,
            finalize_before_import,
            reason_code,
            ..
        } => {
            if *finalize_before_import {
                crate::post_download_gate::reject_source_file_before_import(
                    app,
                    crate::domain_events::DomainEventActor::from(actor),
                    title,
                    &completed.name,
                    source_video,
                    &target_episode_ids,
                    rejection,
                )
                .await;
            }

            let artifact_result = if matches!(
                rejection.skip_reason.as_ref(),
                Some(ImportSkipReason::AlreadyImported | ImportSkipReason::DuplicateFile)
            ) {
                "already_present"
            } else {
                "rejected"
            };

            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                artifact_result,
                reason_code
                    .as_deref()
                    .or_else(|| rejection.skip_reason.as_ref().map(ImportSkipReason::as_str)),
                None,
                &target_episodes,
            )
            .await;
        }
    }

    Ok(outcome)
}
/// Resolve media root path and rename template for a title's facet.
pub(crate) async fn resolve_import_paths(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> AppResult<ImportPathSettings> {
    let rename_settings = crate::facet_handler::rename_facet_settings(&title.facet);
    let media_root = app.title_root_folder_path_override(title).await?;

    let rename_enabled = app.resolve_rename_enabled(&title.facet).await?;
    let rename_template = app
        .read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            rename_settings.template_key,
            None,
        )
        .await?
        .unwrap_or_else(|| rename_settings.default_template.to_string());
    let folder_template = app
        .read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            super::FOLDER_TEMPLATE_KEY,
            Some(title.facet.as_str()),
        )
        .await?;
    let default_folder_template = match title.facet {
        MediaFacet::Movie => super::DEFAULT_FOLDER_TEMPLATE_MOVIE,
        MediaFacet::Series => super::DEFAULT_FOLDER_TEMPLATE_SERIES,
        MediaFacet::Anime => super::DEFAULT_FOLDER_TEMPLATE_ANIME,
    };
    let folder_template = crate::normalize_title_folder_template_or_default(
        folder_template,
        default_folder_template,
        title.facet.as_str(),
    );
    let season_folder_template = app
        .read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            super::SEASON_FOLDER_TEMPLATE_KEY,
            Some(title.facet.as_str()),
        )
        .await?;
    let default_season_folder_template = match title.facet {
        MediaFacet::Movie | MediaFacet::Series => super::DEFAULT_SEASON_FOLDER_TEMPLATE_SERIES,
        MediaFacet::Anime => super::DEFAULT_SEASON_FOLDER_TEMPLATE_ANIME,
    };
    let season_folder_template = crate::normalize_season_folder_template_or_default(
        season_folder_template,
        default_season_folder_template,
        title.facet.as_str(),
    );

    Ok(ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template,
    })
}
/// Compute the destination path for an episode import using the canonical
/// token set: base tokens from parsed release metadata, overridden by the
/// explicit episode values supplied by the caller.
///
/// `ep_num_str` may be empty to leave `{episode}` blank (anime absolute-only
/// files where no per-season episode number is known).
/// `quality_override` replaces the filename-parsed quality token when the
/// caller supplies an explicit label (e.g. manual import).
#[expect(
    clippy::too_many_arguments,
    reason = "episode rename rendering uses the full canonical token set explicitly"
)]
pub(crate) fn episode_import_dest_path(
    title: &scryer_domain::Title,
    parsed: &crate::ParsedReleaseMetadata,
    ext: &str,
    source_path: &Path,
    title_folder_path: &Path,
    rename_enabled: bool,
    rename_template: &str,
    season_folder_template: &str,
    season_num: u32,
    ep_num_str: &str,
    absolute_number: Option<&str>,
    episode_title: Option<&str>,
    quality_override: Option<&str>,
) -> PathBuf {
    let mut tokens = build_rename_tokens(title, parsed, ext);
    tokens.insert("season".to_string(), season_num.to_string());
    tokens.insert("season_order".to_string(), season_num.to_string());
    tokens.insert("episode".to_string(), ep_num_str.to_string());
    tokens.insert(
        "absolute_episode".to_string(),
        absolute_number.unwrap_or("").to_string(),
    );
    tokens.insert(
        "episode_title".to_string(),
        episode_title.unwrap_or("").to_string(),
    );
    if let Some(q) = quality_override {
        tokens.insert("quality".to_string(), q.to_string());
    }
    let rendered = if rename_enabled {
        render_rename_template(rename_template, &tokens)
    } else {
        preserved_import_filename(source_path)
    };
    if use_season_folders(title) {
        let season_folder = render_title_folder_template(season_folder_template, &tokens);
        title_folder_path.join(&season_folder).join(&rendered)
    } else {
        title_folder_path.join(&rendered)
    }
}
/// Check whether the title's tags request season-folder organisation.
/// Defaults to `true` (use season folders) when the tag is absent.
pub(crate) fn use_season_folders(title: &scryer_domain::Title) -> bool {
    title
        .tags
        .iter()
        .find(|t| t.starts_with("scryer:season-folder:"))
        .map(|t| {
            !t.trim_start_matches("scryer:season-folder:")
                .eq_ignore_ascii_case("disabled")
        })
        .unwrap_or(true)
}
/// Build the common rename token map from parsed release metadata.
pub(crate) fn build_rename_tokens(
    title: &scryer_domain::Title,
    parsed: &crate::ParsedReleaseMetadata,
    ext: &str,
) -> BTreeMap<String, String> {
    let mut tokens = BTreeMap::new();
    let fallback_title_year = title.year;
    let resolved_year = parsed.year.or(fallback_title_year);
    tokens.insert("title".to_string(), title.name.clone());
    tokens.insert(
        "year".to_string(),
        resolved_year.map(|y| y.to_string()).unwrap_or_default(),
    );
    tokens.insert(
        "quality".to_string(),
        parsed
            .quality
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
    );
    tokens.insert(
        "source".to_string(),
        parsed
            .source
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    tokens.insert(
        "video_codec".to_string(),
        parsed
            .video_codec
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    tokens.insert(
        "audio".to_string(),
        parsed
            .audio
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    tokens.insert(
        "release_group".to_string(),
        parsed.release_group.clone().unwrap_or_default(),
    );
    tokens.insert(
        "season".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.season)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert(
        "episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.episode_numbers.first().copied())
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert(
        "absolute_episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.absolute_episode)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert("episode_title".to_string(), String::new());
    tokens.insert("ext".to_string(), ext.to_string());
    tokens
}
pub(crate) async fn resolve_target_episodes(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    ep_meta: &crate::ParsedEpisodeMetadata,
    season_str: &str,
) -> Vec<scryer_domain::Episode> {
    let mut episodes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let target_season = crate::parsed_episode_lookup_season(ep_meta, season_str);

    if let Some(air_date) = ep_meta.air_date {
        let air_date_str = air_date.format("%Y-%m-%d").to_string();
        match app
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
            Ok(collections) => {
                let mut matches = Vec::new();
                for collection in collections {
                    match app
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_collection(&collection.id)
                        .await
                    {
                        Ok(collection_episodes) => {
                            matches.extend(collection_episodes.into_iter().filter(|episode| {
                                episode.title_id == title.id
                                    && episode.air_date.as_deref() == Some(air_date_str.as_str())
                            }));
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "daily episode lookup failed during import")
                        }
                    }
                }

                matches.sort_by_key(|episode| {
                    episode
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(u32::MAX)
                });

                if let Some(part) = ep_meta.daily_part {
                    let part_index = part.saturating_sub(1) as usize;
                    if let Some(episode) = matches.into_iter().nth(part_index)
                        && seen.insert(episode.id.clone())
                    {
                        episodes.push(episode);
                    }
                } else {
                    for episode in matches {
                        if seen.insert(episode.id.clone()) {
                            episodes.push(episode);
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "daily collection lookup failed during import")
            }
        }
    }

    for episode_number in &ep_meta.episode_numbers {
        let episode_str = episode_number.to_string();
        match app
            .services
            .catalog
            .shows
            .find_episode_by_title_and_numbers(&title.id, &target_season, &episode_str)
            .await
        {
            Ok(Some(episode)) => {
                if seen.insert(episode.id.clone()) {
                    episodes.push(episode);
                }
            }
            Ok(None) => {
                tracing::debug!(
                    title_id = %title.id,
                    season = %season_str,
                    episode = %episode_str,
                    "no matching episode found for imported file"
                );
            }
            Err(err) => tracing::warn!(error = %err, "episode lookup failed during import"),
        }
    }

    if episodes.is_empty()
        && ep_meta.season.is_some()
        && ep_meta.episode_numbers.is_empty()
        && ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
    {
        match app
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
            Ok(collections) => {
                for collection in collections
                    .into_iter()
                    .filter(|collection| collection.collection_index == target_season)
                {
                    match app
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_collection(&collection.id)
                        .await
                    {
                        Ok(collection_episodes) => {
                            let mut collection_episodes: Vec<_> = collection_episodes
                                .into_iter()
                                .filter(|episode| {
                                    episode.title_id == title.id
                                        && episode.season_number.as_deref()
                                            == Some(target_season.as_str())
                                })
                                .collect();
                            collection_episodes.sort_by_key(|episode| {
                                episode
                                    .episode_number
                                    .as_deref()
                                    .and_then(|value| value.parse::<u32>().ok())
                                    .unwrap_or(u32::MAX)
                            });
                            for episode in collection_episodes {
                                if seen.insert(episode.id.clone()) {
                                    episodes.push(episode);
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "season episode lookup failed during import")
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "season collection lookup failed during import")
            }
        }
    }

    if episodes.is_empty() && !ep_meta.special_absolute_episode_numbers.is_empty() {
        for special_number in &ep_meta.special_absolute_episode_numbers {
            let episode_str = special_number.to_string();
            match app
                .services
                .catalog
                .shows
                .find_episode_by_title_and_numbers(&title.id, "0", &episode_str)
                .await
            {
                Ok(Some(episode)) => {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode);
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id,
                        special = %episode_str,
                        "no matching special episode found during import"
                    );
                }
                Err(err) => {
                    tracing::warn!(error = %err, "special episode lookup failed during import")
                }
            }
        }
    }

    if episodes.is_empty()
        && (ep_meta.absolute_episode.is_some() || !ep_meta.absolute_episode_numbers.is_empty())
    {
        let absolute_numbers: Vec<u32> = if !ep_meta.absolute_episode_numbers.is_empty() {
            ep_meta.absolute_episode_numbers.clone()
        } else if ep_meta.episode_numbers.is_empty() {
            vec![ep_meta.absolute_episode.unwrap_or_default()]
        } else {
            ep_meta.episode_numbers.clone()
        };

        for absolute_number in absolute_numbers {
            let absolute_episode_str = absolute_number.to_string();
            match app
                .services
                .catalog
                .shows
                .find_episode_by_title_and_absolute_number(&title.id, &absolute_episode_str)
                .await
            {
                Ok(Some(episode)) => {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode);
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id,
                        absolute = absolute_number,
                        "no matching episode found by absolute number"
                    );
                }
                Err(err) => {
                    tracing::warn!(error = %err, "episode absolute lookup failed during import")
                }
            }
        }
    }

    episodes
}
fn prefer_broader_coverage_episodes(
    target_episodes: &[scryer_domain::Episode],
    claimed_episodes: Vec<scryer_domain::Episode>,
) -> Vec<scryer_domain::Episode> {
    if claimed_episodes.len() > target_episodes.len() {
        claimed_episodes
    } else {
        target_episodes.to_vec()
    }
}
async fn write_series_sidecars(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    title_folder_path: &Path,
    nfo_enabled: bool,
) {
    if nfo_enabled {
        let tvshow_nfo_path = title_folder_path.join("tvshow.nfo");
        if !tvshow_nfo_path.exists() {
            if let Some(parent) = tvshow_nfo_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let nfo_content = render_tvshow_nfo(title);
            if let Err(err) = tokio::fs::write(&tvshow_nfo_path, nfo_content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %tvshow_nfo_path.display(),
                    "failed to write tvshow NFO sidecar"
                );
            }
        }
    }

    let plexmatch_enabled = match app
        .resolve_plexmatch_write_on_import(Some(&title.library_id), &title.facet)
        .await
    {
        Ok(value) => value.unwrap_or(false),
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id = %title.id,
                "failed to resolve plexmatch sidecar setting"
            );
            false
        }
    };
    if plexmatch_enabled {
        let plexmatch_path = title_folder_path.join(".plexmatch");
        if !plexmatch_path.exists() {
            if let Some(parent) = plexmatch_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let content = render_plexmatch(title);
            if let Err(err) = tokio::fs::write(&plexmatch_path, content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %plexmatch_path.display(),
                    "failed to write .plexmatch hint file"
                );
            }
        }
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "import artifact persistence records the full import outcome for later inspection"
)]
async fn persist_file_import_artifact(
    app: &AppUseCase,
    import_id: &str,
    completed: &CompletedDownload,
    title_id: &str,
    source_path: &Path,
    media_kind: &str,
    result: &str,
    reason_code: Option<&str>,
    imported_media_file_id: Option<&str>,
    episodes: &[scryer_domain::Episode],
) {
    let relative_path = source_path
        .strip_prefix(&completed.dest_dir)
        .ok()
        .map(path_to_stored_string)
        .filter(|path| !path.is_empty());
    let normalized_file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
        .unwrap_or_else(|| source_path.to_string_lossy().to_ascii_lowercase());

    let episode_rows: Vec<(Option<String>, Option<i32>, Option<i32>)> = if episodes.is_empty() {
        vec![(None, None, None)]
    } else {
        episodes
            .iter()
            .map(|episode| {
                (
                    Some(episode.id.clone()),
                    episode
                        .season_number
                        .as_deref()
                        .and_then(|value| value.parse().ok()),
                    episode
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse().ok()),
                )
            })
            .collect()
    };

    for (episode_id, season_number, episode_number) in episode_rows {
        let artifact = ImportArtifact {
            id: Id::new().0,
            source_client_id: Some(completed.client_id.clone()),
            source_system: completed.client_type.clone(),
            source_ref: completed.download_client_item_id.clone(),
            import_id: Some(import_id.to_string()),
            relative_path: relative_path.clone(),
            normalized_file_name: normalized_file_name.clone(),
            media_kind: media_kind.to_string(),
            title_id: Some(title_id.to_string()),
            episode_id,
            season_number,
            episode_number,
            result: result.to_string(),
            reason_code: reason_code.map(str::to_string),
            imported_media_file_id: imported_media_file_id.map(str::to_string),
            created_at: Utc::now(),
        };
        if let Err(error) = app
            .services
            .workflow
            .import_artifacts
            .insert_artifact(artifact)
            .await
        {
            tracing::warn!(
                error = %error,
                import_id,
                source_ref = %completed.download_client_item_id,
                file = %source_path.display(),
                "failed to persist import artifact"
            );
        }
    }
}
// 50 MB

pub(crate) fn is_sample_file(path: &Path) -> bool {
    let filename = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if filename.contains("sample") {
        return true;
    }

    // Small files in multi-episode directories are almost certainly samples/promos
    std::fs::metadata(path)
        .map(|m| m.len() < SAMPLE_SIZE_THRESHOLD)
        .unwrap_or(false)
}
fn resolve_title_from_release_candidate(
    titles: &[Title],
    candidate: &ParsedReleaseMetadata,
    facet_hint: Option<&str>,
) -> Option<Title> {
    if candidate.episode.is_some() {
        crate::import_title_resolution::resolve_monitored_episode_title_from_release(
            titles, candidate, facet_hint,
        )
        .map(|resolved| resolved.title.clone())
    } else {
        crate::import_title_resolution::resolve_monitored_movie_title_from_release(
            titles, candidate,
        )
        .map(|resolved| resolved.title.clone())
    }
}
fn fill_missing_release_metadata(
    target: &mut ParsedReleaseMetadata,
    fallback: &ParsedReleaseMetadata,
    prefer_episode: bool,
) {
    if prefer_episode
        && target.episode.as_ref().is_none_or(|file_episode| {
            fallback.episode.as_ref().is_some_and(|other_episode| {
                prefer_other_episode_info(Some(file_episode), other_episode)
            })
        })
    {
        if fallback.episode.is_some() {
            target.episode = fallback.episode.clone();
        }
    } else if target.episode.is_none() && fallback.episode.is_some() {
        target.episode = fallback.episode.clone();
    }

    if target.imdb_id.is_none() {
        target.imdb_id = fallback.imdb_id.clone();
    }
    if target.tmdb_id.is_none() {
        target.tmdb_id = fallback.tmdb_id.clone();
    }
    if target.year.is_none() {
        target.year = fallback.year;
    }
    if target.quality.is_none() {
        target.quality = fallback.quality.clone();
    }
    if target.source.is_none() {
        target.source = fallback.source;
    }
    if target.video_codec.is_none() {
        target.video_codec = fallback.video_codec;
    }
    if target.video_encoding.is_none() {
        target.video_encoding = fallback.video_encoding.clone();
    }
    if target.audio.is_none() {
        target.audio = fallback.audio;
    }
    if target.audio_channels.is_none() {
        target.audio_channels = fallback.audio_channels.clone();
    }
    if target.release_group.is_none() {
        target.release_group = fallback.release_group.clone();
    }
    if target.streaming_service.is_none() {
        target.streaming_service = fallback.streaming_service;
    }
    if target.edition.is_none() {
        target.edition = fallback.edition.clone();
    }
    if target.normalized_title.trim().is_empty() && !fallback.normalized_title.trim().is_empty() {
        target.normalized_title = fallback.normalized_title.clone();
    }
    if target.normalized_title_variants.is_empty() && !fallback.normalized_title_variants.is_empty()
    {
        target.normalized_title_variants = fallback.normalized_title_variants.clone();
    }
}
fn prefer_other_episode_info(
    file_episode_info: Option<&ParsedEpisodeMetadata>,
    other_episode_info: &ParsedEpisodeMetadata,
) -> bool {
    let Some(file_episode_info) = file_episode_info else {
        return true;
    };

    if file_episode_info.absolute_episode.is_none() && other_episode_info.absolute_episode.is_some()
    {
        return false;
    }

    true
}
fn build_augmented_episode_import_metadata(
    source_video: &Path,
    completed: &CompletedDownload,
    other_video_files: bool,
) -> ParsedReleaseMetadata {
    let mut parsed = parsed_release_from_file_stem(source_video);
    let file_episode = parsed.episode.clone();
    let file_has_usable_title_signal = has_usable_release_title_signal(&parsed);
    if !file_has_usable_title_signal {
        clear_unusable_release_title_signal(&mut parsed);
    }
    let source_parent_info = if file_has_usable_title_signal {
        None
    } else {
        parsed_usable_release_from_parent_folder(source_video)
    };
    let download_client_info =
        normalize_release_title_signal(parse_release_metadata(&completed.name));
    let folder_info = parsed_release_from_folder_name(Path::new(&completed.dest_dir));

    if !other_video_files {
        if let Some(source_parent_info) = source_parent_info.as_ref()
            && let Some(other_episode_info) = source_parent_info.episode.as_ref()
            && !other_episode_info.full_season
            && prefer_other_episode_info(parsed.episode.as_ref(), other_episode_info)
        {
            fill_missing_release_metadata(&mut parsed, source_parent_info, true);
            return parsed;
        }

        if let Some(other_episode_info) = download_client_info.episode.as_ref()
            && !other_episode_info.full_season
            && prefer_other_episode_info(parsed.episode.as_ref(), other_episode_info)
        {
            fill_missing_release_metadata(&mut parsed, &download_client_info, true);
            return parsed;
        }

        if let Some(folder_info) = folder_info.as_ref()
            && let Some(other_episode_info) = folder_info.episode.as_ref()
            && !other_episode_info.full_season
            && prefer_other_episode_info(parsed.episode.as_ref(), other_episode_info)
        {
            fill_missing_release_metadata(&mut parsed, folder_info, true);
            return parsed;
        }
    }

    if let Some(source_parent_info) = source_parent_info.as_ref() {
        fill_missing_release_metadata(&mut parsed, source_parent_info, false);
    }
    fill_missing_release_metadata(&mut parsed, &download_client_info, false);
    if let Some(folder_info) = folder_info.as_ref() {
        fill_missing_release_metadata(&mut parsed, folder_info, false);
    }
    if other_video_files {
        parsed.episode = file_episode;
    }
    parsed
}
fn build_augmented_path_episode_import_metadata(
    source_video: &Path,
    other_video_files: bool,
) -> ParsedReleaseMetadata {
    let mut parsed = parsed_release_from_file_stem(source_video);
    let file_episode = parsed.episode.clone();
    if let Some(source_parent_info) = parsed_usable_release_from_parent_folder(source_video) {
        fill_missing_release_metadata(&mut parsed, &source_parent_info, true);
    }
    if other_video_files {
        parsed.episode = file_episode;
    }
    parsed
}
