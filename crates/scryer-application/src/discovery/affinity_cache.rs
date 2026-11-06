//! Process-local cache of library affinity profiles.
//!
//! A profile summarizes the canonical tags of every title a viewer can read,
//! so computing it reads the whole readable catalog. The cache keeps that
//! work off repeated Discovery requests without caching anything a request
//! must decide for itself:
//!
//! - The key carries the discovery generation, the exact library ids the
//!   request was authorized for, the rail setting that shapes the profile,
//!   and the datastore's affinity revision. Permissions are resolved fresh by
//!   every request and only choose the library ids; they are never cached.
//! - The datastore changes the revision inside the same transaction as any
//!   write the profile depends on, so an entry is never served after such a
//!   write commits. There is no time-based expiry.
//! - A fill whose revision changed while it read is returned to its caller
//!   but never stored.
//! - Concurrent requests for one key share a single fill, and at most
//!   [`DISCOVERY_AFFINITY_CACHE_ENTRIES`] keys are kept.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::OnceCell;

use super::home::DiscoveryLibraryAffinityProfile;
use crate::{AppError, AppResult};

pub(crate) const DISCOVERY_AFFINITY_CACHE_ENTRIES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DiscoveryAffinityCacheKey {
    pub(crate) run_id: Option<String>,
    pub(crate) library_ids: Vec<String>,
    pub(crate) genre_rail_corroboration: bool,
    pub(crate) revision: String,
}

type Slot = Arc<OnceCell<Arc<DiscoveryLibraryAffinityProfile>>>;

#[derive(Default)]
struct Slots {
    by_key: HashMap<DiscoveryAffinityCacheKey, Slot>,
    /// Least recently used first.
    order: VecDeque<DiscoveryAffinityCacheKey>,
}

#[derive(Clone, Default)]
pub(crate) struct DiscoveryAffinityCache {
    slots: Arc<Mutex<Slots>>,
}

enum FillFailure {
    Failed(AppError),
    /// The revision moved while the fill read; the profile is good for this
    /// caller only.
    Stale(DiscoveryLibraryAffinityProfile),
}

impl DiscoveryAffinityCache {
    /// The cached profile for `key`, or the one `fill` computes. `revision`
    /// re-reads the datastore revision once `fill` finishes.
    pub(crate) async fn get_or_fill<F, FillFut, R, RevisionFut>(
        &self,
        key: DiscoveryAffinityCacheKey,
        fill: F,
        revision: R,
    ) -> AppResult<Arc<DiscoveryLibraryAffinityProfile>>
    where
        F: FnOnce() -> FillFut,
        FillFut: Future<Output = AppResult<DiscoveryLibraryAffinityProfile>>,
        R: FnOnce() -> RevisionFut,
        RevisionFut: Future<Output = AppResult<Option<String>>>,
    {
        let slot = self.slot(&key);
        let filled = slot
            .get_or_try_init(|| async {
                let profile = fill().await.map_err(FillFailure::Failed)?;
                match revision().await {
                    Ok(Some(after)) if after == key.revision => Ok(Arc::new(profile)),
                    Ok(_) => Err(FillFailure::Stale(profile)),
                    Err(error) => Err(FillFailure::Failed(error)),
                }
            })
            .await;
        match filled {
            Ok(profile) => Ok(profile.clone()),
            Err(failure) => {
                self.forget(&key, &slot);
                match failure {
                    FillFailure::Failed(error) => Err(error),
                    FillFailure::Stale(profile) => Ok(Arc::new(profile)),
                }
            }
        }
    }

    fn slot(&self, key: &DiscoveryAffinityCacheKey) -> Slot {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(slot) = slots.by_key.get(key).cloned() {
            if let Some(position) = slots.order.iter().position(|entry| entry == key) {
                let entry = slots.order.remove(position).expect("position is in range");
                slots.order.push_back(entry);
            }
            return slot;
        }
        let slot = Slot::default();
        slots.by_key.insert(key.clone(), slot.clone());
        slots.order.push_back(key.clone());
        while slots.order.len() > DISCOVERY_AFFINITY_CACHE_ENTRIES {
            if let Some(evicted) = slots.order.pop_front() {
                slots.by_key.remove(&evicted);
            }
        }
        slot
    }

    /// Drops `key` only while it still names `slot`, so a failed fill never
    /// discards a newer slot for the same key.
    fn forget(&self, key: &DiscoveryAffinityCacheKey, slot: &Slot) {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        if slots
            .by_key
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, slot))
        {
            slots.by_key.remove(key);
            slots.order.retain(|entry| entry != key);
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .by_key
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key(revision: &str, library: &str) -> DiscoveryAffinityCacheKey {
        DiscoveryAffinityCacheKey {
            run_id: Some("generation-1".into()),
            library_ids: vec![library.into()],
            genre_rail_corroboration: true,
            revision: revision.into(),
        }
    }

    fn profile(owned_title_count: usize) -> DiscoveryLibraryAffinityProfile {
        DiscoveryLibraryAffinityProfile {
            owned_title_count,
            ..Default::default()
        }
    }

    async fn fill_count(
        cache: &DiscoveryAffinityCache,
        key: DiscoveryAffinityCacheKey,
        fills: &AtomicUsize,
        after: &str,
    ) -> usize {
        let after = after.to_string();
        cache
            .get_or_fill(
                key,
                || async { Ok(profile(fills.fetch_add(1, Ordering::SeqCst) + 1)) },
                || async move { Ok(Some(after)) },
            )
            .await
            .unwrap()
            .owned_title_count
    }

    #[tokio::test]
    async fn a_hit_reuses_the_profile_and_a_new_revision_misses() {
        let cache = DiscoveryAffinityCache::default();
        let fills = AtomicUsize::new(0);
        assert_eq!(fill_count(&cache, key("r1", "a"), &fills, "r1").await, 1);
        assert_eq!(fill_count(&cache, key("r1", "a"), &fills, "r1").await, 1);
        assert_eq!(fill_count(&cache, key("r2", "a"), &fills, "r2").await, 2);
        // Another library scope is another key: a revoked library changes the
        // ids a request is authorized for, so it can never reuse this entry.
        assert_eq!(fill_count(&cache, key("r2", "b"), &fills, "r2").await, 3);
        let mut other_setting = key("r2", "b");
        other_setting.genre_rail_corroboration = false;
        assert_eq!(fill_count(&cache, other_setting, &fills, "r2").await, 4);
        let mut other_generation = key("r2", "b");
        other_generation.run_id = Some("generation-2".into());
        assert_eq!(fill_count(&cache, other_generation, &fills, "r2").await, 5);
    }

    #[tokio::test]
    async fn a_fill_spanning_a_revision_change_is_returned_but_not_kept() {
        let cache = DiscoveryAffinityCache::default();
        let fills = AtomicUsize::new(0);
        assert_eq!(fill_count(&cache, key("r1", "a"), &fills, "r2").await, 1);
        assert_eq!(cache.len(), 0);
        assert_eq!(fill_count(&cache, key("r1", "a"), &fills, "r1").await, 2);
        assert_eq!(fill_count(&cache, key("r1", "a"), &fills, "r1").await, 2);
    }

    #[tokio::test]
    async fn a_failed_fill_is_not_kept() {
        let cache = DiscoveryAffinityCache::default();
        let failed = cache
            .get_or_fill(
                key("r1", "a"),
                || async { Err(AppError::Repository("synthetic failure".into())) },
                || async { Ok(Some("r1".to_string())) },
            )
            .await;
        assert!(failed.is_err());
        assert_eq!(cache.len(), 0);
        let fills = AtomicUsize::new(0);
        assert_eq!(fill_count(&cache, key("r1", "a"), &fills, "r1").await, 1);
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_fill() {
        let cache = DiscoveryAffinityCache::default();
        let fills = AtomicUsize::new(0);
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let first = cache.get_or_fill(
            key("r1", "a"),
            || async {
                released.await.expect("fill is released");
                Ok(profile(fills.fetch_add(1, Ordering::SeqCst) + 1))
            },
            || async { Ok(Some("r1".to_string())) },
        );
        let second = cache.get_or_fill(
            key("r1", "a"),
            || async { Ok(profile(fills.fetch_add(1, Ordering::SeqCst) + 100)) },
            || async { Ok(Some("r1".to_string())) },
        );
        // join! polls in order: the first call takes the fill and parks on the
        // channel, the second waits on that fill, then the fill is released.
        let (first, second, ()) = tokio::join!(first, second, async {
            release.send(()).expect("fill is waiting");
        });
        assert_eq!(first.unwrap().owned_title_count, 1);
        assert_eq!(second.unwrap().owned_title_count, 1);
        assert_eq!(fills.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_cache_keeps_at_most_its_bound_and_evicts_the_least_recent() {
        let cache = DiscoveryAffinityCache::default();
        let fills = AtomicUsize::new(0);
        for library in 0..DISCOVERY_AFFINITY_CACHE_ENTRIES {
            fill_count(&cache, key("r1", &library.to_string()), &fills, "r1").await;
        }
        // Touch the oldest so the second oldest is the one evicted.
        assert_eq!(fill_count(&cache, key("r1", "0"), &fills, "r1").await, 1);
        fill_count(&cache, key("r1", "extra"), &fills, "r1").await;
        assert_eq!(cache.len(), DISCOVERY_AFFINITY_CACHE_ENTRIES);
        let before = fills.load(Ordering::SeqCst);
        assert_eq!(fill_count(&cache, key("r1", "0"), &fills, "r1").await, 1);
        assert_eq!(fills.load(Ordering::SeqCst), before);
        fill_count(&cache, key("r1", "1"), &fills, "r1").await;
        assert_eq!(fills.load(Ordering::SeqCst), before + 1);
    }
}
