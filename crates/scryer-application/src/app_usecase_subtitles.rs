use std::path::Path;

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::subtitles::provider::{OpenSubtitlesProvider, SubtitleMediaKind};
use crate::subtitles::search::SubtitleSearchOrchestrator;
use crate::subtitles::sync;
use crate::subtitles::wanted::{SubtitleLanguagePref, compute_missing_subtitles_from_streams};
use crate::{AppResult, AppUseCase};
use scryer_domain::SubtitleDownload;

/// Background subtitle poller — searches for missing subtitles on a schedule.
pub async fn start_background_subtitle_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    // Check if subtitles are enabled
    let enabled = app
        .read_setting_string_value("subtitles.enabled", None)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true");

    if !enabled {
        debug!("subtitle poller disabled (subtitles.enabled != true)");
        return;
    }

    info!("background subtitle poller started");

    // Read search interval (default 6 hours)
    let interval_hours: u64 = app
        .read_setting_string_value("subtitles.search_interval_hours", None)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_hours * 3600));
    interval.tick().await; // consume first tick

    // Initial delay to let services fully initialize
    tokio::time::sleep(std::time::Duration::from_secs(120)).await;

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("subtitle poller shutting down");
                return;
            }
            _ = interval.tick() => {
                if let Err(err) = run_subtitle_search_cycle(&app).await {
                    warn!(error = %err, "subtitle search cycle failed");
                }
            }
        }
    }
}

/// Spawn a fire-and-forget subtitle search for a newly imported file.
/// Called from import code when `subtitles.auto_download_on_import` is true.
pub fn spawn_subtitle_search_for_file(app: AppUseCase, title_id: String, media_file_id: String) {
    tokio::spawn(async move {
        if let Err(err) = run_subtitle_search_for_file(&app, &title_id, &media_file_id).await {
            warn!(error = %err, title_id, media_file_id, "on-import subtitle search failed");
        }
    });
}

fn is_series_title(title: &scryer_domain::Title) -> bool {
    title.facet == scryer_domain::MediaFacet::Series
        || title.facet == scryer_domain::MediaFacet::Anime
}

fn subtitle_media_kind(title: &scryer_domain::Title) -> SubtitleMediaKind {
    if is_series_title(title) {
        SubtitleMediaKind::Episode
    } else {
        SubtitleMediaKind::Movie
    }
}

fn title_imdb_ids(title: &scryer_domain::Title) -> (Option<&str>, Option<&str>) {
    let imdb_id = title
        .external_ids
        .iter()
        .find(|external_id| external_id.source == "imdb")
        .map(|external_id| external_id.value.as_str());

    if is_series_title(title) {
        (None, imdb_id)
    } else {
        (imdb_id, None)
    }
}

#[derive(Debug, Clone, Copy)]
struct SubtitleSyncSettings {
    enabled: bool,
    threshold_series: i32,
    threshold_movie: i32,
    max_offset_seconds: i64,
}

impl SubtitleSyncSettings {
    fn threshold_for(self, media_kind: SubtitleMediaKind) -> i32 {
        match media_kind {
            SubtitleMediaKind::Episode => self.threshold_series,
            SubtitleMediaKind::Movie => self.threshold_movie,
        }
    }
}

async fn read_subtitle_sync_settings(app: &AppUseCase) -> AppResult<SubtitleSyncSettings> {
    Ok(SubtitleSyncSettings {
        enabled: app
            .read_setting_string_value("subtitles.sync_enabled", None)
            .await?
            .as_deref()
            != Some("false"),
        threshold_series: app
            .read_setting_string_value("subtitles.sync_threshold_series", None)
            .await?
            .and_then(|value| value.parse().ok())
            .unwrap_or(90),
        threshold_movie: app
            .read_setting_string_value("subtitles.sync_threshold_movie", None)
            .await?
            .and_then(|value| value.parse().ok())
            .unwrap_or(70),
        max_offset_seconds: app
            .read_setting_string_value("subtitles.sync_max_offset_seconds", None)
            .await?
            .and_then(|value| value.parse().ok())
            .unwrap_or(60),
    })
}

async fn maybe_sync_downloaded_subtitle(
    app: &AppUseCase,
    sync_settings: SubtitleSyncSettings,
    media_kind: SubtitleMediaKind,
    video_path: &Path,
    subtitle_path: &Path,
    download_id: Option<&str>,
    score: Option<i32>,
    forced: bool,
) -> Option<sync::SyncResult> {
    let policy = sync::SyncPolicy {
        enabled: sync_settings.enabled,
        forced,
        score,
        threshold: Some(sync_settings.threshold_for(media_kind)),
        max_offset_seconds: sync_settings.max_offset_seconds,
    };

    match sync::sync_subtitle_with_policy(video_path, subtitle_path, policy).await {
        Ok(result) => {
            if result.applied
                && let Some(id) = download_id
                && let Err(err) = app.services.subtitle_downloads.set_synced(id, true).await
            {
                warn!(error = %err, download_id = id, "failed to persist subtitle sync status");
            }

            if result.applied {
                info!(
                    offset_ms = result.offset_ms,
                    consistency_ratio = result.consistency_ratio,
                    nosplit_score = result.nosplit_score,
                    split_score = result.split_score,
                    path = %subtitle_path.display(),
                    "subtitle timing synced"
                );
            } else {
                debug!(
                    reason = ?result.skipped_reason,
                    score,
                    path = %subtitle_path.display(),
                    "subtitle sync skipped"
                );
            }

            Some(result)
        }
        Err(err) => {
            warn!(error = %err, path = %subtitle_path.display(), "subtitle sync failed (non-fatal)");
            None
        }
    }
}

fn sync_summary_suffix(result: Option<&sync::SyncResult>) -> String {
    result
        .map(|sync_result| format!(" [sync: {}]", sync_result.summary()))
        .unwrap_or_default()
}

async fn media_file_episode_context(
    app: &AppUseCase,
    media_file: &crate::TitleMediaFile,
) -> (Option<i32>, Option<i32>) {
    let Some(episode_id) = media_file.episode_id.as_deref() else {
        return (None, None);
    };

    match app.services.shows.get_episode_by_id(episode_id).await {
        Ok(Some(episode)) => (
            episode
                .season_number
                .as_deref()
                .and_then(|value| value.parse::<i32>().ok()),
            episode
                .episode_number
                .as_deref()
                .and_then(|value| value.parse::<i32>().ok()),
        ),
        _ => (None, None),
    }
}

fn embedded_subtitle_streams(
    media_file: &crate::TitleMediaFile,
) -> Vec<crate::SubtitleStreamDetail> {
    if !media_file.subtitle_streams.is_empty() {
        return media_file.subtitle_streams.clone();
    }

    media_file
        .subtitle_languages
        .iter()
        .map(|language| crate::SubtitleStreamDetail {
            codec: None,
            language: Some(language.clone()),
            name: None,
            forced: false,
            default: false,
        })
        .collect()
}

async fn run_subtitle_search_for_file(
    app: &AppUseCase,
    title_id: &str,
    media_file_id: &str,
) -> AppResult<()> {
    // Short delay to let the media file be fully committed
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let api_key = match app
        .read_setting_string_value("subtitles.opensubtitles_api_key", None)
        .await?
    {
        Some(key) if !key.is_empty() => key,
        _ => return Ok(()),
    };

    let languages_json = app
        .read_setting_string_value("subtitles.languages", None)
        .await?
        .unwrap_or_else(|| "[]".to_string());
    let wanted_languages: Vec<SubtitleLanguagePref> =
        serde_json::from_str(&languages_json).unwrap_or_default();
    if wanted_languages.is_empty() {
        return Ok(());
    }

    let username = app
        .read_setting_string_value("subtitles.opensubtitles_username", None)
        .await?
        .unwrap_or_default();
    let password = app
        .read_setting_string_value("subtitles.opensubtitles_password", None)
        .await?
        .unwrap_or_default();
    let include_ai = app
        .read_setting_string_value("subtitles.include_ai_translated", None)
        .await?
        .as_deref()
        == Some("true");
    let include_machine = app
        .read_setting_string_value("subtitles.include_machine_translated", None)
        .await?
        .as_deref()
        == Some("true");

    let title = app
        .services
        .titles
        .get_by_id(title_id)
        .await?
        .ok_or_else(|| crate::AppError::NotFound("title not found".into()))?;
    let mf = app
        .services
        .media_files
        .get_media_file_by_id(media_file_id)
        .await?
        .ok_or_else(|| crate::AppError::NotFound("media file not found".into()))?;

    let is_series = is_series_title(&title);
    let min_score: i32 = if is_series {
        app.read_setting_string_value("subtitles.minimum_score_series", None)
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(240)
    } else {
        app.read_setting_string_value("subtitles.minimum_score_movie", None)
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(70)
    };
    let sync_settings = read_subtitle_sync_settings(app).await?;

    let provider = OpenSubtitlesProvider::new(api_key);
    if !username.is_empty() && !password.is_empty() {
        let _ = provider.login(&username, &password).await;
    }

    let existing = app
        .services
        .subtitle_downloads
        .list_for_media_file(&mf.id)
        .await
        .unwrap_or_default();
    let embedded = embedded_subtitle_streams(&mf);
    let missing = compute_missing_subtitles_from_streams(&wanted_languages, &existing, &embedded);

    let orchestrator = SubtitleSearchOrchestrator::new(min_score);
    let media_kind = subtitle_media_kind(&title);
    let (imdb_id, series_imdb_id) = title_imdb_ids(&title);
    let (season_num, episode_num) = media_file_episode_context(app, &mf).await;
    let file_path = Path::new(&mf.file_path);

    for lang_pref in &missing {
        let results = match orchestrator
            .search(
                &provider,
                file_path,
                media_kind,
                &title.name,
                &title.aliases,
                title.year,
                imdb_id,
                series_imdb_id,
                season_num,
                episode_num,
                std::slice::from_ref(&lang_pref.code),
                mf.release_group.as_deref(),
                mf.source_type.as_deref(),
                mf.video_codec_parsed.as_deref(),
                mf.audio_codec_parsed.as_deref(),
                mf.resolution.as_deref(),
                Some(lang_pref.hearing_impaired),
                include_ai,
                include_machine,
            )
            .await
        {
            Ok(r) => r,
            Err(err) => {
                warn!(error = %err, language = %lang_pref.code, "on-import subtitle search failed");
                continue;
            }
        };

        let mut filtered_results = Vec::new();
        for result in &results {
            let blacklisted = app
                .services
                .subtitle_downloads
                .is_blacklisted(&mf.id, &result.provider, &result.provider_file_id)
                .await
                .unwrap_or(false);
            if !blacklisted {
                filtered_results.push(result);
            }
        }

        let best = filtered_results
            .iter()
            .filter(|r| r.score >= min_score)
            .filter(|r| r.hearing_impaired == lang_pref.hearing_impaired)
            .filter(|r| r.forced == lang_pref.forced)
            .max_by_key(|r| r.score);

        let best = match best {
            Some(b) => b,
            None => continue,
        };

        match crate::subtitles::download::download_and_save(
            &provider,
            &best.provider_file_id,
            file_path,
            &best.language,
            best.forced,
            best.hearing_impaired,
        )
        .await
        {
            Ok((dest_path, _)) => {
                let record = SubtitleDownload {
                    id: scryer_domain::Id::new().0,
                    media_file_id: mf.id.clone(),
                    title_id: title.id.clone(),
                    episode_id: mf.episode_id.clone(),
                    language: best.language.clone(),
                    provider: best.provider.clone(),
                    provider_file_id: Some(best.provider_file_id.clone()),
                    file_path: dest_path.to_string_lossy().to_string(),
                    score: Some(best.score),
                    hearing_impaired: best.hearing_impaired,
                    forced: best.forced,
                    ai_translated: best.ai_translated,
                    machine_translated: best.machine_translated,
                    uploader: best.uploader.clone(),
                    release_info: best.release_info.clone(),
                    synced: false,
                    downloaded_at: Utc::now().to_rfc3339(),
                };
                let record_id = record.id.clone();
                let record_inserted = match app.services.subtitle_downloads.insert(&record).await {
                    Ok(()) => true,
                    Err(err) => {
                        warn!(error = %err, "failed to persist on-import subtitle download record");
                        false
                    }
                };
                let sync_result = maybe_sync_downloaded_subtitle(
                    app,
                    sync_settings,
                    media_kind,
                    file_path,
                    &dest_path,
                    record_inserted.then_some(record_id.as_str()),
                    Some(best.score),
                    best.forced,
                )
                .await;
                info!(
                    title = %title.name,
                    language = %lang_pref.code,
                    sync = sync_summary_suffix(sync_result.as_ref()),
                    "on-import subtitle downloaded"
                );
            }
            Err(err) => {
                warn!(error = %err, "on-import subtitle download failed");
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    }

    Ok(())
}

/// Run a single subtitle search cycle across all monitored titles.
async fn run_subtitle_search_cycle(app: &AppUseCase) -> AppResult<()> {
    // Read subtitle settings
    let api_key = match app
        .read_setting_string_value("subtitles.opensubtitles_api_key", None)
        .await?
    {
        Some(key) if !key.is_empty() => key,
        _ => {
            debug!("no OpenSubtitles API key configured, skipping subtitle search");
            return Ok(());
        }
    };

    let username = app
        .read_setting_string_value("subtitles.opensubtitles_username", None)
        .await?
        .unwrap_or_default();
    let password = app
        .read_setting_string_value("subtitles.opensubtitles_password", None)
        .await?
        .unwrap_or_default();

    let include_ai = app
        .read_setting_string_value("subtitles.include_ai_translated", None)
        .await?
        .as_deref()
        == Some("true");
    let include_machine = app
        .read_setting_string_value("subtitles.include_machine_translated", None)
        .await?
        .as_deref()
        == Some("true");

    let min_score_series: i32 = app
        .read_setting_string_value("subtitles.minimum_score_series", None)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(240);
    let min_score_movie: i32 = app
        .read_setting_string_value("subtitles.minimum_score_movie", None)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(70);

    // Parse wanted languages from settings
    let languages_json = app
        .read_setting_string_value("subtitles.languages", None)
        .await?
        .unwrap_or_else(|| "[]".to_string());
    let wanted_languages: Vec<SubtitleLanguagePref> =
        serde_json::from_str(&languages_json).unwrap_or_default();

    if wanted_languages.is_empty() {
        debug!("no subtitle languages configured, skipping");
        return Ok(());
    }

    let sync_settings = read_subtitle_sync_settings(app).await?;

    // Initialize provider
    let provider = OpenSubtitlesProvider::new(api_key);
    if !username.is_empty()
        && !password.is_empty()
        && let Err(err) = provider.login(&username, &password).await
    {
        warn!(error = %err, "OpenSubtitles login failed, continuing without auth");
    }

    // Get all monitored titles with media files
    let titles = app.services.titles.list(None, None).await?;
    let mut searched = 0u32;
    let mut downloaded = 0u32;

    for title in &titles {
        if !title.monitored {
            continue;
        }

        let media_files = app
            .services
            .media_files
            .list_media_files_for_title(&title.id)
            .await?;

        for mf in &media_files {
            let existing = app
                .services
                .subtitle_downloads
                .list_for_media_file(&mf.id)
                .await
                .unwrap_or_default();

            let embedded = embedded_subtitle_streams(mf);

            let missing =
                compute_missing_subtitles_from_streams(&wanted_languages, &existing, &embedded);
            if missing.is_empty() {
                continue;
            }

            let is_series = is_series_title(title);
            let min_score = if is_series {
                min_score_series
            } else {
                min_score_movie
            };

            let orchestrator = SubtitleSearchOrchestrator::new(min_score);
            let media_kind = subtitle_media_kind(title);
            let (imdb_id, series_imdb_id) = title_imdb_ids(title);

            let file_path = Path::new(&mf.file_path);

            let (season_num, episode_num) = media_file_episode_context(app, mf).await;

            for lang_pref in &missing {
                searched += 1;

                let results = match orchestrator
                    .search(
                        &provider,
                        file_path,
                        media_kind,
                        &title.name,
                        &title.aliases,
                        title.year,
                        imdb_id,
                        series_imdb_id,
                        season_num,
                        episode_num,
                        std::slice::from_ref(&lang_pref.code),
                        mf.release_group.as_deref(),
                        mf.source_type.as_deref(),
                        mf.video_codec_parsed.as_deref(),
                        mf.audio_codec_parsed.as_deref(),
                        mf.resolution.as_deref(),
                        Some(lang_pref.hearing_impaired),
                        include_ai,
                        include_machine,
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(err) => {
                        warn!(
                            error = %err,
                            title = %title.name,
                            language = %lang_pref.code,
                            "subtitle search failed"
                        );
                        continue;
                    }
                };

                // Filter blacklisted results
                let mut filtered_results = Vec::new();
                for r in &results {
                    let blacklisted = app
                        .services
                        .subtitle_downloads
                        .is_blacklisted(&mf.id, &r.provider, &r.provider_file_id)
                        .await
                        .unwrap_or(false);
                    if !blacklisted {
                        filtered_results.push(r);
                    }
                }

                // Pick the best result above min_score
                let best = filtered_results
                    .iter()
                    .filter(|r| r.score >= min_score)
                    .filter(|r| r.hearing_impaired == lang_pref.hearing_impaired)
                    .filter(|r| r.forced == lang_pref.forced)
                    .max_by_key(|r| r.score)
                    .copied();

                let best = match best {
                    Some(b) => b,
                    None => {
                        debug!(
                            title = %title.name,
                            language = %lang_pref.code,
                            results = results.len(),
                            "no subtitle above min_score"
                        );
                        continue;
                    }
                };

                // Download and save
                match crate::subtitles::download::download_and_save(
                    &provider,
                    &best.provider_file_id,
                    file_path,
                    &best.language,
                    best.forced,
                    best.hearing_impaired,
                )
                .await
                {
                    Ok((dest_path, _file)) => {
                        // Record in database
                        let record = SubtitleDownload {
                            id: scryer_domain::Id::new().0,
                            media_file_id: mf.id.clone(),
                            title_id: title.id.clone(),
                            episode_id: mf.episode_id.clone(),
                            language: best.language.clone(),
                            provider: best.provider.clone(),
                            provider_file_id: Some(best.provider_file_id.clone()),
                            file_path: dest_path.to_string_lossy().to_string(),
                            score: Some(best.score),
                            hearing_impaired: best.hearing_impaired,
                            forced: best.forced,
                            ai_translated: best.ai_translated,
                            machine_translated: best.machine_translated,
                            uploader: best.uploader.clone(),
                            release_info: best.release_info.clone(),
                            synced: false,
                            downloaded_at: Utc::now().to_rfc3339(),
                        };

                        let record_inserted = match app
                            .services
                            .subtitle_downloads
                            .insert(&record)
                            .await
                        {
                            Ok(()) => true,
                            Err(err) => {
                                warn!(error = %err, "failed to persist subtitle download record");
                                false
                            }
                        };

                        let sync_result = maybe_sync_downloaded_subtitle(
                            app,
                            sync_settings,
                            media_kind,
                            file_path,
                            &dest_path,
                            record_inserted.then_some(record.id.as_str()),
                            Some(best.score),
                            best.forced,
                        )
                        .await;

                        downloaded += 1;
                        let event_msg = format!(
                            "{} subtitle downloaded for {} (score: {}, provider: {}){}",
                            lang_pref.code,
                            title.name,
                            best.score,
                            best.provider,
                            sync_summary_suffix(sync_result.as_ref()),
                        );
                        info!(
                            title = %title.name,
                            language = %lang_pref.code,
                            provider = %best.provider,
                            score = best.score,
                            path = %dest_path.display(),
                            sync = sync_summary_suffix(sync_result.as_ref()),
                            "subtitle downloaded"
                        );
                        let _ = app
                            .services
                            .record_activity_event(
                                None,
                                Some(title.id.clone()),
                                None,
                                crate::ActivityKind::SubtitleDownloaded,
                                event_msg,
                                crate::ActivitySeverity::Success,
                                vec![crate::ActivityChannel::WebUi],
                            )
                            .await;
                    }
                    Err(err) => {
                        let event_msg = format!(
                            "{} subtitle download failed for {}: {}",
                            lang_pref.code, title.name, err,
                        );
                        warn!(
                            error = %err,
                            title = %title.name,
                            language = %lang_pref.code,
                            "subtitle download failed"
                        );
                        let _ = app
                            .services
                            .record_activity_event(
                                None,
                                Some(title.id.clone()),
                                None,
                                crate::ActivityKind::SubtitleSearchFailed,
                                event_msg,
                                crate::ActivitySeverity::Warning,
                                vec![crate::ActivityChannel::WebUi],
                            )
                            .await;
                    }
                }

                // Rate limiting: small delay between provider requests
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            }
        }
    }

    info!(searched, downloaded, "subtitle search cycle completed");
    Ok(())
}
