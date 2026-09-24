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

use crate::{AppResult, AppUseCase, EpisodeScopedMediaFile, TitleMediaFile};

pub(crate) struct TitleCatalogReads {
    title_id: String,
    episodes: OnceCell<Arc<Vec<Episode>>>,
    collections: OnceCell<Arc<Vec<Collection>>>,
    media_files: OnceCell<Arc<Vec<TitleMediaFile>>>,
    collection_episodes: Mutex<HashMap<String, Arc<Vec<Episode>>>>,
    scoped_media_files: Mutex<HashMap<Vec<String>, Arc<Vec<EpisodeScopedMediaFile>>>>,
}

impl TitleCatalogReads {
    pub(crate) fn new(title_id: &str) -> Self {
        Self {
            title_id: title_id.to_string(),
            episodes: OnceCell::new(),
            collections: OnceCell::new(),
            media_files: OnceCell::new(),
            collection_episodes: Mutex::new(HashMap::new()),
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
