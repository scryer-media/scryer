use async_graphql::{Context, InputObject, Object, SimpleObject};

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx, to_gql_error};

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

        let imdb_id = title
            .external_ids
            .iter()
            .find(|e| e.source == "imdb")
            .map(|e| e.value.as_str());

        let orchestrator =
            scryer_application::subtitles::search::SubtitleSearchOrchestrator::new(0);

        let file_path = std::path::Path::new(&mf.file_path);
        let results = orchestrator
            .search(
                &provider,
                file_path,
                &title.name,
                title.year,
                imdb_id,
                None,
                None,
                &[input.language],
                mf.release_group.as_deref(),
                mf.source_type.as_deref(),
                mf.video_codec_parsed.as_deref(),
                mf.audio_codec_parsed.as_deref(),
                mf.resolution.as_deref(),
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
            score: None,
            hearing_impaired,
            forced,
            ai_translated: false,
            machine_translated: false,
            uploader: None,
            release_info: None,
            synced: false,
            downloaded_at: chrono::Utc::now().to_rfc3339(),
        };
        scryer_infrastructure::queries::subtitle::insert_subtitle_download(db.pool(), &record)
            .await
            .map_err(to_gql_error)?;

        Ok(true)
    }
}
