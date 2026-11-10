//! Process-local cache of library affinity profiles.
//!
//! A profile summarizes the canonical tags of every title a viewer can read,
//! so computing it reads the whole readable catalog. The cache keeps that
//! work off repeated Discovery requests without caching anything a request
//! must decide for itself:
//!
//! - A scope is the discovery generation, the exact library ids the request
//!   was authorized for, and the rail setting that shapes the profile.
//!   Permissions are resolved fresh by every request and only choose the
//!   library ids; they are never cached.
//! - Each scope keeps one profile, with the datastore's affinity revision it
//!   was built from and the time it was built. The datastore changes the
//!   revision inside the same transaction as any write the profile depends
//!   on, so an unchanged revision means unchanged inputs.
//! - Rebuilds are rate limited. A changed revision rebuilds the profile at
//!   once when the kept one is at least [`DISCOVERY_AFFINITY_REBUILD_COOLDOWN`]
//!   old. Inside that window the kept profile is served as is, and the first
//!   request after the window rebuilds once for every write the window
//!   absorbed. A bulk library move therefore costs one rebuild per window, not
//!   one per moved title, and a profile is never more than one window behind.
//! - Concurrent requests for one scope share a single fill, and at most
//!   [`DISCOVERY_AFFINITY_CACHE_ENTRIES`] scopes are kept.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::OnceCell;

use super::home::DiscoveryLibraryAffinityProfile;
use crate::{AppError, AppResult};

pub(crate) const DISCOVERY_AFFINITY_CACHE_ENTRIES: usize = 32;

/// The least time between two rebuilds of one scope's profile.
pub(crate) const DISCOVERY_AFFINITY_REBUILD_COOLDOWN: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DiscoveryAffinityCacheScope {
    pub(crate) run_id: Option<String>,
    pub(crate) library_ids: Vec<String>,
    pub(crate) genre_rail_corroboration: bool,
}

struct Built {
    profile: Arc<DiscoveryLibraryAffinityProfile>,
    /// The revision read before the fill started: the inputs it is good for.
    revision: String,
    built_at: Instant,
}

type Slot = Arc<OnceCell<Built>>;

#[derive(Default)]
struct Slots {
    by_scope: HashMap<DiscoveryAffinityCacheScope, Slot>,
    /// Least recently used first.
    order: VecDeque<DiscoveryAffinityCacheScope>,
}

#[derive(Clone, Default)]
pub(crate) struct DiscoveryAffinityCache {
    slots: Arc<Mutex<Slots>>,
}

impl DiscoveryAffinityCache {
    /// The profile for `scope` at `revision`, or the one `fill` computes.
    ///
    /// A kept profile built from another revision is still returned while it
    /// is younger than the rebuild cooldown; once it is older, `fill` replaces
    /// it. `now` is the request's clock reading so the window is decided
    /// deterministically.
    pub(crate) async fn get_or_fill<F, FillFut>(
        &self,
        scope: DiscoveryAffinityCacheScope,
        revision: String,
        now: Instant,
        fill: F,
    ) -> AppResult<Arc<DiscoveryLibraryAffinityProfile>>
    where
        F: FnOnce() -> FillFut,
        FillFut: Future<Output = AppResult<DiscoveryLibraryAffinityProfile>>,
    {
        let slot = self.slot(&scope, &revision, now);
        let filled = slot
            .get_or_try_init(|| async {
                let profile = fill().await?;
                Ok::<_, AppError>(Built {
                    profile: Arc::new(profile),
                    revision,
                    built_at: now,
                })
            })
            .await;
        match filled {
            Ok(built) => Ok(built.profile.clone()),
            Err(error) => {
                self.forget(&scope, &slot);
                Err(error)
            }
        }
    }

    /// The slot a request at `revision` should read or fill: the kept one
    /// while it is current or inside the cooldown, otherwise a fresh slot
    /// that replaces it.
    fn slot(&self, scope: &DiscoveryAffinityCacheScope, revision: &str, now: Instant) -> Slot {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(slot) = slots.by_scope.get(scope).cloned() {
            if let Some(position) = slots.order.iter().position(|entry| entry == scope) {
                let entry = slots.order.remove(position).expect("position is in range");
                slots.order.push_back(entry);
            }
            let reusable = match slot.get() {
                // A fill in flight serves everyone who arrives before it lands.
                None => true,
                Some(built) => {
                    built.revision == revision
                        || now.saturating_duration_since(built.built_at)
                            < DISCOVERY_AFFINITY_REBUILD_COOLDOWN
                }
            };
            if reusable {
                return slot;
            }
            let fresh = Slot::default();
            slots.by_scope.insert(scope.clone(), fresh.clone());
            return fresh;
        }
        let slot = Slot::default();
        slots.by_scope.insert(scope.clone(), slot.clone());
        slots.order.push_back(scope.clone());
        while slots.order.len() > DISCOVERY_AFFINITY_CACHE_ENTRIES {
            if let Some(evicted) = slots.order.pop_front() {
                slots.by_scope.remove(&evicted);
            }
        }
        slot
    }

    /// Drops `scope` only while it still names `slot`, so a failed fill never
    /// discards a newer slot for the same scope.
    fn forget(&self, scope: &DiscoveryAffinityCacheScope, slot: &Slot) {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        if slots
            .by_scope
            .get(scope)
            .is_some_and(|current| Arc::ptr_eq(current, slot))
        {
            slots.by_scope.remove(scope);
            slots.order.retain(|entry| entry != scope);
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .by_scope
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scope(library: &str) -> DiscoveryAffinityCacheScope {
        DiscoveryAffinityCacheScope {
            run_id: Some("generation-1".into()),
            library_ids: vec![library.into()],
            genre_rail_corroboration: true,
        }
    }

    fn profile(owned_title_count: usize) -> DiscoveryLibraryAffinityProfile {
        DiscoveryLibraryAffinityProfile {
            owned_title_count,
            ..Default::default()
        }
    }

    /// Reads the profile for `scope` at `revision` and `now`, counting fills
    /// in `fills`; the returned count says which fill produced the profile.
    async fn fill_count(
        cache: &DiscoveryAffinityCache,
        scope: DiscoveryAffinityCacheScope,
        revision: &str,
        now: Instant,
        fills: &AtomicUsize,
    ) -> usize {
        cache
            .get_or_fill(scope, revision.to_string(), now, || async {
                Ok(profile(fills.fetch_add(1, Ordering::SeqCst) + 1))
            })
            .await
            .unwrap()
            .owned_title_count
    }

    fn cooldown_elapsed(start: Instant) -> Instant {
        start + DISCOVERY_AFFINITY_REBUILD_COOLDOWN
    }

    #[tokio::test]
    async fn a_hit_reuses_the_profile_and_another_scope_fills_its_own() {
        let cache = DiscoveryAffinityCache::default();
        let fills = AtomicUsize::new(0);
        let start = Instant::now();
        assert_eq!(fill_count(&cache, scope("a"), "r1", start, &fills).await, 1);
        assert_eq!(fill_count(&cache, scope("a"), "r1", start, &fills).await, 1);
        // Even long after the cooldown, an unchanged revision never rebuilds.
        let later = cooldown_elapsed(cooldown_elapsed(start));
        assert_eq!(fill_count(&cache, scope("a"), "r1", later, &fills).await, 1);
        // Another library scope is another entry: a revoked library changes
        // the ids a request is authorized for, so it can never reuse this one.
        assert_eq!(fill_count(&cache, scope("b"), "r1", start, &fills).await, 2);
        let mut other_setting = scope("b");
        other_setting.genre_rail_corroboration = false;
        assert_eq!(
            fill_count(&cache, other_setting, "r1", start, &fills).await,
            3
        );
        let mut other_generation = scope("b");
        other_generation.run_id = Some("generation-2".into());
        assert_eq!(
            fill_count(&cache, other_generation, "r1", start, &fills).await,
            4
        );
    }

    #[tokio::test]
    async fn a_changed_revision_inside_the_cooldown_serves_the_kept_profile() {
        let cache = DiscoveryAffinityCache::default();
        let fills = AtomicUsize::new(0);
        let start = Instant::now();
        assert_eq!(fill_count(&cache, scope("a"), "r1", start, &fills).await, 1);
        // Every write inside the window is absorbed by the kept profile.
        let inside = start + DISCOVERY_AFFINITY_REBUILD_COOLDOWN / 2;
        assert_eq!(
            fill_count(&cache, scope("a"), "r2", inside, &fills).await,
            1
        );
        assert_eq!(
            fill_count(&cache, scope("a"), "r3", inside, &fills).await,
            1
        );
        let just_inside = cooldown_elapsed(start) - Duration::from_millis(1);
        assert_eq!(
            fill_count(&cache, scope("a"), "r4", just_inside, &fills).await,
            1
        );
        assert_eq!(fills.load(Ordering::SeqCst), 1);
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn the_first_request_after_the_cooldown_rebuilds_once_for_every_write() {
        let cache = DiscoveryAffinityCache::default();
        let fills = AtomicUsize::new(0);
        let start = Instant::now();
        assert_eq!(fill_count(&cache, scope("a"), "r1", start, &fills).await, 1);
        let inside = start + Duration::from_secs(1);
        assert_eq!(
            fill_count(&cache, scope("a"), "r2", inside, &fills).await,
            1
        );
        let elapsed = cooldown_elapsed(start);
        assert_eq!(
            fill_count(&cache, scope("a"), "r3", elapsed, &fills).await,
            2
        );
        // The rebuild opens a new window measured from its own request.
        assert_eq!(
            fill_count(&cache, scope("a"), "r3", elapsed, &fills).await,
            2
        );
        let inside_again = elapsed + Duration::from_secs(1);
        assert_eq!(
            fill_count(&cache, scope("a"), "r4", inside_again, &fills).await,
            2
        );
        assert_eq!(
            fill_count(&cache, scope("a"), "r4", cooldown_elapsed(elapsed), &fills).await,
            3
        );
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn a_first_build_never_waits_for_a_cooldown() {
        let cache = DiscoveryAffinityCache::default();
        let fills = AtomicUsize::new(0);
        let start = Instant::now();
        assert_eq!(fill_count(&cache, scope("a"), "r1", start, &fills).await, 1);
        assert_eq!(fill_count(&cache, scope("b"), "r1", start, &fills).await, 2);
    }

    #[tokio::test]
    async fn a_failed_fill_is_not_kept() {
        let cache = DiscoveryAffinityCache::default();
        let start = Instant::now();
        let failed = cache
            .get_or_fill(scope("a"), "r1".to_string(), start, || async {
                Err(AppError::Repository("synthetic failure".into()))
            })
            .await;
        assert!(failed.is_err());
        assert_eq!(cache.len(), 0);
        let fills = AtomicUsize::new(0);
        assert_eq!(fill_count(&cache, scope("a"), "r1", start, &fills).await, 1);
    }

    #[tokio::test]
    async fn a_failed_rebuild_drops_the_stale_profile_and_the_next_request_refills() {
        let cache = DiscoveryAffinityCache::default();
        let fills = AtomicUsize::new(0);
        let start = Instant::now();
        assert_eq!(fill_count(&cache, scope("a"), "r1", start, &fills).await, 1);
        let elapsed = cooldown_elapsed(start);
        let failed = cache
            .get_or_fill(scope("a"), "r2".to_string(), elapsed, || async {
                Err(AppError::Repository("synthetic failure".into()))
            })
            .await;
        assert!(failed.is_err());
        assert_eq!(cache.len(), 0);
        assert_eq!(
            fill_count(&cache, scope("a"), "r2", elapsed, &fills).await,
            2
        );
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_fill() {
        let cache = DiscoveryAffinityCache::default();
        let fills = AtomicUsize::new(0);
        let start = Instant::now();
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let first = cache.get_or_fill(scope("a"), "r1".to_string(), start, || async {
            released.await.expect("fill is released");
            Ok(profile(fills.fetch_add(1, Ordering::SeqCst) + 1))
        });
        let second = cache.get_or_fill(scope("a"), "r1".to_string(), start, || async {
            Ok(profile(fills.fetch_add(1, Ordering::SeqCst) + 100))
        });
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
        let start = Instant::now();
        for library in 0..DISCOVERY_AFFINITY_CACHE_ENTRIES {
            fill_count(&cache, scope(&library.to_string()), "r1", start, &fills).await;
        }
        // Touch the oldest so the second oldest is the one evicted.
        assert_eq!(fill_count(&cache, scope("0"), "r1", start, &fills).await, 1);
        fill_count(&cache, scope("extra"), "r1", start, &fills).await;
        assert_eq!(cache.len(), DISCOVERY_AFFINITY_CACHE_ENTRIES);
        let before = fills.load(Ordering::SeqCst);
        assert_eq!(fill_count(&cache, scope("0"), "r1", start, &fills).await, 1);
        assert_eq!(fills.load(Ordering::SeqCst), before);
        fill_count(&cache, scope("1"), "r1", start, &fills).await;
        assert_eq!(fills.load(Ordering::SeqCst), before + 1);
    }
}
