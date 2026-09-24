//! Process-local read-through cache for `title_anime_numbering_bridges`.
//!
//! Acquisition, RSS, import and scan look a title's bridge up once per wanted
//! item or per release, so an idle catalog re-read the same few hundred bytes
//! tens of times a second. A bridge only changes when its title is hydrated,
//! so every read after the first is served from here.
//!
//! One instance is shared by every store that can change or remove a bridge
//! row: `ShowStore` (the bridge writer), `TitleStore` (title delete cascades
//! the row) and `TitleMergeStore` (retiring the merge source cascades it).
//! Each drops the affected title after its transaction has finished. Absent
//! bridges are cached too, since most series have none and are asked the most.
//!
//! # The generation rule
//!
//! A read that misses takes a ticket carrying the current generation before
//! it queries the database, and installs its result only if no invalidation
//! has happened since. An invalidation bumps the generation and removes the
//! entry under one lock. So a read that fetched the row before a write
//! committed either finds its install refused (the write already
//! invalidated) or has it removed (the write invalidates afterwards). A stale
//! value can never outlive the write that superseded it.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use scryer_application::AppResult;
use scryer_domain::AnimeNumberingBridge;

/// Upper bound on cached titles. Deletes invalidate their entries, so the
/// live catalog bounds the map already; this cap only guards against an
/// unforeseen leak. Reaching it clears the map rather than evicting
/// piecemeal: a clear is always safe and at this size it is rare.
const MAX_ENTRIES: usize = 262_144;

#[derive(Default)]
struct Inner {
    generation: u64,
    /// `None` is a cached "this title has no bridge".
    entries: HashMap<String, Option<Arc<AnimeNumberingBridge>>>,
}

#[derive(Default)]
pub struct AnimeNumberingBridgeCache {
    inner: RwLock<Inner>,
}

/// Proof of the generation a cache miss started at.
#[derive(Clone, Copy, Debug)]
pub struct FillTicket {
    generation: u64,
}

impl AnimeNumberingBridgeCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// `Some(bridge_or_none)` on a hit, `None` on a miss.
    pub fn lookup(&self, title_id: &str) -> Option<Option<AnimeNumberingBridge>> {
        self.read()
            .entries
            .get(title_id)
            .map(|entry| entry.as_deref().cloned())
    }

    /// Start a miss. Take the ticket before the database read.
    pub fn begin_fill(&self) -> FillTicket {
        FillTicket {
            generation: self.read().generation,
        }
    }

    /// Install what a miss read, unless an invalidation happened since its
    /// ticket was taken. Returns whether the value was installed.
    pub fn complete_fill(
        &self,
        ticket: FillTicket,
        title_id: &str,
        bridge: Option<&AnimeNumberingBridge>,
    ) -> bool {
        let mut inner = self.write();
        if inner.generation != ticket.generation {
            return false;
        }
        if inner.entries.len() >= MAX_ENTRIES && !inner.entries.contains_key(title_id) {
            inner.entries.clear();
        }
        inner
            .entries
            .insert(title_id.to_string(), bridge.cloned().map(Arc::new));
        true
    }

    /// Serve `title_id` from the cache, or run `load` and cache its result
    /// under the generation rule.
    pub async fn get_or_load<F, Fut>(
        &self,
        title_id: &str,
        load: F,
    ) -> AppResult<Option<AnimeNumberingBridge>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<Option<AnimeNumberingBridge>>>,
    {
        if let Some(hit) = self.lookup(title_id) {
            return Ok(hit);
        }
        let ticket = self.begin_fill();
        let loaded = load().await?;
        self.complete_fill(ticket, title_id, loaded.as_ref());
        Ok(loaded)
    }

    /// Forget one title. Call after the write that changed its row finished,
    /// whether or not it committed.
    pub fn invalidate(&self, title_id: &str) {
        let mut inner = self.write();
        inner.generation = inner.generation.wrapping_add(1);
        inner.entries.remove(title_id);
    }

    /// Forget every title.
    pub fn clear(&self) {
        let mut inner = self.write();
        inner.generation = inner.generation.wrapping_add(1);
        inner.entries.clear();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.read().entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn bridge(tag: &str) -> AnimeNumberingBridge {
        AnimeNumberingBridge {
            generated_on: tag.to_string(),
            seasons: vec![scryer_domain::AnimeCommunitySeason {
                index: 1,
                anidb_id: None,
                anilist_id: None,
                mal_id: None,
                titles: vec![format!("Synthetic Cour {tag}")],
                ranges: Vec::new(),
                absolute_start: Some(1),
                episode_count: Some(12),
            }],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_hit_does_not_load_again_and_absence_is_cached() {
        let cache = AnimeNumberingBridgeCache::new();
        let loads = AtomicUsize::new(0);
        for _ in 0..3 {
            let got = cache
                .get_or_load("title-a", || async {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok(Some(bridge("a")))
                })
                .await
                .unwrap();
            assert_eq!(got, Some(bridge("a")));
            let absent = cache
                .get_or_load("title-none", || async {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok(None)
                })
                .await
                .unwrap();
            assert_eq!(absent, None);
        }
        assert_eq!(loads.load(Ordering::SeqCst), 2, "one load per title");
    }

    #[tokio::test]
    async fn a_failed_load_caches_nothing() {
        let cache = AnimeNumberingBridgeCache::new();
        cache
            .get_or_load("title-a", || async {
                Err(scryer_application::AppError::Repository("boom".into()))
            })
            .await
            .unwrap_err();
        assert_eq!(cache.lookup("title-a"), None);
    }

    #[test]
    fn a_fill_that_straddles_an_invalidation_is_refused() {
        let cache = AnimeNumberingBridgeCache::new();
        let ticket = cache.begin_fill();
        // The write lands and invalidates between the read and the install.
        cache.invalidate("title-a");
        assert!(!cache.complete_fill(ticket, "title-a", Some(&bridge("stale"))));
        assert_eq!(cache.lookup("title-a"), None);

        // An invalidation of an unrelated title refuses the fill as well: the
        // generation is global, which costs a refill, never a stale value.
        let ticket = cache.begin_fill();
        cache.invalidate("title-b");
        assert!(!cache.complete_fill(ticket, "title-a", None));

        let ticket = cache.begin_fill();
        cache.clear();
        assert!(!cache.complete_fill(ticket, "title-a", None));

        let ticket = cache.begin_fill();
        assert!(cache.complete_fill(ticket, "title-a", Some(&bridge("fresh"))));
        assert_eq!(cache.lookup("title-a"), Some(Some(bridge("fresh"))));
    }

    #[test]
    fn an_install_before_the_invalidation_is_removed_by_it() {
        let cache = AnimeNumberingBridgeCache::new();
        let ticket = cache.begin_fill();
        assert!(cache.complete_fill(ticket, "title-a", Some(&bridge("stale"))));
        cache.invalidate("title-a");
        assert_eq!(cache.lookup("title-a"), None);
    }

    #[test]
    fn reaching_the_cap_clears_instead_of_growing() {
        let cache = AnimeNumberingBridgeCache::new();
        {
            let mut inner = cache.write();
            for index in 0..MAX_ENTRIES {
                inner.entries.insert(format!("title-{index}"), None);
            }
        }
        let ticket = cache.begin_fill();
        assert!(cache.complete_fill(ticket, "title-new", None));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.lookup("title-new"), Some(None));
    }
}
