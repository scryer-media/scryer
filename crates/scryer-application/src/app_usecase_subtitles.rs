use std::path::Path;

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::subtitles::provider::OpenSubtitlesProvider;
use crate::subtitles::search::SubtitleSearchOrchestrator;
use crate::subtitles::wanted::{SubtitleLanguagePref, compute_missing_subtitles};
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

            let embedded: Vec<String> = mf.subtitle_languages.clone();

            let missing = compute_missing_subtitles(&wanted_languages, &existing, &embedded);
            if missing.is_empty() {
                continue;
            }

            let is_series = title.facet == scryer_domain::MediaFacet::Tv
                || title.facet == scryer_domain::MediaFacet::Anime;
            let min_score = if is_series {
                min_score_series
            } else {
                min_score_movie
            };

            let orchestrator = SubtitleSearchOrchestrator::new(min_score);
            let imdb_id = title
                .external_ids
                .iter()
                .find(|e| e.source == "imdb")
                .map(|e| e.value.as_str());

            let file_path = Path::new(&mf.file_path);

            // Look up season/episode for series
            let (season_num, episode_num) = if let Some(ep_id) = &mf.episode_id {
                match app.services.shows.get_episode_by_id(ep_id).await {
                    Ok(Some(ep)) => (
                        ep.season_number
                            .as_ref()
                            .and_then(|s| s.parse::<i32>().ok()),
                        ep.episode_number
                            .as_ref()
                            .and_then(|s| s.parse::<i32>().ok()),
                    ),
                    _ => (None, None),
                }
            } else {
                (None, None)
            };

            for lang_pref in &missing {
                searched += 1;

                let results = match orchestrator
                    .search(
                        &provider,
                        file_path,
                        &title.name,
                        title.year,
                        imdb_id,
                        season_num,
                        episode_num,
                        std::slice::from_ref(&lang_pref.code),
                        mf.release_group.as_deref(),
                        mf.source_type.as_deref(),
                        mf.video_codec_parsed.as_deref(),
                        mf.audio_codec_parsed.as_deref(),
                        mf.resolution.as_deref(),
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

                // Pick the best result above min_score
                let best = results
                    .iter()
                    .filter(|r| r.score >= min_score)
                    .filter(|r| {
                        r.hearing_impaired == lang_pref.hearing_impaired
                            || !lang_pref.hearing_impaired
                    })
                    .filter(|r| r.forced == lang_pref.forced)
                    .max_by_key(|r| r.score);

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
                    &lang_pref.code,
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
                            language: lang_pref.code.clone(),
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

                        if let Err(err) = app.services.subtitle_downloads.insert(&record).await {
                            warn!(error = %err, "failed to persist subtitle download record");
                        }

                        downloaded += 1;
                        let event_msg = format!(
                            "{} subtitle downloaded for {} (score: {}, provider: {})",
                            lang_pref.code, title.name, best.score, best.provider,
                        );
                        info!(
                            title = %title.name,
                            language = %lang_pref.code,
                            provider = %best.provider,
                            score = best.score,
                            path = %dest_path.display(),
                            "subtitle downloaded"
                        );
                        let _ = app
                            .services
                            .record_activity_event(
                                None,
                                Some(title.id.clone()),
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
