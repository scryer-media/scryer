use super::*;
use crate::domain_events::DomainEventActor;
use crate::library::movie_scan_scope::MovieScanScope;
use crate::library_filename_parser::{
    LibraryFilenameExistingRecord, LibraryFilenameFallbackPolicy, LibraryFilenameParseInput,
    LibraryFilenameParseMode, parse_library_filename,
};
use crate::library_scan_unmatched::build_title_bound_unmatched_scan_item;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};

fn hydration_source_for_scan_mode(
    mode: LibraryScanMode,
) -> crate::catalog_workflow::HydrationSource {
    match mode {
        LibraryScanMode::Full => crate::catalog_workflow::HydrationSource::LibraryScanFull,
        LibraryScanMode::Additive => crate::catalog_workflow::HydrationSource::LibraryScanAdditive,
    }
}

pub(super) async fn title_requires_scan_hydration(
    app: &AppUseCase,
    title: &Title,
    metadata_language: &str,
) -> AppResult<bool> {
    if !title
        .external_ids
        .iter()
        .any(|external_id| external_id.source.eq_ignore_ascii_case("tvdb"))
    {
        return Ok(false);
    }

    if title.metadata_fetched_at.is_none()
        || title.metadata_language.as_deref() != Some(metadata_language)
    {
        return Ok(true);
    }

    let Some(handler) = app.facet_registry.get(&title.facet) else {
        return Ok(false);
    };
    if !handler.has_episodes() {
        return Ok(false);
    }

    let episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await?;
    Ok(episodes.is_empty())
}

async fn discover_movie_title_files(
    app: &AppUseCase,
    title: &Title,
) -> AppResult<Vec<LibraryFile>> {
    let import_paths = crate::import_workflow::resolve_import_paths(app, title).await?;
    let media_root_path = PathBuf::from(&import_paths.media_root);
    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .unwrap_or_default();
    let mut discovered_files = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut candidate_paths = Vec::<PathBuf>::new();
    let mut seen_candidate_paths = HashSet::new();

    for collection in collections {
        let Some(ordered_path) = collection.ordered_path else {
            continue;
        };
        let ordered_path_buf = stored_path_to_path_buf(&ordered_path);
        if let Some(parent) = ordered_path_buf.parent()
            && parent != media_root_path.as_path()
            && seen_candidate_paths.insert(path_to_stored_string(parent))
        {
            candidate_paths.push(parent.to_path_buf());
        }
        if !seen_paths.insert(ordered_path.clone()) {
            continue;
        }

        match tokio::fs::metadata(&ordered_path_buf).await {
            Ok(metadata) if metadata.is_file() => {}
            Ok(metadata) if metadata.is_dir() => {
                if seen_candidate_paths.insert(ordered_path.clone()) {
                    candidate_paths.push(ordered_path_buf);
                }
                continue;
            }
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %ordered_path,
                    "failed to inspect tracked movie path during title scan discovery"
                );
                continue;
            }
        }

        discovered_files.push(LibraryFile {
            path: ordered_path.clone(),
            display_name: ordered_path_buf
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string(),
            nfo_path: matching_movie_nfo_path(&ordered_path_buf),
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        });
    }

    if !discovered_files.is_empty() {
        return Ok(discovered_files);
    }

    let default_candidate_path = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(stored_path_to_path_buf)
        .unwrap_or_else(|| {
            crate::effective_title_folder_path(
                &import_paths.media_root,
                title,
                &import_paths.folder_template,
                None,
            )
        });
    if default_candidate_path != media_root_path
        && seen_candidate_paths.insert(path_to_stored_string(&default_candidate_path))
    {
        candidate_paths.push(default_candidate_path);
    }

    for candidate_path in candidate_paths {
        match tokio::fs::metadata(&candidate_path).await {
            Ok(metadata) if metadata.is_dir() => {
                let files = app
                    .services
                    .library
                    .library_scanner
                    .scan_library(path_to_stored_string(&candidate_path).as_str())
                    .await?;
                for file in files {
                    if seen_paths.insert(file.path.clone()) {
                        discovered_files.push(file);
                    }
                }
                if !discovered_files.is_empty() {
                    return Ok(discovered_files);
                }
            }
            Ok(metadata) if metadata.is_file() => {
                return Ok(vec![LibraryFile {
                    path: path_to_stored_string(&candidate_path),
                    display_name: candidate_path
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    nfo_path: matching_movie_nfo_path(&candidate_path),
                    size_bytes: None,
                    source_signature_scheme: None,
                    source_signature_value: None,
                }]);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => warn!(
                error = %error,
                title_id = %title.id,
                path = %candidate_path.display(),
                "failed to inspect movie scan candidate path"
            ),
        }
    }

    Ok(Vec::new())
}

async fn tracked_movie_path_confirmed_missing(path: &Path) -> bool {
    match tokio::fs::metadata(path).await {
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let Some(parent) = path.parent() else {
                return false;
            };
            tokio::fs::metadata(parent)
                .await
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
        }
        Err(error) => {
            warn!(
                error = %error,
                path = %path.display(),
                "failed to inspect tracked movie path during stale cleanup"
            );
            false
        }
    }
}

fn title_external_id<'a>(title: &'a Title, source: &str) -> Option<&'a str> {
    if source == "imdb"
        && let Some(imdb_id) = title.imdb_id.as_deref()
        && !imdb_id.trim().is_empty()
    {
        return Some(imdb_id.trim());
    }

    title
        .external_ids
        .iter()
        .find(|external_id| {
            external_id.source.eq_ignore_ascii_case(source) && !external_id.value.trim().is_empty()
        })
        .map(|external_id| external_id.value.trim())
}

fn media_analysis_from_title_media_file(file: &TitleMediaFile) -> MediaFileAnalysis {
    MediaFileAnalysis {
        video_codec: file.video_codec,
        video_width: file.video_width,
        video_height: file.video_height,
        video_bitrate_kbps: file.video_bitrate_kbps,
        video_bit_depth: file.video_bit_depth,
        video_hdr_format: file.video_hdr_format.clone(),
        video_frame_rate: file.video_frame_rate.clone(),
        video_profile: file.video_profile.clone(),
        audio_codec: file.audio_codec.clone(),
        audio_profile: file.audio_profile.clone(),
        audio_channels: file.audio_channels,
        audio_bitrate_kbps: file.audio_bitrate_kbps,
        audio_languages: file.audio_languages.clone(),
        audio_streams: file.audio_streams.clone(),
        subtitle_languages: file.subtitle_languages.clone(),
        subtitle_codecs: file.subtitle_codecs.clone(),
        subtitle_streams: file.subtitle_streams.clone(),
        has_multiaudio: file.has_multiaudio,
        duration_seconds: file.duration_seconds,
        num_chapters: file.num_chapters,
        container_format: file.container_format.clone(),
    }
}

fn audio_channels_label(channels: i32) -> String {
    match channels {
        8 => "7.1".to_string(),
        7 | 6 => "5.1".to_string(),
        3 | 2 => "2.0".to_string(),
        1 => "1.0".to_string(),
        value => value.to_string(),
    }
}

fn parsed_release_for_movie_media_file(file: &TitleMediaFile) -> crate::ParsedReleaseMetadata {
    let file_path = stored_path_to_path_buf(&file.file_path);
    let fallback_name = file_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let raw_title = file
        .grabbed_release_title
        .as_deref()
        .or(file.scene_name.as_deref())
        .unwrap_or(fallback_name);
    let mut parsed = parse_release_metadata(raw_title);

    if let Some(quality) = file
        .quality_label
        .as_ref()
        .or(file.resolution.as_ref())
        .filter(|value| !value.trim().is_empty())
    {
        parsed.quality = Some(quality.clone());
    }
    if let Some(codec) = file.video_codec_parsed {
        parsed.video_codec = Some(codec);
    }
    if let Some(codec) = file
        .audio_codec_parsed
        .as_deref()
        .or(file.audio_codec.as_deref())
        .and_then(crate::release_parser::AudioCodec::parse)
    {
        parsed.audio = Some(codec);
    }
    if let Some(channels) = file
        .audio_channels_parsed
        .clone()
        .or_else(|| file.audio_channels.map(audio_channels_label))
        .filter(|value| !value.trim().is_empty())
    {
        parsed.audio_channels = Some(channels);
    }

    let acceptance = crate::post_download_gate::ImportedFileAcceptance {
        analysis: Some(media_analysis_from_title_media_file(file)),
        scan_error: None,
        rule_file_doc: None,
    };
    crate::post_download_gate::rescore_from_mediainfo(&parsed, &acceptance).0
}

fn score_movie_media_file_for_primary(
    title: &Title,
    profile: &crate::QualityProfile,
    required_audio_languages: &[String],
    persona: &crate::ScoringPersona,
    category: &str,
    file: &TitleMediaFile,
) -> i32 {
    let parsed = parsed_release_for_movie_media_file(file);
    crate::post_download_gate::build_import_profile_decision(
        profile,
        required_audio_languages,
        persona,
        &parsed,
        category,
        title.runtime_minutes,
        Some(file.size_bytes),
        false,
    )
    .preference_score
}

async fn normalize_movie_file_roles_after_scan(
    app: &AppUseCase,
    title: &Title,
    movie_scope: &MovieScanScope,
    newly_imported_file_count: usize,
    allow_existing_additional_role_promotion: bool,
) -> bool {
    let mut media_files = match app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
    {
        Ok(files) => files
            .into_iter()
            .filter(|file| movie_scope.file_is_in_scan_scope(&file.file_path))
            .collect::<Vec<_>>(),
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to list movie media files for scan role normalization"
            );
            return false;
        }
    };
    if media_files.is_empty() {
        return false;
    }
    media_files.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.id.cmp(&right.id))
    });

    let primary_files = media_files
        .iter()
        .filter(|file| file.role.is_primary())
        .collect::<Vec<_>>();
    let should_rank_primary = newly_imported_file_count == media_files.len()
        || (primary_files.is_empty() && allow_existing_additional_role_promotion);
    if primary_files.is_empty() && !should_rank_primary {
        return false;
    }
    let selected_primary_id = if should_rank_primary {
        let category = crate::post_download_gate::facet_to_category_hint(&title.facet);
        let profile_lookup = crate::catalog::discovery::QualityProfileLookup {
            title_tags: &title.tags,
            library_id: Some(title.library_id.as_str()),
            imdb_id: title_external_id(title, "imdb"),
            tvdb_id: title_external_id(title, "tvdb"),
            category_hint: Some(category),
        };
        let profile = match app.resolve_quality_profile(profile_lookup).await {
            Ok(profile) => profile,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    "failed to resolve quality profile for movie scan role selection"
                );
                crate::QualityProfile::default()
            }
        };
        let required_audio_languages = app
            .resolve_required_audio_languages(
                Some(&title.id),
                Some(&title.library_id),
                Some(category),
            )
            .await
            .unwrap_or_default();
        let persona = app
            .resolve_scoring_persona(Some(&title.library_id), Some(category))
            .await
            .unwrap_or_default();

        let mut ranked = Vec::with_capacity(media_files.len());
        for file in &media_files {
            let score = score_movie_media_file_for_primary(
                title,
                &profile,
                &required_audio_languages,
                &persona,
                category,
                file,
            );
            ranked.push((
                file.id.clone(),
                file.file_path.clone(),
                file.size_bytes,
                score,
            ));
        }
        ranked.sort_by(|left, right| {
            right
                .3
                .cmp(&left.3)
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.0.cmp(&right.0))
        });
        ranked[0].0.clone()
    } else if let [file] = primary_files.as_slice() {
        file.id.clone()
    } else {
        let mut primary_files = primary_files;
        primary_files.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.file_path.cmp(&right.file_path))
                .then_with(|| left.id.cmp(&right.id))
        });
        primary_files[0].id.clone()
    };

    let additional_file_ids = media_files
        .iter()
        .filter(|file| file.id != selected_primary_id)
        .map(|file| file.id.clone())
        .collect::<Vec<_>>();
    let needs_update = media_files.iter().any(|file| {
        if file.id == selected_primary_id {
            !file.role.is_primary()
        } else {
            !file.role.is_additional()
        }
    });
    if !needs_update {
        return false;
    }

    match app
        .services
        .library
        .media_files
        .set_media_file_roles_for_title(&title.id, &selected_primary_id, &additional_file_ids)
        .await
    {
        Ok(()) => true,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                primary_file_id = %selected_primary_id,
                "failed to normalize movie media file roles after scan"
            );
            false
        }
    }
}

fn episodic_media_file_coverage_key(file: &crate::EpisodeScopedMediaFile) -> Vec<String> {
    let mut episode_ids = file.episode_ids.clone();
    episode_ids.sort();
    episode_ids.dedup();
    episode_ids
}

fn select_primary_episodic_media_file(
    files: &[&crate::EpisodeScopedMediaFile],
    allow_existing_additional_role_promotion: bool,
) -> Option<String> {
    let primary_files = files
        .iter()
        .copied()
        .filter(|file| file.media_file.role.is_primary())
        .collect::<Vec<_>>();
    if let [file] = primary_files.as_slice() {
        return Some(file.media_file.id.clone());
    }

    let mut ranked = if primary_files.is_empty() {
        if !allow_existing_additional_role_promotion {
            return None;
        }
        files.to_vec()
    } else {
        primary_files
    };
    ranked.sort_by(|left, right| {
        right
            .media_file
            .acquisition_score
            .unwrap_or(0)
            .cmp(&left.media_file.acquisition_score.unwrap_or(0))
            .then_with(|| right.media_file.size_bytes.cmp(&left.media_file.size_bytes))
            .then_with(|| left.media_file.file_path.cmp(&right.media_file.file_path))
            .then_with(|| left.media_file.id.cmp(&right.media_file.id))
    });
    Some(ranked[0].media_file.id.clone())
}

async fn normalize_episodic_file_roles_after_scan(
    app: &AppUseCase,
    title: &Title,
    episode_ids: &HashSet<String>,
    allow_existing_additional_role_promotion: bool,
) -> bool {
    if episode_ids.is_empty() {
        return false;
    }

    let mut episode_ids = episode_ids.iter().cloned().collect::<Vec<_>>();
    episode_ids.sort();

    let scoped_files = match app
        .services
        .library
        .media_files
        .list_live_media_files_for_episode_ids(&title.id, &episode_ids)
        .await
    {
        Ok(files) => files,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to list episodic media files for scan role normalization"
            );
            return false;
        }
    };
    if scoped_files.is_empty() {
        return false;
    }

    let mut normalized_coverages = HashSet::new();
    let mut title_updated = false;
    for episode_id in episode_ids {
        let candidates = scoped_files
            .iter()
            .filter(|file| file.episode_ids.iter().any(|id| id == &episode_id))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            continue;
        }

        let coverage_key = episodic_media_file_coverage_key(candidates[0]);
        if candidates
            .iter()
            .any(|file| episodic_media_file_coverage_key(file) != coverage_key)
        {
            debug!(
                title_id = %title.id,
                episode_id = %episode_id,
                "skipping episodic media file role normalization for mixed episode coverage"
            );
            continue;
        }
        if !normalized_coverages.insert(coverage_key) {
            continue;
        }

        let Some(selected_primary_id) = select_primary_episodic_media_file(
            &candidates,
            allow_existing_additional_role_promotion,
        ) else {
            continue;
        };
        let additional_file_ids = candidates
            .iter()
            .filter(|file| file.media_file.id != selected_primary_id)
            .map(|file| file.media_file.id.clone())
            .collect::<Vec<_>>();
        let needs_update = candidates.iter().any(|file| {
            if file.media_file.id == selected_primary_id {
                !file.media_file.role.is_primary()
            } else {
                !file.media_file.role.is_additional()
            }
        });
        if !needs_update {
            continue;
        }

        match app
            .services
            .library
            .media_files
            .set_media_file_roles_for_title(&title.id, &selected_primary_id, &additional_file_ids)
            .await
        {
            Ok(()) => title_updated = true,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    episode_id = %episode_id,
                    primary_file_id = %selected_primary_id,
                    "failed to normalize episodic media file roles after scan"
                );
            }
        }
    }

    title_updated
}

async fn cleanup_missing_movie_title_records(
    app: &AppUseCase,
    title: &Title,
    cleanup: &LibraryScanMovieCleanupContext,
    movie_scope: &MovieScanScope,
) -> bool {
    let mut title_updated = false;

    let media_files = match app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
    {
        Ok(media_files) => media_files,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to list movie media files during stale cleanup"
            );
            Vec::new()
        }
    };

    for media_file in media_files {
        if movie_scope.file_is_outside_canonical_folder(&media_file.file_path) {
            if let Err(error) = app
                .delete_media_file_record_with_dependents(&media_file.id)
                .await
            {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    media_file_id = %media_file.id,
                    file_path = %media_file.file_path,
                    "failed to detach out-of-folder movie media file after title scan"
                );
            } else {
                title_updated = true;
            }
            continue;
        }

        let file_path = stored_path_to_path_buf(&media_file.file_path);
        if !tracked_movie_path_confirmed_missing(file_path.as_path()).await {
            continue;
        }
        if let Err(error) = app
            .delete_media_file_record_with_dependents(&media_file.id)
            .await
        {
            warn!(
                error = %error,
                title_id = %title.id,
                media_file_id = %media_file.id,
                file_path = %media_file.file_path,
                "failed to delete stale movie media file after title scan"
            );
        } else {
            title_updated = true;
        }
    }

    let collections = match app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
    {
        Ok(collections) => collections,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to list movie collections during stale cleanup"
            );
            Vec::new()
        }
    };
    let cleanup_ids = cleanup
        .stale_collection_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();

    for collection in collections {
        let outside_canonical_folder = collection
            .ordered_path
            .as_deref()
            .is_some_and(|path| movie_scope.file_is_outside_canonical_folder(path));
        let missing_by_path = if let Some(path) = collection.ordered_path.as_deref() {
            tracked_movie_path_confirmed_missing(Path::new(path)).await
        } else {
            false
        };
        if !outside_canonical_folder && !missing_by_path && !cleanup_ids.contains(&collection.id) {
            continue;
        }

        if let Err(error) = app
            .services
            .catalog
            .shows
            .delete_collection(&collection.id)
            .await
        {
            warn!(
                error = %error,
                collection_id = %collection.id,
                title_id = %title.id,
                "failed to delete stale movie collection after title scan"
            );
        } else {
            title_updated = true;
        }
    }

    title_updated
}

async fn hydrate_library_scan_workset(
    app: &AppUseCase,
    coordinator: &LibraryScanCoordinator,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    hydration_targets: Vec<crate::catalog_workflow::HydrationTarget>,
    track_metadata_progress: bool,
    cancel_token: Option<&CancellationToken>,
) -> AppResult<()> {
    for chunk in hydration_targets.chunks(crate::catalog_workflow::HYDRATION_BULK_BATCH_SIZE) {
        if library_scan_cancel_requested(cancel_token) {
            break;
        }
        let hydration_outcome = app
            .hydrate_titles_bulk_cancellable(chunk.to_vec(), cancel_token)
            .await?;

        for (title_id, hydrated) in hydration_outcome.hydrated_titles {
            if let Some(work) = workset.get_mut(&title_id) {
                work.title = hydrated;
            }
            if track_metadata_progress {
                coordinator.mark_metadata_completed(1).await;
            }
        }

        for (title_id, reason) in hydration_outcome.failed_titles {
            if let Some(work) = workset.remove(&title_id) {
                warn!(
                    title_id = %title_id,
                    reason = %reason,
                    "library scan title hydration failed"
                );
                if track_metadata_progress {
                    coordinator.mark_metadata_failed(1).await;
                }
                coordinator
                    .mark_file_failed(work.discovered_file_count())
                    .await;
            }
        }

        if track_metadata_progress {
            coordinator.publish_progress().await;
        }
    }

    Ok(())
}

impl AppUseCase {
    pub(crate) async fn execute_library_scan_workset(
        &self,
        actor: &User,
        session_id: &str,
        mut workset: HashMap<String, LibraryScanTitleWork>,
        cancel_token: Option<CancellationToken>,
    ) -> AppResult<LibraryScanSummary> {
        if library_scan_cancel_requested(cancel_token.as_ref()) {
            return Ok(LibraryScanSummary::default());
        }

        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        let metadata_language = self.metadata_language().await;
        let file_total = workset
            .values()
            .map(LibraryScanTitleWork::discovered_file_count)
            .sum::<usize>();
        coordinator.add_file_total(file_total).await;
        coordinator.mark_file_total_known().await;

        let hydration_source = self
            .runtime
            .library
            .library_scan_tracker
            .get_session(session_id)
            .await
            .map(|session| hydration_source_for_scan_mode(session.mode))
            .unwrap_or(crate::catalog_workflow::HydrationSource::LibraryScanFull);

        let mut hydration_targets = Vec::new();
        for work in workset.values() {
            let needs_hydration =
                title_requires_scan_hydration(self, &work.title, &metadata_language).await?;
            if needs_hydration {
                hydration_targets.push(crate::catalog_workflow::HydrationTarget {
                    title: work.title.clone(),
                    requested_tvdb_id: None,
                    sync_wanted_after_completion: false,
                    source: hydration_source,
                });
            }
        }

        let track_hydration_metadata_progress = self
            .runtime
            .library
            .library_scan_tracker
            .get_session(session_id)
            .await
            .is_none_or(|session| session.metadata_progress.total == 0);

        if track_hydration_metadata_progress {
            coordinator
                .add_metadata_total(hydration_targets.len())
                .await;
            coordinator.mark_metadata_total_known().await;
            coordinator.publish_progress().await;
        }
        if !hydration_targets.is_empty() {
            hydrate_library_scan_workset(
                self,
                &coordinator,
                &mut workset,
                hydration_targets,
                track_hydration_metadata_progress,
                cancel_token.as_ref(),
            )
            .await?;
        }

        self.run_library_scan_title_work_pool(actor, session_id, workset, cancel_token)
            .await
    }

    async fn run_library_scan_title_work_pool(
        &self,
        actor: &User,
        session_id: &str,
        workset: HashMap<String, LibraryScanTitleWork>,
        cancel_token: Option<CancellationToken>,
    ) -> AppResult<LibraryScanSummary> {
        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        let mut summary = LibraryScanSummary::default();
        let mut pending = workset.into_values();
        let mut work_set = tokio::task::JoinSet::new();

        for _ in 0..LIBRARY_SCAN_TITLE_WALK_CONCURRENCY {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
            let Some(work) = pending.next() else {
                break;
            };
            let app = self.clone();
            let actor = actor.clone();
            let session_id = session_id.to_string();
            let title_id = work.title.id.clone();
            let discovered_file_count = work.discovered_file_count();
            let absorb_walk_summary =
                matches!(work.facet_plan, LibraryScanTitleFacetPlan::Movie(_));
            let created_in_scan = work.created_in_scan;
            let walk_cancel_token = cancel_token.clone();
            work_set.spawn(async move {
                let result = app
                    .walk_library_title(
                        &actor,
                        LibraryScanTitleWalkRequest {
                            work,
                            session_id: Some(session_id),
                            cancel_token: walk_cancel_token,
                        },
                    )
                    .await;
                (
                    title_id,
                    discovered_file_count,
                    absorb_walk_summary,
                    created_in_scan,
                    result,
                )
            });
        }

        while let Some(result) = work_set.join_next().await {
            let (
                title_id,
                discovered_file_count,
                absorb_walk_summary,
                created_in_scan,
                walk_result,
            ) = result.map_err(|error| AppError::Repository(error.to_string()))?;

            match walk_result {
                Ok(walk_result) => {
                    if absorb_walk_summary {
                        let mut delta = walk_result.summary;
                        if created_in_scan {
                            delta.imported = delta.imported.saturating_sub(1);
                        }
                        summary.absorb(&delta);
                    }
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id = %title_id,
                        "library scan title walk failed"
                    );
                    coordinator.mark_file_failed(discovered_file_count).await;
                    coordinator.publish_progress().await;
                }
            }

            if !library_scan_cancel_requested(cancel_token.as_ref())
                && let Some(work) = pending.next()
            {
                let app = self.clone();
                let actor = actor.clone();
                let session_id = session_id.to_string();
                let title_id = work.title.id.clone();
                let discovered_file_count = work.discovered_file_count();
                let absorb_walk_summary =
                    matches!(work.facet_plan, LibraryScanTitleFacetPlan::Movie(_));
                let created_in_scan = work.created_in_scan;
                let walk_cancel_token = cancel_token.clone();
                work_set.spawn(async move {
                    let result = app
                        .walk_library_title(
                            &actor,
                            LibraryScanTitleWalkRequest {
                                work,
                                session_id: Some(session_id),
                                cancel_token: walk_cancel_token,
                            },
                        )
                        .await;
                    (
                        title_id,
                        discovered_file_count,
                        absorb_walk_summary,
                        created_in_scan,
                        result,
                    )
                });
            }
        }

        Ok(summary)
    }

    pub async fn scan_title_library(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_management_permission(actor, &title.library_id)
            .await?;

        let facet_plan = match title.facet {
            MediaFacet::Movie => {
                LibraryScanTitleFacetPlan::Movie(LibraryScanMovieCleanupContext::default())
            }
            MediaFacet::Series | MediaFacet::Anime => LibraryScanTitleFacetPlan::Episodic,
        };
        let work = LibraryScanTitleWork {
            title,
            facet_plan,
            discovered_files: None,
            mode: LibraryScanTitleWalkMode::OneOff,
            created_in_scan: false,
        };

        let metadata_language = self.metadata_language().await;
        let mut request = LibraryScanTitleWalkRequest {
            work,
            session_id: None,
            cancel_token: None,
        };

        if title_requires_scan_hydration(self, &request.work.title, &metadata_language).await? {
            let mut hydration_outcome = self
                .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
                    title: request.work.title.clone(),
                    requested_tvdb_id: None,
                    sync_wanted_after_completion: false,
                    source: crate::catalog_workflow::HydrationSource::Interactive,
                }])
                .await?;
            request.work.title = hydration_outcome
                .hydrated_titles
                .remove(title_id)
                .ok_or_else(|| {
                    AppError::Repository(
                        hydration_outcome
                            .failed_titles
                            .remove(title_id)
                            .unwrap_or_else(|| {
                                "title metadata hydration failed before title scan".to_string()
                            }),
                    )
                })?;
        }

        Ok(self.walk_library_title(actor, request).await?.summary)
    }

    pub(crate) async fn scan_title_library_with_discovered_files(
        &self,
        actor: &User,
        title: Title,
        discovered_files: Vec<LibraryFile>,
    ) -> AppResult<LibraryScanSummary> {
        if let Err(error) = self
            .require_library_management_permission(actor, &title.library_id)
            .await
        {
            match error {
                AppError::Unauthorized(_) => {
                    self.require_library_permission(
                        actor,
                        &title.library_id,
                        scryer_domain::LibraryPermission::ManageTitles,
                    )
                    .await?;
                }
                error => return Err(error),
            }
        }

        let facet_plan = match title.facet {
            MediaFacet::Movie => {
                LibraryScanTitleFacetPlan::Movie(LibraryScanMovieCleanupContext::default())
            }
            MediaFacet::Series | MediaFacet::Anime => LibraryScanTitleFacetPlan::Episodic,
        };
        let mut request = LibraryScanTitleWalkRequest {
            work: LibraryScanTitleWork {
                title,
                facet_plan,
                discovered_files: Some(discovered_files),
                mode: LibraryScanTitleWalkMode::OneOff,
                created_in_scan: false,
            },
            session_id: None,
            cancel_token: None,
        };

        let metadata_language = self.metadata_language().await;
        if title_requires_scan_hydration(self, &request.work.title, &metadata_language).await? {
            let title_id = request.work.title.id.clone();
            let mut hydration_outcome = self
                .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
                    title: request.work.title.clone(),
                    requested_tvdb_id: None,
                    sync_wanted_after_completion: false,
                    source: crate::catalog_workflow::HydrationSource::Interactive,
                }])
                .await?;
            request.work.title = hydration_outcome
                .hydrated_titles
                .remove(&title_id)
                .ok_or_else(|| {
                    AppError::Repository(
                        hydration_outcome
                            .failed_titles
                            .remove(&title_id)
                            .unwrap_or_else(|| {
                                "title metadata hydration failed before title scan".to_string()
                            }),
                    )
                })?;
        }

        Ok(self.walk_library_title(actor, request).await?.summary)
    }

    pub(crate) async fn walk_library_title(
        &self,
        actor: &User,
        request: LibraryScanTitleWalkRequest,
    ) -> AppResult<LibraryTitleWalkResult> {
        let LibraryScanTitleWalkRequest {
            work,
            session_id,
            cancel_token,
        } = request;
        match work.facet_plan {
            LibraryScanTitleFacetPlan::Movie(cleanup) => {
                self.walk_movie_library_title(
                    work.title,
                    session_id.as_deref(),
                    work.discovered_files,
                    cleanup,
                    work.mode,
                    cancel_token,
                )
                .await
            }
            LibraryScanTitleFacetPlan::Episodic => {
                self.walk_episodic_library_title(
                    actor,
                    work.title,
                    session_id.as_deref(),
                    work.discovered_files,
                    work.mode,
                    cancel_token,
                )
                .await
            }
        }
    }

    async fn walk_movie_library_title(
        &self,
        title: Title,
        session_id: Option<&str>,
        pre_scanned_files: Option<Vec<LibraryFile>>,
        cleanup: LibraryScanMovieCleanupContext,
        mode: LibraryScanTitleWalkMode,
        cancel_token: Option<CancellationToken>,
    ) -> AppResult<LibraryTitleWalkResult> {
        let started_at = Instant::now();
        let session_coordinator =
            session_id.map(|value| LibraryScanCoordinator::new(self.clone(), value.to_string()));
        let mut summary = LibraryScanSummary::default();
        let discovered_files = match pre_scanned_files {
            Some(files) => files,
            None => {
                let files = discover_movie_title_files(self, &title).await?;
                if let Some(coordinator) = session_coordinator.as_ref() {
                    coordinator.add_file_total(files.len()).await;
                    coordinator.mark_file_total_known().await;
                }
                files
            }
        };
        let discovered_file_count = discovered_files.len();
        let movie_scope = MovieScanScope::from_scan_inputs(
            cleanup.canonical_folder_path.as_deref(),
            title.folder_path.as_deref(),
            cleanup.scan_folder_path.as_deref(),
            &discovered_files,
        );

        debug!(
            title_id = %title.id,
            title_name = %title.name,
            session_id = session_id.unwrap_or("none"),
            pre_scanned_file_count = discovered_file_count,
            "movie title scan stage: start"
        );

        for file in &discovered_files {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
            finalize_movie_scan_file(self, &title, file, &mut summary, cancel_token.as_ref()).await;
            if let Some(coordinator) = session_coordinator.as_ref() {
                coordinator.mark_file_completed(1).await;
            }
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
        }

        if !library_scan_cancel_requested(cancel_token.as_ref()) {
            let cleanup_updated =
                cleanup_missing_movie_title_records(self, &title, &cleanup, &movie_scope).await;
            let roles_updated = normalize_movie_file_roles_after_scan(
                self,
                &title,
                &movie_scope,
                summary.imported,
                mode.allows_existing_additional_role_promotion(),
            )
            .await;
            if cleanup_updated || roles_updated {
                self.emit_title_updated_activity(None, &title).await;
            }
        }

        info!(
            title_id = %title.id,
            title_name = %title.name,
            files = discovered_file_count,
            imported = summary.imported,
            skipped = summary.skipped,
            elapsed_ms = elapsed_ms_u64(started_at),
            "movie title scan completed"
        );
        if let Some(coordinator) = session_coordinator.as_ref() {
            coordinator.publish_progress().await;
        }

        Ok(LibraryTitleWalkResult { summary })
    }

    async fn walk_episodic_library_title(
        &self,
        actor: &User,
        title: Title,
        session_id: Option<&str>,
        pre_scanned_files: Option<Vec<LibraryFile>>,
        mode: LibraryScanTitleWalkMode,
        cancel_token: Option<CancellationToken>,
    ) -> AppResult<LibraryTitleWalkResult> {
        let started_at = Instant::now();
        let session_coordinator =
            session_id.map(|value| LibraryScanCoordinator::new(self.clone(), value.to_string()));
        let scoped_discovered_files = pre_scanned_files.is_some();
        let pre_scanned_file_count = pre_scanned_files.as_ref().map(Vec::len);
        let scan_mode = mode.as_file_finalize_mode();

        let handler = self.facet_registry.get(&title.facet).ok_or_else(|| {
            AppError::Validation("library scan is not supported for this facet".into())
        })?;
        if !handler.has_episodes() {
            return Err(AppError::Validation(
                "title library scan is only supported for episodic titles".into(),
            ));
        }

        let import_paths = crate::import_workflow::resolve_import_paths(self, &title).await?;
        let title_dir = crate::effective_title_folder_path(
            &import_paths.media_root,
            &title,
            &import_paths.folder_template,
            None,
        );
        let title_dir_str = path_to_stored_string(&title_dir);
        debug!(
            title_id = %title.id,
            title_name = %title.name,
            session_id = session_id.unwrap_or("none"),
            scan_mode = %scan_mode.as_str(),
            title_dir = %title_dir_str,
            pre_scanned_file_count,
            "title scan stage: start"
        );
        let mut walk_elapsed = Duration::ZERO;
        let mut stat_elapsed = Duration::ZERO;
        let mut analyze_elapsed = Duration::ZERO;
        let mut db_elapsed = Duration::ZERO;

        if !scoped_discovered_files && tokio::fs::metadata(&title_dir).await.is_err() {
            tokio::fs::create_dir_all(&title_dir).await.map_err(|err| {
                AppError::Repository(format!(
                    "failed to recreate title directory {}: {err}",
                    title_dir.display()
                ))
            })?;
        }

        let discovered_files = match pre_scanned_files {
            Some(files) => files,
            None => {
                let scan_result = scan_episodic_title_directory_for_progress_metrics(
                    self.services.library.library_scanner.clone(),
                    &title_dir,
                )
                .await?;
                walk_elapsed =
                    walk_elapsed.saturating_add(Duration::from_millis(scan_result.walk_ms));
                stat_elapsed =
                    stat_elapsed.saturating_add(Duration::from_millis(scan_result.stat_ms));
                if let Some(coordinator) = session_coordinator.as_ref() {
                    coordinator.add_file_total(scan_result.files.len()).await;
                    coordinator.mark_file_total_known().await;
                }
                scan_result.files
            }
        };
        let db_started = Instant::now();
        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        let series_movie_links = self
            .services
            .catalog
            .shows
            .list_series_movie_links_for_title(&title.id)
            .await
            .unwrap_or_default();
        let title_episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await
            .unwrap_or_default();
        db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
        debug!(
            title_id = %title.id,
            title_name = %title.name,
            discovered_files = discovered_files.len(),
            existing_files = existing_files.len(),
            collections = collections.len(),
            series_movie_links = series_movie_links.len(),
            title_episodes = title_episodes.len(),
            "title scan stage: db state loaded"
        );
        debug!(
            title_id = %title.id,
            title_name = %title.name,
            "title scan stage: episode context loaded"
        );

        let mut existing_records_by_path: HashMap<String, TitleMediaFile> = HashMap::new();
        let mut episode_links: HashSet<(String, String)> = HashSet::new();
        let mut role_normalization_episode_ids = HashSet::new();

        for file in &existing_files {
            existing_records_by_path
                .entry(file.file_path.clone())
                .or_insert_with(|| file.clone());
            if let Some(episode_id) = file.episode_id.as_ref() {
                episode_links.insert((file.id.clone(), episode_id.clone()));
            }
        }
        let mut remaining_existing_paths = existing_records_by_path
            .keys()
            .cloned()
            .collect::<HashSet<_>>();

        let mut summary = LibraryScanSummary::default();
        let mut layout_summary = TitleScanLayoutSummary::default();
        let mut seen_paths = HashSet::new();
        let analysis_limit = self.runtime.library.library_scan_analysis_limit.clone();
        let mut pending_progress = TitleScanProgressDelta::default();
        let mut unchanged_file_skips = 0usize;
        let mut analyzed_files = 0usize;
        let mut external_subtitle_cache =
            crate::subtitles::ExternalSubtitleDirectoryCache::default();
        let actor_event = DomainEventActor::from(actor);

        'file_chunks: for file_chunk in discovered_files.chunks(TITLE_SCAN_FILE_BATCH_SIZE) {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
            let files = file_chunk.to_vec();
            let mut planned_files = Vec::new();
            let mut title_updated_in_batch = false;

            for file in files {
                if library_scan_cancel_requested(cancel_token.as_ref()) {
                    break;
                }
                if !file.path.trim().is_empty() {
                    seen_paths.insert(file.path.clone());
                }
                remaining_existing_paths.remove(&file.path);
                summary.scanned += 1;

                let source_path = stored_path_to_path_buf(&file.path);
                let snapshot = if let Some(snapshot) = file_source_snapshot_from_library_file(&file)
                {
                    snapshot
                } else {
                    let stat_started = Instant::now();
                    let metadata = match tokio::fs::metadata(&source_path).await {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            stat_elapsed = stat_elapsed.saturating_add(stat_started.elapsed());
                            warn!(
                                error = %error,
                                title_id = %title.id,
                                file_path = %file.path,
                                "failed to read file metadata during title scan"
                            );
                            summary.skipped += 1;
                            pending_progress.absorb(TitleScanProgressDelta::completed(1));
                            flush_title_scan_progress_batch(
                                self,
                                session_id,
                                &mut pending_progress,
                            )
                            .await;
                            continue;
                        }
                    };
                    stat_elapsed = stat_elapsed.saturating_add(stat_started.elapsed());

                    FileSourceSnapshot {
                        size_bytes: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
                        signature: file_source_signature_from_metadata(&metadata),
                    }
                };

                let existing = existing_records_by_path.get(&file.path);
                let existing_snapshot_matches = existing
                    .is_some_and(|existing| title_media_file_matches_snapshot(existing, &snapshot));

                let filename_parse = parse_library_filename(&LibraryFilenameParseInput {
                    path: &source_path,
                    display_name: Some(file.display_name.as_str()),
                    library_root: None,
                    title: Some(&title),
                    facet: Some(&title.facet),
                    collections: &collections,
                    series_movie_links: &series_movie_links,
                    episodes: &title_episodes,
                    existing_record: existing.map(|existing| LibraryFilenameExistingRecord {
                        episode_id: existing.episode_id.as_deref(),
                        snapshot_matches: existing_snapshot_matches,
                    }),
                    mode: LibraryFilenameParseMode::TitleScan,
                    fallback_policy: if existing.is_none() {
                        LibraryFilenameFallbackPolicy::NeedReleaseMetadata
                    } else {
                        LibraryFilenameFallbackPolicy::WhenNeeded
                    },
                });
                let target_episodes = filename_parse.target_episodes();
                let series_movie_link_id = filename_parse
                    .target_series_movie_link_id()
                    .map(str::to_string);

                if target_episodes.is_empty() && series_movie_link_id.is_none() {
                    let reason = filename_parse.unmatched_reason().unwrap_or_else(|| {
                        if filename_parse.episode_identity.is_some() {
                            "episode_lookup_failed"
                        } else {
                            "episode_identity_missing"
                        }
                    });
                    debug!(
                        title_id = %title.id,
                        title_name = %title.name,
                        file_path = %file.path,
                        display_name = %file.display_name,
                        title_dir = %title_dir_str,
                        discovered_files = discovered_files.len(),
                        parsed_episode = ?filename_parse.episode_identity,
                        strategy = ?filename_parse.strategy,
                        release_fallback_used = filename_parse.release_fallback_used,
                        reason,
                        "title scan: episode target missing"
                    );
                    let unmatched_item = build_title_bound_unmatched_scan_item(
                        &title.facet,
                        &title.library_id,
                        &title.id,
                        session_id,
                        &title_dir_str,
                        &file.path,
                        &file.display_name,
                        &title.name,
                        title.year.map(|value| value as u32),
                        reason,
                    );
                    if let Err(error) =
                        persist_library_scan_unmatched_item(self, &unmatched_item).await
                    {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_path = %file.path,
                            "failed to persist unmatched title scan item"
                        );
                    }
                    summary.unmatched += 1;
                    pending_progress.absorb(TitleScanProgressDelta::completed(1));
                    flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;
                    continue;
                }

                summary.matched += 1;
                for episode in &target_episodes {
                    role_normalization_episode_ids.insert(episode.id.clone());
                }
                let layout_observation =
                    classify_title_scan_layout(&title_dir, &source_path, &target_episodes);
                layout_summary.observe(layout_observation);

                let record = if let Some(existing) = existing {
                    let desired_scheme = snapshot
                        .signature
                        .as_ref()
                        .map(|value| value.scheme.clone());
                    let desired_value =
                        snapshot.signature.as_ref().map(|value| value.value.clone());
                    PlannedTitleScanRecord::Existing {
                        file_id: existing.id.clone(),
                        should_skip_analysis: existing_snapshot_matches,
                        should_refresh_source_signature: existing.size_bytes != snapshot.size_bytes
                            || existing.source_signature_scheme != desired_scheme
                            || existing.source_signature_value != desired_value
                            || existing.scan_status != "scanned",
                    }
                } else {
                    PlannedTitleScanRecord::New
                };

                planned_files.push(PlannedTitleScanFile {
                    file,
                    parsed: filename_parse.parsed_release,
                    target_episodes,
                    series_movie_link_id,
                    snapshot,
                    record,
                });
            }

            planned_files.sort_by(|left, right| left.file.path.cmp(&right.file.path));
            debug!(
                title_id = %title.id,
                title_name = %title.name,
                chunk_files = planned_files.len(),
                "title scan stage: chunk planned"
            );
            debug!(
                title_id = %title.id,
                title_name = %title.name,
                "title scan stage: analysis phase begin"
            );

            let mut analysis_set = tokio::task::JoinSet::new();
            let mut pending_analysis_plans = std::collections::VecDeque::new();
            for plan in planned_files {
                if library_scan_cancel_requested(cancel_token.as_ref()) {
                    break;
                }
                let should_analyze = match &plan.record {
                    PlannedTitleScanRecord::Existing {
                        should_skip_analysis,
                        ..
                    } => !should_skip_analysis,
                    PlannedTitleScanRecord::New => true,
                };

                if !should_analyze {
                    unchanged_file_skips += 1;
                    let file_path = plan.file.path.clone();
                    let outcome = finalize_title_scan_file(
                        self,
                        &title,
                        plan,
                        None,
                        scan_mode.clone(),
                        &mut episode_links,
                        &mut summary,
                        &mut db_elapsed,
                        &mut external_subtitle_cache,
                    )
                    .await;
                    if outcome.progress.failed == 0
                        && let Err(error) = clear_library_scan_unmatched_item(
                            self,
                            &title.facet,
                            &title.library_id,
                            &file_path,
                        )
                        .await
                    {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_path = %file_path,
                            "failed to clear unmatched title scan item"
                        );
                    }
                    pending_progress.absorb(outcome.progress);
                    title_updated_in_batch |= outcome.title_updated;
                    flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;
                    continue;
                }

                analyzed_files += 1;
                pending_analysis_plans.push_back(plan);
            }
            debug!(
                title_id = %title.id,
                title_name = %title.name,
                pending_analysis = pending_analysis_plans.len(),
                "title scan stage: analysis tasks queued"
            );

            while !pending_analysis_plans.is_empty() || !analysis_set.is_empty() {
                while !library_scan_cancel_requested(cancel_token.as_ref())
                    && analysis_set.len() < GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY
                {
                    let Some(plan) = pending_analysis_plans.pop_front() else {
                        break;
                    };
                    let analyzer = self.services.library.media_analyzer.clone();
                    let analysis_limit = analysis_limit.clone();
                    let file_path = plan.file.path.clone();
                    analysis_set.spawn(async move {
                        tracing::debug!(file_path = %file_path, "title scan analysis task: start");
                        let _permit = analysis_limit
                            .acquire_owned()
                            .await
                            .map_err(|error| AppError::Repository(error.to_string()))?;
                        let analysis_started = Instant::now();
                        let outcome =
                            analyzer.analyze_file(stored_path_to_path_buf(&file_path)).await?;
                        tracing::debug!(file_path = %file_path, "title scan analysis task: complete");
                        Ok::<(PlannedTitleScanFile, MediaAnalysisOutcome, Duration), AppError>((
                            plan,
                            outcome,
                            analysis_started.elapsed(),
                        ))
                    });
                }

                if library_scan_cancel_requested(cancel_token.as_ref()) {
                    pending_analysis_plans.clear();
                    analysis_set.abort_all();
                    break;
                }

                let Some(result) =
                    await_cancellable(cancel_token.as_ref(), analysis_set.join_next())
                        .await
                        .flatten()
                else {
                    pending_analysis_plans.clear();
                    analysis_set.abort_all();
                    break;
                };

                let (plan, analysis_outcome, analysis_duration) =
                    result.map_err(|error| AppError::Repository(error.to_string()))??;
                analyze_elapsed = analyze_elapsed.saturating_add(analysis_duration);
                if library_scan_cancel_requested(cancel_token.as_ref()) {
                    continue;
                }
                let file_path = plan.file.path.clone();
                debug!(
                    title_id = %title.id,
                    title_name = %title.name,
                    file_path = %file_path,
                    "title scan stage: finalize file begin"
                );
                let outcome = finalize_title_scan_file(
                    self,
                    &title,
                    plan,
                    Some(analysis_outcome),
                    scan_mode.clone(),
                    &mut episode_links,
                    &mut summary,
                    &mut db_elapsed,
                    &mut external_subtitle_cache,
                )
                .await;
                if outcome.progress.failed == 0
                    && let Err(error) = clear_library_scan_unmatched_item(
                        self,
                        &title.facet,
                        &title.library_id,
                        &file_path,
                    )
                    .await
                {
                    warn!(
                        error = %error,
                        title_id = %title.id,
                        file_path = %file_path,
                        "failed to clear unmatched title scan item"
                    );
                }
                pending_progress.absorb(outcome.progress);
                title_updated_in_batch |= outcome.title_updated;
                flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;
                debug!(
                    title_id = %title.id,
                    title_name = %title.name,
                    file_path = %file_path,
                    "title scan stage: finalize file complete"
                );
            }

            if title_updated_in_batch {
                self.emit_title_updated_activity(actor_event.clone(), &title)
                    .await;
            }

            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break 'file_chunks;
            }
        }

        flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;

        if !library_scan_cancel_requested(cancel_token.as_ref()) {
            let mut title_updated_after_scan = false;

            if !scoped_discovered_files {
                reconcile_library_scan_unmatched_items(
                    self,
                    &title.facet,
                    &title_dir_str,
                    &seen_paths,
                )
                .await?;
                for stale_path in remaining_existing_paths {
                    let Some(record) = existing_records_by_path.get(&stale_path).cloned() else {
                        continue;
                    };
                    if !stale_path.starts_with(title_dir_str.as_str()) {
                        continue;
                    }
                    if stored_path_to_path_buf(&record.file_path).exists() {
                        continue;
                    }
                    let db_started = Instant::now();
                    let delete_result = self
                        .delete_media_file_record_with_dependents(&record.id)
                        .await;
                    db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                    if let Err(error) = delete_result {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_path = %record.file_path,
                            "failed to delete stale media file during title scan"
                        );
                    } else {
                        title_updated_after_scan = true;
                    }
                }

                if title.folder_path.as_deref() != Some(title_dir_str.as_str()) {
                    let db_started = Instant::now();
                    self.services
                        .catalog
                        .titles
                        .set_folder_path(&title.id, &title_dir_str)
                        .await?;
                    db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                    title_updated_after_scan = true;
                }

                if let Some(use_season_folders) = layout_summary.inferred_use_season_folders()
                    && crate::use_season_folders(&title) != use_season_folders
                {
                    let tags = merge_title_scan_option_tags(title.tags.clone(), use_season_folders);
                    let db_started = Instant::now();
                    self.apply_title_metadata_update(actor, &title.id, None, None, Some(tags))
                        .await?;
                    db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                    title_updated_after_scan = true;
                }
            }

            if normalize_episodic_file_roles_after_scan(
                self,
                &title,
                &role_normalization_episode_ids,
                mode.allows_existing_additional_role_promotion(),
            )
            .await
            {
                title_updated_after_scan = true;
            }

            if title_updated_after_scan {
                self.emit_title_updated_activity(actor_event.clone(), &title)
                    .await;
            }
        }

        debug!(
            title_id = %title.id,
            path = %title_dir.display(),
            scanned = summary.scanned,
            matched = summary.matched,
            imported = summary.imported,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            walk_ms = u64::try_from(walk_elapsed.as_millis()).unwrap_or(u64::MAX),
            stat_ms = u64::try_from(stat_elapsed.as_millis()).unwrap_or(u64::MAX),
            analyze_ms = u64::try_from(analyze_elapsed.as_millis()).unwrap_or(u64::MAX),
            db_ms = u64::try_from(db_elapsed.as_millis()).unwrap_or(u64::MAX),
            analyzed_files,
            unchanged_file_skips,
            batch_size = TITLE_SCAN_FILE_BATCH_SIZE,
            worker_concurrency = GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY,
            elapsed_ms = elapsed_ms_u64(started_at),
            "title library scan completed"
        );

        Ok(LibraryTitleWalkResult { summary })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LibraryScanTitleWalkRequest {
    pub(crate) work: LibraryScanTitleWork,
    pub(crate) session_id: Option<String>,
    pub(crate) cancel_token: Option<CancellationToken>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{Episode, ExternalId, MediaFacet, Title};
    use std::path::Path;

    fn numeric_series_title() -> Title {
        Title {
            id: "title-13".into(),
            name: "13".into(),
            facet: MediaFacet::Series,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            monitored: true,
            tags: vec![],
            external_ids: vec![ExternalId {
                source: "tvdb".into(),
                value: "131313".into(),
            }],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn numeric_series_episode(season: &str, episode: &str) -> Episode {
        Episode {
            id: format!("episode-{season}-{episode}"),
            title_id: "title-13".into(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some(episode.into()),
            season_number: Some(season.into()),
            episode_label: Some(format!("S{season:0>2}E{episode:0>2}")),
            title: Some(format!("Day {season} 800 A.M. 900 A.M.")),
            air_date: None,
            duration_seconds: None,
            image_url: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn parses_anonymized_numeric_series_season_two_filename_for_title_scan() {
        let title = numeric_series_title();
        let episodes = vec![
            numeric_series_episode("1", "1"),
            numeric_series_episode("2", "1"),
        ];
        let path = Path::new(
            "/library/13 (2024)/Season 02/13 (2024) - S02E01 - Day 2 800 A.M. 900 A.M. [WEBDL-1080p] [EAC3 5.1] [h265].mkv",
        );

        let parsed = parse_library_filename(&LibraryFilenameParseInput {
            path,
            display_name: Some("13 (2024) - S02E01"),
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::NeedReleaseMetadata,
        });
        let episode = parsed
            .parsed_release
            .episode
            .as_ref()
            .expect("episode metadata");

        assert_eq!(parsed.parsed_release.normalized_title, "13");
        assert_eq!(episode.season, Some(2));
        assert_eq!(episode.episode_numbers, vec![1]);
    }
}
