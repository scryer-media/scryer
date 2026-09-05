#[expect(
    clippy::too_many_arguments,
    reason = "completed import orchestration carries durable evidence and retry context explicitly"
)]
async fn run_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
    manual_title_id: Option<&str>,
    started_at: chrono::DateTime<Utc>,
    archive_password: Option<&str>,
    preparation_permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> AppResult<ImportResult> {
    let mut preparation_permit = Some(match preparation_permit {
        Some(permit) => permit,
        None => {
            app.runtime
                .imports
                .execution_coordinator
                .acquire_preparation()
                .await
        }
    });
    let target = match Box::pin(resolve_completed_import_target(
        app,
        import_id,
        completed,
        release_evidence,
        manual_title_id,
        started_at,
        archive_password,
        &mut preparation_permit,
    ))
    .await?
    {
        CompletedImportTargetResolution::Ready(target) => target,
        CompletedImportTargetResolution::Finished(result) => return Ok(*result),
    };

    drop(preparation_permit.take());
    let result = dispatch_completed_import_target(
        app,
        actor,
        import_id,
        completed,
        release_evidence,
        started_at,
        &target,
    )
    .await;

    // Clean up extracted archive directory if we created one
    if let Some(ref dir) = target.extracted_dir {
        crate::archive_extractor::cleanup_extracted_dir(dir).await;
    }

    result
}

/// Carry out a refused import.
///
/// One place, shared by the title and series-movie-link paths (the episode path
/// does the same thing through `EpisodeImportOutcome`), because *what a refusal
/// costs the release* is a property of the decision, not of the scope. The
/// judgement itself was made in [`crate::import_decide::decide_import`]; this
/// only executes the disposition it returned.
///
/// - [`RejectionDisposition::Blocklist`] recycles the bytes, burns the release
///   for this title and reopens the scope's search, so the retry seeks a
///   different candidate instead of the same lie.
/// - [`RejectionDisposition::Skip`] records `already_present`: the release lost
///   a fair comparison and there is nothing to look for.
/// - [`RejectionDisposition::Hold`] records `skipped`: nobody can decide this
///   without an operator.
///
/// All three leave the download in `ImportBlocked` — `result_state.rs` maps
/// every non-`Imported` decision there — so the dispositions differ only in
/// their side effects (design §9, D17 restated).
async fn carry_out_import_rejection(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    context: ImportRejectionContext<'_>,
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    let ImportRejectionContext {
        rejection,
        disposition,
        release_title,
        source_title,
        source_video,
        dest_path,
        source_size,
        quality,
        episode_ids,
        series_movie_link_id,
        episode_artifacts,
    } = context;

    tracing::info!(
        title_id = %title.id,
        code = rejection.recycle_reason,
        ?disposition,
        "{}",
        rejection.message
    );

    let (artifact_result, decision) = match disposition {
        crate::import_decide::RejectionDisposition::Blocklist => {
            crate::post_download_gate::reject_source_file_before_import(
                app,
                crate::domain_events::DomainEventActor::from(actor),
                title,
                release_title,
                source_video,
                crate::post_download_gate::BlocklistAttribution {
                    episode_ids,
                    collection_id: None,
                    series_movie_link_id,
                },
                None,
                &rejection,
            )
            .await;
            ("rejected", ImportDecision::Rejected)
        }
        crate::import_decide::RejectionDisposition::Skip => {
            ("already_present", ImportDecision::Skipped)
        }
        crate::import_decide::RejectionDisposition::Hold => ("skipped", ImportDecision::Skipped),
    };

    persist_file_import_artifact(
        app,
        import_id,
        completed,
        title.id.as_str(),
        source_video,
        "movie",
        artifact_result,
        Some(rejection.recycle_reason),
        None,
        episode_artifacts,
    )
    .await?;
    let release_burned = decision == ImportDecision::Rejected;
    let result = ImportResult {
        import_id: import_id.to_string(),
        decision,
        skip_reason: rejection.skip_reason.clone(),
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: source_title.map(str::to_string),
        source_path: path_to_stored_string(source_video),
        dest_path: Some(path_to_stored_string(dest_path)),
        quality,
        episode_ids: episode_ids.to_vec(),
        file_size_bytes: Some(source_size),
        link_type: None,
        error_message: Some(rejection.message),
        // Only the `Blocklist` disposition burns the release (and is the only one
        // that yields `Rejected` here); the tracked download then fails instead
        // of parking as import-blocked.
        release_burned,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
        .await?;
    Ok(result)
}

/// The per-import facts [`carry_out_import_rejection`] needs, bundled so the two
/// call sites cannot swap two `Option<&str>` arguments.
struct ImportRejectionContext<'a> {
    rejection: crate::post_download_gate::ImportedFileRejection,
    disposition: crate::import_decide::RejectionDisposition,
    release_title: &'a str,
    source_title: Option<&'a str>,
    source_video: &'a Path,
    dest_path: &'a Path,
    source_size: i64,
    quality: Option<String>,
    episode_ids: &'a [String],
    /// Set on the series-movie-link path, so a rejection files under the link
    /// rather than only under the episode it happens to be tied to.
    series_movie_link_id: Option<&'a str>,
    episode_artifacts: &'a [scryer_domain::Episode],
}

struct CompletedImportTarget {
    title: scryer_domain::Title,
    is_series: bool,
    video_files: Vec<ImportVideoFile>,
    extracted_dir: Option<PathBuf>,
    series_movie_link_id: Option<String>,
}

enum CompletedImportTargetResolution {
    Ready(Box<CompletedImportTarget>),
    Finished(Box<ImportResult>),
}

pub(super) async fn archive_extraction_destination_for_title(
    app: &AppUseCase,
    import_id: &str,
    title: &scryer_domain::Title,
) -> AppResult<crate::archive_extractor::ArchiveExtractionDestination> {
    let ImportPathSettings {
        media_root,
        folder_template,
        ..
    } = resolve_import_paths(app, title).await?;
    let staging_parent = effective_title_folder_path(&media_root, title, &folder_template, None);
    persist_title_folder_path_if_missing(app, title, &staging_parent).await?;
    Ok(
        crate::archive_extractor::ArchiveExtractionDestination::new(staging_parent, import_id)
            .with_stale_cleanup_parent(media_root),
    )
}

struct TitlelessArchiveMatch {
    title: scryer_domain::Title,
    extracted_dir: PathBuf,
}

enum TitlelessArchiveRelocation {
    Ready(PathBuf),
    ReextractUnderMatchedTitle,
}

async fn relocate_titleless_archive_workspace_for_title(
    title: &scryer_domain::Title,
    destination: crate::archive_extractor::ArchiveExtractionDestination,
    extracted_dir: PathBuf,
) -> AppResult<TitlelessArchiveRelocation> {
    let target_parent = destination.staging_parent().to_path_buf();
    if extracted_dir.parent() == Some(target_parent.as_path()) {
        return Ok(TitlelessArchiveRelocation::Ready(extracted_dir));
    }

    tokio::fs::create_dir_all(&target_parent)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to create archive staging parent {}: {error}",
                target_parent.display()
            ))
        })?;

    let mut target = target_parent.join(
        extracted_dir
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(".scryer-ax-relocated")),
    );
    if target.exists() {
        target = target_parent.join(format!(
            ".scryer-ax-{:016x}",
            uuid::Uuid::new_v4().as_u128() as u64
        ));
    }

    match tokio::fs::rename(&extracted_dir, &target).await {
        Ok(()) => Ok(TitlelessArchiveRelocation::Ready(target)),
        Err(error) => {
            crate::archive_extractor::cleanup_extracted_dir(&extracted_dir).await;
            if crate::fs_safety::is_cross_device_error(&error) {
                return Ok(TitlelessArchiveRelocation::ReextractUnderMatchedTitle);
            }
            Err(AppError::Validation(format!(
                "archive matched title '{}' but extracted workspace {} could not be moved to {} without copying: {error}",
                title.name,
                extracted_dir.display(),
                target.display()
            )))
        }
    }
}

async fn archive_extraction_destination_for_completed_facet(
    app: &AppUseCase,
    import_id: &str,
    completed: &CompletedDownload,
) -> AppResult<
    Option<(
        crate::archive_extractor::ArchiveExtractionDestination,
        MediaFacet,
    )>,
> {
    let Some(facet) = archive_probe_facet_from_completed(completed) else {
        return Ok(None);
    };
    let Some(library) = app
        .services
        .catalog
        .libraries
        .default_for_facet(facet.clone())
        .await?
    else {
        return Ok(None);
    };
    let Some(root_path) = library
        .roots
        .iter()
        .find(|root| root.is_default)
        .or_else(|| library.roots.first())
        .map(|root| root.path.clone())
    else {
        return Ok(None);
    };

    Ok(Some((
        crate::archive_extractor::ArchiveExtractionDestination::new(root_path, import_id),
        facet,
    )))
}

fn archive_probe_facet_from_completed(completed: &CompletedDownload) -> Option<MediaFacet> {
    extract_parameter(&completed.parameters, "*scryer_facet")
        .or_else(|| completed.category.clone())
        .and_then(|hint| media_facet_from_archive_hint(&hint))
}

fn media_facet_from_archive_hint(value: &str) -> Option<MediaFacet> {
    match value.trim().to_ascii_lowercase().as_str() {
        "movie" | "movies" => Some(MediaFacet::Movie),
        "series" | "show" | "shows" | "tv" => Some(MediaFacet::Series),
        "anime" => Some(MediaFacet::Anime),
        _ => None,
    }
}

fn archive_extraction_would_be_needed_best_effort(dir: &Path) -> bool {
    match crate::archive_extractor::archive_extraction_would_be_needed(dir) {
        Ok(needed) => needed,
        Err(error) => {
            tracing::warn!(
                error = %error,
                path = %dir.display(),
                "failed to inspect archive need while building unmatched import detail"
            );
            false
        }
    }
}

async fn mark_import_extracting(app: &AppUseCase, import_id: &str) -> AppResult<()> {
    app.update_import_transfer_progress_and_notify(
        import_id,
        scryer_domain::ImportTransferPhase::Extracting,
        0,
        0,
    )
    .await
}

async fn try_match_titleless_archive_from_inner_video(
    app: &AppUseCase,
    import_id: &str,
    completed: &CompletedDownload,
    dest_dir: &Path,
    archive_password: Option<&str>,
) -> AppResult<Option<TitlelessArchiveMatch>> {
    if !archive_extraction_would_be_needed_best_effort(dest_dir) {
        return Ok(None);
    }
    let Some((destination, facet)) =
        archive_extraction_destination_for_completed_facet(app, import_id, completed).await?
    else {
        return Ok(None);
    };

    let archive_provider = app
        .services
        .integrations
        .archive_extractor_plugin_provider
        .available()
        .cloned();

    mark_import_extracting(app, import_id).await?;
    let Some(extracted_dir) = ({
        let _archive_extraction_permit = app
            .runtime
            .imports
            .execution_coordinator
            .acquire_archive_extraction()
            .await;
        crate::archive_extractor::extract_archives_if_needed(
            dest_dir,
            Some(destination),
            archive_password,
            archive_provider.clone(),
        )
        .await?
    }) else {
        return Ok(None);
    };

    let is_series = matches!(facet, MediaFacet::Series | MediaFacet::Anime);
    let video_files = match find_video_files(&extracted_dir, is_series) {
        Ok(video_files) => video_files,
        Err(error) => {
            crate::archive_extractor::cleanup_extracted_dir(&extracted_dir).await;
            return Err(error);
        }
    };
    if video_files.is_empty() {
        crate::archive_extractor::cleanup_extracted_dir(&extracted_dir).await;
        return Ok(None);
    }

    let titles = app
        .services
        .catalog
        .titles
        .list_for_matching(None, None)
        .await?;
    // The titleless-archive probe predates filename recovery and is untouched
    // by it: SAB and NZBGet unpack before Scryer sees the download, so the
    // gated clients never reach this path.
    let probe_files: Vec<ImportVideoFile> = video_files
        .iter()
        .cloned()
        .map(ImportVideoFile::physical)
        .collect();
    for candidate in title_evidence_candidates_from_video_files(&probe_files) {
        if let Some(title) =
            resolve_title_from_release_candidate(&titles, &candidate, Some(facet.as_str()))
        {
            let destination =
                match archive_extraction_destination_for_title(app, import_id, &title).await {
                    Ok(destination) => destination,
                    Err(error) => {
                        crate::archive_extractor::cleanup_extracted_dir(&extracted_dir).await;
                        return Err(error);
                    }
                };
            let relocation =
                relocate_titleless_archive_workspace_for_title(&title, destination, extracted_dir)
                    .await?;
            let extracted_dir = match relocation {
                TitlelessArchiveRelocation::Ready(extracted_dir) => extracted_dir,
                TitlelessArchiveRelocation::ReextractUnderMatchedTitle => {
                    mark_import_extracting(app, import_id).await?;
                    let destination =
                        archive_extraction_destination_for_title(app, import_id, &title).await?;
                    ({
                        let _archive_extraction_permit = app
                            .runtime
                            .imports
                            .execution_coordinator
                            .acquire_archive_extraction()
                            .await;
                        crate::archive_extractor::extract_archives_if_needed(
                            dest_dir,
                            Some(destination),
                            archive_password,
                            archive_provider.clone(),
                        )
                        .await?
                    })
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "archive matched title '{}' but could not be re-extracted under the matched title destination",
                            title.name
                        ))
                    })?
                }
            };
            return Ok(Some(TitlelessArchiveMatch {
                title,
                extracted_dir,
            }));
        }
    }

    crate::archive_extractor::cleanup_extracted_dir(&extracted_dir).await;
    Ok(None)
}

#[expect(
    clippy::too_many_arguments,
    reason = "completed import target resolution carries explicit durable evidence and preparation ownership"
)]
async fn resolve_completed_import_target(
    app: &AppUseCase,
    import_id: &str,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
    manual_title_id: Option<&str>,
    started_at: chrono::DateTime<Utc>,
    archive_password: Option<&str>,
    preparation_permit: &mut Option<tokio::sync::OwnedSemaphorePermit>,
) -> AppResult<CompletedImportTargetResolution> {
    // 2. TITLE MATCHING
    let mut title = None;
    let dest_dir = Path::new(&completed.dest_dir);
    let mut extracted_dir: Option<PathBuf> = None;
    // One srrdb session for this whole `run_import` call: the setting is read
    // at most once, results are reused between the titleless probe below and
    // the final file list, and one outage stops the rest of this import.
    let mut srrdb = SrrdbFilenameRecovery::default();
    if let Some(manual_title_id) = manual_title_id.map(str::trim).filter(|id| !id.is_empty()) {
        if let Some(submission_title_id) = release_evidence.title_id()
            && submission_title_id != manual_title_id
        {
            return Err(AppError::Validation(format!(
                "manual title {manual_title_id:?} is outside the durable Scryer submission title {submission_title_id:?}"
            )));
        }
        title = app
            .services
            .catalog
            .titles
            .get_by_id(manual_title_id)
            .await?;
    } else if let Some(title_id) = release_evidence.title_id() {
        title = app.services.catalog.titles.get_by_id(title_id).await?;
    }

    if title.is_none() {
        let titles = app
            .services
            .catalog
            .titles
            .list_for_matching(None, None)
            .await?;
        // Without a client-reported release name the largest non-sample video
        // (never an arbitrary first file) is the release claim.
        let release_title = release_evidence.release_title(None).or_else(|| {
            let files = find_video_files(dest_dir, true)
                .ok()
                .filter(|files| !files.is_empty())
                .or_else(|| find_video_files(dest_dir, false).ok())?;
            let file = pick_largest_file(&files).ok()?;
            release_evidence.release_title(Some(&file))
        });
        if let Some(release_title) = release_title {
            let parsed_release_title =
                normalize_release_title_signal(parse_release_metadata(&release_title));
            title = resolve_title_from_release_candidate(
                &titles,
                &parsed_release_title,
                release_evidence.facet(),
            );
        }
    }

    if title.is_none() {
        drop(preparation_permit.take());
        let archive_match = try_match_titleless_archive_from_inner_video(
            app,
            import_id,
            completed,
            dest_dir,
            archive_password,
        )
        .await;
        *preparation_permit = Some(
            app.runtime
                .imports
                .execution_coordinator
                .acquire_preparation()
                .await,
        );
        if let Some(archive_match) = archive_match? {
            title = Some(archive_match.title);
            extracted_dir = Some(archive_match.extracted_dir);
        }
    }

    // Whether the release name Scryer already holds for this download parses
    // to a usable title. srrdb only ever helps a download whose own name says
    // nothing: a usable name that matched no library title is an adopted
    // download for something Scryer does not have, and the recovered scene
    // name would carry the same title, so hashing it would buy nothing.
    let release_evidence_title_unusable = release_evidence
        .release_title(None)
        .as_deref()
        .and_then(parse_usable_release_title)
        .is_none();

    if title.is_none() && release_evidence_title_unusable {
        // Last resort before giving up: neither the release name nor the
        // download's own video files carry a title signal, so ask srrdb for
        // their original names and try the ordinary release-candidate match
        // again with those. The results are memoized and reused by the file
        // list built further down.
        let probe_dir = extracted_dir.as_deref().unwrap_or(dest_dir);
        let probe_files = find_video_files(probe_dir, true)
            .ok()
            .filter(|files| !files.is_empty())
            .or_else(|| find_video_files(probe_dir, false).ok())
            .unwrap_or_default();
        if !probe_files.is_empty() {
            // Nothing has identified the title, so every obfuscated-stem file
            // has to speak for itself.
            let enriched = srrdb.enrich(app, completed, probe_files, true).await;
            if enriched.iter().any(|file| file.logical_name.is_some()) {
                let titles = app
                    .services
                    .catalog
                    .titles
                    .list_for_matching(None, None)
                    .await?;
                for candidate in title_evidence_candidates_from_video_files(&enriched) {
                    if let Some(matched) = resolve_title_from_release_candidate(
                        &titles,
                        &candidate,
                        release_evidence.facet(),
                    ) {
                        title = Some(matched);
                        break;
                    }
                }
            }
        }
    }

    let title = match title {
        Some(t) => t,
        None => {
            let archive_message = if archive_extraction_would_be_needed_best_effort(dest_dir) {
                "; archived downloads require a facet/category hint and configured library root before Scryer can stage extraction under the import destination"
            } else {
                ""
            };
            let result = ImportResult {
                decision: ImportDecision::Unmatched,
                skip_reason: Some(ImportSkipReason::UnresolvedIdentity),
                error_message: Some(format!(
                    "could not match download '{}' to any monitored title{}",
                    release_evidence
                        .release_title(None)
                        .unwrap_or_else(|| "unnamed download".to_string()),
                    archive_message
                )),
                release_burned: false,
                ..base_completed_import_result(import_id, completed, release_evidence, started_at)
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
            release_burned: false,
            ..base_completed_import_result(import_id, completed, release_evidence, started_at)
        };
        let result_json = serde_json::to_string(&result).ok();
        app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
            .await?;
        return Ok(CompletedImportTargetResolution::Finished(Box::new(result)));
    }

    // 3. FIND VIDEO FILES (extract archives first if needed)
    let is_series = matches!(title.facet, MediaFacet::Series | MediaFacet::Anime);
    if extracted_dir.is_none() {
        let extraction_destination = if archive_extraction_would_be_needed_best_effort(dest_dir) {
            Some(archive_extraction_destination_for_title(app, import_id, &title).await?)
        } else {
            None
        };
        if extraction_destination.is_some() {
            mark_import_extracting(app, import_id).await?;
        }
        let extraction_stream = if let Some(destination) = extraction_destination.as_ref() {
            Some(
                app.runtime
                    .imports
                    .active_streams
                    .register(
                        import_id,
                        &title.library_id,
                        title.facet.clone(),
                        dest_dir,
                        destination.staging_parent(),
                    )
                    .await,
            )
        } else {
            None
        };
        drop(preparation_permit.take());
        let extraction = {
            let _archive_extraction_permit = app
                .runtime
                .imports
                .execution_coordinator
                .acquire_archive_extraction()
                .await;
            if let Some(stream) = &extraction_stream {
                stream.mark_extracting().await;
            }
            crate::archive_extractor::extract_archives_if_needed(
                dest_dir,
                extraction_destination,
                archive_password,
                app.services
                    .integrations
                    .archive_extractor_plugin_provider
                    .available()
                    .cloned(),
            )
            .await
        };
        if let Some(stream) = extraction_stream {
            stream.finish().await;
        }
        *preparation_permit = Some(
            app.runtime
                .imports
                .execution_coordinator
                .acquire_preparation()
                .await,
        );
        extracted_dir = extraction?;
    }
    let effective_dir = extracted_dir.as_deref().unwrap_or(dest_dir);
    let video_files = match if is_series {
        find_video_files(effective_dir, true)
    } else {
        find_video_files(effective_dir, false)
    } {
        Ok(video_files) => video_files,
        Err(error) => {
            if let Some(ref dir) = extracted_dir {
                crate::archive_extractor::cleanup_extracted_dir(dir).await;
            }
            return Err(error);
        }
    };
    // Reuses anything the titleless probe already resolved; only files it never
    // saw are hashed and looked up here.
    //
    // Whether a file needs a name of its own is a property of this download,
    // not of its folder. A multi-file series pack always does: whatever named
    // the release says which title and season, never which member is which
    // episode, and `file_episode_identity_for_title` reads only the file stem.
    // Otherwise it needs one only when the release name itself is unusable:
    // with a single video file and a usable release name the release name
    // already carries the episode or the year and the quality, planning
    // proceeds on it today, and hashing plus a third-party request would buy
    // nothing.
    let needs_own_name = (is_series && video_files.len() > 1) || release_evidence_title_unusable;
    let video_files = srrdb
        .enrich(app, completed, video_files, needs_own_name)
        .await;

    if video_files.is_empty() {
        if let Some(ref dir) = extracted_dir {
            crate::archive_extractor::cleanup_extracted_dir(dir).await;
        }
        let result = ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            title_id: Some(title.id.clone()),
            error_message: Some(format!("no video files found in {}", completed.dest_dir)),
            release_burned: false,
            ..base_completed_import_result(import_id, completed, release_evidence, started_at)
        };
        let result_json = serde_json::to_string(&result).ok();
        let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
        app.update_import_status_and_notify(import_id, status, result_json)
            .await?;
        return Ok(CompletedImportTargetResolution::Finished(Box::new(result)));
    }

    let series_movie_link_id = match release_evidence.scope() {
        Some(SubmissionScope::SeriesMovie {
            series_movie_link_id,
        }) => Some(series_movie_link_id.clone()),
        Some(SubmissionScope::Collection { collection_id }) => {
            match app
                .services
                .catalog
                .shows
                .find_series_movie_link_by_legacy_collection_id(collection_id)
                .await
            {
                Ok(Some(link)) => Some(link.id),
                Ok(None) => None,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        collection_id,
                        "failed to resolve legacy series movie collection id"
                    );
                    None
                }
            }
        }
        _ => None,
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
    release_evidence: &ReleaseEvidence,
    started_at: chrono::DateTime<Utc>,
    target: &CompletedImportTarget,
) -> AppResult<ImportResult> {
    // The target title is settled and nothing has been written yet: the last
    // point an import can be refused cleanly while an operation owns the title
    // (FR-084). The refusal reaches the operator as a failed import record.
    app.ensure_location_ownership_allows_title(
        &crate::location::ownership_guard::COMPLETED_IMPORT_ENTRY,
        &target.title.id,
    )
    .await?;
    // Branch on facet: movies import the single largest file, series import all episode files.
    //
    // The automatic lane's operator-intent signal is the submission purpose; the
    // manual lane passes the bypass mode directly (see
    // `operator_initiated_import`).
    let runtime_sample_mode = if release_evidence.purpose().is_manual_replacement() {
        crate::post_download_gate::RuntimeSampleValidationMode::BypassRuntimeSampleCheck
    } else {
        crate::post_download_gate::RuntimeSampleValidationMode::EnforceAutomatic
    };
    if let Some(ref series_movie_link_id) = target.series_movie_link_id {
        Box::pin(import_series_movie_download(
            app,
            actor,
            &target.title,
            import_id,
            completed,
            release_evidence,
            &target.video_files,
            started_at,
            series_movie_link_id,
            runtime_sample_mode,
        ))
        .await
    } else if target.is_series {
        let source_root = target
            .extracted_dir
            .as_deref()
            .unwrap_or_else(|| Path::new(&completed.dest_dir));
        Box::pin(import_series_download(
            app,
            actor,
            &target.title,
            import_id,
            completed,
            release_evidence,
            source_root,
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
            release_evidence,
            &target.video_files,
            started_at,
            runtime_sample_mode,
        ))
        .await
    }
}

/// Did an operator choose this file, rather than the acquisition loop?
///
/// The runtime-sample bypass is that signal in practice — the manual lane has
/// always passed it, and it means exactly "a person picked this, do not apply
/// the automatic safety rails". The two paths that reach here read it from one
/// place so they cannot disagree about what "manual" means.
///
/// **What this actually changed**, stated precisely because the reading matters
/// for how much the predicate can be trusted: on the *automatic* lane it is
/// byte-identical to what it replaced. `dispatch_completed_import_target`
/// derives `runtime_sample_mode` from
/// `release_evidence.purpose().is_manual_replacement()`, and the movie and link
/// paths previously read `release_evidence.purpose()` themselves — the same bit,
/// spelled twice. The behaviour change is confined to the manual **movie** path
/// (`manual.rs`), which passes `BypassRuntimeSampleCheck` directly and whose
/// operator-chosen file was therefore eligible to be refused and its release
/// blocklisted for the title.
///
/// The manual *link* path does not reach here at all:
/// `execute_manual_series_movie_import` never calls
/// `import_series_movie_download`, so it has always been outside the verdict
/// gate. Pinned by
/// `a_manual_series_movie_link_import_never_reaches_the_verdict_gate`.
fn operator_initiated_import(
    runtime_sample_mode: crate::post_download_gate::RuntimeSampleValidationMode,
) -> bool {
    matches!(
        runtime_sample_mode,
        crate::post_download_gate::RuntimeSampleValidationMode::BypassRuntimeSampleCheck
    )
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

/// Runtime-sample validation for a movie import, choosing the manual bypass mode
/// for a manual operator replacement (which always lands, like a manual file pick).
fn manual_aware_runtime_sample_validation(
    expected_runtime_seconds: Option<i32>,
    manual_replacement: bool,
) -> crate::post_download_gate::RuntimeSampleValidation {
    if manual_replacement {
        crate::post_download_gate::RuntimeSampleValidation::manual_override(
            expected_runtime_seconds,
        )
    } else {
        crate::post_download_gate::RuntimeSampleValidation::automatic(expected_runtime_seconds)
    }
}

/// Park an automatic movie-scoped replacement for manual resolution: nothing is
/// transferred, no media-file record changes, and the release is not burned into
/// the blocklist. The skipped result leaves the tracked download in the blocked
/// import queue so the operator decides.
#[expect(
    clippy::too_many_arguments,
    reason = "held replacements report the same source, sizing, and timing context as a normal movie import result"
)]
async fn hold_replacement_for_manual_resolution(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
    source_video: &Path,
    source_size: i64,
    quality: Option<String>,
    code: &'static str,
    message: String,
    started_at: DateTime<Utc>,
) -> AppResult<ImportResult> {
    tracing::info!(
        title_id = %title.id,
        file = %source_video.display(),
        code,
        "holding movie import for manual resolution"
    );
    persist_file_import_artifact(
        app,
        import_id,
        completed,
        title.id.as_str(),
        source_video,
        "movie",
        "rejected",
        Some(code),
        None,
        &[],
    )
    .await?;
    let result = ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: release_evidence.release_title(Some(source_video)),
        source_path: path_to_stored_string(source_video),
        dest_path: None,
        quality,
        episode_ids: Vec::new(),
        file_size_bytes: Some(source_size),
        link_type: None,
        error_message: Some(message),
        release_burned: false,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
    app.update_import_status_and_notify(import_id, status, result_json)
        .await?;
    Ok(result)
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
    release_evidence: &ReleaseEvidence,
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
    let source_title = release_evidence.release_title(Some(source_video));
    let full_folder_path =
        effective_title_folder_path(media_root, title, folder_template, parsed.year);
    ensure_import_title_folder_available(app, title, &full_folder_path).await?;
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
        let artifact_result = if code.is_duplicate_file() {
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
            Some(code.as_str()),
            None,
            linked_episode_artifacts,
        )
        .await?;
        let skip_reason =
            Some(skip_reason_for_import_check_rejection(app, code, &dest_path).await?);
        let result = ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
            skip_reason,
            title_id: Some(title.id.clone()),
            source_system: Some(completed.client_type.clone()),
            source_ref: Some(completed.download_client_item_id.clone()),
            source_title: release_evidence.release_title(Some(source_video)),
            source_path: path_to_stored_string(source_video),
            dest_path: Some(path_to_stored_string(&dest_path)),
            quality: parsed.quality.clone(),
            episode_ids: linked_episode_ids.clone(),
            file_size_bytes: Some(source_size),
            link_type: None,
            error_message: Some(reason),
            release_burned: false,
            started_at,
            completed_at: Utc::now(),
        };
        let result_json = serde_json::to_string(&result).ok();
        let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
        app.update_import_status_and_notify(import_id, status, result_json)
            .await?;
        return Ok(result);
    }

    let import_mode = crate::seeding_gate::resolve_seeding_safe_import_mode(
        app,
        Some(&title.library_id),
        &title.facet,
        Some(completed),
    )
    .await?;
    persist_title_folder_path_if_missing(app, title, &full_folder_path).await?;
    let destination_ownership =
        series_movie_context
            .as_ref()
            .map_or_else(ImportDestinationOwnership::title, |context| {
                ImportDestinationOwnership::series_movie(
                    context.series_movie_link_id,
                    context.linked_episode_id,
                )
            });
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
        Some(completed),
    )
    .await?;

    let media_file_input = crate::InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: path_to_stored_string(&dest_path),
        size_bytes: file_result.size_bytes as i64,
        announced_size_bytes: crate::canonical_scoring::persisted_announced_size_bytes(
            file_result.size_bytes as i64,
            release_evidence.announced_size_bytes(),
        ),
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
        grabbed_release_title: source_title.clone(),
        grabbed_at: Some(started_at.to_rfc3339()),
        edition: parsed.edition.clone(),
        ..Default::default()
    };
    let imported_media_file_id = file_result
        .insert_or_reuse_media_file(app, &media_file_input)
        .await?
        .media_file_id;
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
    .await?;

    let link_type =
        finalize_import_source_cleanup(app, import_mode, &file_result, &dest_path, Some(completed))
            .await?;

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Imported,
        skip_reason: None,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: source_title.clone(),
        source_path: path_to_stored_string(source_video),
        dest_path: Some(path_to_stored_string(&dest_path)),
        quality: parsed.quality.clone(),
        episode_ids: linked_episode_ids.clone(),
        file_size_bytes: Some(file_result.size_bytes as i64),
        link_type: Some(link_type),
        error_message: None,
        release_burned: false,
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
                source_title,
                source_path: Some(path_to_stored_string(source_video)),
                dest_path: Some(path_to_stored_string(&dest_path)),
                quality: parsed.quality.clone(),
                episode_ids: linked_episode_ids,
                // Single-file import, so the file's size is also the total.
                size_bytes: Some(file_result.size_bytes as i64),
            }),
        ))
        .await;

    Ok(result)
}

#[expect(
    clippy::too_many_arguments,
    reason = "movie import keeps operational completion and release evidence as separate inputs"
)]
async fn import_movie_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
    video_files: &[ImportVideoFile],
    started_at: chrono::DateTime<Utc>,
    runtime_sample_mode: crate::post_download_gate::RuntimeSampleValidationMode,
) -> AppResult<ImportResult> {
    let largest = pick_largest_import_video_file(video_files)?;
    // Only release parsing reads the recovered name; every path below is the
    // physical file.
    let parse_video = largest.parse_path().into_owned();
    let source_video = largest.physical;
    let source_title = release_evidence.release_title(Some(&parse_video));
    let source_size = std::fs::metadata(&source_video)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        ..
    } = resolve_import_paths(app, title).await?;

    let parsed =
        build_augmented_movie_import_metadata_for_title(&parse_video, release_evidence, title);
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
    let import_purpose = release_evidence.purpose();
    let origin = release_evidence.import_origin();
    if import_purpose.is_additional_file() {
        return import_additional_movie_download(
            app,
            actor,
            title,
            import_id,
            completed,
            release_evidence,
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
    let manual_replacement = operator_initiated_import(runtime_sample_mode);
    let existing_files = existing_files
        .into_iter()
        .filter(|file| file.role.is_primary())
        .collect::<Vec<_>>();
    let quality_profile = resolve_import_quality_profile(app, title).await?;
    let existing_score = existing_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        .and_then(|file| file.acquisition_score);
    let runtime_sample_validation = manual_aware_runtime_sample_validation(
        title
            .runtime_minutes
            .filter(|runtime_minutes| *runtime_minutes > 0)
            .map(|runtime_minutes| runtime_minutes.saturating_mul(60)),
        manual_replacement,
    );
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
        runtime_sample_validation,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(rejection) => {
            // A band miss is held for the operator, not burned: expected
            // runtimes are estimates and legitimate outliers (extended cuts,
            // double-length specials) must stay grabbable after review.
            if rejection.recycle_reason == crate::post_download_gate::RUNTIME_OUT_OF_BAND_CODE {
                return hold_replacement_for_manual_resolution(
                    app,
                    title,
                    import_id,
                    completed,
                    release_evidence,
                    &source_video,
                    source_size,
                    parsed.quality.clone(),
                    crate::post_download_gate::RUNTIME_OUT_OF_BAND_CODE,
                    rejection.message.clone(),
                    started_at,
                )
                .await;
            }
            if origin == crate::import_decide::ImportOrigin::OperatorQueued {
                return hold_replacement_for_manual_resolution(
                    app,
                    title,
                    import_id,
                    completed,
                    release_evidence,
                    &source_video,
                    source_size,
                    parsed.quality.clone(),
                    rejection.recycle_reason,
                    format!(
                        "held for manual import because the file failed {}: {}",
                        rejection.recycle_reason, rejection.message
                    ),
                    started_at,
                )
                .await;
            }
            crate::post_download_gate::reject_source_file_before_import(
                app,
                crate::domain_events::DomainEventActor::from(actor),
                title,
                source_title.as_deref().unwrap_or(""),
                &source_video,
                crate::post_download_gate::BlocklistAttribution::default(),
                None,
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
            .await?;
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Rejected,
                skip_reason: rejection.skip_reason.clone(),
                title_id: Some(title.id.clone()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: source_title.clone(),
                source_path: path_to_stored_string(&source_video),
                dest_path: None,
                quality: parsed.quality.clone(),
                episode_ids: Vec::new(),
                file_size_bytes: Some(source_size),
                link_type: None,
                error_message: Some(rejection.message),
                release_burned: true,
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
    ensure_import_title_folder_available(app, title, &full_folder_path).await?;

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
        let artifact_result = if code.is_duplicate_file() {
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
            Some(code.as_str()),
            None,
            &[],
        )
        .await?;
        let skip_reason =
            Some(skip_reason_for_import_check_rejection(app, code, &dest_path).await?);
        let result = ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
            skip_reason,
            title_id: Some(title.id.clone()),
            source_system: Some(completed.client_type.clone()),
            source_ref: Some(completed.download_client_item_id.clone()),
            source_title: source_title.clone(),
            source_path: path_to_stored_string(&source_video),
            dest_path: Some(path_to_stored_string(&dest_path)),
            quality: prepared.parsed.quality.clone(),
            episode_ids: Vec::new(),
            file_size_bytes: Some(source_size),
            link_type: None,
            error_message: Some(reason),
            release_burned: false,
            started_at,
            completed_at: Utc::now(),
        };
        let result_json = serde_json::to_string(&result).ok();
        let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
        app.update_import_status_and_notify(import_id, status, result_json)
            .await?;
        return Ok(result);
    }

    // Replace guard: with no catalog runtime the band could not run at the gate,
    // so an automatic overwrite is measured against the incumbent file instead.
    if let Some(replaced_file) = existing_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        && let Some(message) = crate::post_download_gate::replace_runtime_band_block(
            runtime_sample_validation,
            prepared.accepted.as_ref(),
            crate::post_download_gate::incumbent_replace_runtime_seconds([
                replaced_file.duration_seconds
            ]),
        )
    {
        return hold_replacement_for_manual_resolution(
            app,
            title,
            import_id,
            completed,
            release_evidence,
            &source_video,
            source_size,
            prepared.parsed.quality.clone(),
            crate::post_download_gate::REPLACE_BLOCKED_RUNTIME_MISMATCH_CODE,
            message,
            started_at,
        )
        .await;
    }

    let import_mode = crate::seeding_gate::resolve_seeding_safe_import_mode(
        app,
        Some(&title.library_id),
        &title.facet,
        Some(completed),
    )
    .await?;

    // **The one import decision** (design §3): subject, landed score, truth
    // verdict and admission in one call, over the same incumbents and the same
    // comparator the grab path consulted. This used to be a bare
    // `new_score > old_score` against a stored `acquisition_score`, and — until
    // `decide_import` made it unrepresentable — a *refused* admission here fell
    // straight through to the first-import insert below, writing a second
    // primary file for the movie it had just refused.
    let scoring_context = app
        .resolve_canonical_scoring_context(title, &quality_profile)
        .await;
    let title_scope = crate::SubmissionScope::Title;
    let decision_input = crate::import_decide::ImportDecisionInput {
        title,
        scoring_context: &scoring_context,
        scope: &title_scope,
        // A movie is one member: its own runtime, and nothing to reinterpret.
        scope_size_basis: crate::quality_profile::CoverageSizeBasis::single(title.runtime_minutes),
        // The announced half of the evidence: the parse as it came off the
        // release name. `prepared.parsed` already carries the probe's findings,
        // so passing it would make both scoring passes identical.
        parsed: &parsed,
        accepted: prepared.accepted.as_ref(),
        prior_rescore_changes: &prepared.rescore_changes,
        landed_size_bytes: source_size,
        announced_size_bytes: release_evidence.announced_size_bytes(),
        is_filler: false,
        origin,
        operator_intent: manual_replacement,
        incumbent_rows: crate::import_decide::IncumbentRows::Title(&existing_files),
        scope_label: "this title",
    };
    let plan = match crate::import_decide::decide_import(app, &decision_input).await {
        crate::import_decide::ImportDecisionOutcome::Admit(plan) => plan,
        crate::import_decide::ImportDecisionOutcome::Reject {
            rejection,
            disposition,
        } => {
            return carry_out_import_rejection(
                app,
                actor,
                title,
                import_id,
                completed,
                ImportRejectionContext {
                    rejection,
                    disposition,
                    release_title: &prepared.parsed.raw_title,
                    source_title: source_title.as_deref(),
                    source_video: &source_video,
                    dest_path: &dest_path,
                    source_size,
                    quality: prepared.parsed.quality.clone(),
                    episode_ids: &[],
                    series_movie_link_id: None,
                    episode_artifacts: &[],
                },
                started_at,
            )
            .await;
        }
    };
    if let Some(directive) = plan.blocklist_after_import.as_ref() {
        tracing::info!(title_id = %title.id, code = directive.code, "{}", directive.reason);
        crate::post_download_gate::blocklist_release_for_title(
            app,
            title,
            &prepared.parsed.raw_title,
            Some(directive.reason.clone()),
        )
        .await;
    }
    let new_score = plan.score;

    if let crate::import_decide::SupersededIncumbents::Title(superseded) = &plan.superseded
        && let Some(existing_file) = superseded.first()
    {
        let old_score = plan.previous_best_score;
        let old_file_recycle_context =
            crate::upgrade::resolve_old_file_recycle_context(app, title, existing_file).await?;

        persist_title_folder_path_if_missing(app, title, &full_folder_path).await?;
        match crate::upgrade::execute_upgrade(
            app,
            actor,
            import_id,
            title,
            existing_file,
            &source_video,
            &dest_path,
            &prepared,
            plan.parsed.quality.as_deref(),
            new_score,
            old_score,
            plan.scoring_log.clone(),
            &[],
            Some(&media_root),
            Some(old_file_recycle_context.media_root.as_str()),
            &old_file_recycle_context.recycle_config,
            import_mode,
            release_evidence.announced_size_bytes(),
            Some(completed),
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
                .await?;
                crate::upgrade::finalize_upgrade_source_cleanup(app, &outcome, Some(completed))
                    .await?;
                let result = ImportResult {
                    import_id: import_id.to_string(),
                    decision: ImportDecision::Imported,
                    skip_reason: None,
                    title_id: Some(title.id.clone()),
                    source_system: Some(completed.client_type.clone()),
                    source_ref: Some(completed.download_client_item_id.clone()),
                    source_title: source_title.clone(),
                    source_path: path_to_stored_string(&source_video),
                    dest_path: Some(path_to_stored_string(&dest_path)),
                    quality: prepared.parsed.quality.clone(),
                    episode_ids: Vec::new(),
                    file_size_bytes: Some(source_size),
                    link_type: (import_mode == scryer_domain::ImportMode::Move)
                        .then_some(scryer_domain::ImportStrategy::Move),
                    error_message: None,
                    release_burned: false,
                    started_at,
                    completed_at: Utc::now(),
                };
                tracing::info!(
                    title = %title.name,
                    old_score = outcome.old_score,
                    new_score = outcome.new_score,
                    "movie file upgraded"
                );
                mark_wanted_completed(app, &title.id, None, true).await;
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
                // The transfer itself failed a safety check; nothing was judged
                // about the release, so it is held rather than burned.
                return carry_out_import_rejection(
                    app,
                    actor,
                    title,
                    import_id,
                    completed,
                    ImportRejectionContext {
                        rejection,
                        disposition: crate::import_decide::RejectionDisposition::Hold,
                        release_title: &prepared.parsed.raw_title,
                        source_title: source_title.as_deref(),
                        source_video: &source_video,
                        dest_path: &dest_path,
                        source_size,
                        quality: prepared.parsed.quality.clone(),
                        episode_ids: &[],
                        series_movie_link_id: None,
                        episode_artifacts: &[],
                    },
                    started_at,
                )
                .await;
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

    persist_title_folder_path_if_missing(app, title, &full_folder_path).await?;
    let destination_ownership = ImportDestinationOwnership::title();
    let file_result = import_file_with_record_progress(
        app,
        import_id,
        &title.library_id,
        &title.facet,
        &destination_ownership,
        &source_video,
        &dest_path,
        import_mode,
        Some(&prepared.source_snapshot),
        Some(completed),
    )
    .await?;

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
            release_evidence.announced_size_bytes(),
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
        original_file_path: Some(path_to_stored_string(source_video.clone())),
        grabbed_release_title: source_title.clone(),
        grabbed_at: Some(started_at.to_rfc3339()),
        acquisition_score: Some(acq_score),
        scoring_log: post_download_score.scoring_log.clone(),
        ..Default::default()
    };
    let imported_media_file_id = match file_result
        .insert_or_reuse_media_file(app, &media_file_input)
        .await
    {
        Ok(persistence) => {
            let file_id = persistence.media_file_id;
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
    .await?;

    let link_type =
        finalize_import_source_cleanup(app, import_mode, &file_result, &dest_path, Some(completed))
            .await?;

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

    mark_wanted_completed(app, &title.id, None, true).await;

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Imported,
        skip_reason: None,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: source_title.clone(),
        source_path: path_to_stored_string(&source_video),
        dest_path: Some(path_to_stored_string(&dest_path)),
        quality: prepared.parsed.quality.clone(),
        episode_ids: Vec::new(),
        file_size_bytes: Some(file_result.size_bytes as i64),
        link_type: Some(link_type),
        error_message: None,
        release_burned: false,
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
                source_title,
                source_path: Some(path_to_stored_string(&source_video)),
                dest_path: Some(path_to_stored_string(&dest_path)),
                quality: prepared.parsed.quality.clone(),
                episode_ids: Vec::new(),
                // Single-file import, so the file's size is also the total.
                size_bytes: Some(file_result.size_bytes as i64),
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
    release_evidence: &ReleaseEvidence,
    video_files: &[ImportVideoFile],
    started_at: chrono::DateTime<Utc>,
    series_movie_link_id: &str,
    runtime_sample_mode: crate::post_download_gate::RuntimeSampleValidationMode,
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
                release_burned: false,
                ..base_completed_import_result(import_id, completed, release_evidence, started_at)
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
                error_message: Some(format!(
                    "series movie link {series_movie_link_id} not found"
                )),
                release_burned: false,
                ..base_completed_import_result(import_id, completed, release_evidence, started_at)
            };
            let result_json = serde_json::to_string(&result).ok();
            let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
            app.update_import_status_and_notify(import_id, status, result_json)
                .await?;
            return Ok(result);
        }
    };
    let movie = &link.movie;

    let largest = pick_largest_import_video_file(video_files)?;
    // Only release parsing reads the recovered name; every path below is the
    // physical file.
    let parse_video = largest.parse_path().into_owned();
    let source_video = largest.physical;
    let source_title = release_evidence.release_title(Some(&parse_video));
    let source_size = std::fs::metadata(&source_video)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template,
        specials_folder_template,
    } = resolve_import_paths(app, title).await?;

    // A series movie is searched and grabbed under the movie's identity
    // (`series_movie_search_title` swaps in the movie's name, facet, year and
    // ids), so the import parse must use that same identity for parity.
    let search_title = crate::acquisition_release_search::series_movie_search_title(title, &link);
    let parsed = build_augmented_movie_import_metadata_for_title(
        &parse_video,
        release_evidence,
        &search_title,
    );

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

    let full_folder_path = effective_title_folder_path(&media_root, title, &folder_template, None);
    ensure_import_title_folder_available(app, title, &full_folder_path).await?;
    let use_season_folders = app.resolve_use_season_folders(title).await?;
    let dest_path = episodic_import_parent_path(
        title,
        use_season_folders,
        &full_folder_path,
        &season_folder_template,
        &specials_folder_template,
        0,
    )
    .join(&rendered_filename);

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
    // What actually occupies this link, wherever it lives. The path-scoped list
    // above only ever sees a file already sitting at the name this import would
    // write; every other incumbent — rename disabled, a changed template, a
    // container change — was invisible to the gate that has to displace it.
    let series_movie_link_files: Vec<_> = existing_files
        .iter()
        .filter(|file| {
            file.role.is_primary()
                && file
                    .series_movie_link_ids
                    .iter()
                    .any(|link_id| link_id == series_movie_link_id)
        })
        .cloned()
        .collect();
    let import_purpose = release_evidence.purpose();
    let origin = release_evidence.import_origin();
    if import_purpose.is_additional_file() {
        return import_additional_movie_download(
            app,
            actor,
            title,
            import_id,
            completed,
            release_evidence,
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
    let manual_replacement = operator_initiated_import(runtime_sample_mode);
    let quality_profile = resolve_import_quality_profile(app, title).await?;
    let existing_score = series_movie_link_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        .and_then(|file| file.acquisition_score);
    // No fallback to the owning series runtime: a 24-minute parent episode
    // expectation would put every normal-length linked film outside the band.
    // An unknown movie runtime means the band cannot run (permissive); the
    // incumbent replace guard still protects replacements.
    let runtime_sample_validation = manual_aware_runtime_sample_validation(
        movie
            .runtime_minutes
            .filter(|runtime_minutes| *runtime_minutes > 0)
            .map(|runtime_minutes| runtime_minutes.saturating_mul(60)),
        manual_replacement,
    );
    let prepared = match crate::post_download_gate::prepare_import_candidate(
        app,
        title,
        &parsed,
        &quality_profile,
        &source_video,
        source_size,
        !series_movie_link_files.is_empty(),
        existing_score,
        false,
        runtime_sample_validation,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(rejection) => {
            // A band miss is held for the operator, not burned: expected
            // runtimes are estimates and legitimate outliers (extended cuts,
            // double-length specials) must stay grabbable after review.
            if rejection.recycle_reason == crate::post_download_gate::RUNTIME_OUT_OF_BAND_CODE {
                return hold_replacement_for_manual_resolution(
                    app,
                    title,
                    import_id,
                    completed,
                    release_evidence,
                    &source_video,
                    source_size,
                    parsed.quality.clone(),
                    crate::post_download_gate::RUNTIME_OUT_OF_BAND_CODE,
                    rejection.message.clone(),
                    started_at,
                )
                .await;
            }
            if origin == crate::import_decide::ImportOrigin::OperatorQueued {
                return hold_replacement_for_manual_resolution(
                    app,
                    title,
                    import_id,
                    completed,
                    release_evidence,
                    &source_video,
                    source_size,
                    parsed.quality.clone(),
                    rejection.recycle_reason,
                    format!(
                        "held for manual import because the file failed {}: {}",
                        rejection.recycle_reason, rejection.message
                    ),
                    started_at,
                )
                .await;
            }
            crate::post_download_gate::reject_source_file_before_import(
                app,
                crate::domain_events::DomainEventActor::from(actor),
                title,
                source_title.as_deref().unwrap_or(""),
                &source_video,
                crate::post_download_gate::BlocklistAttribution {
                    series_movie_link_id: Some(series_movie_link_id),
                    ..Default::default()
                },
                None,
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
            .await?;
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Rejected,
                skip_reason: rejection.skip_reason.clone(),
                title_id: Some(title.id.clone()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: source_title.clone(),
                source_path: path_to_stored_string(&source_video),
                dest_path: Some(path_to_stored_string(&dest_path)),
                quality: parsed.quality.clone(),
                episode_ids: Vec::new(),
                file_size_bytes: Some(source_size),
                link_type: None,
                error_message: Some(rejection.message),
                release_burned: true,
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;
            return Ok(result);
        }
    };

    // Replace guard: with no catalog runtime the band could not run at the gate,
    // so an automatic overwrite is measured against the incumbent file instead.
    if let Some(replaced_file) = series_movie_link_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        && let Some(message) = crate::post_download_gate::replace_runtime_band_block(
            runtime_sample_validation,
            prepared.accepted.as_ref(),
            crate::post_download_gate::incumbent_replace_runtime_seconds([
                replaced_file.duration_seconds
            ]),
        )
    {
        return hold_replacement_for_manual_resolution(
            app,
            title,
            import_id,
            completed,
            release_evidence,
            &source_video,
            source_size,
            prepared.parsed.quality.clone(),
            crate::post_download_gate::REPLACE_BLOCKED_RUNTIME_MISMATCH_CODE,
            message,
            started_at,
        )
        .await;
    }

    // Upgrade check: if there's an existing file for this series movie, score and compare.
    let import_mode = crate::seeding_gate::resolve_seeding_safe_import_mode(
        app,
        Some(&title.library_id),
        &title.facet,
        Some(completed),
    )
    .await?;

    // **The one import decision** (design §3). A linked movie is a scope like
    // any other; hand-rolling its comparison is what let it disagree with the
    // grab that fetched the file.
    let scoring_context = app
        .resolve_canonical_scoring_context(title, &quality_profile)
        .await;
    let link_scope = crate::SubmissionScope::SeriesMovie {
        series_movie_link_id: series_movie_link_id.to_string(),
    };
    let decision_input = crate::import_decide::ImportDecisionInput {
        title,
        scoring_context: &scoring_context,
        scope: &link_scope,
        // The linked movie is one member; see the title path above.
        scope_size_basis: crate::quality_profile::CoverageSizeBasis::single(movie.runtime_minutes),
        // The announced half of the evidence; see the title path above.
        parsed: &parsed,
        accepted: prepared.accepted.as_ref(),
        prior_rescore_changes: &prepared.rescore_changes,
        landed_size_bytes: source_size,
        announced_size_bytes: release_evidence.announced_size_bytes(),
        is_filler: false,
        origin,
        operator_intent: manual_replacement,
        // Every primary file of the title, not the handful that happen to sit at
        // this import's destination path: the subject holds *every* file linked
        // to this series-movie link, wherever it lives, so a path filter here
        // used to leave the lookup empty and panic (D14/A1).
        incumbent_rows: crate::import_decide::IncumbentRows::Title(&existing_files),
        scope_label: "this series-movie link",
    };
    let plan = match crate::import_decide::decide_import(app, &decision_input).await {
        crate::import_decide::ImportDecisionOutcome::Admit(plan) => plan,
        crate::import_decide::ImportDecisionOutcome::Reject {
            rejection,
            disposition,
        } => {
            return carry_out_import_rejection(
                app,
                actor,
                title,
                import_id,
                completed,
                ImportRejectionContext {
                    rejection,
                    disposition,
                    release_title: &prepared.parsed.raw_title,
                    source_title: source_title.as_deref(),
                    source_video: &source_video,
                    dest_path: &dest_path,
                    source_size,
                    quality: prepared.parsed.quality.clone(),
                    episode_ids: &linked_episode_ids,
                    series_movie_link_id: Some(series_movie_link_id),
                    episode_artifacts: &linked_episode_artifacts,
                },
                started_at,
            )
            .await;
        }
    };
    if let Some(directive) = plan.blocklist_after_import.as_ref() {
        tracing::info!(title_id = %title.id, code = directive.code, "{}", directive.reason);
        crate::post_download_gate::blocklist_release_for_title(
            app,
            title,
            &prepared.parsed.raw_title,
            Some(directive.reason.clone()),
        )
        .await;
    }
    let new_score = plan.score;

    if let crate::import_decide::SupersededIncumbents::Title(superseded) = &plan.superseded
        && let Some(existing_file) = superseded.first()
    {
        let old_score = plan.previous_best_score;
        {
            let old_file_recycle_context =
                crate::upgrade::resolve_old_file_recycle_context(app, title, existing_file).await?;

            persist_title_folder_path_if_missing(app, title, &full_folder_path).await?;
            match crate::upgrade::execute_upgrade(
                app,
                actor,
                import_id,
                title,
                existing_file,
                &source_video,
                &dest_path,
                &prepared,
                plan.parsed.quality.as_deref(),
                new_score,
                old_score,
                plan.scoring_log.clone(),
                &[],
                Some(&media_root),
                Some(old_file_recycle_context.media_root.as_str()),
                &old_file_recycle_context.recycle_config,
                import_mode,
                release_evidence.announced_size_bytes(),
                Some(completed),
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
                    .await?;
                    crate::upgrade::finalize_upgrade_source_cleanup(app, &outcome, Some(completed))
                        .await?;
                    tracing::info!(
                        title = %title.name,
                        movie = %movie.title,
                        old_score = outcome.old_score,
                        new_score = outcome.new_score,
                        "series movie file upgraded"
                    );
                    if let Some(linked_episode_id) = link.linked_episode_id.as_deref()
                        && let Err(error) = app
                            .services
                            .library
                            .media_files
                            .set_media_file_roles_for_episode(
                                &title.id,
                                linked_episode_id,
                                &outcome.new_file_id,
                                &[],
                            )
                            .await
                    {
                        tracing::warn!(
                            error = %error,
                            file_id = %outcome.new_file_id,
                            episode_id = %linked_episode_id,
                            series_movie_link_id = %series_movie_link_id,
                            "failed to promote upgraded series movie file for linked episode"
                        );
                    }
                    mark_wanted_completed_for_series_movie_link(
                        app,
                        &title.id,
                        series_movie_link_id,
                        true,
                    )
                    .await;
                    let result = ImportResult {
                        import_id: import_id.to_string(),
                        decision: ImportDecision::Imported,
                        skip_reason: None,
                        title_id: Some(title.id.clone()),
                        source_system: Some(completed.client_type.clone()),
                        source_ref: Some(completed.download_client_item_id.clone()),
                        source_title: source_title.clone(),
                        source_path: path_to_stored_string(&source_video),
                        dest_path: Some(path_to_stored_string(&dest_path)),
                        quality: prepared.parsed.quality.clone(),
                        episode_ids: Vec::new(),
                        file_size_bytes: Some(source_size),
                        link_type: (import_mode == scryer_domain::ImportMode::Move)
                            .then_some(scryer_domain::ImportStrategy::Move),
                        error_message: None,
                        release_burned: false,
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
                        "rejected",
                        rejection.skip_reason.as_ref().map(ImportSkipReason::as_str),
                        None,
                        &[],
                    )
                    .await?;
                    let result = ImportResult {
                        import_id: import_id.to_string(),
                        decision: ImportDecision::Rejected,
                        skip_reason: rejection.skip_reason.clone(),
                        title_id: Some(title.id.clone()),
                        source_system: Some(completed.client_type.clone()),
                        source_ref: Some(completed.download_client_item_id.clone()),
                        source_title: source_title.clone(),
                        source_path: path_to_stored_string(&source_video),
                        dest_path: Some(path_to_stored_string(&dest_path)),
                        quality: prepared.parsed.quality.clone(),
                        episode_ids: Vec::new(),
                        file_size_bytes: Some(source_size),
                        link_type: None,
                        error_message: Some(rejection.message),
                        release_burned: false,
                        started_at,
                        completed_at: Utc::now(),
                    };
                    let result_json = serde_json::to_string(&result).ok();
                    let status = completed_import_status_for_result(&result, ImportStatus::Skipped);
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
        }
    }

    persist_title_folder_path_if_missing(app, title, &full_folder_path).await?;
    // Ensure the configured episodic destination directory exists.
    if let Some(parent) = dest_path.parent()
        && let Err(err) = tokio::fs::create_dir_all(parent).await
    {
        tracing::warn!(error = %err, path = %parent.display(), "failed to create episodic import directory");
    }

    // Import file (hardlink or copy)
    let destination_ownership = ImportDestinationOwnership::series_movie(
        series_movie_link_id,
        link.linked_episode_id.as_deref(),
    );
    let file_result = import_file_with_record_progress(
        app,
        import_id,
        &title.library_id,
        &title.facet,
        &destination_ownership,
        &source_video,
        &dest_path,
        import_mode,
        Some(&prepared.source_snapshot),
        Some(completed),
    )
    .await?;

    // The persisted bar must be the score of the bytes that actually landed
    // (I7), and the transfer can change the size. Same context, same pipeline,
    // one number different — no second profile resolution.
    let post_download_score =
        crate::import_decide::rescore_landed_size(&decision_input, file_result.size_bytes as i64);
    let acq_score = post_download_score.score;

    let imported_media_file_id = match file_result
        .insert_or_reuse_media_file(
            app,
            &crate::InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: path_to_stored_string(&dest_path),
                size_bytes: file_result.size_bytes as i64,
                announced_size_bytes: crate::canonical_scoring::persisted_announced_size_bytes(
                    file_result.size_bytes as i64,
                    release_evidence.announced_size_bytes(),
                ),
                quality_label: post_download_score.parsed.quality.clone(),
                scene_name: Some(prepared.parsed.raw_title.clone()),
                release_group: post_download_score.parsed.release_group.clone(),
                source_type: crate::release_parser::parsed_release_source_type(
                    &post_download_score.parsed,
                ),
                resolution: post_download_score.parsed.quality.clone(),
                video_codec_parsed: post_download_score.parsed.video_codec,
                audio_codec_parsed: post_download_score
                    .parsed
                    .audio
                    .as_ref()
                    .map(ToString::to_string),
                audio_channels_parsed: post_download_score.parsed.audio_channels.clone(),
                original_file_path: Some(path_to_stored_string(&source_video)),
                grabbed_release_title: source_title.clone(),
                grabbed_at: Some(started_at.to_rfc3339()),
                acquisition_score: Some(acq_score),
                scoring_log: post_download_score.scoring_log.clone(),
                ..Default::default()
            },
        )
        .await
    {
        Ok(persistence) => {
            let file_id = persistence.media_file_id;
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
    .await?;

    let link_type =
        finalize_import_source_cleanup(app, import_mode, &file_result, &dest_path, Some(completed))
            .await?;

    if let Some(file_id) = imported_media_file_id.as_deref()
        && let Some(linked_episode_id) = link.linked_episode_id.as_deref()
        && let Err(err) = app
            .services
            .library
            .media_files
            .set_media_file_roles_for_episode(&title.id, linked_episode_id, file_id, &[])
            .await
    {
        tracing::warn!(
            error = %err,
            file_id = %file_id,
            episode_id = %linked_episode_id,
            series_movie_link_id = %series_movie_link_id,
            "failed to promote imported series movie file for linked episode"
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

    mark_wanted_completed_for_series_movie_link(app, &title.id, series_movie_link_id, true).await;

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
        source_title: source_title.clone(),
        source_path: path_to_stored_string(&source_video),
        dest_path: Some(path_to_stored_string(&dest_path)),
        quality: prepared.parsed.quality.clone(),
        episode_ids: linked_episode_ids.clone(),
        file_size_bytes: Some(file_result.size_bytes as i64),
        link_type: Some(link_type),
        error_message: None,
        release_burned: false,
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
            source_title,
            source_path: Some(path_to_stored_string(&source_video)),
            dest_path: Some(path_to_stored_string(&dest_path)),
            quality: prepared.parsed.quality.clone(),
            episode_ids: linked_episode_ids.clone(),
            // Single-file import, so the file's size is also the total.
            size_bytes: Some(file_result.size_bytes as i64),
        }),
    ))
    .await?;

    Ok(result)
}
async fn mark_wanted_completed_for_series_movie_link(
    app: &AppUseCase,
    title_id: &str,
    series_movie_link_id: &str,
    landed_import: bool,
) {
    match app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            statuses: vec!["wanted".into()],
            media_types: vec!["series_movie".into()],
            title_id: Some(title_id.to_string()),
            limit: 100,
            ..AcquisitionScopeStatesQuery::default()
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
                        .acquisition_scope_states
                        .transition_acquisition_scope_to_completed(
                            &AcquisitionScopeCompleteTransition {
                                id: item.id.clone(),
                                last_search_at: Some(now),
                                grabbed_release: if landed_import {
                                    None
                                } else {
                                    item.grabbed_release.clone()
                                },
                            },
                        )
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

#[cfg(test)]
mod archive_relocation_tests {
    #[cfg(unix)]
    #[test]
    fn cross_device_rename_error_is_detected() {
        let error = std::io::Error::from_raw_os_error(18);
        assert!(crate::fs_safety::is_cross_device_error(&error));
    }

    #[test]
    fn unrelated_rename_error_is_not_cross_device() {
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert!(!crate::fs_safety::is_cross_device_error(&error));
    }
}
