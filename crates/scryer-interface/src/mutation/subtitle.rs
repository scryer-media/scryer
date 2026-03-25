use async_graphql::{Context, InputObject, Object, SimpleObject};

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx, to_gql_error};

#[derive(InputObject)]
pub struct BlacklistSubtitleInput {
    pub subtitle_download_id: String,
    pub reason: Option<String>,
}

type GqlResult<T> = async_graphql::Result<T>;

#[derive(Default)]
pub struct SubtitleMutations;

#[derive(InputObject)]
pub struct SearchSubtitlesInput {
    pub media_file_id: String,
    pub language: String,
}

#[derive(InputObject)]
pub struct DownloadSubtitleInput {
    pub media_file_id: String,
    pub provider_file_id: String,
    pub language: String,
    pub forced: Option<bool>,
    pub hearing_impaired: Option<bool>,
    pub score: Option<i32>,
    pub release_info: Option<String>,
    pub uploader: Option<String>,
    pub ai_translated: Option<bool>,
    pub machine_translated: Option<bool>,
}

#[derive(SimpleObject)]
pub struct SubtitleSearchResult {
    pub provider: String,
    pub provider_file_id: String,
    pub language: String,
    pub release_info: Option<String>,
    pub score: i32,
    pub hearing_impaired: bool,
    pub forced: bool,
    pub ai_translated: bool,
    pub machine_translated: bool,
    pub uploader: Option<String>,
    pub download_count: Option<i64>,
    pub hash_matched: bool,
}

/// Read a subtitle setting value from the settings DB.
async fn read_subtitle_setting(
    db: &scryer_infrastructure::SqliteServices,
    key: &str,
) -> Option<String> {
    let rec = db
        .get_setting_with_defaults("system", key, None)
        .await
        .ok()
        .flatten()?;
    let raw = rec.effective_value_json;
    // Strip JSON string quotes if present
    serde_json::from_str::<String>(&raw).ok().or_else(|| {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "null" {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

async fn read_subtitle_i32_setting(
    db: &scryer_infrastructure::SqliteServices,
    key: &str,
) -> Option<i32> {
    read_subtitle_setting(db, key)
        .await
        .and_then(|value| value.parse::<i32>().ok())
}

#[Object]
impl SubtitleMutations {
    /// Search for subtitles for a media file in a given language.
    async fn search_subtitles(
        &self,
        ctx: &Context<'_>,
        input: SearchSubtitlesInput,
    ) -> GqlResult<Vec<SubtitleSearchResult>> {
        let app = app_from_ctx(ctx)?;
        let _actor = actor_from_ctx(ctx)?;
        let db = settings_db_from_ctx(ctx)?;

        let mf = app
            .services
            .media_files
            .get_media_file_by_id(&input.media_file_id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| async_graphql::Error::new("media file not found"))?;
        let title = app
            .services
            .titles
            .get_by_id(&mf.title_id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| async_graphql::Error::new("title not found"))?;

        let api_key = read_subtitle_setting(&db, "subtitles.opensubtitles_api_key")
            .await
            .ok_or_else(|| async_graphql::Error::new("OpenSubtitles API key not configured"))?;

        let username = read_subtitle_setting(&db, "subtitles.opensubtitles_username")
            .await
            .unwrap_or_default();
        let password = read_subtitle_setting(&db, "subtitles.opensubtitles_password")
            .await
            .unwrap_or_default();

        let include_ai = read_subtitle_setting(&db, "subtitles.include_ai_translated")
            .await
            .as_deref()
            == Some("true");
        let include_machine = read_subtitle_setting(&db, "subtitles.include_machine_translated")
            .await
            .as_deref()
            == Some("true");

        let provider = scryer_application::subtitles::provider::OpenSubtitlesProvider::new(api_key);
        if !username.is_empty() && !password.is_empty() {
            let _ = provider.login(&username, &password).await;
        }

        let is_series = title.facet == scryer_domain::MediaFacet::Series
            || title.facet == scryer_domain::MediaFacet::Anime;
        let imdb_id = if is_series {
            None
        } else {
            title
                .external_ids
                .iter()
                .find(|e| e.source == "imdb")
                .map(|e| e.value.as_str())
        };
        let series_imdb_id = if is_series {
            title
                .external_ids
                .iter()
                .find(|e| e.source == "imdb")
                .map(|e| e.value.as_str())
        } else {
            None
        };
        let (season_num, episode_num) = if let Some(episode_id) = mf.episode_id.as_deref() {
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
        } else {
            (None, None)
        };

        let orchestrator =
            scryer_application::subtitles::search::SubtitleSearchOrchestrator::new(0);

        let file_path = std::path::Path::new(&mf.file_path);
        let results = orchestrator
            .search(
                &provider,
                file_path,
                if is_series {
                    scryer_application::subtitles::provider::SubtitleMediaKind::Episode
                } else {
                    scryer_application::subtitles::provider::SubtitleMediaKind::Movie
                },
                &title.name,
                &title.aliases,
                title.year,
                imdb_id,
                series_imdb_id,
                season_num,
                episode_num,
                &[input.language],
                mf.release_group.as_deref(),
                mf.source_type.as_deref(),
                mf.video_codec_parsed.as_deref(),
                mf.audio_codec_parsed.as_deref(),
                mf.resolution.as_deref(),
                None,
                include_ai,
                include_machine,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(results
            .into_iter()
            .map(|r| SubtitleSearchResult {
                provider: r.provider,
                provider_file_id: r.provider_file_id,
                language: r.language,
                release_info: r.release_info,
                score: r.score,
                hearing_impaired: r.hearing_impaired,
                forced: r.forced,
                ai_translated: r.ai_translated,
                machine_translated: r.machine_translated,
                uploader: r.uploader,
                download_count: r.download_count,
                hash_matched: r.hash_matched,
            })
            .collect())
    }

    /// Download a specific subtitle and save to disk next to the video.
    async fn download_subtitle(
        &self,
        ctx: &Context<'_>,
        input: DownloadSubtitleInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let _actor = actor_from_ctx(ctx)?;
        let db = settings_db_from_ctx(ctx)?;

        let mf = app
            .services
            .media_files
            .get_media_file_by_id(&input.media_file_id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| async_graphql::Error::new("media file not found"))?;
        let title = app
            .services
            .titles
            .get_by_id(&mf.title_id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| async_graphql::Error::new("title not found"))?;

        let api_key = read_subtitle_setting(&db, "subtitles.opensubtitles_api_key")
            .await
            .ok_or_else(|| async_graphql::Error::new("OpenSubtitles API key not configured"))?;

        let username = read_subtitle_setting(&db, "subtitles.opensubtitles_username")
            .await
            .unwrap_or_default();
        let password = read_subtitle_setting(&db, "subtitles.opensubtitles_password")
            .await
            .unwrap_or_default();

        let provider = scryer_application::subtitles::provider::OpenSubtitlesProvider::new(api_key);
        if !username.is_empty() && !password.is_empty() {
            let _ = provider.login(&username, &password).await;
        }

        let forced = input.forced.unwrap_or(false);
        let hearing_impaired = input.hearing_impaired.unwrap_or(false);

        let file_path = std::path::Path::new(&mf.file_path);
        let (dest_path, _) = scryer_application::subtitles::download::download_and_save(
            &provider,
            &input.provider_file_id,
            file_path,
            &input.language,
            forced,
            hearing_impaired,
        )
        .await
        .map_err(to_gql_error)?;

        // Persist to DB
        let record = scryer_domain::SubtitleDownload {
            id: scryer_domain::Id::new().0,
            media_file_id: mf.id.clone(),
            title_id: mf.title_id.clone(),
            episode_id: mf.episode_id.clone(),
            language: input.language,
            provider: "opensubtitles".to_string(),
            provider_file_id: Some(input.provider_file_id),
            file_path: dest_path.to_string_lossy().to_string(),
            score: input.score,
            hearing_impaired,
            forced,
            ai_translated: input.ai_translated.unwrap_or(false),
            machine_translated: input.machine_translated.unwrap_or(false),
            uploader: input.uploader,
            release_info: input.release_info,
            synced: false,
            downloaded_at: chrono::Utc::now().to_rfc3339(),
        };
        app.services
            .subtitle_downloads
            .insert(&record)
            .await
            .map_err(to_gql_error)?;

        let is_series = title.facet == scryer_domain::MediaFacet::Series
            || title.facet == scryer_domain::MediaFacet::Anime;
        let policy = scryer_application::subtitles::sync::SyncPolicy {
            enabled: read_subtitle_setting(&db, "subtitles.sync_enabled")
                .await
                .as_deref()
                != Some("false"),
            forced,
            score: input.score,
            threshold: Some(if is_series {
                read_subtitle_i32_setting(&db, "subtitles.sync_threshold_series")
                    .await
                    .unwrap_or(90)
            } else {
                read_subtitle_i32_setting(&db, "subtitles.sync_threshold_movie")
                    .await
                    .unwrap_or(70)
            }),
            max_offset_seconds: i64::from(
                read_subtitle_i32_setting(&db, "subtitles.sync_max_offset_seconds")
                    .await
                    .unwrap_or(60),
            ),
        };

        match scryer_application::subtitles::sync::sync_subtitle_with_policy(
            file_path, &dest_path, policy,
        )
        .await
        {
            Ok(result) => {
                if result.applied
                    && let Err(err) = app
                        .services
                        .subtitle_downloads
                        .set_synced(&record.id, true)
                        .await
                {
                    tracing::warn!(error = %err, download_id = %record.id, "failed to persist subtitle sync status");
                }

                tracing::info!(
                    media_file_id = %mf.id,
                    subtitle_download_id = %record.id,
                    summary = %result.summary(),
                    "manual subtitle sync completed"
                );
            }
            Err(err) => {
                tracing::warn!(error = %err, "subtitle sync failed (non-fatal)");
            }
        }

        Ok(true)
    }

    /// Blacklist a downloaded subtitle: delete the file and DB record, then add to blacklist.
    async fn blacklist_subtitle(
        &self,
        ctx: &Context<'_>,
        input: BlacklistSubtitleInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let _actor = actor_from_ctx(ctx)?;

        // Delete the download record (returns the record so we have the file_path)
        let record = app
            .services
            .subtitle_downloads
            .delete(&input.subtitle_download_id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| async_graphql::Error::new("subtitle download not found"))?;

        // Delete the file from disk
        let path = std::path::Path::new(&record.file_path);
        if path.exists()
            && let Err(err) = tokio::fs::remove_file(path).await
        {
            tracing::warn!(error = %err, path = %record.file_path, "failed to delete subtitle file");
        }

        // Insert into blacklist
        if let Some(provider_file_id) = &record.provider_file_id {
            app.services
                .subtitle_downloads
                .blacklist(
                    &record.media_file_id,
                    &record.provider,
                    provider_file_id,
                    &record.language,
                    input.reason.as_deref(),
                )
                .await
                .map_err(to_gql_error)?;
        }

        Ok(true)
    }
}
