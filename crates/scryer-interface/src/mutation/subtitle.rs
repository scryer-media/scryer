use async_graphql::{Context, Enum, ID, InputObject, Object, SimpleObject};
use scryer_application::{
    AppError, DownloadSubtitleForMediaFileRequest, SubtitleSearchStatus as AppSubtitleSearchStatus,
};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};

#[derive(InputObject)]
/// Identifies an external subtitle deletion and its optional destructive-action confirmation.
pub struct DeleteExternalSubtitleInput {
    /// External subtitle file ID to delete.
    pub external_subtitle_id: ID,
    /// Fingerprint returned by the required deletion preview.
    pub preview_fingerprint: Option<String>,
    /// Required confirmation text when the preview reports a protected deletion.
    pub typed_confirmation: Option<String>,
}

#[derive(InputObject)]
/// Identifies a provider-backed subtitle to delete and add to the subtitle blocklist.
pub struct BlocklistExternalSubtitleInput {
    /// External subtitle file ID to delete and blocklist.
    pub external_subtitle_id: ID,
    /// Optional operator-supplied reason stored with the blocklist entry.
    pub reason: Option<String>,
    /// Fingerprint returned by the required deletion preview.
    pub preview_fingerprint: Option<String>,
    /// Required confirmation text when the preview reports a protected deletion.
    pub typed_confirmation: Option<String>,
}

type GqlResult<T> = async_graphql::Result<T>;

#[derive(Default)]
pub struct SubtitleMutations;

#[derive(InputObject)]
/// Selects a media file and language for subtitle provider searches.
pub struct SearchSubtitlesInput {
    /// Media file ID whose release metadata is used for matching.
    pub media_file_id: ID,
    /// Requested subtitle language code; null or blank searches the
    /// administrator's preferred subtitle language.
    pub language: Option<String>,
}

#[derive(InputObject)]
/// Identifies a provider result and its matching metadata for subtitle download.
pub struct DownloadSubtitleInput {
    /// Media file ID that will receive the external subtitle.
    pub media_file_id: ID,
    /// Subtitle provider name; null defaults to OpenSubtitles.
    pub provider: Option<String>,
    /// Provider-specific subtitle file ID.
    pub provider_file_id: String,
    /// Subtitle language code.
    pub language: String,
    /// Whether the subtitle is intended only for forced dialogue; null defaults to false.
    pub forced: Option<bool>,
    /// Whether the subtitle includes hearing-impaired cues; null defaults to false.
    pub hearing_impaired: Option<bool>,
    /// Optional provider match score retained with the downloaded subtitle.
    pub score: Option<i32>,
    /// Optional release information retained for later matching and blocklisting.
    pub release_info: Option<String>,
    /// Optional provider uploader name.
    pub uploader: Option<String>,
    /// Whether the subtitle was translated with generative AI; null defaults to false.
    pub ai_translated: Option<bool>,
    /// Whether the subtitle was machine translated; null defaults to false.
    pub machine_translated: Option<bool>,
}

#[derive(SimpleObject)]
/// Confirms the provider subtitle saved for a media file.
pub struct DownloadSubtitlePayload {
    /// Media file ID that received the subtitle.
    pub media_file_id: ID,
    /// Provider-specific subtitle file ID that was downloaded.
    pub provider_file_id: String,
    /// Whether the subtitle download and import completed.
    pub downloaded: bool,
}

#[derive(SimpleObject)]
/// Reports the result of deleting an external subtitle.
pub struct DeleteExternalSubtitlePayload {
    /// External subtitle ID targeted by the deletion.
    pub id: ID,
    /// Whether the subtitle file and tracked record were deleted.
    pub deleted: bool,
}

#[derive(SimpleObject)]
/// Reports the result of deleting and blocklisting an external subtitle.
pub struct BlocklistExternalSubtitlePayload {
    /// External subtitle ID targeted by the operation.
    pub id: ID,
    /// Whether the subtitle was deleted and its provider result blocklisted.
    pub blocklisted: bool,
}

#[derive(SimpleObject)]
/// Describes a subtitle provider result and its match characteristics.
pub struct SubtitleSearchResult {
    /// Provider that returned the subtitle.
    pub provider: String,
    /// Provider-specific subtitle file ID used for download.
    pub provider_file_id: String,
    /// Subtitle language code.
    pub language: String,
    /// Provider release information; null when unavailable.
    pub release_info: Option<String>,
    /// Raw subtitle match score.
    pub score: i32,
    /// Match score normalized to a percentage.
    pub score_percent: i32,
    /// Whether the subtitle includes hearing-impaired cues.
    pub hearing_impaired: bool,
    /// Whether the subtitle is intended only for forced dialogue.
    pub forced: bool,
    /// Whether the subtitle was translated with generative AI.
    pub ai_translated: bool,
    /// Whether the subtitle was machine translated.
    pub machine_translated: bool,
    /// Provider uploader name; null when unavailable.
    pub uploader: Option<String>,
    /// Provider-reported download count; null when unavailable.
    pub download_count: Option<i64>,
    /// Whether the subtitle matched the media file hash.
    pub hash_matched: bool,
}

/// Why a subtitle search returned the results it did.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum SubtitleSearchStatus {
    /// Providers were queried and the results are the matches they returned.
    Ready,
    /// No subtitle provider is configured and enabled, so nothing was queried.
    NoProviders,
    /// A provider is configured but could not be used for this search, most
    /// often because its credentials or plugin are missing.
    ProviderUnavailable,
    /// Subtitle handling is switched off in catalog settings.
    Disabled,
}

impl From<AppSubtitleSearchStatus> for SubtitleSearchStatus {
    fn from(status: AppSubtitleSearchStatus) -> Self {
        match status {
            AppSubtitleSearchStatus::Ready => Self::Ready,
            AppSubtitleSearchStatus::NoProviders => Self::NoProviders,
            AppSubtitleSearchStatus::ProviderUnavailable => Self::ProviderUnavailable,
            AppSubtitleSearchStatus::Disabled => Self::Disabled,
        }
    }
}

#[derive(SimpleObject)]
/// Reports one subtitle search: what was searched, what the caller may search
/// next, and the provider matches found.
pub struct SubtitleSearchPayload {
    /// Why the search returned these results.
    pub status: SubtitleSearchStatus,
    /// Language code that was actually searched; the requested language when
    /// one was given, otherwise the administrator's preferred language.
    pub language: String,
    /// Subtitle languages an administrator configured, in configured order,
    /// for the language picker.
    pub available_languages: Vec<String>,
    /// Provider matches found for the searched language; empty unless the
    /// status is READY.
    pub results: Vec<SubtitleSearchResult>,
}

fn from_subtitle_match(
    result: scryer_application::subtitles::SubtitleMatch,
) -> SubtitleSearchResult {
    SubtitleSearchResult {
        provider: result.provider,
        provider_file_id: result.provider_file_id,
        language: result.language,
        release_info: result.release_info,
        score: result.score,
        score_percent: result.score_percent,
        hearing_impaired: result.hearing_impaired,
        forced: result.forced,
        ai_translated: result.ai_translated,
        machine_translated: result.machine_translated,
        uploader: result.uploader,
        download_count: result.download_count,
        hash_matched: result.hash_matched,
    }
}

#[Object]
impl SubtitleMutations {
    /// Search configured providers for subtitles matching a media file, and
    /// report why the search returned what it did.
    async fn search_subtitles(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Media file ID and, optionally, the subtitle language to search for.")]
        input: SearchSubtitlesInput,
    ) -> GqlResult<SubtitleSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let media_file_id = input.media_file_id.to_string();
        let requested_language = input.language.as_deref();
        let outcome = app
            .search_subtitles_for_media_file(&actor, &media_file_id, requested_language)
            .await
            .map_err(to_gql_error)?;
        Ok(SubtitleSearchPayload {
            status: outcome.status.into(),
            language: outcome.language,
            available_languages: outcome.available_languages,
            results: outcome
                .results
                .into_iter()
                .map(from_subtitle_match)
                .collect(),
        })
    }

    /// Download a provider subtitle and save it beside the selected media file.
    async fn download_subtitle(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Provider result, media file ID, language, and matching metadata to retain."
        )]
        input: DownloadSubtitleInput,
    ) -> GqlResult<DownloadSubtitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let media_file_id = input.media_file_id;
        let media_file_id_string = media_file_id.to_string();
        let provider_file_id = input.provider_file_id;
        app.download_subtitle_for_media_file(
            &actor,
            DownloadSubtitleForMediaFileRequest {
                media_file_id: media_file_id_string,
                provider_name: input
                    .provider
                    .unwrap_or_else(|| "opensubtitles".to_string()),
                provider_file_id: provider_file_id.clone(),
                language: input.language,
                forced: input.forced.unwrap_or(false),
                hearing_impaired: input.hearing_impaired.unwrap_or(false),
                score: input.score,
                release_info: input.release_info,
                uploader: input.uploader,
                ai_translated: input.ai_translated.unwrap_or(false),
                machine_translated: input.machine_translated.unwrap_or(false),
            },
        )
        .await
        .map_err(to_gql_error)?;
        Ok(DownloadSubtitlePayload {
            media_file_id,
            provider_file_id,
            downloaded: true,
        })
    }

    /// Delete an external subtitle file and its tracked record.
    async fn delete_external_subtitle(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "External subtitle ID plus the required preview fingerprint and any confirmation text."
        )]
        input: DeleteExternalSubtitleInput,
    ) -> GqlResult<DeleteExternalSubtitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let external_subtitle_id = input.external_subtitle_id;
        let external_subtitle_id_string = external_subtitle_id.to_string();
        let preview_fingerprint = input.preview_fingerprint.as_deref().ok_or_else(|| {
            to_gql_error(AppError::validation_refused(
                scryer_interface_core::refusal::INTERFACE_SUBTITLE_DELETE_CONFIRMATION_REQUIRED,
                "delete preview confirmation is required before deleting subtitle files on disk",
            ))
        })?;

        app.delete_external_subtitle(
            &actor,
            &external_subtitle_id_string,
            preview_fingerprint,
            input.typed_confirmation.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;

        Ok(DeleteExternalSubtitlePayload {
            id: external_subtitle_id,
            deleted: true,
        })
    }

    /// Delete a provider-backed subtitle and add its provider result to the subtitle blocklist.
    async fn blocklist_external_subtitle(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "External subtitle ID, optional reason, required preview fingerprint, and any confirmation text."
        )]
        input: BlocklistExternalSubtitleInput,
    ) -> GqlResult<BlocklistExternalSubtitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let external_subtitle_id = input.external_subtitle_id;
        let external_subtitle_id_string = external_subtitle_id.to_string();
        let preview_fingerprint = input.preview_fingerprint.as_deref().ok_or_else(|| {
            to_gql_error(AppError::validation_refused(
                scryer_interface_core::refusal::INTERFACE_SUBTITLE_DELETE_CONFIRMATION_REQUIRED,
                "delete preview confirmation is required before deleting subtitle files on disk",
            ))
        })?;

        app.blocklist_external_subtitle(
            &actor,
            &external_subtitle_id_string,
            input.reason.as_deref(),
            preview_fingerprint,
            input.typed_confirmation.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;

        Ok(BlocklistExternalSubtitlePayload {
            id: external_subtitle_id,
            blocklisted: true,
        })
    }
}
