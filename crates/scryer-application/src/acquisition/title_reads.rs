//! Catalog reads for one title, memoized for the length of one operation.
//!
//! An acquisition title walk evaluates every due scope of a title in turn, and
//! each scope's evaluation, admission subject and grab path used to re-read the
//! title's whole episode list, its collections and its media files. A long
//! anime title has ~850 episode rows, so a walk over 30 scopes decoded ~25k
//! rows to answer the same question 30 times.
//!
//! A [`TitleCatalogReads`] is created by the operation that owns the title for
//! a bounded stretch — one title walk, one pending-release decision — and
//! passed down to the helpers that read. The first read of each kind goes to
//! the store; later reads in the same operation reuse it. Nothing is kept past
//! the operation, so there is no invalidation to get wrong: the freshness bound
//! is the operation's own length, which is the same bound the walk already
//! accepted for the episode list and submissions it loads up front.
//!
//! Failed reads are not memoized; the next caller retries the store.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use scryer_domain::{Collection, Episode};
use tokio::sync::OnceCell;

use crate::{AppResult, AppUseCase, EpisodeScopedMediaFile, ScopedExternalId, TitleMediaFile};

pub(crate) struct TitleCatalogReads {
    #[cfg(test)]
    pub(crate) scoring_preparations: std::sync::atomic::AtomicUsize,
    title_id: String,
    episodes: OnceCell<Arc<Vec<Episode>>>,
    collections: OnceCell<Arc<Vec<Collection>>>,
    anime_numbering_bridge: OnceCell<Option<scryer_domain::AnimeNumberingBridge>>,
    media_files: OnceCell<Arc<Vec<TitleMediaFile>>>,
    primary_file_cutoffs: OnceCell<Arc<Vec<crate::MediaFileCutoffSummary>>>,
    collection_episodes: Mutex<HashMap<String, Arc<Vec<Episode>>>>,
    collection_external_ids: Mutex<HashMap<String, Arc<Vec<ScopedExternalId>>>>,
    episode_external_ids: OnceCell<Arc<HashMap<String, Vec<ScopedExternalId>>>>,
    scoped_media_files: Mutex<HashMap<Vec<String>, Arc<Vec<EpisodeScopedMediaFile>>>>,
}

impl TitleCatalogReads {
    pub(crate) fn new(title_id: &str) -> Self {
        Self {
            #[cfg(test)]
            scoring_preparations: Default::default(),
            title_id: title_id.to_string(),
            episodes: OnceCell::new(),
            collections: OnceCell::new(),
            anime_numbering_bridge: OnceCell::new(),
            media_files: OnceCell::new(),
            primary_file_cutoffs: OnceCell::new(),
            collection_episodes: Mutex::new(HashMap::new()),
            collection_external_ids: Mutex::new(HashMap::new()),
            episode_external_ids: OnceCell::new(),
            scoped_media_files: Mutex::new(HashMap::new()),
        }
    }

    /// Seeded with an episode list the caller already read for this title.
    pub(crate) fn with_episodes(title_id: &str, episodes: Vec<Episode>) -> Self {
        let reads = Self::new(title_id);
        let _ = reads.episodes.set(Arc::new(episodes));
        reads
    }

    /// Whether this memo answers for `title_id`. Callers that may be handed a
    /// different title (a series-movie walk searches under a derived record)
    /// fall back to a fresh memo rather than reading another title's rows.
    pub(crate) fn covers(&self, title_id: &str) -> bool {
        self.title_id == title_id
    }

    pub(crate) async fn episodes(&self, app: &AppUseCase) -> AppResult<Arc<Vec<Episode>>> {
        self.episodes
            .get_or_try_init(|| async {
                app.services
                    .catalog
                    .shows
                    .list_episodes_for_title(&self.title_id)
                    .await
                    .map(Arc::new)
            })
            .await
            .cloned()
    }

    pub(crate) async fn collections(&self, app: &AppUseCase) -> AppResult<Arc<Vec<Collection>>> {
        self.collections
            .get_or_try_init(|| async {
                app.services
                    .catalog
                    .shows
                    .list_collections_for_title(&self.title_id)
                    .await
                    .map(Arc::new)
            })
            .await
            .cloned()
    }

    pub(crate) async fn anime_numbering_bridge(
        &self,
        app: &AppUseCase,
    ) -> AppResult<Option<scryer_domain::AnimeNumberingBridge>> {
        self.anime_numbering_bridge
            .get_or_try_init(|| {
                app.services
                    .catalog
                    .shows
                    .get_anime_numbering_bridge(&self.title_id)
            })
            .await
            .cloned()
    }

    /// Compact ownership and cutoff facts shared across scopes. Full incumbent
    /// payloads are loaded separately for the episode sets being scored.
    pub(crate) async fn primary_file_cutoffs(
        &self,
        app: &AppUseCase,
    ) -> AppResult<Arc<Vec<crate::MediaFileCutoffSummary>>> {
        self.primary_file_cutoffs
            .get_or_try_init(|| async {
                app.services
                    .library
                    .media_files
                    .list_primary_file_cutoffs_for_title(&self.title_id)
                    .await
                    .map(Arc::new)
            })
            .await
            .cloned()
    }

    /// Every live media file of the title, as `list_media_files_for_title`
    /// returns them (one row per file-episode link).
    pub(crate) async fn media_files(
        &self,
        app: &AppUseCase,
    ) -> AppResult<Arc<Vec<TitleMediaFile>>> {
        self.media_files
            .get_or_try_init(|| async {
                app.services
                    .library
                    .media_files
                    .list_media_files_for_title(&self.title_id)
                    .await
                    .map(Arc::new)
            })
            .await
            .cloned()
    }

    pub(crate) async fn episodes_for_collection(
        &self,
        app: &AppUseCase,
        collection_id: &str,
    ) -> AppResult<Arc<Vec<Episode>>> {
        if let Some(hit) = lock(&self.collection_episodes).get(collection_id) {
            return Ok(Arc::clone(hit));
        }
        let episodes = Arc::new(
            app.services
                .catalog
                .shows
                .list_episodes_for_collection(collection_id)
                .await?,
        );
        lock(&self.collection_episodes)
            .entry(collection_id.to_string())
            .or_insert_with(|| Arc::clone(&episodes));
        Ok(episodes)
    }

    /// A collection's scoped external ids. Every episode stage of a season
    /// asks for the same collection's AniDB mapping.
    pub(crate) async fn collection_external_ids(
        &self,
        app: &AppUseCase,
        collection_id: &str,
    ) -> AppResult<Arc<Vec<ScopedExternalId>>> {
        if let Some(hit) = lock(&self.collection_external_ids).get(collection_id) {
            return Ok(Arc::clone(hit));
        }
        let ids = Arc::new(
            app.services
                .catalog
                .shows
                .list_collection_external_ids(collection_id)
                .await?,
        );
        lock(&self.collection_external_ids)
            .entry(collection_id.to_string())
            .or_insert_with(|| Arc::clone(&ids));
        Ok(ids)
    }

    /// One episode's scoped external ids, exactly as
    /// `list_episode_external_ids` returns them. The first ask reads the whole
    /// title's episode ids in one query; every episode stage of an anime walk
    /// asks for its own AniDB mapping.
    pub(crate) async fn episode_external_ids(
        &self,
        app: &AppUseCase,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        let by_episode = self
            .episode_external_ids
            .get_or_try_init(|| async {
                let rows = app
                    .services
                    .catalog
                    .shows
                    .list_episode_external_ids_for_title(&self.title_id)
                    .await?;
                let mut by_episode: HashMap<String, Vec<ScopedExternalId>> = HashMap::new();
                for row in rows {
                    by_episode
                        .entry(row.scope_id.clone())
                        .or_default()
                        .push(row);
                }
                Ok::<_, crate::AppError>(Arc::new(by_episode))
            })
            .await?;
        Ok(by_episode.get(episode_id).cloned().unwrap_or_default())
    }

    /// `list_live_media_files_for_episode_ids` for this title, memoized per
    /// episode set. The store's answer depends on the set as a whole (a file's
    /// role is primary when any link *in the set* is primary), so the key is
    /// the exact set, order-insensitive.
    pub(crate) async fn live_media_files_for_episode_ids(
        &self,
        app: &AppUseCase,
        episode_ids: &[String],
    ) -> AppResult<Arc<Vec<EpisodeScopedMediaFile>>> {
        let mut key = episode_ids.to_vec();
        key.sort();
        key.dedup();
        if let Some(hit) = lock(&self.scoped_media_files).get(&key) {
            return Ok(Arc::clone(hit));
        }
        let files = Arc::new(
            app.services
                .library
                .media_files
                .list_live_media_files_for_episode_ids(&self.title_id, episode_ids)
                .await?,
        );
        lock(&self.scoped_media_files)
            .entry(key)
            .or_insert_with(|| Arc::clone(&files));
        Ok(files)
    }
}

/// The memo holds plain data, so a poisoned lock is still a consistent map.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
