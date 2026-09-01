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

    // Specials routinely run far shorter (or longer) than the series' nominal
    // runtime; falling back to it would put legitimate season-0 files outside
    // the plausibility band. Without a real per-episode duration they stay
    // permissive.
    let targets_special = target_episodes.iter().any(|episode| {
        episode
            .season_number
            .as_deref()
            .map(str::trim)
            .and_then(|season| season.parse::<u32>().ok())
            == Some(0)
    });
    if targets_special {
        return None;
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
    // The client-side download this file came from, when there is one. Only
    // used to decide whether a configured `Move` is safe: a still-seeding
    // torrent forces hardlink-or-copy.
    completed: Option<&CompletedDownload>,
    rename_enabled: bool,
    rename_template: &str,
    season_folder_template: &str,
    specials_folder_template: &str,
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
    origin: crate::import_decide::ImportOrigin,
    announced_size_bytes: Option<i64>,
    additional_import: bool,
) -> AppResult<EpisodeImportOutcome> {
    let use_season_folders = app.resolve_use_season_folders(title).await?;
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
                disposition: crate::import_decide::RejectionDisposition::Hold,
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
            use_season_folders,
            &effective_parsed,
            &ext,
            source_video,
            title_folder_path,
            rename_enabled,
            rename_template,
            season_folder_template,
            specials_folder_template,
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
                reason_code: Some(code.as_str().to_string()),
                skip_reason: Some(
                    skip_reason_for_import_check_rejection(app, code, &dest_path).await?,
                ),
                episode_ids: target_episode_ids.clone(),
            });
        }

        let import_mode = crate::seeding_gate::resolve_seeding_safe_import_mode(
            app,
            Some(&title.library_id),
            &title.facet,
            completed,
        )
        .await?;
        persist_title_folder_path_if_missing(app, title, title_folder_path).await?;
        let destination_ownership = ImportDestinationOwnership::episodes(&target_episode_ids);
        let file_result = import_file_with_record_progress(
            app,
            import_id,
            &title.library_id,
            &title.facet,
            &destination_ownership,
            source_video,
            &dest_path,
            import_mode,
            None,
            completed,
        )
        .await?;
        let media_file_input = crate::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: path_to_stored_string(&dest_path),
            size_bytes: file_result.size_bytes as i64,
            announced_size_bytes: crate::canonical_scoring::persisted_announced_size_bytes(
                file_result.size_bytes as i64,
                announced_size_bytes,
            ),
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
        let media_file_id = file_result
            .insert_or_reuse_media_file(app, &media_file_input)
            .await?
            .media_file_id;
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

        let link_type = if import_mode == scryer_domain::ImportMode::Move {
            scryer_domain::ImportStrategy::Move
        } else {
            file_result.strategy
        };

        return Ok(EpisodeImportOutcome::Imported {
            dest_path: path_to_stored_string(&dest_path),
            episode_ids: target_episode_ids,
            imported_media_file_id: Some(media_file_id),
            reason_code: Some("additional_file".to_string()),
            link_type: Some(link_type),
            source_cleanup: file_result.source_cleanup.clone().map(Box::new),
            destination_permit: file_result.destination_permit(),
            size_bytes: Some(file_result.size_bytes as i64),
            // An additional file never reaches the gate, so it never earns one.
            blocklist_after_import: None,
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
    let precheck_parsed = parsed_with_quality_override(parsed, precheck_quality_label.as_deref());
    let precheck_dest_path = episode_import_dest_path(
        title,
        use_season_folders,
        &precheck_parsed,
        &precheck_ext,
        source_video,
        title_folder_path,
        rename_enabled,
        rename_template,
        season_folder_template,
        specials_folder_template,
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
            reason_code: Some(code.as_str().to_string()),
            skip_reason: Some(
                skip_reason_for_import_check_rejection(app, code, &precheck_dest_path).await?,
            ),
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
            // A band miss is held for the operator (ImportBlocked), not
            // burned: expected runtimes are estimates and legitimate outliers
            // must stay grabbable after review.
            if rejection.recycle_reason == crate::post_download_gate::RUNTIME_OUT_OF_BAND_CODE {
                return Ok(EpisodeImportOutcome::Skipped {
                    message: rejection.message.clone(),
                    reason_code: Some(rejection.recycle_reason.to_string()),
                    skip_reason: Some(ImportSkipReason::PolicyMismatch),
                    episode_ids: target_episode_ids.clone(),
                });
            }
            // The probe refused the bytes outright. A corrupt container or a
            // source that changed under the import means the release is not
            // what it claimed, so it is burned and the scope reopened. A
            // user/system rule veto on the file is also an import failure.
            let rejection = origin.held_rejection(rejection);
            let disposition =
                crate::import_decide::prepare_rejection_disposition_for_origin(&rejection, origin);
            return Ok(EpisodeImportOutcome::Rejected {
                rejection,
                disposition,
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
        let rejection = crate::post_download_gate::ImportedFileRejection {
            message: issue.message,
            recycle_reason: super::coverage_validation::COVERAGE_RUNTIME_MISMATCH_CODE,
            skip_reason: Some(ImportSkipReason::PolicyMismatch),
            blocking_rule_codes: Vec::new(),
        };
        let rejection = origin.held_rejection(rejection);
        return Ok(EpisodeImportOutcome::Rejected {
            disposition: crate::import_decide::prepare_rejection_disposition_for_origin(
                &rejection, origin,
            ),
            rejection,
            reason_code: Some(
                super::coverage_validation::COVERAGE_RUNTIME_MISMATCH_CODE.to_string(),
            ),
            episode_ids: target_episode_ids.clone(),
        });
    }

    // Replace guard: overwriting a library file is stricter than filling a gap.
    // With no catalog runtime the band could not run at the gate, so compare the
    // probed duration against what the incumbent file(s) actually hold and park
    // an implausible replacement for manual resolution instead of burning it.
    if !existing_incumbents.is_empty()
        && let Some(message) = crate::post_download_gate::replace_runtime_band_block(
            runtime_sample_validation,
            prepared.accepted.as_ref(),
            crate::post_download_gate::incumbent_replace_runtime_seconds(
                existing_incumbents
                    .iter()
                    .map(|incumbent| incumbent.media_file.duration_seconds),
            ),
        )
    {
        tracing::info!(
            title_id = %title.id,
            file = %source_video.display(),
            code = crate::post_download_gate::REPLACE_BLOCKED_RUNTIME_MISMATCH_CODE,
            "holding episode replacement for manual resolution"
        );
        return Ok(EpisodeImportOutcome::Skipped {
            message,
            reason_code: Some(
                crate::post_download_gate::REPLACE_BLOCKED_RUNTIME_MISMATCH_CODE.to_string(),
            ),
            skip_reason: Some(ImportSkipReason::PolicyMismatch),
            episode_ids: target_episode_ids.clone(),
        });
    }

    // The announced half of the evidence: the canonical import parse *before*
    // the probe merged its findings in, with only the operator's explicit
    // quality override applied. `prepared.parsed` already carries the analysis,
    // so handing that in would make both scoring passes identical and no
    // release could ever be caught contradicting itself.
    let announced_parsed = match quality_override
        .as_deref()
        .and_then(|value| non_empty_string(Some(value.to_string())))
    {
        Some(label) => parsed_with_quality_override(parsed, Some(label.as_str())),
        None => parsed.clone(),
    };

    let ext = precheck_ext;
    let effective_quality_label = quality_override
        .as_deref()
        .and_then(|value| non_empty_string(Some(value.to_string())))
        .or_else(|| prepared.parsed.quality.clone());
    let effective_parsed =
        parsed_with_quality_override(&prepared.parsed, effective_quality_label.as_deref());
    let dest_path = episode_import_dest_path(
        title,
        use_season_folders,
        &effective_parsed,
        &ext,
        source_video,
        title_folder_path,
        rename_enabled,
        rename_template,
        season_folder_template,
        specials_folder_template,
        rename_season,
        rename_episode_number,
        rename_absolute_number,
        rename_episode_title,
        effective_quality_label.as_deref(),
    );
    let import_mode = crate::seeding_gate::resolve_seeding_safe_import_mode(
        app,
        Some(&title.library_id),
        &title.facet,
        completed,
    )
    .await?;

    let manual_replacement = matches!(
        runtime_sample_mode,
        crate::post_download_gate::RuntimeSampleValidationMode::BypassRuntimeSampleCheck
    );

    // **One runtime basis per scope** (D4): the episodes this file actually
    // holds, not the series average. Size scoring is runtime-derived, so scoring
    // a double-length premiere or a 7-minute special against the average puts it
    // in a different size band than the grab decision used — the same file,
    // scored two ways. The grab lane has always used the covered episodes'
    // runtime (`coverage_size_basis`); this is the same derivation, and it
    // carries the member count and per-member runtime a pack is judged by.
    let scope_size_basis = crate::acquisition_coverage::episode_span_size_basis(
        target_episodes,
        &target_episode_ids,
        title.runtime_minutes,
    )
    .or_runtime(title.runtime_minutes);

    // **The one import decision** (design §3). Subject, landed score, truth
    // verdict and admission all live in `decide_import`; what is left here is
    // carrying out its plan.
    let scoring_context = app
        .resolve_canonical_scoring_context(title, quality_profile)
        .await;
    let episode_scope = crate::SubmissionScope::EpisodeSet {
        episode_ids: target_episode_ids.clone(),
    };
    let decision_input = crate::import_decide::ImportDecisionInput {
        title,
        scoring_context: &scoring_context,
        scope: &episode_scope,
        scope_size_basis,
        parsed: &announced_parsed,
        accepted: prepared.accepted.as_ref(),
        prior_rescore_changes: &prepared.rescore_changes,
        landed_size_bytes: source_size,
        announced_size_bytes,
        is_filler,
        origin,
        operator_intent: manual_replacement,
        incumbent_rows: crate::import_decide::IncumbentRows::Episodes(&existing_incumbents),
        scope_label: "this episode",
    };
    let plan = match crate::import_decide::decide_import(app, &decision_input).await {
        crate::import_decide::ImportDecisionOutcome::Admit(plan) => plan,
        crate::import_decide::ImportDecisionOutcome::Reject {
            rejection,
            disposition,
        } => {
            tracing::info!(
                title_id = %title.id,
                code = rejection.recycle_reason,
                ?disposition,
                "{}",
                rejection.message
            );
            let reason_code = Some(rejection.recycle_reason.to_string());
            return Ok(EpisodeImportOutcome::Rejected {
                rejection,
                disposition,
                reason_code,
                episode_ids: target_episode_ids.clone(),
            });
        }
    };
    // Reported to the caller rather than written here: a pack's members share
    // one release, so the blocklist row is deduplicated at the file loop.
    let blocklist_after_import = plan.blocklist_after_import.clone();
    let new_score = plan.score;

    if let crate::import_decide::SupersededIncumbents::Episodes(superseded) = &plan.superseded
        && let Some((primary_incumbent, additional_superseded)) = superseded.split_first()
    {
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
            &primary_incumbent.media_file,
        )
        .await?;

        persist_title_folder_path_if_missing(app, title, title_folder_path).await?;
        match crate::upgrade::execute_upgrade(
            app,
            actor,
            import_id,
            title,
            &primary_incumbent.media_file,
            source_video,
            &dest_path,
            &prepared,
            plan.parsed.quality.as_deref(),
            new_score,
            plan.previous_best_score,
            plan.scoring_log.clone(),
            &target_episode_ids,
            Some(replacement_media_root),
            Some(old_file_recycle_context.media_root.as_str()),
            &old_file_recycle_context.recycle_config,
            import_mode,
            announced_size_bytes,
            completed,
        )
        .await
        {
            Ok(crate::upgrade::UpgradeResult::Upgraded(outcome)) => {
                if outcome.recycle_entry_committed {
                    cleanup_superseded_episode_incumbents(
                        app,
                        title,
                        additional_superseded,
                        &outcome.new_file_id,
                        &dest_path,
                    )
                    .await;
                } else if !additional_superseded.is_empty() {
                    tracing::warn!(
                        title_id = %title.id,
                        replacement_file_id = %outcome.new_file_id,
                        superseded_files = additional_superseded.len(),
                        "skipping superseded episode cleanup because primary recycle entry was not committed"
                    );
                }
                tracing::info!(
                    title = %title.name,
                    old_score = outcome.old_score,
                    new_score = outcome.new_score,
                    superseded_files = additional_superseded.len() + 1,
                    "episode file upgraded"
                );
                for episode_id in &target_episode_ids {
                    mark_wanted_completed(app, &title.id, Some(episode_id), true).await;
                }
                return Ok(EpisodeImportOutcome::Imported {
                    dest_path: path_to_stored_string(&dest_path),
                    episode_ids: target_episode_ids,
                    imported_media_file_id: None,
                    reason_code: Some("upgrade".to_string()),
                    link_type: (import_mode == scryer_domain::ImportMode::Move)
                        .then_some(scryer_domain::ImportStrategy::Move),
                    source_cleanup: outcome.source_cleanup.clone(),
                    destination_permit: outcome.destination_permit.clone(),
                    size_bytes: Some(outcome.new_size_bytes),
                    blocklist_after_import,
                });
            }
            Ok(crate::upgrade::UpgradeResult::Rejected(rejection)) => {
                // The transfer itself failed a safety check; nothing was judged
                // about the release, so it keeps its place in the search space.
                return Ok(EpisodeImportOutcome::Rejected {
                    rejection,
                    disposition: crate::import_decide::RejectionDisposition::Hold,
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

    persist_title_folder_path_if_missing(app, title, title_folder_path).await?;
    let destination_ownership = ImportDestinationOwnership::episodes(&target_episode_ids);
    let file_result = import_file_with_record_progress(
        app,
        import_id,
        &title.library_id,
        &title.facet,
        &destination_ownership,
        source_video,
        &dest_path,
        import_mode,
        Some(&prepared.source_snapshot),
        completed,
    )
    .await?;

    // The persisted bar must be the score of the bytes that actually landed
    // (I7), and the transfer can change the size. Same context, same pipeline,
    // one number different — no second profile resolution.
    let post_download_score =
        crate::import_decide::rescore_landed_size(&decision_input, file_result.size_bytes as i64);
    let acq_score = post_download_score.score;

    let media_file_input = crate::InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: path_to_stored_string(&dest_path),
        size_bytes: file_result.size_bytes as i64,
        announced_size_bytes: crate::canonical_scoring::persisted_announced_size_bytes(
            file_result.size_bytes as i64,
            announced_size_bytes,
        ),
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
    let media_file_id = file_result
        .insert_or_reuse_media_file(app, &media_file_input)
        .await?
        .media_file_id;
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

    let mut episode_role_failed = false;
    for episode in target_episodes {
        if let Err(err) = app
            .services
            .library
            .media_files
            .set_media_file_roles_for_episode(&title.id, &episode.id, &media_file_id, &[])
            .await
        {
            tracing::warn!(
                error = %err,
                episode_id = %episode.id,
                file_id = %media_file_id,
                "failed to promote imported file for episode"
            );
            episode_role_failed = true;
        }
    }
    if episode_role_failed && import_mode == scryer_domain::ImportMode::Move {
        return Err(AppError::Repository(format!(
            "move import source cleanup blocked because episode role assignment failed for {}",
            dest_path.display()
        )));
    }

    let link_type = if import_mode == scryer_domain::ImportMode::Move {
        scryer_domain::ImportStrategy::Move
    } else {
        file_result.strategy
    };

    for episode in target_episodes {
        mark_wanted_completed(app, &title.id, Some(&episode.id), true).await;
    }

    Ok(EpisodeImportOutcome::Imported {
        dest_path: path_to_stored_string(&dest_path),
        episode_ids: target_episode_ids,
        imported_media_file_id: Some(media_file_id),
        reason_code: None,
        link_type: Some(link_type),
        source_cleanup: file_result.source_cleanup.clone().map(Box::new),
        destination_permit: file_result.destination_permit(),
        size_bytes: Some(file_result.size_bytes as i64),
        blocklist_after_import,
    })
}
/// Mark an existing acquisition-state row completed for a title scope.
/// If no row exists, leave it absent: convergence derives target-ness from
/// library state, so passive scans/imports must not synthesize wanted rows.
/// `landed_import` is whether a file actually landed for this scope: `true` from
/// the import paths, `false` from a passive library scan or a manual completion
/// that only observed a file already on disk. It decides whether the in-flight
/// grab is cleared, and it replaces reading `Option<score>` as a flag.
pub(crate) async fn mark_wanted_completed(
    app: &AppUseCase,
    title_id: &str,
    episode_id: Option<&str>,
    landed_import: bool,
) {
    let now = Utc::now().to_rfc3339();

    match app
        .services
        .workflow
        .acquisition_scope_states
        .complete_acquisition_scope_for_title(title_id, episode_id, Some(&now), landed_import)
        .await
    {
        Ok(true) => {}
        Ok(false) => {}
        Err(err) => {
            tracing::warn!(error = %err, title_id = %title_id, "failed to mark wanted item completed");
        }
    }
}
