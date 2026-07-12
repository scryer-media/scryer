fn expected_runtime_seconds_for_episode_import(
    title: &scryer_domain::Title,
    target_episodes: &[scryer_domain::Episode],
) -> Option<i32> {
    let positive_episode_durations = target_episodes
        .iter()
        .filter_map(|episode| episode.duration_seconds.filter(|duration| *duration > 0))
        .collect::<Vec<_>>();
    if !target_episodes.is_empty() && positive_episode_durations.len() == target_episodes.len() {
        let total_seconds = positive_episode_durations
            .into_iter()
            .sum::<i64>()
            .min(i64::from(i32::MAX));
        return Some(total_seconds as i32);
    }

    let episode_count = i32::try_from(target_episodes.len().max(1)).unwrap_or(i32::MAX);
    title
        .runtime_minutes
        .filter(|runtime_minutes| *runtime_minutes > 0)
        .map(|runtime_minutes| {
            runtime_minutes
                .saturating_mul(60)
                .saturating_mul(episode_count)
        })
}

#[expect(
    clippy::too_many_arguments,
    reason = "resolved episode imports coordinate rename tokens, coverage, and scoring in one step"
)]
async fn execute_resolved_episode_import(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    rename_enabled: bool,
    rename_template: &str,
    season_folder_template: &str,
    title_folder_path: &Path,
    source_video: &Path,
    parsed: &crate::ParsedReleaseMetadata,
    target_episodes: &[scryer_domain::Episode],
    coverage_episodes: &[scryer_domain::Episode],
    rename_season: u32,
    rename_episode_number: &str,
    rename_absolute_number: Option<&str>,
    rename_episode_title: Option<&str>,
    quality_profile: &crate::QualityProfile,
    quality_override: Option<String>,
    runtime_sample_mode: crate::post_download_gate::RuntimeSampleValidationMode,
    additional_import: bool,
) -> AppResult<EpisodeImportOutcome> {
    let source_size = std::fs::metadata(source_video)
        .map(|metadata| metadata.len() as i64)
        .unwrap_or(0);
    let target_episode_ids = target_episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect::<Vec<_>>();
    let is_filler = target_episodes.iter().any(|episode| episode.is_filler);
    let existing_incumbents = app
        .services
        .library
        .media_files
        .list_live_media_files_for_episode_ids(&title.id, &target_episode_ids)
        .await
        .unwrap_or_default();
    let existing_files = existing_incumbents
        .iter()
        .map(|incumbent| incumbent.media_file.clone())
        .collect::<Vec<_>>();
    if additional_import {
        if target_episode_ids.len() != 1 {
            return Ok(EpisodeImportOutcome::Rejected {
                rejection: crate::post_download_gate::ImportedFileRejection {
                    message: "additional-file episode imports support exactly one episode"
                        .to_string(),
                    recycle_reason: "additional_file_multi_episode",
                    skip_reason: Some(ImportSkipReason::PolicyMismatch),
                    blocking_rule_codes: vec!["additional_file_multi_episode".to_string()],
                },
                finalize_before_import: false,
                reason_code: Some("additional_file_multi_episode".to_string()),
                episode_ids: target_episode_ids.clone(),
            });
        }

        let ext = scryer_domain::canonical_video_extension(source_video)
            .unwrap_or("mkv")
            .to_string();
        let effective_quality_label = quality_override
            .as_deref()
            .and_then(|value| non_empty_string(Some(value.to_string())))
            .or_else(|| parsed.quality.clone());
        let effective_parsed =
            parsed_with_quality_override(parsed, effective_quality_label.as_deref());
        let canonical_dest_path = episode_import_dest_path(
            title,
            &effective_parsed,
            &ext,
            source_video,
            title_folder_path,
            rename_enabled,
            rename_template,
            season_folder_template,
            rename_season,
            rename_episode_number,
            rename_absolute_number,
            rename_episode_title,
            effective_quality_label.as_deref(),
        );
        let dest_path = additional_import_dest_path(&canonical_dest_path, &effective_parsed);
        let check_ctx = crate::import_checks::ImportCheckContext {
            source_path: source_video,
            dest_path: &dest_path,
            source_size: source_size as u64,
            parsed: &effective_parsed,
            existing_files: &existing_files,
        };
        if let crate::import_checks::ImportVerdict::Reject { reason, code } =
            crate::import_checks::run_import_checks(&check_ctx)
        {
            return Ok(EpisodeImportOutcome::Skipped {
                message: reason,
                reason_code: Some(code.to_string()),
                skip_reason: Some(skip_reason_for_import_check_code(code)),
                episode_ids: target_episode_ids.clone(),
            });
        }

        let import_mode = app
            .resolve_import_mode(Some(&title.library_id), &title.facet)
            .await?;
        let file_result = import_file_with_record_progress(
            app,
            import_id,
            source_video,
            &dest_path,
            import_mode,
            None,
        )
        .await?;
        let media_file_input = crate::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: path_to_stored_string(&dest_path),
            size_bytes: file_result.size_bytes as i64,
            role: crate::MediaFileRole::Additional,
            quality_label: effective_quality_label.clone(),
            scene_name: Some(effective_parsed.raw_title.clone()),
            release_group: effective_parsed.release_group.clone(),
            source_type: crate::release_parser::parsed_release_source_type(&effective_parsed),
            resolution: effective_quality_label,
            video_codec_parsed: effective_parsed.video_codec,
            audio_codec_parsed: effective_parsed.audio.as_ref().map(ToString::to_string),
            audio_channels_parsed: effective_parsed.audio_channels.clone(),
            original_file_path: Some(path_to_stored_string(source_video)),
            grabbed_release_title: Some(effective_parsed.raw_title.clone()),
            edition: effective_parsed.edition.clone(),
            ..Default::default()
        };
        let media_file_id = app
            .services
            .library
            .media_files
            .insert_media_file(&media_file_input)
            .await?;
        analyze_and_persist_imported_media_file(app, &title.id, &media_file_id, &dest_path).await;
        if let Err(error) = crate::subtitles::reconcile_external_subtitles_for_media_file(
            app,
            &title.id,
            &media_file_id,
            target_episode_ids.first().map(String::as_str),
            &dest_path,
        )
        .await
        {
            tracing::warn!(
                error = %error,
                title_id = %title.id,
                file_id = %media_file_id,
                dest_path = %dest_path.display(),
                "failed to reconcile external subtitles after additional episode import"
            );
        }
        maybe_trigger_subtitle_search(app, &title.id, &media_file_id);

        for episode in target_episodes {
            if let Err(err) = app
                .services
                .library
                .media_files
                .link_file_to_episode(&media_file_id, &episode.id)
                .await
            {
                tracing::warn!(error = %err, episode_id = %episode.id, "failed to link additional file to episode");
                if import_mode == scryer_domain::ImportMode::Move {
                    return Err(AppError::Repository(format!(
                        "move import source cleanup blocked because episode linking failed for {}",
                        dest_path.display()
                    )));
                }
            }
        }

        let link_type =
            finalize_import_source_cleanup(app, import_mode, &file_result, &dest_path).await?;

        return Ok(EpisodeImportOutcome::Imported {
            dest_path: path_to_stored_string(&dest_path),
            episode_ids: target_episode_ids,
            imported_media_file_id: Some(media_file_id),
            reason_code: Some("additional_file".to_string()),
            link_type: Some(link_type),
        });
    }
    let existing_incumbents = existing_incumbents
        .into_iter()
        .filter(|incumbent| incumbent.media_file.role.is_primary())
        .collect::<Vec<_>>();
    let existing_files = existing_incumbents
        .iter()
        .map(|incumbent| incumbent.media_file.clone())
        .collect::<Vec<_>>();
    let existing_score = existing_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        .and_then(|file| file.acquisition_score);
    let expected_runtime_seconds =
        expected_runtime_seconds_for_episode_import(title, target_episodes);
    let runtime_sample_validation = match runtime_sample_mode {
        crate::post_download_gate::RuntimeSampleValidationMode::EnforceAutomatic => {
            crate::post_download_gate::RuntimeSampleValidation::automatic(expected_runtime_seconds)
        }
        crate::post_download_gate::RuntimeSampleValidationMode::BypassRuntimeSampleCheck => {
            crate::post_download_gate::RuntimeSampleValidation::manual_override(
                expected_runtime_seconds,
            )
        }
    };
    let precheck_ext = scryer_domain::canonical_video_extension(source_video)
        .unwrap_or("mkv")
        .to_string();
    let precheck_quality_label = quality_override
        .as_deref()
        .and_then(|value| non_empty_string(Some(value.to_string())))
        .or_else(|| parsed.quality.clone());
    let precheck_parsed =
        parsed_with_quality_override(parsed, precheck_quality_label.as_deref());
    let precheck_dest_path = episode_import_dest_path(
        title,
        &precheck_parsed,
        &precheck_ext,
        source_video,
        title_folder_path,
        rename_enabled,
        rename_template,
        season_folder_template,
        rename_season,
        rename_episode_number,
        rename_absolute_number,
        rename_episode_title,
        precheck_quality_label.as_deref(),
    );

    let check_ctx = crate::import_checks::ImportCheckContext {
        source_path: source_video,
        dest_path: &precheck_dest_path,
        source_size: source_size as u64,
        parsed: &precheck_parsed,
        existing_files: &existing_files,
    };
    if let crate::import_checks::ImportVerdict::Reject { reason, code } =
        crate::import_checks::run_import_checks(&check_ctx)
    {
        tracing::debug!(file = %precheck_dest_path.display(), %code, %reason, "skipping episode file");
        return Ok(EpisodeImportOutcome::Skipped {
            message: reason,
            reason_code: Some(code.to_string()),
            skip_reason: Some(skip_reason_for_import_check_code(code)),
            episode_ids: target_episode_ids.clone(),
        });
    }

    let prepared = match crate::post_download_gate::prepare_import_candidate(
        app,
        title,
        parsed,
        quality_profile,
        source_video,
        source_size,
        !existing_files.is_empty(),
        existing_score,
        is_filler,
        runtime_sample_validation,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(rejection) => {
            return Ok(EpisodeImportOutcome::Rejected {
                rejection,
                finalize_before_import: true,
                reason_code: None,
                episode_ids: target_episode_ids.clone(),
            });
        }
    };

    if let Err(issue) = super::coverage_validation::validate_broad_episode_coverage(
        title,
        &prepared.parsed,
        coverage_episodes,
        prepared.accepted.as_ref(),
    ) {
        tracing::info!(
            code = issue.code,
            expected_runtime_minutes = issue.expected_runtime_minutes,
            actual_runtime_minutes = issue.actual_runtime_minutes,
            covered_episode_count = issue.covered_episode_count,
            real_runtime_coverage_count = issue.real_runtime_coverage_count,
            file = %source_video.display(),
            "rejecting implausible episode coverage during import"
        );
        return Ok(EpisodeImportOutcome::Rejected {
            rejection: crate::post_download_gate::ImportedFileRejection {
                message: issue.message,
                recycle_reason: super::coverage_validation::COVERAGE_RUNTIME_MISMATCH_CODE,
                skip_reason: Some(ImportSkipReason::PolicyMismatch),
                blocking_rule_codes: Vec::new(),
            },
            finalize_before_import: true,
            reason_code: Some(
                super::coverage_validation::COVERAGE_RUNTIME_MISMATCH_CODE.to_string(),
            ),
            episode_ids: target_episode_ids.clone(),
        });
    }

    let ext = precheck_ext;
    let effective_quality_label = quality_override
        .as_deref()
        .and_then(|value| non_empty_string(Some(value.to_string())))
        .or_else(|| prepared.parsed.quality.clone());
    let effective_parsed =
        parsed_with_quality_override(&prepared.parsed, effective_quality_label.as_deref());
    let dest_path = episode_import_dest_path(
        title,
        &effective_parsed,
        &ext,
        source_video,
        title_folder_path,
        rename_enabled,
        rename_template,
        season_folder_template,
        rename_season,
        rename_episode_number,
        rename_absolute_number,
        rename_episode_title,
        effective_quality_label.as_deref(),
    );
    let import_mode = app
        .resolve_import_mode(Some(&title.library_id), &title.facet)
        .await?;

    if !existing_incumbents.is_empty() {
        let post_download_score =
            crate::post_download_gate::compute_post_download_acquisition_decision(
                app,
                &effective_parsed,
                prepared.accepted.as_ref(),
                quality_profile,
                title,
                title.runtime_minutes,
                source_size,
                true,
                existing_score,
                &prepared.rescore_changes,
                is_filler,
            )
            .await;
        let new_score = post_download_score.score;
        let upgrade_plan = match build_episode_upgrade_plan(
            &existing_incumbents,
            &target_episode_ids,
            new_score,
        ) {
            Ok(plan) => plan,
            Err(rejection) => {
                return Ok(EpisodeImportOutcome::Rejected {
                    rejection,
                    finalize_before_import: true,
                    reason_code: None,
                    episode_ids: target_episode_ids.clone(),
                });
            }
        };
        let replacement_media_root = title_folder_path
            .parent()
            .and_then(|path| path.to_str())
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "cannot safely upgrade {} because no configured media root could be resolved",
                    title.name
                ))
            })?;
        let old_file_recycle_context = crate::upgrade::resolve_old_file_recycle_context(
            app,
            title,
            &upgrade_plan.primary_incumbent.media_file,
        )
        .await?;

        match crate::upgrade::execute_upgrade(
            app,
            actor,
            title,
            &upgrade_plan.primary_incumbent.media_file,
            source_video,
            &dest_path,
            &prepared,
            post_download_score.parsed.quality.as_deref(),
            new_score,
            upgrade_plan.previous_best_score,
            post_download_score.scoring_log.clone(),
            &target_episode_ids,
            Some(replacement_media_root),
            Some(old_file_recycle_context.media_root.as_str()),
            &old_file_recycle_context.recycle_config,
            import_mode,
        )
        .await
        {
            Ok(crate::upgrade::UpgradeResult::Upgraded(outcome)) => {
                if outcome.recycle_entry_committed {
                    cleanup_superseded_episode_incumbents(
                        app,
                        title,
                        &upgrade_plan.additional_superseded,
                        &outcome.new_file_id,
                        &dest_path,
                    )
                    .await;
                } else if !upgrade_plan.additional_superseded.is_empty() {
                    tracing::warn!(
                        title_id = %title.id,
                        replacement_file_id = %outcome.new_file_id,
                        superseded_files = upgrade_plan.additional_superseded.len(),
                        "skipping superseded episode cleanup because primary recycle entry was not committed"
                    );
                }
                tracing::info!(
                    title = %title.name,
                    old_score = outcome.old_score,
                    new_score = outcome.new_score,
                    superseded_files = upgrade_plan.additional_superseded.len() + 1,
                    "episode file upgraded"
                );
                for episode_id in &target_episode_ids {
                    mark_wanted_completed(app, &title.id, Some(episode_id), Some(outcome.new_score))
                        .await;
                }
                return Ok(EpisodeImportOutcome::Imported {
                    dest_path: path_to_stored_string(&dest_path),
                    episode_ids: target_episode_ids,
                    imported_media_file_id: None,
                    reason_code: Some("upgrade".to_string()),
                    link_type: (import_mode == scryer_domain::ImportMode::Move)
                        .then_some(scryer_domain::ImportStrategy::Move),
                });
            }
            Ok(crate::upgrade::UpgradeResult::Rejected(rejection)) => {
                return Ok(EpisodeImportOutcome::Rejected {
                    rejection,
                    finalize_before_import: false,
                    reason_code: None,
                    episode_ids: target_episode_ids.clone(),
                });
            }
            Err(err) => {
                tracing::error!(error = %err, "episode upgrade failed");
                return Err(err);
            }
        }
    }

    let file_result = import_file_with_record_progress(
        app,
        import_id,
        source_video,
        &dest_path,
        import_mode,
        Some(&prepared.source_snapshot),
    )
    .await?;

    let has_existing = existing_files
        .iter()
        .any(|file| file.file_path == path_to_stored_string(&dest_path));
    let post_download_score = crate::post_download_gate::compute_post_download_acquisition_decision(
        app,
        &effective_parsed,
        prepared.accepted.as_ref(),
        quality_profile,
        title,
        title.runtime_minutes,
        file_result.size_bytes as i64,
        has_existing,
        existing_score,
        &prepared.rescore_changes,
        is_filler,
    )
    .await;
    let acq_score = post_download_score.score;

    let media_file_input = crate::InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: path_to_stored_string(&dest_path),
        size_bytes: file_result.size_bytes as i64,
        quality_label: post_download_score.parsed.quality.clone(),
        scene_name: Some(prepared.parsed.raw_title.clone()),
        release_group: post_download_score.parsed.release_group.clone(),
        source_type: crate::release_parser::parsed_release_source_type(&post_download_score.parsed),
        resolution: post_download_score.parsed.quality.clone(),
        video_codec_parsed: post_download_score.parsed.video_codec,
        audio_codec_parsed: post_download_score
            .parsed
            .audio
            .as_ref()
            .map(ToString::to_string),
        audio_channels_parsed: post_download_score.parsed.audio_channels.clone(),
        original_file_path: Some(path_to_stored_string(source_video)),
        acquisition_score: Some(acq_score),
        scoring_log: post_download_score.scoring_log.clone(),
        ..Default::default()
    };
    let media_file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&media_file_input)
        .await?;
    crate::post_download_gate::persist_media_analysis_result(
        &app.services.library.media_files,
        &media_file_id,
        prepared.accepted.as_ref(),
    )
    .await;
    if let Err(error) = crate::subtitles::reconcile_external_subtitles_for_media_file(
        app,
        &title.id,
        &media_file_id,
        if target_episodes.len() == 1 {
            target_episodes.first().map(|episode| episode.id.as_str())
        } else {
            None
        },
        &dest_path,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            title_id = %title.id,
            file_id = %media_file_id,
            dest_path = %dest_path.display(),
            "failed to reconcile external subtitles after import"
        );
    }
    maybe_trigger_subtitle_search(app, &title.id, &media_file_id);

    let mut episode_link_failed = false;
    for episode in target_episodes {
        if let Err(err) = app
            .services
            .library
            .media_files
            .link_file_to_episode(&media_file_id, &episode.id)
            .await
        {
            tracing::warn!(error = %err, episode_id = %episode.id, "failed to link file to episode");
            episode_link_failed = true;
        }
    }
    if episode_link_failed && import_mode == scryer_domain::ImportMode::Move {
        return Err(AppError::Repository(format!(
            "move import source cleanup blocked because episode linking failed for {}",
            dest_path.display()
        )));
    }

    let link_type =
        finalize_import_source_cleanup(app, import_mode, &file_result, &dest_path).await?;

    for episode in target_episodes {
        mark_wanted_completed(app, &title.id, Some(&episode.id), Some(acq_score)).await;
    }

    Ok(EpisodeImportOutcome::Imported {
        dest_path: path_to_stored_string(&dest_path),
        episode_ids: target_episode_ids,
        imported_media_file_id: Some(media_file_id),
        reason_code: None,
        link_type: Some(link_type),
    })
}
/// Mark a wanted item as completed for a title (and optionally a specific episode).
/// If `imported_score` is provided, it becomes the new `current_score`.
/// If the quality profile allows upgrades, the item re-enters "wanted" status
/// with a recomputed schedule (the 24h cooldown in `evaluate_upgrade` prevents churn).
pub(crate) async fn mark_wanted_completed(
    app: &AppUseCase,
    title_id: &str,
    episode_id: Option<&str>,
    imported_score: Option<i32>,
) {
    let now = Utc::now().to_rfc3339();

    match app
        .services
        .workflow
        .wanted_items
        .complete_wanted_item_for_title(title_id, episode_id, Some(&now), imported_score)
        .await
    {
        Ok(true) => {}
        Ok(false) => {}
        Err(err) => {
            tracing::warn!(error = %err, title_id = %title_id, "failed to mark wanted item completed");
        }
    }
}
