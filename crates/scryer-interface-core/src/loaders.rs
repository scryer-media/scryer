//! Request-scoped dataloaders for relationship and enrichment fields.
//!
//! Freshness contract: a [`RequestLoaders`] instance lives for exactly one
//! GraphQL HTTP request — it is built in the axum handler after the actor is
//! resolved and injected via `request.data(...)`. The dataloader cache is
//! therefore a per-request consistency snapshot, never a cross-request cache;
//! it needs no TTL and no invalidation because it dies with the request.
//!
//! The WebSocket subscription path deliberately gets NO loaders: WS data is
//! per-connection, and a loader there would cache for the socket lifetime.
//! Resolvers must treat the absence of `RequestLoaders` in context as normal
//! and fall back to direct application calls.
//!
//! Permission semantics: every batch application method is actor-scoped and
//! silently drops ids the actor cannot see, so a missing key in a loader
//! result means "absent or not visible" — exactly the `None`/empty semantics
//! the nullable relationship fields already have.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_graphql::dataloader::{DataLoader, Loader};
use scryer_application::{
    AcquisitionScopeState, AppUseCase, CollectionEpisodeProgressSummary,
    CollectionMediaSizeSummary, EpisodeMediaAvailability, EpisodeMediaSizeSummary,
    PrimaryCollectionSummary, TitleEpisodeProgressSummary, TitleMediaFile, TitleMediaSizeSummary,
    TitleMovieMediaSummary, TitleQualitySummary, TitleRatingSummary,
};
use scryer_domain::{Collection, Episode, Library, SeriesMovieLink, Title, User};

use crate::to_gql_error;

type GqlError = async_graphql::Error;

/// Shared per-request context captured by every loader.
struct LoaderCtx {
    app: AppUseCase,
    actor: User,
}

/// Coalescing window for batching key lookups within one request.
const LOAD_DELAY: Duration = Duration::from_millis(1);

macro_rules! loader {
    ($name:ident, $key:ty, $value:ty, |$ctx:ident, $keys:ident| $body:expr) => {
        pub struct $name(Arc<LoaderCtx>);

        impl Loader<$key> for $name {
            type Value = $value;
            type Error = GqlError;

            async fn load(
                &self,
                $keys: &[$key],
            ) -> Result<HashMap<$key, Self::Value>, Self::Error> {
                let $ctx = &*self.0;
                $body
            }
        }
    };
}

/// Index a `Vec<T>` into a map keyed by an id extractor.
fn by_id<K, T>(items: Vec<T>, key: impl Fn(&T) -> K) -> HashMap<K, T>
where
    K: std::hash::Hash + Eq,
{
    items.into_iter().map(|item| (key(&item), item)).collect()
}

/// Distinct first elements of pair keys, preserving first-seen order.
fn dedup_first(keys: &[(String, String)]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    keys.iter()
        .filter(|(first, _)| seen.insert(first.clone()))
        .map(|(first, _)| first.clone())
        .collect()
}

loader!(TitleLoader, String, Title, |ctx, keys| {
    let titles = ctx
        .app
        .get_titles_by_ids(&ctx.actor, keys)
        .await
        .map_err(to_gql_error)?;
    Ok(by_id(titles, |t| t.id.clone()))
});

loader!(TitleForManagementLoader, String, Title, |ctx, keys| {
    let titles = ctx
        .app
        .get_titles_by_ids_for_management(&ctx.actor, keys)
        .await
        .map_err(to_gql_error)?;
    Ok(by_id(titles, |t| t.id.clone()))
});

loader!(LibraryLoader, String, Library, |ctx, keys| {
    // One permission-scoped listing covers every key; libraries are few.
    let libraries = ctx
        .app
        .list_libraries_for_permission(&ctx.actor, None, scryer_domain::LibraryPermission::View)
        .await
        .map_err(to_gql_error)?;
    let mut map = by_id(libraries, |l| l.id.clone());
    map.retain(|id, _| keys.contains(id));
    Ok(map)
});

loader!(CollectionLoader, String, Collection, |ctx, keys| {
    let collections = ctx
        .app
        .get_collections_by_ids(&ctx.actor, keys)
        .await
        .map_err(to_gql_error)?;
    Ok(by_id(collections, |c| c.id.clone()))
});

loader!(EpisodeLoader, String, Episode, |ctx, keys| {
    let episodes = ctx
        .app
        .get_episodes_by_ids(&ctx.actor, keys)
        .await
        .map_err(to_gql_error)?;
    Ok(by_id(episodes, |e| e.id.clone()))
});

loader!(
    MediaFilesForTitleLoader,
    String,
    Vec<TitleMediaFile>,
    |ctx, keys| {
        ctx.app
            .list_media_files_for_titles(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    CollectionsForTitleLoader,
    String,
    Vec<Collection>,
    |ctx, keys| {
        // Two batched calls: resolve visible titles, then their collections.
        let titles = ctx
            .app
            .get_titles_by_ids(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        ctx.app
            .list_collections_for_titles(&ctx.actor, &titles)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    EpisodesForCollectionLoader,
    String,
    Vec<Episode>,
    |ctx, keys| {
        ctx.app
            .list_episodes_for_collections(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    RequiredAudioOverrideLoader,
    String,
    Vec<String>,
    |ctx, keys| {
        ctx.app
            .load_title_required_audio_overrides(keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    TitleMetadataLanguageOverrideLoader,
    String,
    String,
    |ctx, keys| {
        ctx.app
            .load_title_metadata_language_overrides(keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    LibraryMetadataLanguageOverrideLoader,
    String,
    String,
    |ctx, keys| {
        ctx.app
            .load_library_metadata_language_overrides(keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    LibraryUseSeasonFoldersOverrideLoader,
    String,
    bool,
    |ctx, keys| {
        ctx.app
            .load_use_season_folders_overrides(keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    FacetUseSeasonFoldersOverrideLoader,
    String,
    bool,
    |ctx, keys| {
        ctx.app
            .load_use_season_folders_overrides(keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(GlobalMetadataLanguageLoader, String, String, |ctx, keys| {
    let language = ctx.app.global_metadata_language().await;
    Ok(keys
        .iter()
        .cloned()
        .map(|key| (key, language.clone()))
        .collect())
});

loader!(
    PrimaryCollectionSummaryLoader,
    String,
    PrimaryCollectionSummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_primary_collection_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

loader!(
    MediaSizeSummaryLoader,
    String,
    TitleMediaSizeSummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_title_media_size_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

loader!(
    CollectionMediaSizeSummaryLoader,
    (String, String),
    CollectionMediaSizeSummary,
    |ctx, keys| {
        let title_ids = dedup_first(keys);
        let summaries = ctx
            .app
            .list_collection_media_size_summaries(&ctx.actor, &title_ids)
            .await
            .map_err(to_gql_error)?;
        let mut summaries = by_id(summaries, |summary| {
            (summary.title_id.clone(), summary.collection_id.clone())
        });
        summaries.retain(|key, _| keys.contains(key));
        Ok(summaries)
    }
);

loader!(
    EpisodeMediaSizeSummaryLoader,
    (String, String),
    EpisodeMediaSizeSummary,
    |ctx, keys| {
        let title_ids = dedup_first(keys);
        let summaries = ctx
            .app
            .list_episode_media_size_summaries(&ctx.actor, &title_ids)
            .await
            .map_err(to_gql_error)?;
        let mut summaries = by_id(summaries, |summary| {
            (summary.title_id.clone(), summary.episode_id.clone())
        });
        summaries.retain(|key, _| keys.contains(key));
        Ok(summaries)
    }
);

loader!(
    QualitySummaryLoader,
    String,
    TitleQualitySummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_title_quality_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

loader!(
    EffectiveQualitySummaryLoader,
    String,
    TitleQualitySummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_title_effective_quality_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

loader!(
    EpisodeProgressSummaryLoader,
    String,
    TitleEpisodeProgressSummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_title_episode_progress_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

// Keyed (title_id, collection_id): the batch app method takes title ids while
// the summaries come back keyed by collection id.
loader!(
    CollectionEpisodeProgressLoader,
    (String, String),
    CollectionEpisodeProgressSummary,
    |ctx, keys| {
        let title_ids = dedup_first(keys);
        let summaries = ctx
            .app
            .list_collection_episode_progress_summaries(&ctx.actor, &title_ids)
            .await
            .map_err(to_gql_error)?;
        let by_collection: HashMap<String, CollectionEpisodeProgressSummary> = summaries
            .into_iter()
            .map(|summary| (summary.collection_id.clone(), summary))
            .collect();
        Ok(keys
            .iter()
            .filter_map(|key| {
                by_collection
                    .get(&key.1)
                    .map(|summary| (key.clone(), summary.clone()))
            })
            .collect())
    }
);

loader!(
    SeriesMovieLinksForTitleLoader,
    String,
    Vec<SeriesMovieLink>,
    |ctx, keys| {
        ctx.app
            .list_series_movie_links_for_titles(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    WantedItemLoader,
    String,
    AcquisitionScopeState,
    |ctx, keys| {
        let items = ctx
            .app
            .get_wanted_items_by_ids(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(items, |item| item.id.clone()))
    }
);

loader!(
    WantedItemForManagementLoader,
    String,
    AcquisitionScopeState,
    |ctx, keys| {
        let items = ctx
            .app
            .get_wanted_items_by_ids_for_management(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(items, |item| item.id.clone()))
    }
);

// Keyed (title_id, episode_id): resolves the episode-scoped acquisition state.
loader!(
    TitleWantedItemLoader,
    (String, String),
    AcquisitionScopeState,
    |ctx, keys| {
        let title_ids = dedup_first(keys);
        let states = ctx
            .app
            .get_title_wanted_items_for_titles(&ctx.actor, &title_ids)
            .await
            .map_err(to_gql_error)?;
        Ok(states
            .into_iter()
            .filter_map(|state| {
                state
                    .episode_id
                    .clone()
                    .map(|episode_id| ((state.title_id.clone(), episode_id), state))
            })
            .collect())
    }
);

// Keyed (title_id, episode_id): compact primary-file availability for collapsed
// episode rows. This deliberately does not hydrate TitleMediaFile values.
loader!(
    EpisodeMediaAvailabilityLoader,
    (String, String),
    EpisodeMediaAvailability,
    |ctx, keys| {
        let title_ids = dedup_first(keys);
        let availability = ctx
            .app
            .list_episode_media_availability(&ctx.actor, &title_ids)
            .await
            .map_err(to_gql_error)?;
        Ok(availability
            .into_iter()
            .map(|summary| {
                (
                    (summary.title_id.clone(), summary.episode_id.clone()),
                    summary,
                )
            })
            .collect())
    }
);

// Keyed (title_id, episode_id): one scoped-files fetch per distinct title.
loader!(
    EpisodeMediaFilesLoader,
    (String, String),
    Vec<TitleMediaFile>,
    |ctx, keys| {
        let mut episode_ids_by_title: HashMap<&str, Vec<String>> = HashMap::new();
        for (title_id, episode_id) in keys {
            episode_ids_by_title
                .entry(title_id.as_str())
                .or_default()
                .push(episode_id.clone());
        }
        let mut map = HashMap::new();
        for (title_id, episode_ids) in episode_ids_by_title {
            let files_by_episode = ctx
                .app
                .list_episode_media_files_for_title(&ctx.actor, title_id, &episode_ids)
                .await
                .map_err(to_gql_error)?;
            for (episode_id, files) in files_by_episode {
                map.insert((title_id.to_string(), episode_id), files);
            }
        }
        Ok(map)
    }
);

loader!(RatingsLoader, String, TitleRatingSummary, |ctx, keys| {
    let ratings = ctx
        .app
        .list_title_ratings(&ctx.actor, keys)
        .await
        .map_err(to_gql_error)?;
    Ok(ratings.into_iter().collect())
});

loader!(
    ListMembershipsForTitleLoader,
    String,
    Vec<scryer_application::lists::TitleListMembership>,
    |ctx, keys| {
        ctx.app
            .public_list_memberships_for_titles(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)
    }
);

loader!(
    MovieMediaSummaryLoader,
    String,
    TitleMovieMediaSummary,
    |ctx, keys| {
        let summaries = ctx
            .app
            .list_title_movie_media_summaries(&ctx.actor, keys)
            .await
            .map_err(to_gql_error)?;
        Ok(by_id(summaries, |s| s.title_id.clone()))
    }
);

/// Everything a request needs to resolve relationships with batched reads.
///
/// Build once per HTTP request; never store beyond it.
pub struct RequestLoaders {
    pub title: DataLoader<TitleLoader>,
    pub title_for_management: DataLoader<TitleForManagementLoader>,
    pub library: DataLoader<LibraryLoader>,
    pub collection: DataLoader<CollectionLoader>,
    pub episode: DataLoader<EpisodeLoader>,
    pub media_files_for_title: DataLoader<MediaFilesForTitleLoader>,
    pub collections_for_title: DataLoader<CollectionsForTitleLoader>,
    pub episodes_for_collection: DataLoader<EpisodesForCollectionLoader>,
    pub required_audio_override: DataLoader<RequiredAudioOverrideLoader>,
    pub title_metadata_language_override: DataLoader<TitleMetadataLanguageOverrideLoader>,
    pub library_metadata_language_override: DataLoader<LibraryMetadataLanguageOverrideLoader>,
    pub library_use_season_folders_override: DataLoader<LibraryUseSeasonFoldersOverrideLoader>,
    pub facet_use_season_folders_override: DataLoader<FacetUseSeasonFoldersOverrideLoader>,
    pub global_metadata_language: DataLoader<GlobalMetadataLanguageLoader>,
    pub primary_collection_summary: DataLoader<PrimaryCollectionSummaryLoader>,
    pub media_size_summary: DataLoader<MediaSizeSummaryLoader>,
    pub collection_media_size_summary: DataLoader<CollectionMediaSizeSummaryLoader>,
    pub episode_media_size_summary: DataLoader<EpisodeMediaSizeSummaryLoader>,
    pub quality_summary: DataLoader<QualitySummaryLoader>,
    pub effective_quality_summary: DataLoader<EffectiveQualitySummaryLoader>,
    pub episode_progress_summary: DataLoader<EpisodeProgressSummaryLoader>,
    pub collection_episode_progress: DataLoader<CollectionEpisodeProgressLoader>,
    pub ratings: DataLoader<RatingsLoader>,
    pub movie_media_summary: DataLoader<MovieMediaSummaryLoader>,
    pub list_memberships_for_title: DataLoader<ListMembershipsForTitleLoader>,
    pub series_movie_links_for_title: DataLoader<SeriesMovieLinksForTitleLoader>,
    pub wanted_item: DataLoader<WantedItemLoader>,
    pub wanted_item_for_management: DataLoader<WantedItemForManagementLoader>,
    pub title_wanted_item: DataLoader<TitleWantedItemLoader>,
    pub episode_media_availability: DataLoader<EpisodeMediaAvailabilityLoader>,
    pub episode_media_files: DataLoader<EpisodeMediaFilesLoader>,
}

impl RequestLoaders {
    pub fn new(app: AppUseCase, actor: User) -> Arc<Self> {
        let ctx = Arc::new(LoaderCtx { app, actor });
        macro_rules! dl {
            ($loader:ident) => {
                DataLoader::new($loader(ctx.clone()), tokio::spawn).delay(LOAD_DELAY)
            };
        }
        Arc::new(Self {
            title: dl!(TitleLoader),
            title_for_management: dl!(TitleForManagementLoader),
            library: dl!(LibraryLoader),
            collection: dl!(CollectionLoader),
            episode: dl!(EpisodeLoader),
            media_files_for_title: dl!(MediaFilesForTitleLoader),
            collections_for_title: dl!(CollectionsForTitleLoader),
            episodes_for_collection: dl!(EpisodesForCollectionLoader),
            required_audio_override: dl!(RequiredAudioOverrideLoader),
            title_metadata_language_override: dl!(TitleMetadataLanguageOverrideLoader),
            library_metadata_language_override: dl!(LibraryMetadataLanguageOverrideLoader),
            library_use_season_folders_override: dl!(LibraryUseSeasonFoldersOverrideLoader),
            facet_use_season_folders_override: dl!(FacetUseSeasonFoldersOverrideLoader),
            global_metadata_language: dl!(GlobalMetadataLanguageLoader),
            primary_collection_summary: dl!(PrimaryCollectionSummaryLoader),
            media_size_summary: dl!(MediaSizeSummaryLoader),
            collection_media_size_summary: dl!(CollectionMediaSizeSummaryLoader),
            episode_media_size_summary: dl!(EpisodeMediaSizeSummaryLoader),
            quality_summary: dl!(QualitySummaryLoader),
            effective_quality_summary: dl!(EffectiveQualitySummaryLoader),
            episode_progress_summary: dl!(EpisodeProgressSummaryLoader),
            collection_episode_progress: dl!(CollectionEpisodeProgressLoader),
            ratings: dl!(RatingsLoader),
            movie_media_summary: dl!(MovieMediaSummaryLoader),
            list_memberships_for_title: dl!(ListMembershipsForTitleLoader),
            series_movie_links_for_title: dl!(SeriesMovieLinksForTitleLoader),
            wanted_item: dl!(WantedItemLoader),
            wanted_item_for_management: dl!(WantedItemForManagementLoader),
            title_wanted_item: dl!(TitleWantedItemLoader),
            episode_media_availability: dl!(EpisodeMediaAvailabilityLoader),
            episode_media_files: dl!(EpisodeMediaFilesLoader),
        })
    }
}

/// Fetch the request's loaders, if this execution path injected them.
///
/// `None` on the WebSocket/subscription path (and any other path that does
/// not inject loaders) — callers must fall back to direct application calls.
pub fn loaders_from_ctx<'a>(
    ctx: &'a async_graphql::Context<'_>,
) -> Option<&'a Arc<RequestLoaders>> {
    ctx.data_opt::<Arc<RequestLoaders>>()
}
