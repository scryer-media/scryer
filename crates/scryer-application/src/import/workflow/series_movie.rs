async fn run_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    completed: &CompletedDownload,
    started_at: chrono::DateTime<Utc>,
    archive_password: Option<&str>,
) -> AppResult<ImportResult> {
    let target = match Box::pin(resolve_completed_import_target(
        app,
        import_id,
        completed,
        started_at,
        archive_password,
    ))
    .await?
    {
        CompletedImportTargetResolution::Ready(target) => target,
        CompletedImportTargetResolution::Finished(result) => return Ok(*result),
    };

    let result =
        dispatch_completed_import_target(app, actor, import_id, completed, started_at, &target)
            .await;

    // Clean up extracted archive directory if we created one
    if let Some(ref dir) = target.extracted_dir {
        crate::archive_extractor::cleanup_extracted_dir(dir).await;
    }

    result
}

struct CompletedImportTarget {
    title: scryer_domain::Title,
    is_series: bool,
    video_files: Vec<PathBuf>,
    extracted_dir: Option<PathBuf>,
    series_movie_link_id: Option<String>,
}

enum CompletedImportTargetResolution {
    Ready(Box<CompletedImportTarget>),
    Finished(Box<ImportResult>),
}

async fn resolve_completed_import_target(
    app: &AppUseCase,
    import_id: &str,
    completed: &CompletedDownload,
    started_at: chrono::DateTime<Utc>,
    archive_password: Option<&str>,
) -> AppResult<CompletedImportTargetResolution> {
    // 2. TITLE MATCHING
    let mut title = None;
    let dest_dir = Path::new(&completed.dest_dir);
    let mut extracted_dir: Option<PathBuf> = None;
    let mut title_evidence_video_files: Option<Vec<PathBuf>> = None;
    let parsed_completed_name =
        normalize_release_title_signal(parse_release_metadata(&completed.name));
    let parsed_completed_folder = parsed_release_from_folder_name(dest_dir);
    if let Some(title_id) = extract_parameter(&completed.parameters, "*scryer_title_id") {
        let title_id = title_id.trim();
        if !title_id.is_empty() {
            title = app.services.catalog.titles.get_by_id(title_id).await?;
        }
    }

    // fallback to IMDb ID if needed
    if title.is_none() {
        let imdb_id = extract_parameter(&completed.parameters, "*scryer_imdb_id")
            .and_then(|value| normalize_imdb_id(&value));

        title = match imdb_id {
            Some(target_imdb_id) => {
                let titles = app
                    .services
                    .catalog
                    .titles
                    .list_for_matching(None, None)
                    .await?;
                let mut matches = titles
                    .into_iter()
                    .filter(|title| {
                        title.external_ids.iter().any(|external_id| {
                            external_id.source.eq_ignore_ascii_case("imdb")
                                && normalize_imdb_id(&external_id.value).as_deref()
                                    == Some(target_imdb_id.as_str())
                        })
                    })
                    .collect::<Vec<_>>();

                if matches.len() == 1 {
                    matches.pop()
                } else {
                    None
                }
            }
            None => None,
        };
    }

    if title.is_none() {
        let titles = app
            .services
            .catalog
            .titles
            .list_for_matching(None, None)
            .await?;
        let facet_hint = extract_parameter(&completed.parameters, "*scryer_facet")
            .or_else(|| completed.category.clone());

        title = resolve_title_from_release_candidate(
            &titles,
            &parsed_completed_name,
            facet_hint.as_deref(),
        );

        if title.is_none()
            && let Some(parsed_completed_folder) = parsed_completed_folder.as_ref()
        {
            title = resolve_title_from_release_candidate(
                &titles,
                parsed_completed_folder,
                facet_hint.as_deref(),
            );
        }

        if title.is_none() {
            extracted_dir =
                crate::archive_extractor::extract_archives_if_needed(dest_dir, archive_password)
                    .await?;
            let effective_dir = extracted_dir.as_deref().unwrap_or(dest_dir);
            let video_files = find_video_files(effective_dir, true)?;

            for candidate in title_evidence_candidates_from_video_files(&video_files) {
                title = resolve_title_from_release_candidate(
                    &titles,
                    &candidate,
                    facet_hint.as_deref(),
                );
                if title.is_some() {
                    break;
                }
            }

            title_evidence_video_files = Some(video_files);
        }
    }

    let title = match title {
        Some(t) => t,
        None => {
            let result = ImportResult {
                decision: ImportDecision::Unmatched,
                skip_reason: Some(ImportSkipReason::UnresolvedIdentity),
                error_message: Some(format!(
                    "could not match download '{}' to any monitored title",
                    completed.name
                )),
                ..base_completed_import_result(import_id, completed, started_at)
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;

            return Ok(CompletedImportTargetResolution::Finished(Box::new(result)));
        }
    };

    // Validate supported facets
    if !matches!(
        title.facet,
        MediaFacet::Movie | MediaFacet::Series | MediaFacet::Anime
    ) {
        let result = ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::PolicyMismatch),
            title_id: Some(title.id.clone()),
            error_message: Some(format!(
                "title '{}' has unsupported facet '{:?}', skipping import",
                title.name, title.facet
            )),
            ..base_completed_import_result(import_id, completed, started_at)
        };
        let result_json = serde_json::to_string(&result).ok();
        app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
            .await?;
        return Ok(CompletedImportTargetResolution::Finished(Box::new(result)));
    }

    // 3. FIND VIDEO FILES (extract archives first if needed)
    let is_series = matches!(title.facet, MediaFacet::Series | MediaFacet::Anime);
    if extracted_dir.is_none() {
        extracted_dir =
            crate::archive_extractor::extract_archives_if_needed(dest_dir, archive_password)
                .await?;
    }
    let effective_dir = extracted_dir.as_deref().unwrap_or(dest_dir);
    let video_files = if is_series {
        title_evidence_video_files
            .take()
            .unwrap_or(find_video_files(effective_dir, true)?)
    } else {
        find_video_files(effective_dir, false)?
    };

    if video_files.is_empty() {
        let result = ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            title_id: Some(title.id.clone()),
            error_message: Some(format!("no video files found in {}", completed.dest_dir)),
            ..base_completed_import_result(import_id, completed, started_at)
        };
        let result_json = serde_json::to_string(&result).ok();
        let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
        app.update_import_status_and_notify(import_id, status, result_json)
            .await?;
        return Ok(CompletedImportTargetResolution::Finished(Box::new(result)));
    }

    let series_movie_link_id =
        if let Some(series_movie_link_id) =
            extract_parameter(&completed.parameters, "*scryer_series_movie_link_id")
        {
            Some(series_movie_link_id)
        } else if let Some(legacy_collection_id) =
            extract_parameter(&completed.parameters, "*scryer_collection_id")
        {
            match app
                .services
                .catalog
                .shows
                .find_series_movie_link_by_legacy_collection_id(&legacy_collection_id)
                .await
            {
                Ok(Some(link)) => Some(link.id),
                Ok(None) => None,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        legacy_collection_id = %legacy_collection_id,
                        "failed to resolve legacy series movie collection id"
                    );
                    None
                }
            }
        } else {
            None
        };

    Ok(CompletedImportTargetResolution::Ready(Box::new(
        CompletedImportTarget {
            title,
            is_series,
            video_files,
            extracted_dir,
            series_movie_link_id,
        },
    )))
}

async fn dispatch_completed_import_target(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    completed: &CompletedDownload,
    started_at: chrono::DateTime<Utc>,
    target: &CompletedImportTarget,
) -> AppResult<ImportResult> {
    // Branch on facet: movies import the single largest file, series import all episode files.
    if let Some(ref series_movie_link_id) = target.series_movie_link_id {
        Box::pin(import_series_movie_download(
            app,
            actor,
            &target.title,
            import_id,
            completed,
            &target.video_files,
            started_at,
            series_movie_link_id,
        ))
        .await
    } else if target.is_series {
        Box::pin(import_series_download(
            app,
            actor,
            &target.title,
            import_id,
            completed,
            &target.video_files,
            started_at,
        ))
        .await
    } else {
        Box::pin(import_movie_download(
            app,
            actor,
            &target.title,
            import_id,
            completed,
            &target.video_files,
            started_at,
        ))
        .await
    }
}
// ---------------------------------------------------------------------------
// Movie import: pick largest file, single import
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct SeriesMovieAdditionalImportContext<'a> {
    series_movie_link_id: &'a str,
    linked_episode_id: Option<&'a str>,
    linked_episode_artifacts: &'a [scryer_domain::Episode],
}

#[expect(
    clippy::too_many_arguments,
    reason = "additional movie imports share the normal movie path context without using the upgrade gate"
)]
async fn import_additional_movie_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    source_video: &Path,
    source_size: i64,
    parsed: &ParsedReleaseMetadata,
    media_root: &str,
    rename_enabled: bool,
    rename_template: &str,
    folder_template: &str,
    canonical_dest_path: Option<&Path>,
    series_movie_context: Option<SeriesMovieAdditionalImportContext<'_>>,
    existing_files: &[crate::TitleMediaFile],
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    let canonical_dest_path = if let Some(canonical_dest_path) = canonical_dest_path {
        canonical_dest_path.to_path_buf()
    } else {
        let ext = scryer_domain::canonical_video_extension(source_video)
            .unwrap_or("mkv")
            .to_string();
        let tokens = build_rename_tokens(title, parsed, &ext);
        let rendered_filename = if rename_enabled {
            render_rename_template(rename_template, &tokens)
        } else {
            preserved_import_filename(source_video)
        };
        let full_folder_path =
            effective_title_folder_path(media_root, title, folder_template, parsed.year);
        full_folder_path.join(&rendered_filename)
    };
    let dest_path = additional_import_dest_path(&canonical_dest_path, parsed);
    let linked_episode_artifacts = series_movie_context
        .map(|context| context.linked_episode_artifacts)
        .unwrap_or(&[]);
    let linked_episode_ids = series_movie_context
        .and_then(|context| context.linked_episode_id)
        .map(|episode_id| vec![episode_id.to_string()])
        .unwrap_or_default();

    let check_ctx = crate::import_checks::ImportCheckContext {
        source_path: source_video,
        dest_path: &dest_path,
        source_size: source_size as u64,
        parsed,
        existing_files,
    };
    if let crate::import_checks::ImportVerdict::Reject { reason, code } =
        crate::import_checks::run_import_checks(&check_ctx)
    {
        let artifact_result = if code == "duplicate_file" {
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
            "movie",
            artifact_result,
            Some(code),
            None,
            linked_episode_artifacts,
        )
        .await;
        let skip_reason = Some(match code {
            "duplicate_file" => ImportSkipReason::AlreadyImported,
            "insufficient_disk_space" => ImportSkipReason::DiskFull,
            "invalid_extension" | "sample_file" | "sample_directory" => {
                ImportSkipReason::PolicyMismatch
            }
            _ => ImportSkipReason::PolicyMismatch,
        });
        let result = ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
            skip_reason,
            title_id: Some(title.id.clone()),
            source_system: Some(completed.client_type.clone()),
            source_ref: Some(completed.download_client_item_id.clone()),
            source_title: Some(completed.name.clone()),
            source_path: path_to_stored_string(source_video),
            dest_path: Some(path_to_stored_string(&dest_path)),
            quality: parsed.quality.clone(),
            episode_ids: linked_episode_ids.clone(),
            file_size_bytes: Some(source_size),
            link_type: None,
            error_message: Some(reason),
            started_at,
            completed_at: Utc::now(),
        };
        let result_json = serde_json::to_string(&result).ok();
        let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
        app.update_import_status_and_notify(import_id, status, result_json)
            .await?;
        return Ok(result);
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
        quality_label: parsed.quality.clone(),
        scene_name: Some(parsed.raw_title.clone()),
        release_group: parsed.release_group.clone(),
        source_type: crate::release_parser::parsed_release_source_type(parsed),
        resolution: parsed.quality.clone(),
        video_codec_parsed: parsed.video_codec,
        audio_codec_parsed: parsed.audio.as_ref().map(ToString::to_string),
        audio_channels_parsed: parsed.audio_channels.clone(),
        original_file_path: Some(path_to_stored_string(source_video)),
        grabbed_release_title: Some(completed.name.clone()),
        grabbed_at: Some(started_at.to_rfc3339()),
        edition: parsed.edition.clone(),
        ..Default::default()
    };
    let imported_media_file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&media_file_input)
        .await?;
    if let Some(context) = series_movie_context {
        if let Err(error) = app
            .services
            .library
            .media_files
            .link_file_to_series_movie(&imported_media_file_id, context.series_movie_link_id)
            .await
        {
            tracing::warn!(
                error = %error,
                file_id = %imported_media_file_id,
                series_movie_link_id = %context.series_movie_link_id,
                "failed to link additional imported file to series movie"
            );
        }
        if let Some(linked_episode_id) = context.linked_episode_id
            && let Err(error) = app
                .services
                .library
                .media_files
                .link_file_to_episode(&imported_media_file_id, linked_episode_id)
                .await
        {
            tracing::warn!(
                error = %error,
                file_id = %imported_media_file_id,
                episode_id = %linked_episode_id,
                series_movie_link_id = %context.series_movie_link_id,
                "failed to link additional imported series movie file to linked episode"
            );
        }
    }
    analyze_and_persist_imported_media_file(app, &title.id, &imported_media_file_id, &dest_path)
        .await;
    if let Err(error) = crate::subtitles::reconcile_external_subtitles_for_media_file(
        app,
        &title.id,
        &imported_media_file_id,
        None,
        &dest_path,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            title_id = %title.id,
            file_id = %imported_media_file_id,
            dest_path = %dest_path.display(),
            "failed to reconcile external subtitles after additional movie import"
        );
    }
    maybe_trigger_subtitle_search(app, &title.id, &imported_media_file_id);
    let link_type =
        finalize_import_source_cleanup(app, import_mode, &file_result, &dest_path).await?;

    persist_file_import_artifact(
        app,
        import_id,
        completed,
        title.id.as_str(),
        source_video,
        "movie",
        "imported",
        Some("additional_file"),
        Some(imported_media_file_id.as_str()),
        linked_episode_artifacts,
    )
    .await;

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Imported,
        skip_reason: None,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: Some(completed.name.clone()),
        source_path: path_to_stored_string(source_video),
        dest_path: Some(path_to_stored_string(&dest_path)),
        quality: parsed.quality.clone(),
        episode_ids: linked_episode_ids.clone(),
        file_size_bytes: Some(file_result.size_bytes as i64),
        link_type: Some(link_type),
        error_message: None,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    app.update_import_status_and_notify(import_id, ImportStatus::Completed, result_json)
        .await?;

    let _ = app
        .append_domain_event(new_title_domain_event(
            actor,
            title,
            DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: title_context_snapshot(title),
                media_updates: vec![created_media_update(path_to_stored_string(&dest_path))],
                imported_count: 1,
                import_id: Some(import_id.to_string()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: Some(completed.name.clone()),
                source_path: Some(path_to_stored_string(source_video)),
                dest_path: Some(path_to_stored_string(&dest_path)),
                quality: parsed.quality.clone(),
                episode_ids: linked_episode_ids,
            }),
        ))
        .await;

    Ok(result)
}

async fn import_movie_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    let source_video = pick_largest_file(video_files)?;
    let source_size = std::fs::metadata(&source_video)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template: _,
    } = resolve_import_paths(app, title).await?;

    let parsed = build_augmented_movie_import_metadata(&source_video, completed);
    let existing_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|file| file.role.is_primary())
        .collect::<Vec<_>>();
    if completed_import_purpose(app, completed)
        .await
        .is_additional_file()
    {
        return import_additional_movie_download(
            app,
            actor,
            title,
            import_id,
            completed,
            &source_video,
            source_size,
            &parsed,
            &media_root,
            rename_enabled,
            &rename_template,
            &folder_template,
            None,
            None,
            &existing_files,
            started_at,
        )
        .await;
    }
    let existing_files = existing_files
        .into_iter()
        .filter(|file| file.role.is_primary())
        .collect::<Vec<_>>();
    let quality_profile = resolve_import_quality_profile(app, title).await;
    let existing_score = existing_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        .and_then(|file| file.acquisition_score);
    let prepared = match crate::post_download_gate::prepare_import_candidate(
        app,
        title,
        &parsed,
        &quality_profile,
        &source_video,
        source_size,
        !existing_files.is_empty(),
        existing_score,
        false,
        crate::post_download_gate::RuntimeSampleValidation::automatic(
            title
                .runtime_minutes
                .filter(|runtime_minutes| *runtime_minutes > 0)
                .map(|runtime_minutes| runtime_minutes.saturating_mul(60)),
        ),
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(rejection) => {
            crate::post_download_gate::reject_source_file_before_import(
                app,
                crate::domain_events::DomainEventActor::from(actor),
                title,
                &completed.name,
                &source_video,
                &[],
                &rejection,
            )
            .await;
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                &source_video,
                "movie",
                "rejected",
                rejection.skip_reason.as_ref().map(ImportSkipReason::as_str),
                None,
                &[],
            )
            .await;
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Rejected,
                skip_reason: rejection.skip_reason.clone(),
                title_id: Some(title.id.clone()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: Some(completed.name.clone()),
                source_path: path_to_stored_string(&source_video),
                dest_path: None,
                quality: parsed.quality.clone(),
                episode_ids: Vec::new(),
                file_size_bytes: Some(source_size),
                link_type: None,
                error_message: Some(rejection.message),
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
            app.update_import_status_and_notify(import_id, status, result_json)
                .await?;
            return Ok(result);
        }
    };

    let ext = scryer_domain::canonical_video_extension(&source_video)
        .unwrap_or("mkv")
        .to_string();
    let tokens = build_rename_tokens(title, &prepared.parsed, &ext);
    let rendered_filename = if rename_enabled {
        render_rename_template(&rename_template, &tokens)
    } else {
        preserved_import_filename(&source_video)
    };

    let full_folder_path =
        effective_title_folder_path(&media_root, title, &folder_template, prepared.parsed.year);

    let dest_path = full_folder_path.join(&rendered_filename);
    let check_ctx = crate::import_checks::ImportCheckContext {
        source_path: &source_video,
        dest_path: &dest_path,
        source_size: source_size as u64,
        parsed: &prepared.parsed,
        existing_files: &existing_files,
    };
    if let crate::import_checks::ImportVerdict::Reject { reason, code } =
        crate::import_checks::run_import_checks(&check_ctx)
    {
        let artifact_result = if code == "duplicate_file" {
            "already_present"
        } else {
            "rejected"
        };
        persist_file_import_artifact(
            app,
            import_id,
            completed,
            title.id.as_str(),
            &source_video,
            "movie",
            artifact_result,
            Some(code),
            None,
            &[],
        )
        .await;
        let skip_reason = Some(match code {
            "duplicate_file" => ImportSkipReason::AlreadyImported,
            "insufficient_disk_space" => ImportSkipReason::DiskFull,
            "invalid_extension" | "sample_file" | "sample_directory" => {
                ImportSkipReason::PolicyMismatch
            }
            _ => ImportSkipReason::PolicyMismatch,
        });
        let result = ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
            skip_reason,
            title_id: Some(title.id.clone()),
            source_system: Some(completed.client_type.clone()),
            source_ref: Some(completed.download_client_item_id.clone()),
            source_title: Some(completed.name.clone()),
            source_path: path_to_stored_string(&source_video),
            dest_path: Some(path_to_stored_string(&dest_path)),
            quality: prepared.parsed.quality.clone(),
            episode_ids: Vec::new(),
            file_size_bytes: Some(source_size),
            link_type: None,
            error_message: Some(reason),
            started_at,
            completed_at: Utc::now(),
        };
        let result_json = serde_json::to_string(&result).ok();
        let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
        app.update_import_status_and_notify(import_id, status, result_json)
            .await?;
        return Ok(result);
    }

    let import_mode = app
        .resolve_import_mode(Some(&title.library_id), &title.facet)
        .await?;

    if let Some(existing_file) = existing_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
    {
            let old_score = existing_file.acquisition_score.unwrap_or(0);
            let post_download_score =
                crate::post_download_gate::compute_post_download_acquisition_decision(
                    app,
                    &prepared.parsed,
                    prepared.accepted.as_ref(),
                    &quality_profile,
                    title,
                    title.runtime_minutes,
                    source_size,
                    true,
                    Some(old_score),
                    &prepared.rescore_changes,
                    false,
                )
                .await;
            let new_score = post_download_score.score;
            if new_score > old_score {
                let old_file_recycle_context =
                    crate::upgrade::resolve_old_file_recycle_context(app, title, existing_file)
                        .await?;

                match crate::upgrade::execute_upgrade(
                    app,
                    actor,
                    title,
                    existing_file,
                    &source_video,
                    &dest_path,
                    &prepared,
                    post_download_score.parsed.quality.as_deref(),
                    new_score,
                    old_score,
                    post_download_score.scoring_log.clone(),
                    &[],
                    Some(&media_root),
                    Some(old_file_recycle_context.media_root.as_str()),
                    &old_file_recycle_context.recycle_config,
                    import_mode,
                )
                .await
                {
                    Ok(crate::upgrade::UpgradeResult::Upgraded(outcome)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "imported",
                            Some("upgrade"),
                            None,
                            &[],
                        )
                        .await;
                        let result = ImportResult {
                            import_id: import_id.to_string(),
                            decision: ImportDecision::Imported,
                            skip_reason: None,
                            title_id: Some(title.id.clone()),
                            source_system: Some(completed.client_type.clone()),
                            source_ref: Some(completed.download_client_item_id.clone()),
                            source_title: Some(completed.name.clone()),
                            source_path: path_to_stored_string(&source_video),
                            dest_path: Some(path_to_stored_string(&dest_path)),
                            quality: prepared.parsed.quality.clone(),
                            episode_ids: Vec::new(),
                            file_size_bytes: Some(source_size),
                            link_type: (import_mode == scryer_domain::ImportMode::Move)
                                .then_some(scryer_domain::ImportStrategy::Move),
                            error_message: None,
                            started_at,
                            completed_at: Utc::now(),
                        };
                        tracing::info!(
                            title = %title.name,
                            old_score = outcome.old_score,
                            new_score = outcome.new_score,
                            "movie file upgraded"
                        );
                        persist_title_folder_path_if_missing(app, title, &full_folder_path).await;
                        mark_wanted_completed(app, &title.id, None, Some(outcome.new_score)).await;
                        let result_json = serde_json::to_string(&result).ok();
                        app.update_import_status_and_notify(
                            import_id,
                            ImportStatus::Completed,
                            result_json,
                        )
                        .await?;
                        return Ok(result);
                    }
                    Ok(crate::upgrade::UpgradeResult::Rejected(rejection)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "already_present",
                            rejection.skip_reason.as_ref().map(ImportSkipReason::as_str),
                            None,
                            &[],
                        )
                        .await;
                        let result = ImportResult {
                            import_id: import_id.to_string(),
                            decision: ImportDecision::Rejected,
                            skip_reason: rejection.skip_reason.clone(),
                            title_id: Some(title.id.clone()),
                            source_system: Some(completed.client_type.clone()),
                            source_ref: Some(completed.download_client_item_id.clone()),
                            source_title: Some(completed.name.clone()),
                            source_path: path_to_stored_string(&source_video),
                            dest_path: Some(path_to_stored_string(&dest_path)),
                            quality: prepared.parsed.quality.clone(),
                            episode_ids: Vec::new(),
                            file_size_bytes: Some(source_size),
                            link_type: None,
                            error_message: Some(rejection.message),
                            started_at,
                            completed_at: Utc::now(),
                        };
                        let result_json = serde_json::to_string(&result).ok();
                        app.update_import_status_and_notify(
                            import_id,
                            ImportStatus::Skipped,
                            result_json,
                        )
                        .await?;
                        return Ok(result);
                    }
                    Err(err) => {
                        if import_mode == scryer_domain::ImportMode::Move {
                            tracing::error!(error = %err, "movie upgrade failed in move mode");
                            return Err(err);
                        }
                        tracing::error!(
                            error = %err,
                            "upgrade failed, falling through to normal import"
                        );
                    }
                }
            }
    }

    let file_result = import_file_with_record_progress(
        app,
        import_id,
        &source_video,
        &dest_path,
        import_mode,
        Some(&prepared.source_snapshot),
    )
    .await?;
    persist_title_folder_path_if_missing(app, title, &full_folder_path).await;

    let nfo_enabled = app
        .resolve_nfo_write_on_import(Some(&title.library_id), &title.facet)
        .await?;
    if nfo_enabled {
        let nfo_path = dest_path.with_extension("nfo");
        let nfo_content = render_movie_nfo(title);
        if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await {
            tracing::warn!(
                error = %err,
                path = %nfo_path.display(),
                "failed to write movie NFO sidecar"
            );
        }
    }

    let post_download_score = crate::post_download_gate::compute_post_download_acquisition_decision(
        app,
        &prepared.parsed,
        prepared.accepted.as_ref(),
        &quality_profile,
        title,
        title.runtime_minutes,
        file_result.size_bytes as i64,
        !existing_files.is_empty(),
        existing_files
            .iter()
            .max_by_key(|file| file.acquisition_score.unwrap_or(0))
            .and_then(|file| file.acquisition_score),
        &prepared.rescore_changes,
        false,
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
        original_file_path: Some(path_to_stored_string(source_video.clone())),
        acquisition_score: Some(acq_score),
        scoring_log: post_download_score.scoring_log.clone(),
        ..Default::default()
    };
    let imported_media_file_id = match app
        .services
        .library
        .media_files
        .insert_media_file(&media_file_input)
        .await
    {
        Ok(file_id) => {
            crate::post_download_gate::persist_media_analysis_result(
                &app.services.library.media_files,
                &file_id,
                prepared.accepted.as_ref(),
            )
            .await;
            if let Err(error) = crate::subtitles::reconcile_external_subtitles_for_media_file(
                app, &title.id, &file_id, None, &dest_path,
            )
            .await
            {
                tracing::warn!(
                    error = %error,
                    title_id = %title.id,
                    file_id = %file_id,
                    dest_path = %dest_path.display(),
                    "failed to reconcile external subtitles after import"
                );
            }
            maybe_trigger_subtitle_search(app, &title.id, &file_id);
            Some(file_id)
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                title_id = %title.id,
                dest_path = %dest_path.display(),
                "failed to insert media_files record (import will still succeed)"
            );
            if import_mode == scryer_domain::ImportMode::Move {
                return Err(AppError::Repository(format!(
                    "move import source cleanup blocked because media file insert failed: {err}"
                )));
            }
            None
        }
    };

    let link_type =
        finalize_import_source_cleanup(app, import_mode, &file_result, &dest_path).await?;

    persist_file_import_artifact(
        app,
        import_id,
        completed,
        title.id.as_str(),
        &source_video,
        "movie",
        "imported",
        None,
        imported_media_file_id.as_deref(),
        &[],
    )
    .await;

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: CollectionType::Movie,
        collection_index: "1".to_string(),
        label: prepared.parsed.quality.clone(),
        ordered_path: Some(path_to_stored_string(&dest_path)),
        narrative_order: None,
        first_episode_number: None,
        last_episode_number: None,
        monitored: true,
        created_at: Utc::now(),
    };
    if let Err(err) = app
        .services
        .catalog
        .shows
        .create_collection(collection)
        .await
    {
        tracing::warn!(
            error = %err,
            title_id = %title.id,
            "failed to create collection record"
        );
    }

    spawn_post_processing(PostProcessingContext {
        app: app.clone(),
        actor: crate::domain_events::DomainEventActor::from(actor),
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        dest_path: dest_path.clone(),
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
        season: None,
        episode: None,
        quality: prepared.parsed.quality.clone(),
    });

    mark_wanted_completed(app, &title.id, None, Some(acq_score)).await;

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Imported,
        skip_reason: None,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: Some(completed.name.clone()),
        source_path: path_to_stored_string(&source_video),
        dest_path: Some(path_to_stored_string(&dest_path)),
        quality: prepared.parsed.quality.clone(),
        episode_ids: Vec::new(),
        file_size_bytes: Some(file_result.size_bytes as i64),
        link_type: Some(link_type),
        error_message: None,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    app.update_import_status_and_notify(import_id, ImportStatus::Completed, result_json)
        .await?;

    let _ = app
        .append_domain_event(new_title_domain_event(
            actor,
            title,
            DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: title_context_snapshot(title),
                media_updates: vec![created_media_update(path_to_stored_string(&dest_path))],
                imported_count: 1,
                import_id: Some(import_id.to_string()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: Some(completed.name.clone()),
                source_path: Some(path_to_stored_string(&source_video)),
                dest_path: Some(path_to_stored_string(&dest_path)),
                quality: prepared.parsed.quality.clone(),
                episode_ids: Vec::new(),
            }),
        ))
        .await;

    Ok(result)
}
// ---------------------------------------------------------------------------
// Series movie import: movie-shaped item stored inside the owning series
// ---------------------------------------------------------------------------

#[expect(
    clippy::too_many_arguments,
    reason = "series movie imports coordinate title, source, and link state in a single workflow step"
)]
async fn import_series_movie_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
    series_movie_link_id: &str,
) -> AppResult<ImportResult> {
    let link = match app
        .services
        .catalog
        .shows
        .get_series_movie_link_by_id(series_movie_link_id)
        .await?
    {
        Some(link) if link.series_title_id == title.id => link,
        Some(_) => {
            let result = ImportResult {
                decision: ImportDecision::Failed,
                skip_reason: None,
                title_id: Some(title.id.clone()),
                error_message: Some(format!(
                    "series movie link {series_movie_link_id} does not belong to title {}",
                    title.id
                )),
                ..base_completed_import_result(import_id, completed, started_at)
            };
            let result_json = serde_json::to_string(&result).ok();
            let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
            app.update_import_status_and_notify(import_id, status, result_json)
                .await?;
            return Ok(result);
        }
        None => {
            let result = ImportResult {
                decision: ImportDecision::Failed,
                skip_reason: None,
                title_id: Some(title.id.clone()),
                error_message: Some(format!("series movie link {series_movie_link_id} not found")),
                ..base_completed_import_result(import_id, completed, started_at)
            };
            let result_json = serde_json::to_string(&result).ok();
            let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
            app.update_import_status_and_notify(import_id, status, result_json)
                .await?;
            return Ok(result);
        }
    };
    let movie = &link.movie;

    let source_video = pick_largest_file(video_files)?;
    let source_size = std::fs::metadata(&source_video)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template: _,
    } = resolve_import_paths(app, title).await?;

    let parsed = build_augmented_movie_import_metadata(&source_video, completed);

    let ext = scryer_domain::canonical_video_extension(&source_video)
        .unwrap_or("mkv")
        .to_string();

    let linked_episode = if let Some(linked_episode_id) = link.linked_episode_id.as_deref() {
        app.services
            .catalog
            .shows
            .get_episode_by_id(linked_episode_id)
            .await?
    } else {
        None
    };
    let linked_episode_ids = linked_episode
        .as_ref()
        .map(|episode| vec![episode.id.clone()])
        .unwrap_or_default();
    let linked_episode_artifacts = linked_episode.iter().cloned().collect::<Vec<_>>();
    let season_episode = linked_episode
        .as_ref()
        .and_then(|episode| {
            let season = episode.season_number.as_deref()?.parse::<i32>().ok()?;
            let episode_number = episode.episode_number.as_deref()?.parse::<i32>().ok()?;
            Some(format!("S{season:02}E{episode_number:02}"))
        })
        .unwrap_or_else(|| "S00E00".to_string());
    let rendered_filename = if rename_enabled {
        sanitize_filesystem_component(&format!(
            "{} - {} - {}.{}",
            title.name, season_episode, movie.title, ext
        ))
    } else {
        preserved_import_filename(&source_video)
    };

    // Build destination: <media_root>/<title folder>/Season 00/<filename>
    let full_folder_path = effective_title_folder_path(&media_root, title, &folder_template, None);

    let dest_path = full_folder_path.join("Season 00").join(&rendered_filename);

    // Pre-import checks (same as movie import)
    let existing_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default();
    let series_movie_files: Vec<_> = existing_files
        .iter()
        .filter(|file| file.file_path == path_to_stored_string(&dest_path))
        .cloned()
        .collect();
    if completed_import_purpose(app, completed)
        .await
        .is_additional_file()
    {
        return import_additional_movie_download(
            app,
            actor,
            title,
            import_id,
            completed,
            &source_video,
            source_size,
            &parsed,
            &media_root,
            rename_enabled,
            &rename_template,
            &folder_template,
            Some(&dest_path),
            Some(SeriesMovieAdditionalImportContext {
                series_movie_link_id,
                linked_episode_id: link.linked_episode_id.as_deref(),
                linked_episode_artifacts: &linked_episode_artifacts,
            }),
            &series_movie_files,
            started_at,
        )
        .await;
    }
    let quality_profile = resolve_import_quality_profile(app, title).await;
    let existing_score = series_movie_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        .and_then(|file| file.acquisition_score);
    let prepared = match crate::post_download_gate::prepare_import_candidate(
        app,
        title,
        &parsed,
        &quality_profile,
        &source_video,
        source_size,
        !series_movie_files.is_empty(),
        existing_score,
        false,
        crate::post_download_gate::RuntimeSampleValidation::automatic(
            movie
                .runtime_minutes
                .or(title.runtime_minutes)
                .filter(|runtime_minutes| *runtime_minutes > 0)
                .map(|runtime_minutes| runtime_minutes.saturating_mul(60)),
        ),
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(rejection) => {
            crate::post_download_gate::reject_source_file_before_import(
                app,
                crate::domain_events::DomainEventActor::from(actor),
                title,
                &completed.name,
                &source_video,
                &[],
                &rejection,
            )
            .await;
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                &source_video,
                "movie",
                "rejected",
                rejection.skip_reason.as_ref().map(ImportSkipReason::as_str),
                None,
                &[],
            )
            .await;
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Rejected,
                skip_reason: rejection.skip_reason.clone(),
                title_id: Some(title.id.clone()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: Some(completed.name.clone()),
                source_path: path_to_stored_string(&source_video),
                dest_path: Some(path_to_stored_string(&dest_path)),
                quality: parsed.quality.clone(),
                episode_ids: Vec::new(),
                file_size_bytes: Some(source_size),
                link_type: None,
                error_message: Some(rejection.message),
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;
            return Ok(result);
        }
    };

    // Upgrade check: if there's an existing file for this series movie, score and compare.
    let import_mode = app
        .resolve_import_mode(Some(&title.library_id), &title.facet)
        .await?;

    if let Some(existing_file) = series_movie_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
    {
            let old_score = existing_file.acquisition_score.unwrap_or(0);
            let post_download_score =
                crate::post_download_gate::compute_post_download_acquisition_decision(
                    app,
                    &prepared.parsed,
                    prepared.accepted.as_ref(),
                    &quality_profile,
                    title,
                    movie.runtime_minutes,
                    source_size,
                    true,
                    Some(old_score),
                    &prepared.rescore_changes,
                    false,
                )
                .await;
            let new_score = post_download_score.score;
            if new_score > old_score {
                let old_file_recycle_context =
                    crate::upgrade::resolve_old_file_recycle_context(app, title, existing_file)
                        .await?;

                match crate::upgrade::execute_upgrade(
                    app,
                    actor,
                    title,
                    existing_file,
                    &source_video,
                    &dest_path,
                    &prepared,
                    post_download_score.parsed.quality.as_deref(),
                    new_score,
                    old_score,
                    post_download_score.scoring_log.clone(),
                    &[],
                    Some(&media_root),
                    Some(old_file_recycle_context.media_root.as_str()),
                    &old_file_recycle_context.recycle_config,
                    import_mode,
                )
                .await
                {
                    Ok(crate::upgrade::UpgradeResult::Upgraded(outcome)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "imported",
                            Some("upgrade"),
                            None,
                            &[],
                        )
                        .await;
                        tracing::info!(
                            title = %title.name,
                            movie = %movie.title,
                            old_score = outcome.old_score,
                            new_score = outcome.new_score,
                            "series movie file upgraded"
                        );
                        persist_title_folder_path_if_missing(app, title, &full_folder_path).await;
                        if let Err(error) = app
                            .services
                            .library
                            .media_files
                            .link_file_to_series_movie(&outcome.new_file_id, series_movie_link_id)
                            .await
                        {
                            tracing::warn!(
                                error = %error,
                                file_id = %outcome.new_file_id,
                                series_movie_link_id = %series_movie_link_id,
                                "failed to link upgraded file to series movie"
                            );
                        }
                        if let Some(linked_episode_id) = link.linked_episode_id.as_deref()
                            && let Err(error) = app
                                .services
                                .library
                                .media_files
                                .link_file_to_episode(&outcome.new_file_id, linked_episode_id)
                                .await
                        {
                            tracing::warn!(
                                error = %error,
                                file_id = %outcome.new_file_id,
                                episode_id = %linked_episode_id,
                                series_movie_link_id = %series_movie_link_id,
                                "failed to link upgraded series movie file to linked episode"
                            );
                        }
                        mark_wanted_completed_for_series_movie_link(
                            app,
                            &title.id,
                            series_movie_link_id,
                            Some(outcome.new_score),
                        )
                        .await;
                        let result = ImportResult {
                            import_id: import_id.to_string(),
                            decision: ImportDecision::Imported,
                            skip_reason: None,
                            title_id: Some(title.id.clone()),
                            source_system: Some(completed.client_type.clone()),
                            source_ref: Some(completed.download_client_item_id.clone()),
                            source_title: Some(completed.name.clone()),
                            source_path: path_to_stored_string(&source_video),
                            dest_path: Some(path_to_stored_string(&dest_path)),
                            quality: prepared.parsed.quality.clone(),
                            episode_ids: Vec::new(),
                            file_size_bytes: Some(source_size),
                            link_type: (import_mode == scryer_domain::ImportMode::Move)
                                .then_some(scryer_domain::ImportStrategy::Move),
                            error_message: None,
                            started_at,
                            completed_at: Utc::now(),
                        };
                        let result_json = serde_json::to_string(&result).ok();
                        app.update_import_status_and_notify(
                            import_id,
                            ImportStatus::Completed,
                            result_json,
                        )
                        .await?;
                        return Ok(result);
                    }
                    Ok(crate::upgrade::UpgradeResult::Rejected(rejection)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "already_present",
                            rejection.skip_reason.as_ref().map(ImportSkipReason::as_str),
                            None,
                            &[],
                        )
                        .await;
                        let result = ImportResult {
                            import_id: import_id.to_string(),
                            decision: ImportDecision::Rejected,
                            skip_reason: rejection.skip_reason.clone(),
                            title_id: Some(title.id.clone()),
                            source_system: Some(completed.client_type.clone()),
                            source_ref: Some(completed.download_client_item_id.clone()),
                            source_title: Some(completed.name.clone()),
                            source_path: path_to_stored_string(&source_video),
                            dest_path: Some(path_to_stored_string(&dest_path)),
                            quality: prepared.parsed.quality.clone(),
                            episode_ids: Vec::new(),
                            file_size_bytes: Some(source_size),
                            link_type: None,
                            error_message: Some(rejection.message),
                            started_at,
                            completed_at: Utc::now(),
                        };
                        let result_json = serde_json::to_string(&result).ok();
                        let status =
                            completed_import_status_for_result(&result, ImportStatus::Skipped);
                        app.update_import_status_and_notify(import_id, status, result_json)
                            .await?;
                        return Ok(result);
                    }
                    Err(err) => {
                        if import_mode == scryer_domain::ImportMode::Move {
                            tracing::error!(
                                error = %err,
                                "series movie upgrade failed in move mode"
                            );
                            return Err(err);
                        }
                        tracing::error!(
                            error = %err,
                            "series movie upgrade failed, falling through to normal import"
                        );
                    }
                }
            } else {
                // New file is not better — skip
                persist_file_import_artifact(
                    app,
                    import_id,
                    completed,
                    title.id.as_str(),
                    &source_video,
                    "movie",
                    "already_present",
                    Some("existing_better_or_equal"),
                    None,
                    &linked_episode_artifacts,
                )
                .await;
                let result = ImportResult {
                    import_id: import_id.to_string(),
                    decision: ImportDecision::Skipped,
                    skip_reason: Some(ImportSkipReason::PolicyMismatch),
                    title_id: Some(title.id.clone()),
                    source_system: Some(completed.client_type.clone()),
                    source_ref: Some(completed.download_client_item_id.clone()),
                    source_title: Some(completed.name.clone()),
                    source_path: path_to_stored_string(&source_video),
                    dest_path: Some(path_to_stored_string(&dest_path)),
                    quality: prepared.parsed.quality.clone(),
                    episode_ids: linked_episode_ids.clone(),
                    file_size_bytes: Some(source_size),
                    link_type: None,
                    error_message: Some(format!(
                        "new score {new_score} not better than existing {old_score}"
                    )),
                    started_at,
                    completed_at: Utc::now(),
                };
                let result_json = serde_json::to_string(&result).ok();
                app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                    .await?;
                return Ok(result);
            }
    }

    // Ensure Season 00 directory exists
    if let Some(parent) = dest_path.parent()
        && let Err(err) = tokio::fs::create_dir_all(parent).await
    {
        tracing::warn!(error = %err, path = %parent.display(), "failed to create Season 00 directory");
    }

    // Import file (hardlink or copy)
    let file_result = import_file_with_record_progress(
        app,
        import_id,
        &source_video,
        &dest_path,
        import_mode,
        Some(&prepared.source_snapshot),
    )
    .await?;
    persist_title_folder_path_if_missing(app, title, &full_folder_path).await;

    let post_download_score = crate::post_download_gate::compute_post_download_acquisition_decision(
        app,
        &prepared.parsed,
        prepared.accepted.as_ref(),
        &quality_profile,
        title,
        movie.runtime_minutes,
        file_result.size_bytes as i64,
        !series_movie_files.is_empty(),
        series_movie_files
            .iter()
            .max_by_key(|file| file.acquisition_score.unwrap_or(0))
            .and_then(|file| file.acquisition_score),
        &prepared.rescore_changes,
        false,
    )
    .await;
    let acq_score = post_download_score.score;

    let imported_media_file_id = match app
        .services
        .library
        .media_files
        .insert_media_file(&crate::InsertMediaFileInput {
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
            original_file_path: Some(path_to_stored_string(&source_video)),
            acquisition_score: Some(acq_score),
            scoring_log: post_download_score.scoring_log.clone(),
            ..Default::default()
        })
        .await
    {
        Ok(file_id) => {
            crate::post_download_gate::persist_media_analysis_result(
                &app.services.library.media_files,
                &file_id,
                prepared.accepted.as_ref(),
            )
            .await;
            if let Err(error) = crate::subtitles::reconcile_external_subtitles_for_media_file(
                app, &title.id, &file_id, None, &dest_path,
            )
            .await
            {
                tracing::warn!(
                    error = %error,
                    title_id = %title.id,
                    file_id = %file_id,
                    dest_path = %dest_path.display(),
                    "failed to reconcile external subtitles after import"
                );
            }
            maybe_trigger_subtitle_search(app, &title.id, &file_id);
            Some(file_id)
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                title_id = %title.id,
                dest_path = %dest_path.display(),
                "failed to insert series movie media_files record"
            );
            if import_mode == scryer_domain::ImportMode::Move {
                return Err(AppError::Repository(format!(
                    "move import source cleanup blocked because media file insert failed: {err}"
                )));
            }
            None
        }
    };

    let link_type =
        finalize_import_source_cleanup(app, import_mode, &file_result, &dest_path).await?;

    persist_file_import_artifact(
        app,
        import_id,
        completed,
        title.id.as_str(),
        &source_video,
        "movie",
        "imported",
        None,
        imported_media_file_id.as_deref(),
        &linked_episode_artifacts,
    )
    .await;

    if let Some(file_id) = imported_media_file_id.as_deref()
        && let Err(err) = app
            .services
            .library
            .media_files
            .link_file_to_series_movie(file_id, series_movie_link_id)
            .await
    {
        tracing::warn!(
            error = %err,
            file_id = %file_id,
            series_movie_link_id = %series_movie_link_id,
            "failed to link imported file to series movie"
        );
    }
    if let Some(file_id) = imported_media_file_id.as_deref()
        && let Some(linked_episode_id) = link.linked_episode_id.as_deref()
        && let Err(err) = app
            .services
            .library
            .media_files
            .link_file_to_episode(file_id, linked_episode_id)
            .await
    {
        tracing::warn!(
            error = %err,
            file_id = %file_id,
            episode_id = %linked_episode_id,
            series_movie_link_id = %series_movie_link_id,
            "failed to link imported series movie file to linked episode"
        );
    }

    // Write Jellyfin-compatible NFO with airsbefore_season
    let nfo_enabled = app
        .resolve_nfo_write_on_import(Some(&title.library_id), &title.facet)
        .await?;
    if nfo_enabled {
        let nfo_path = dest_path.with_extension("nfo");
        let nfo_content =
            crate::nfo::render_series_movie_episode_nfo(movie, &season_episode, link.after_season);
        if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await {
            tracing::warn!(
                error = %err,
                path = %nfo_path.display(),
                "failed to write series movie NFO sidecar"
            );
        }
    }

    mark_wanted_completed_for_series_movie_link(
        app,
        &title.id,
        series_movie_link_id,
        Some(acq_score),
    )
    .await;

    // Spawn post-processing
    spawn_post_processing(PostProcessingContext {
        app: app.clone(),
        actor: crate::domain_events::DomainEventActor::from(actor),
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        dest_path: dest_path.clone(),
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
        season: None,
        episode: None,
        quality: prepared.parsed.quality.clone(),
    });

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Imported,
        skip_reason: None,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: Some(completed.name.clone()),
        source_path: path_to_stored_string(&source_video),
        dest_path: Some(path_to_stored_string(&dest_path)),
        quality: prepared.parsed.quality.clone(),
        episode_ids: linked_episode_ids.clone(),
        file_size_bytes: Some(file_result.size_bytes as i64),
        link_type: Some(link_type),
        error_message: None,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    app.update_import_status_and_notify(import_id, ImportStatus::Completed, result_json)
        .await?;

    app.append_domain_event(new_title_domain_event(
        actor,
        title,
        DomainEventPayload::ImportCompleted(ImportCompletedEventData {
            title: title_context_snapshot(title),
            media_updates: vec![created_media_update(path_to_stored_string(&dest_path))],
            imported_count: 1,
            import_id: Some(import_id.to_string()),
            source_system: Some(completed.client_type.clone()),
            source_ref: Some(completed.download_client_item_id.clone()),
            source_title: Some(completed.name.clone()),
            source_path: Some(path_to_stored_string(&source_video)),
            dest_path: Some(path_to_stored_string(&dest_path)),
            quality: prepared.parsed.quality.clone(),
            episode_ids: linked_episode_ids.clone(),
        }),
    ))
    .await?;

    Ok(result)
}
async fn mark_wanted_completed_for_series_movie_link(
    app: &AppUseCase,
    title_id: &str,
    series_movie_link_id: &str,
    imported_score: Option<i32>,
) {
    match app
        .services
        .workflow
        .wanted_items
        .list_wanted_items(WantedItemsQuery {
            statuses: vec!["wanted".into()],
            media_types: vec!["series_movie".into()],
            title_id: Some(title_id.to_string()),
            limit: 100,
            ..WantedItemsQuery::default()
        })
        .await
    {
        Ok(items) => {
            for item in items {
                if item.series_movie_link_id.as_deref() == Some(series_movie_link_id) {
                    let now = Utc::now().to_rfc3339();
                    let _ = app
                        .services
                        .workflow
                        .wanted_items
                        .transition_wanted_to_completed(&WantedCompleteTransition {
                            id: item.id.clone(),
                            last_search_at: Some(now),
                            search_count: item.search_count,
                            current_score: imported_score.or(item.current_score),
                            grabbed_release: if imported_score.is_some() {
                                None
                            } else {
                                item.grabbed_release.clone()
                            },
                        })
                        .await;
                    return;
                }
            }
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                title_id = title_id,
                series_movie_link_id = series_movie_link_id,
                "failed to look up wanted item for series movie"
            );
        }
    }
}
