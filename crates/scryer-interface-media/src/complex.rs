use async_graphql::{ComplexObject, Context, ID, Result as GqlResult};
use scryer_application::{AcquisitionScopeStatesQuery, ReleaseDecisionsQuery};
use scryer_interface_core::{
    actor_from_ctx, app_from_ctx, loaders::loaders_from_ctx, to_gql_error,
};

use crate::mappers::{
    fallback_episode_media_availability, from_collection, from_discovery_item,
    from_download_queue_item, from_episode, from_episode_media_availability, from_library_settings,
    from_pending_release, from_release_decision, from_series_movie_link, from_submission_scope,
    from_title, from_title_credit, from_title_media_file, from_title_rating_summary,
    from_wanted_item,
};
use crate::types::*;

const RELATION_PAGE_MAX_LIMIT: i32 = 300;

fn from_media_server_playback_link(
    link: scryer_application::MediaServerPlaybackLink,
) -> MediaServerPlaybackLinkPayload {
    MediaServerPlaybackLinkPayload {
        connection_id: link.connection_id.into(),
        display_name: link.display_name,
        provider: MediaServerProviderValue::from_domain(link.provider),
        href: link.href,
    }
}

fn title_scope_from_facet(facet: MediaFacetValue) -> ContentScopeValue {
    match facet {
        MediaFacetValue::Movie => ContentScopeValue::Movie,
        MediaFacetValue::Series => ContentScopeValue::Series,
        MediaFacetValue::Anime => ContentScopeValue::Anime,
    }
}

fn relation_page_limit(limit: i32) -> i32 {
    limit.clamp(1, RELATION_PAGE_MAX_LIMIT)
}

fn relation_page_offset(offset: i32) -> i32 {
    offset.max(0)
}

#[ComplexObject]
impl MovieEntityPayload {
    /// Cast and crew cached during the movie's latest metadata hydration.
    async fn credits(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleCreditPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let movie_entity_id = self.id.to_string();
        app.movie_entity_credits(&actor, self.permission_title_id.as_ref(), &movie_entity_id)
            .await
            .map(|credits| {
                credits
                    .into_iter()
                    .map(|credit| {
                        crate::mappers::from_movie_entity_credit(&app, &movie_entity_id, credit)
                    })
                    .collect()
            })
            .map_err(to_gql_error)
    }
}

#[ComplexObject]
impl LibraryPayload {
    /// Effective title quality-profile ID for this library; requires title-management or catalog-settings access.
    async fn quality_profile_id(&self, ctx: &Context<'_>) -> GqlResult<ID> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.title_quality_profile_id_for_library(&actor, self.id.as_ref())
            .await
            .map(Into::into)
            .map_err(to_gql_error)
    }

    /// Quality-profile IDs currently allowed for requests in this library; access requires request, title-management, or relevant application permission.
    async fn request_quality_profile_ids(&self, ctx: &Context<'_>) -> GqlResult<Vec<ID>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .request_quality_profile_settings_for_library(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(settings.profile_ids.into_iter().map(Into::into).collect())
    }

    /// Default request quality-profile ID, selected from the library's effective allowed profiles.
    async fn request_quality_profile_default_id(&self, ctx: &Context<'_>) -> GqlResult<ID> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .request_quality_profile_settings_for_library(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(settings.default_profile_id.into())
    }

    /// Effective library settings; requires library-management permission.
    async fn settings(&self, ctx: &Context<'_>) -> GqlResult<LibrarySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_library_settings(&actor, self.id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(from_library_settings(settings))
    }
}

#[ComplexObject]
impl TitlePayload {
    /// Provider-native playback links for this title, when an exact catalog mapping exists.
    async fn playback_links(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<MediaServerPlaybackLinkPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.media_server_playback_links(
            &actor,
            scryer_domain::MediaServerPlaybackEntityKind::Title,
            self.id.as_ref(),
        )
        .await
        .map(|links| {
            links
                .into_iter()
                .map(from_media_server_playback_link)
                .collect()
        })
        .map_err(to_gql_error)
    }

    /// Effective target quality-profile label for the title.
    async fn quality_tier(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .effective_quality_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| summary.quality_tier));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_effective_quality_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| summary.quality_tier))
        })
        .await
    }

    /// Lowest live media-file quality tier for the title.
    async fn current_quality_tier(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .quality_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| summary.quality_tier));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_quality_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| summary.quality_tier))
        })
        .await
    }

    /// Aggregated media-file size in bytes for the title.
    async fn size_bytes(&self, ctx: &Context<'_>) -> GqlResult<Option<Long>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .media_size_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| Long::from(summary.total_size_bytes)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_media_size_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| Long::from(summary.total_size_bytes)))
        })
        .await
    }

    /// Owned-vs-total episode progress, excluding specials.
    async fn episodes_owned(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .episode_progress_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| summary.owned_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_episode_progress_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| summary.owned_episodes))
        })
        .await
    }

    /// Monitored episode count, excluding specials.
    async fn episodes_monitored(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .episode_progress_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| summary.monitored_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_episode_progress_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| summary.monitored_episodes))
        })
        .await
    }

    /// Total episode count, excluding specials.
    async fn episodes_total(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .episode_progress_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.map(|summary| summary.total_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_episode_progress_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .map(|summary| summary.total_episodes))
        })
        .await
    }

    /// Primary movie media resolution.
    async fn media_resolution(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .movie_media_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.and_then(|summary| summary.resolution));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_movie_media_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .and_then(|summary| summary.resolution))
        })
        .await
    }

    /// Primary movie media HDR format.
    async fn media_hdr(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .movie_media_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.and_then(|summary| summary.hdr_format));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_movie_media_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .and_then(|summary| summary.hdr_format))
        })
        .await
    }

    /// Primary movie media audio codec.
    async fn media_audio_codec(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .movie_media_summary
                .load_one(self.id.to_string())
                .await?;
            return Ok(summary.and_then(|summary| summary.audio_codec));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let id = self.id.to_string();
            let summaries = app
                .list_title_movie_media_summaries(&actor, std::slice::from_ref(&id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.title_id == id)
                .and_then(|summary| summary.audio_codec))
        })
        .await
    }

    /// Name of the title's library when it is visible to the caller, otherwise null.
    async fn library_name(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let library = loaders
                .library
                .load_one(self.library_id.to_string())
                .await?;
            return Ok(library.map(|library| library.name));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let library_id = self.library_id.to_string();
            let libraries = app
                .list_libraries_for_permission(&actor, None, scryer_domain::LibraryPermission::View)
                .await
                .map_err(to_gql_error)?;
            Ok(libraries
                .into_iter()
                .find(|library| library.id == library_id)
                .map(|library| library.name))
        })
        .await
    }

    /// Stable slug of the title's caller-visible library, otherwise null.
    async fn library_slug(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let library = loaders
                .library
                .load_one(self.library_id.to_string())
                .await?;
            return Ok(library.map(|library| library.slug));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let library_id = self.library_id.to_string();
            let libraries = app
                .list_libraries_for_permission(&actor, None, scryer_domain::LibraryPermission::View)
                .await
                .map_err(to_gql_error)?;
            Ok(libraries
                .into_iter()
                .find(|library| library.id == library_id)
                .map(|library| library.slug))
        })
        .await
    }

    /// Aggregated title ratings; the rating is null and source lists are empty when no rating exists.
    async fn ratings(&self, ctx: &Context<'_>) -> GqlResult<TitleRatingPayload> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders.ratings.load_one(self.id.to_string()).await?;
            return Ok(summary.map(from_title_rating_summary).unwrap_or_else(|| {
                TitleRatingPayload {
                    rating: None,
                    rating_sources: Vec::new(),
                    external_ratings: Vec::new(),
                }
            }));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            app.title_ratings(&actor, self.id.as_ref())
                .await
                .map(from_title_rating_summary)
                .map_err(to_gql_error)
        })
        .await
    }

    /// Cast and crew cached from this title's last metadata hydration, ordered
    /// by billing rank. Reads the local cache only; hydration refills it.
    async fn credits(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Credit kinds to include, e.g. [\"actor\", \"voice_actor\"] for cast. \
                    Omit or leave empty for every cached kind."
        )]
        kinds: Option<Vec<String>>,
        #[graphql(
            desc = "Maximum credits to return; defaults to 15 and clamps to 0 through 50.",
            default = 15
        )]
        limit: i32,
    ) -> GqlResult<Vec<TitleCreditPayload>> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title_id = self.id.as_ref();
            let credits = app
                .title_credits(&actor, title_id, kinds.as_deref(), i64::from(limit))
                .await
                .map_err(to_gql_error)?;
            Ok(credits
                .into_iter()
                .map(|credit| from_title_credit(&app, title_id, credit))
                .collect())
        })
        .await
    }

    /// Discovery items related to this title, limited to the requested count.
    async fn more_like_this(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Maximum related items to return; defaults to 12 and clamps to 0 through 100.",
            default = 12
        )]
        limit: i32,
    ) -> GqlResult<Vec<DiscoveryItemPayload>> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let items = app
                .title_more_like_this(&actor, self.id.as_ref(), i64::from(limit.clamp(0, 100)))
                .await
                .map_err(to_gql_error)?;
            Ok(items
                .into_iter()
                .map(|item| from_discovery_item(&app, item))
                .collect())
        })
        .await
    }

    /// Absolute root-folder path for this title; requires view permission on its library.
    async fn root_folder_path(&self, ctx: &Context<'_>) -> GqlResult<String> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            app.require_library_permission(
                &actor,
                self.library_id.as_ref(),
                scryer_domain::LibraryPermission::View,
            )
            .await
            .map_err(to_gql_error)?;
            app.title_root_folder_path_for_parts(
                self.root_folder_id.as_ref(),
                self.library_id.as_ref(),
                &self.facet.into_domain(),
            )
            .await
            .map_err(to_gql_error)
        })
        .await
    }

    /// Explicit metadata-language override, or null when the global default is inherited.
    async fn metadata_language_override(&self, ctx: &Context<'_>) -> GqlResult<Option<String>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            return loaders
                .title_metadata_language_override
                .load_one(self.id.to_string())
                .await;
        }
        Box::pin(async move {
            app_from_ctx(ctx)?
                .title_metadata_language_override(self.id.as_ref())
                .await
                .map_err(to_gql_error)
        })
        .await
    }

    /// Metadata language after applying title and library overrides.
    async fn effective_metadata_language(&self, ctx: &Context<'_>) -> GqlResult<String> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            if let Some(language) = loaders
                .title_metadata_language_override
                .load_one(self.id.to_string())
                .await?
            {
                return Ok(language);
            }
            if let Some(language) = loaders
                .library_metadata_language_override
                .load_one(self.library_id.to_string())
                .await?
            {
                return Ok(language);
            }
            return Ok(loaders
                .global_metadata_language
                .load_one("metadata-language".to_owned())
                .await?
                .unwrap_or_else(|| "eng".to_owned()));
        }
        Box::pin(async move {
            app_from_ctx(ctx)?
                .effective_metadata_language_for_title(self.id.as_ref())
                .await
                .map_err(to_gql_error)
        })
        .await
    }

    /// Whether this title inherits metadata language from its library or the global default.
    async fn inherits_metadata_language(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            return Ok(loaders
                .title_metadata_language_override
                .load_one(self.id.to_string())
                .await?
                .is_none());
        }
        Box::pin(async move {
            Ok(app_from_ctx(ctx)?
                .title_metadata_language_override(self.id.as_ref())
                .await
                .map_err(to_gql_error)?
                .is_none())
        })
        .await
    }

    /// Explicit season-folder override, or null when library/facet settings are inherited.
    async fn use_season_folders_override(&self) -> Option<bool> {
        (self.facet != MediaFacetValue::Movie)
            .then_some(self.use_season_folders)
            .flatten()
    }

    /// Whether this title uses season folders after applying inheritance.
    async fn effective_use_season_folders(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        if self.facet == MediaFacetValue::Movie {
            return Ok(true);
        }
        if let Some(use_season_folders) = self.use_season_folders {
            return Ok(use_season_folders);
        }
        if let Some(loaders) = loaders_from_ctx(ctx) {
            if let Some(use_season_folders) = loaders
                .library_use_season_folders_override
                .load_one(self.library_id.to_string())
                .await?
            {
                return Ok(use_season_folders);
            }
            return Ok(loaders
                .facet_use_season_folders_override
                .load_one(self.facet.into_domain().as_str().to_owned())
                .await?
                .unwrap_or(true));
        }
        Box::pin(async move {
            app_from_ctx(ctx)?
                .effective_use_season_folders_for_title(self.id.as_ref())
                .await
                .map_err(to_gql_error)
        })
        .await
    }

    /// Whether this title inherits the season-folder setting from its library or facet.
    async fn inherits_use_season_folders(&self) -> bool {
        self.facet == MediaFacetValue::Movie || self.use_season_folders.is_none()
    }

    /// Filler policy after applying the title override or library/facet default.
    async fn effective_filler_policy(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<FillerPolicyValue>> {
        if self.facet != MediaFacetValue::Anime {
            return Ok(None);
        }
        if let Some(policy) = self.filler_policy {
            return Ok(Some(policy));
        }
        app_from_ctx(ctx)?
            .effective_filler_policy_for_title(self.id.as_ref())
            .await
            .map(|policy| policy.and_then(|policy| FillerPolicyValue::from_app_str(&policy)))
            .map_err(to_gql_error)
    }

    /// Recap policy after applying the title override or library/facet default.
    async fn effective_recap_policy(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<RecapPolicyValue>> {
        if self.facet != MediaFacetValue::Anime {
            return Ok(None);
        }
        if let Some(policy) = self.recap_policy {
            return Ok(Some(policy));
        }
        app_from_ctx(ctx)?
            .effective_recap_policy_for_title(self.id.as_ref())
            .await
            .map(|policy| policy.and_then(|policy| RecapPolicyValue::from_app_str(&policy)))
            .map_err(to_gql_error)
    }

    /// Title-specific required audio-language override, or null when the facet setting is inherited.
    async fn required_audio_languages_override(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<Vec<String>>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            return loaders
                .required_audio_override
                .load_one(self.id.to_string())
                .await;
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            app.load_title_required_audio_override(self.id.as_ref())
                .await
                .map_err(to_gql_error)
        })
        .await
    }

    /// Configured audio requirements after title, library, and facet inheritance.
    /// The `original` token remains unresolved in this configuration field.
    async fn effective_required_audio_languages(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<String>> {
        Box::pin(async move {
            let override_languages = if let Some(loaders) = loaders_from_ctx(ctx) {
                loaders
                    .required_audio_override
                    .load_one(self.id.to_string())
                    .await?
            } else {
                app_from_ctx(ctx)?
                    .load_title_required_audio_override(self.id.as_ref())
                    .await
                    .map_err(to_gql_error)?
            };
            if let Some(languages) = override_languages {
                return Ok(languages);
            }
            app_from_ctx(ctx)?
                .resolve_required_audio_languages(
                    None,
                    Some(self.library_id.as_ref()),
                    Some(title_scope_from_facet(self.facet).as_scope_id()),
                )
                .await
                .map_err(to_gql_error)
        })
        .await
    }

    /// Whether the title uses the facet-level required audio-language setting without an override.
    async fn inherits_required_audio_languages(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            return Ok(loaders
                .required_audio_override
                .load_one(self.id.to_string())
                .await?
                .is_none());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            Ok(app
                .load_title_required_audio_override(self.id.as_ref())
                .await?
                .is_none())
        })
        .await
    }

    /// Collections belonging to this title, or an empty list when none are available.
    async fn collections(&self, ctx: &Context<'_>) -> GqlResult<Vec<CollectionPayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let collections = loaders
                .collections_for_title
                .load_one(self.id.to_string())
                .await?
                .unwrap_or_default();
            return Ok(collections.into_iter().map(from_collection).collect());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let collections = app
                .list_collections(&actor, self.id.as_ref())
                .await
                .map_err(to_gql_error)?;
            Ok(collections.into_iter().map(from_collection).collect())
        })
        .await
    }

    /// Series-movie links for this title, or an empty list when none exist.
    async fn series_movie_links(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<SeriesMovieLinkPayload>> {
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let links = loaders
                .series_movie_links_for_title
                .load_one(self.id.to_string())
                .await?
                .unwrap_or_default();
            return Ok(links
                .into_iter()
                .map(|link| from_series_movie_link(&image_app, link))
                .collect());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let links = app
                .list_series_movie_links(&actor, self.id.as_ref())
                .await
                .map_err(to_gql_error)?;
            Ok(links
                .into_iter()
                .map(|link| from_series_movie_link(&app, link))
                .collect())
        })
        .await
    }

    /// Media files for this title that are visible to the caller, or an empty list when none are available.
    async fn media_files(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleMediaFilePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let files = loaders
                .media_files_for_title
                .load_one(self.id.to_string())
                .await?
                .unwrap_or_default();
            return Ok(files.into_iter().map(from_title_media_file).collect());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let files = app
                .list_title_media_files(&actor, self.id.as_ref())
                .await
                .map_err(to_gql_error)?;
            Ok(files.into_iter().map(from_title_media_file).collect())
        })
        .await
    }

    /// Wanted scopes for this title, optionally filtered by status and returned as a page without a total count.
    async fn wanted_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Optional wanted status filter; omitted includes all statuses.")]
        status: Option<WantedStatusValue>,
        #[graphql(
            desc = "Page size; defaults to 50 and clamps to 1 through 300.",
            default = 50
        )]
        limit: i32,
        #[graphql(
            desc = "Zero-based page offset; defaults to 0 and negative values become 0.",
            default = 0
        )]
        offset: i32,
    ) -> GqlResult<WantedItemsPagePayload> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let limit = relation_page_limit(limit);
            let offset = relation_page_offset(offset);
            let (items, _total_count) = app
                .list_acquisition_scope_states(
                    &actor,
                    AcquisitionScopeStatesQuery {
                        statuses: status
                            .map(|value| value.as_str().to_string())
                            .into_iter()
                            .collect(),
                        media_types: Vec::new(),
                        title_id: Some(self.id.to_string()),
                        library_ids: Vec::new(),
                        title_search: None,
                        latest_decision_codes: Vec::new(),
                        limit: i64::from(limit),
                        offset: i64::from(offset),
                    },
                )
                .await
                .map_err(to_gql_error)?;
            Ok(WantedItemsPagePayload {
                items: items
                    .into_iter()
                    .map(from_wanted_item)
                    .collect::<scryer_application::AppResult<Vec<_>>>()
                    .map_err(to_gql_error)?,
            })
        })
        .await
    }

    /// Release decisions for this title with a total count and continuation flag.
    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Page size; defaults to 50 and clamps to 1 through 300.",
            default = 50
        )]
        limit: i64,
        #[graphql(
            desc = "Zero-based page offset; defaults to 0 and negative values become 0.",
            default = 0
        )]
        offset: i32,
    ) -> GqlResult<ReleaseDecisionsPagePayload> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let limit = relation_page_limit(limit.min(i64::from(i32::MAX)) as i32);
            let offset = relation_page_offset(offset);
            let (decisions, total_count) = app
                .list_release_decisions_page(
                    &actor,
                    ReleaseDecisionsQuery {
                        wanted_item_id: None,
                        title_id: Some(self.id.to_string()),
                        limit: i64::from(limit),
                        offset: i64::from(offset),
                    },
                )
                .await
                .map_err(to_gql_error)?;
            let items = decisions
                .into_iter()
                .map(from_release_decision)
                .collect::<scryer_application::AppResult<Vec<_>>>()
                .map_err(to_gql_error)?;
            let has_more = i64::from(offset).saturating_add(items.len() as i64) < total_count;
            Ok(ReleaseDecisionsPagePayload {
                items,
                total_count,
                has_more,
            })
        })
        .await
    }

    /// Queue items associated with this title after the supplied activity filters are applied.
    async fn download_queue_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Include all queue activity instead of only active activity; defaults to false."
        )]
        include_all_activity: Option<bool>,
        #[graphql(desc = "Restrict results to history entries; defaults to false.")]
        include_history_only: Option<bool>,
        #[graphql(desc = "Include import activity; defaults to false.")]
        include_import_activity: Option<bool>,
        #[graphql(desc = "Activity-state filter; omitted defaults to ALL.")]
        activity_filter: Option<DownloadActivityFilterValue>,
    ) -> GqlResult<Vec<DownloadQueueItemPayload>> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let items = app
                .list_download_queue_for_title(
                    &actor,
                    self.id.as_ref(),
                    include_all_activity.unwrap_or(false),
                    include_history_only.unwrap_or(false),
                    include_import_activity.unwrap_or(false),
                    activity_filter
                        .unwrap_or(DownloadActivityFilterValue::All)
                        .into_application(),
                )
                .await
                .map_err(to_gql_error)?;
            Ok(items.into_iter().map(from_download_queue_item).collect())
        })
        .await
    }
}

#[ComplexObject]
impl CollectionPayload {
    /// Size in bytes of the media file associated with this collection, or null when unavailable.
    async fn file_size_bytes(&self, ctx: &Context<'_>) -> GqlResult<Option<Long>> {
        let Some(ordered_path) = self.ordered_path.clone() else {
            return Ok(None);
        };
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let size = app
                .collection_media_size_bytes(&actor, self.title_id.as_ref(), &ordered_path)
                .await
                .map_err(to_gql_error)?;
            Ok(size.map(Long::from))
        })
        .await
    }

    /// Owned-vs-total episode progress for this collection, populated when requested.
    async fn episodes_owned(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .collection_episode_progress
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?;
            return Ok(summary.map(|summary| summary.owned_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title_id = self.title_id.to_string();
            let collection_id = self.id.to_string();
            let summaries = app
                .list_collection_episode_progress_summaries(&actor, std::slice::from_ref(&title_id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.collection_id == collection_id)
                .map(|summary| summary.owned_episodes))
        })
        .await
    }

    /// Monitored episode count for this collection, populated when requested.
    async fn episodes_monitored(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .collection_episode_progress
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?;
            return Ok(summary.map(|summary| summary.monitored_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title_id = self.title_id.to_string();
            let collection_id = self.id.to_string();
            let summaries = app
                .list_collection_episode_progress_summaries(&actor, std::slice::from_ref(&title_id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.collection_id == collection_id)
                .map(|summary| summary.monitored_episodes))
        })
        .await
    }

    /// Total countable episode count for this collection, populated when requested.
    async fn episodes_total(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .collection_episode_progress
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?;
            return Ok(summary.map(|summary| summary.total_episodes));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title_id = self.title_id.to_string();
            let collection_id = self.id.to_string();
            let summaries = app
                .list_collection_episode_progress_summaries(&actor, std::slice::from_ref(&title_id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.collection_id == collection_id)
                .map(|summary| summary.total_episodes))
        })
        .await
    }

    /// Total episode records in this collection including uncountable placeholders (TBA/undated), populated when requested.
    async fn episode_records_total(&self, ctx: &Context<'_>) -> GqlResult<Option<i64>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let summary = loaders
                .collection_episode_progress
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?;
            return Ok(summary.map(|summary| summary.episode_records_total));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title_id = self.title_id.to_string();
            let collection_id = self.id.to_string();
            let summaries = app
                .list_collection_episode_progress_summaries(&actor, std::slice::from_ref(&title_id))
                .await
                .map_err(to_gql_error)?;
            Ok(summaries
                .into_iter()
                .find(|summary| summary.collection_id == collection_id)
                .map(|summary| summary.episode_records_total))
        })
        .await
    }

    /// Parent title for this collection, or null if the title is no longer available to the caller.
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders.title.load_one(self.title_id.to_string()).await?;
            return Ok(title.map(|title| from_title(&image_app, title)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(|title| from_title(&app, title));
            Ok(title)
        })
        .await
    }

    /// Episodes belonging to this collection, or an empty list when none are available.
    async fn episodes(&self, ctx: &Context<'_>) -> GqlResult<Vec<EpisodePayload>> {
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let episodes = loaders
                .episodes_for_collection
                .load_one(self.id.to_string())
                .await?
                .unwrap_or_default();
            return Ok(episodes
                .into_iter()
                .map(|episode| from_episode(&image_app, episode))
                .collect());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let episodes = app
                .list_episodes(&actor, self.id.as_ref())
                .await
                .map_err(to_gql_error)?;
            Ok(episodes
                .into_iter()
                .map(|episode| from_episode(&app, episode))
                .collect())
        })
        .await
    }
}

#[ComplexObject]
impl EpisodePayload {
    /// Provider-native playback links for this episode, when an exact catalog mapping exists.
    async fn playback_links(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<MediaServerPlaybackLinkPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.media_server_playback_links(
            &actor,
            scryer_domain::MediaServerPlaybackEntityKind::Episode,
            self.id.as_ref(),
        )
        .await
        .map(|links| {
            links
                .into_iter()
                .map(from_media_server_playback_link)
                .collect()
        })
        .map_err(to_gql_error)
    }

    /// Parent title for this episode, or null if it is no longer available to the caller.
    async fn parent_title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders.title.load_one(self.title_id.to_string()).await?;
            return Ok(title.map(|title| from_title(&image_app, title)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(|title| from_title(&app, title));
            Ok(title)
        })
        .await
    }

    /// Containing collection when the episode has one, otherwise null.
    async fn collection(&self, ctx: &Context<'_>) -> GqlResult<Option<CollectionPayload>> {
        let Some(collection_id) = self.collection_id.as_deref() else {
            return Ok(None);
        };
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let collection = loaders
                .collection
                .load_one(collection_id.to_string())
                .await?;
            return Ok(collection.map(from_collection));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let collection = app
                .get_collection(&actor, collection_id)
                .await
                .map_err(to_gql_error)?
                .map(from_collection);
            Ok(collection)
        })
        .await
    }

    /// Acquisition state for this episode, or null when no wanted state exists.
    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let state = loaders
                .title_wanted_item
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?;
            return state
                .map(from_wanted_item)
                .transpose()
                .map_err(to_gql_error);
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let wanted_item = app
                .get_title_wanted_item(&actor, self.title_id.as_ref(), Some(self.id.as_ref()))
                .await
                .map_err(to_gql_error)?
                .map(from_wanted_item)
                .transpose()
                .map_err(to_gql_error)?;
            Ok(wanted_item)
        })
        .await
    }

    /// Media readiness for this episode, including missing and unmonitored states.
    async fn media_availability(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<EpisodeMediaAvailabilityPayload> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let availability = loaders
                .episode_media_availability
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?;
            return Ok(availability
                .map(from_episode_media_availability)
                .unwrap_or_else(|| fallback_episode_media_availability(self.monitored)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let availability = app
                .list_episode_media_availability(&actor, std::slice::from_ref(&*self.title_id))
                .await
                .map_err(to_gql_error)?
                .into_iter()
                .find(|summary| summary.episode_id == self.id.as_ref());
            Ok(availability
                .map(from_episode_media_availability)
                .unwrap_or_else(|| fallback_episode_media_availability(self.monitored)))
        })
        .await
    }

    /// Media files associated with this episode, or an empty list when none are available.
    async fn media_files(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleMediaFilePayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let files = loaders
                .episode_media_files
                .load_one((self.title_id.to_string(), self.id.to_string()))
                .await?
                .unwrap_or_default();
            return Ok(files.into_iter().map(from_title_media_file).collect());
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let files = app
                .list_episode_media_files(&actor, self.title_id.as_ref(), self.id.as_ref())
                .await
                .map_err(to_gql_error)?;
            Ok(files.into_iter().map(from_title_media_file).collect())
        })
        .await
    }
}

#[ComplexObject]
impl TitleMediaFilePayload {
    /// Parent title for this media file, or null if it is no longer available to the caller.
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders.title.load_one(self.title_id.to_string()).await?;
            return Ok(title.map(|title| from_title(&image_app, title)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(|title| from_title(&app, title));
            Ok(title)
        })
        .await
    }

    /// Episode represented by this file, or null for title-level media.
    async fn episode(&self, ctx: &Context<'_>) -> GqlResult<Option<EpisodePayload>> {
        let Some(episode_id) = self.episode_id.as_deref() else {
            return Ok(None);
        };
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let episode = loaders.episode.load_one(episode_id.to_string()).await?;
            return Ok(episode.map(|episode| from_episode(&image_app, episode)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let episode = app
                .get_episode(&actor, episode_id)
                .await
                .map_err(to_gql_error)?
                .map(|episode| from_episode(&app, episode));
            Ok(episode)
        })
        .await
    }
}

#[ComplexObject]
impl WantedItemPayload {
    /// Title targeted by this wanted scope, or null when it is no longer available to the caller.
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders.title.load_one(self.title_id.to_string()).await?;
            return Ok(title.map(|title| from_title(&image_app, title)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(|title| from_title(&app, title));
            Ok(title)
        })
        .await
    }

    /// Collection targeted by this scope, or null when the scope is not collection-based.
    async fn collection(&self, ctx: &Context<'_>) -> GqlResult<Option<CollectionPayload>> {
        let Some(collection_id) = self.collection_id.as_deref() else {
            return Ok(None);
        };
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let collection = loaders
                .collection
                .load_one(collection_id.to_string())
                .await?;
            return Ok(collection.map(from_collection));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let collection = app
                .get_collection(&actor, collection_id)
                .await
                .map_err(to_gql_error)?
                .map(from_collection);
            Ok(collection)
        })
        .await
    }

    /// Episode targeted by this scope, or null for title, collection, and series-movie scopes.
    async fn episode(&self, ctx: &Context<'_>) -> GqlResult<Option<EpisodePayload>> {
        let Some(episode_id) = self.episode_id.as_deref() else {
            return Ok(None);
        };
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let episode = loaders.episode.load_one(episode_id.to_string()).await?;
            return Ok(episode.map(|episode| from_episode(&image_app, episode)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let episode = app
                .get_episode(&actor, episode_id)
                .await
                .map_err(to_gql_error)?
                .map(|episode| from_episode(&app, episode));
            Ok(episode)
        })
        .await
    }

    /// Release decisions for this wanted scope with a total count and continuation flag.
    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Page size; defaults to 50 and clamps to 1 through 300.",
            default = 50
        )]
        limit: i64,
        #[graphql(
            desc = "Zero-based page offset; defaults to 0 and negative values become 0.",
            default = 0
        )]
        offset: i32,
    ) -> GqlResult<ReleaseDecisionsPagePayload> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let limit = relation_page_limit(limit.min(i64::from(i32::MAX)) as i32);
            let offset = relation_page_offset(offset);
            let (decisions, total_count) = app
                .list_release_decisions_page(
                    &actor,
                    ReleaseDecisionsQuery {
                        wanted_item_id: Some(self.id.to_string()),
                        title_id: None,
                        limit: i64::from(limit),
                        offset: i64::from(offset),
                    },
                )
                .await
                .map_err(to_gql_error)?;
            let items = decisions
                .into_iter()
                .map(from_release_decision)
                .collect::<scryer_application::AppResult<Vec<_>>>()
                .map_err(to_gql_error)?;
            let has_more = i64::from(offset).saturating_add(items.len() as i64) < total_count;
            Ok(ReleaseDecisionsPagePayload {
                items,
                total_count,
                has_more,
            })
        })
        .await
    }

    /// Pending releases for this wanted scope with a total count and continuation flag.
    async fn pending_releases(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Page size; defaults to 50 and clamps to 1 through 300.",
            default = 50
        )]
        limit: i32,
        #[graphql(
            desc = "Zero-based page offset; defaults to 0 and negative values become 0.",
            default = 0
        )]
        offset: i32,
    ) -> GqlResult<PendingReleasesPayload> {
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let limit = relation_page_limit(limit);
            let offset = relation_page_offset(offset);
            let (releases, total) = app
                .list_pending_releases_for_wanted_item_page(
                    &actor,
                    self.id.as_ref(),
                    i64::from(limit),
                    i64::from(offset),
                )
                .await
                .map_err(to_gql_error)?;
            let total_count = total.min(i64::from(i32::MAX)) as i32;
            let items = releases
                .into_iter()
                .map(from_pending_release)
                .collect::<Vec<_>>();
            let has_more =
                i64::from(offset).saturating_add(items.len() as i64) < i64::from(total_count);
            Ok(PendingReleasesPayload {
                items,
                has_more,
                total_count,
            })
        })
        .await
    }
}

#[ComplexObject]
impl ReleaseDecisionPayload {
    /// Title associated with this release decision, or null when it is no longer available to the caller.
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders.title.load_one(self.title_id.to_string()).await?;
            return Ok(title.map(|title| from_title(&image_app, title)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(|title| from_title(&app, title));
            Ok(title)
        })
        .await
    }

    /// Wanted scope evaluated by this decision, or null when it is no longer available to the caller.
    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let item = loaders
                .wanted_item
                .load_one(self.wanted_item_id.to_string())
                .await?;
            return item.map(from_wanted_item).transpose().map_err(to_gql_error);
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let item = app
                .get_wanted_item(&actor, self.wanted_item_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(from_wanted_item)
                .transpose()
                .map_err(to_gql_error)?;
            Ok(item)
        })
        .await
    }
}

#[ComplexObject]
impl DownloadQueueItemPayload {
    /// Acquisition scope for this queue item, or its episode scope when no more specific scope is available.
    async fn queue_scope(&self, ctx: &Context<'_>) -> GqlResult<Option<QueueDownloadScopePayload>> {
        Box::pin(async move {
            let client_type = self.client_type.trim();
            let download_client_item_id = self.download_client_item_id.trim();
            if client_type.is_empty() || download_client_item_id.is_empty() {
                return Ok(self
                    .episode_id
                    .as_ref()
                    .map(|episode_id| QueueDownloadScopePayload::episode(episode_id.clone())));
            }

            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let client_id = self.client_id.trim();
            let client_id = if client_id.is_empty() {
                None
            } else {
                Some(client_id)
            };
            let scope = app
                .find_download_queue_scope(&actor, client_id, client_type, download_client_item_id)
                .await
                .map_err(to_gql_error)?;

            Ok(scope.map(from_submission_scope).or_else(|| {
                self.episode_id
                    .as_ref()
                    .map(|episode_id| QueueDownloadScopePayload::episode(episode_id.clone()))
            }))
        })
        .await
    }

    /// Caller-visible matched title, or null when the queue item is unmatched or inaccessible.
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let Some(title_id) = self.title_id.as_deref() else {
            return Ok(None);
        };
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders
                .title_for_management
                .load_one(title_id.to_string())
                .await?;
            return Ok(title.map(|title| from_title(&image_app, title)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title_for_management(&actor, title_id)
                .await
                .map_err(to_gql_error)?
                .map(|title| from_title(&app, title));
            Ok(title)
        })
        .await
    }
}

#[ComplexObject]
impl PendingReleasePayload {
    /// Title associated with this pending release, or null when it is no longer available to the caller.
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let image_app = app_from_ctx(ctx)?;
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let title = loaders
                .title_for_management
                .load_one(self.title_id.to_string())
                .await?;
            return Ok(title.map(|title| from_title(&image_app, title)));
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let title = app
                .get_title_for_management(&actor, self.title_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(|title| from_title(&app, title));
            Ok(title)
        })
        .await
    }

    /// Wanted scope holding this release, or null when it is no longer available to the caller.
    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        if let Some(loaders) = loaders_from_ctx(ctx) {
            let item = loaders
                .wanted_item_for_management
                .load_one(self.wanted_item_id.to_string())
                .await?;
            return item.map(from_wanted_item).transpose().map_err(to_gql_error);
        }
        Box::pin(async move {
            let app = app_from_ctx(ctx)?;
            let actor = actor_from_ctx(ctx)?;
            let wanted_item = app
                .get_wanted_item_for_management(&actor, self.wanted_item_id.as_ref())
                .await
                .map_err(to_gql_error)?
                .map(from_wanted_item)
                .transpose()
                .map_err(to_gql_error)?;
            Ok(wanted_item)
        })
        .await
    }
}
