//! Process-local read caches for the settings and quality-profile stores.
//!
//! Both stores serve values that only change when the store itself writes
//! them, yet background jobs read them tens of thousands of times an hour.
//! The cache here keeps the last value read for each key until a write
//! invalidates it.
//!
//! Every fill is fenced by a generation counter. A reader takes a
//! [`CacheTicket`] before it queries the database and may install its result
//! only while the generation on that ticket is still current. Writers bump the
//! generation after their write settles, so a read whose query could have
//! observed the pre-write state can never install that state: either it fills
//! before the bump (and the bump clears it) or it fills after (and the stale
//! ticket is refused).

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{PoisonError, RwLock};

/// The generation a reader observed before it went to the database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CacheTicket {
    generation: u64,
}

pub(crate) enum CacheLookup<V> {
    Hit(V),
    Miss(CacheTicket),
}

struct CacheState<K, V, T> {
    generation: u64,
    /// What every cached value was decoded under (the settings store uses the
    /// encryption key fingerprint). `None` until the first fill.
    tag: Option<T>,
    entries: HashMap<K, V>,
}

pub(crate) struct GenerationCache<K, V, T = ()> {
    state: RwLock<CacheState<K, V, T>>,
    max_entries: usize,
    #[cfg(test)]
    fill_pause: test_support::FillPause,
}

impl<K, V, T> GenerationCache<K, V, T>
where
    K: Eq + Hash,
    V: Clone,
    T: PartialEq,
{
    /// `max_entries` bounds memory for key spaces that grow with the library
    /// (per-title scope ids). Reaching it drops every entry; the hot keys are
    /// read back on their next use.
    pub(crate) fn new(max_entries: usize) -> Self {
        Self {
            state: RwLock::new(CacheState {
                generation: 0,
                tag: None,
                entries: HashMap::new(),
            }),
            max_entries: max_entries.max(1),
            #[cfg(test)]
            fill_pause: test_support::FillPause::default(),
        }
    }

    /// A hit returns the cached value. A miss returns the ticket the caller
    /// must hand back to [`Self::fill`] after reading the database.
    pub(crate) fn lookup<Q>(&self, tag: &T, key: &Q) -> CacheLookup<V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        if state.tag.as_ref() == Some(tag)
            && let Some(value) = state.entries.get(key)
        {
            return CacheLookup::Hit(value.clone());
        }
        CacheLookup::Miss(CacheTicket {
            generation: state.generation,
        })
    }

    /// Install a value read under `ticket`. Refused when any invalidation
    /// happened since the ticket was taken. A different `tag` means every
    /// cached value was decoded under something that no longer holds, so they
    /// are all dropped and every older ticket is retired before this value
    /// goes in.
    pub(crate) fn fill(&self, ticket: CacheTicket, tag: T, key: K, value: V) -> bool {
        let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
        if state.generation != ticket.generation {
            return false;
        }
        if state.tag.as_ref() != Some(&tag) {
            state.entries.clear();
            state.tag = Some(tag);
            state.generation = state.generation.wrapping_add(1);
        }
        if state.entries.len() >= self.max_entries && !state.entries.contains_key(&key) {
            state.entries.clear();
        }
        state.entries.insert(key, value);
        true
    }

    /// Drop every entry and retire every outstanding ticket.
    pub(crate) fn invalidate_all(&self) {
        let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
        state.entries.clear();
        state.generation = state.generation.wrapping_add(1);
    }

    /// Drop the entries `stale` selects and retire every outstanding ticket.
    /// Retiring all tickets (not only the matching keys') keeps the fence
    /// simple: an unrelated read that loses its fill just reads again.
    pub(crate) fn invalidate_where(&self, mut stale: impl FnMut(&K) -> bool) {
        let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
        state.entries.retain(|key, _| !stale(key));
        state.generation = state.generation.wrapping_add(1);
    }

    /// Test seam between a miss's database read and its fill. Compiled out of
    /// non-test builds.
    #[cfg(test)]
    pub(crate) async fn before_fill(&self) {
        self.fill_pause.wait_if_armed().await;
    }

    #[cfg(test)]
    pub(crate) fn arm_fill_pause(&self) -> test_support::FillPauseHandle {
        self.fill_pause.arm()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .len()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    use tokio::sync::oneshot;

    /// Parks the next cache fill after its database read so a test can land a
    /// write in that window, then release the fill.
    #[derive(Default)]
    pub(crate) struct FillPause {
        armed: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
    }

    pub(crate) struct FillPauseHandle {
        pub(crate) reached: oneshot::Receiver<()>,
        pub(crate) release: oneshot::Sender<()>,
    }

    impl FillPause {
        pub(crate) fn arm(&self) -> FillPauseHandle {
            let (reached_tx, reached_rx) = oneshot::channel();
            let (release_tx, release_rx) = oneshot::channel();
            *self.armed.lock().expect("fill pause lock") = Some((reached_tx, release_rx));
            FillPauseHandle {
                reached: reached_rx,
                release: release_tx,
            }
        }

        pub(crate) async fn wait_if_armed(&self) {
            let armed = self.armed.lock().expect("fill pause lock").take();
            if let Some((reached, release)) = armed {
                let _ = reached.send(());
                let _ = release.await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn miss_ticket(lookup: CacheLookup<u32>) -> CacheTicket {
        match lookup {
            CacheLookup::Miss(ticket) => ticket,
            CacheLookup::Hit(value) => panic!("expected a miss, got {value}"),
        }
    }

    #[test]
    fn a_fill_under_a_ticket_taken_before_an_invalidation_is_refused() {
        let cache: GenerationCache<&str, u32> = GenerationCache::new(16);
        let ticket = miss_ticket(cache.lookup(&(), "key"));
        cache.invalidate_all();
        assert!(!cache.fill(ticket, (), "key", 1));
        miss_ticket(cache.lookup(&(), "key"));

        let fresh = miss_ticket(cache.lookup(&(), "key"));
        assert!(cache.fill(fresh, (), "key", 2));
        assert!(matches!(cache.lookup(&(), "key"), CacheLookup::Hit(2)));
    }

    #[test]
    fn a_targeted_invalidation_drops_only_its_keys_but_retires_every_ticket() {
        let cache: GenerationCache<&str, u32> = GenerationCache::new(16);
        let first = miss_ticket(cache.lookup(&(), "kept"));
        assert!(cache.fill(first, (), "kept", 1));
        let second = miss_ticket(cache.lookup(&(), "dropped"));
        assert!(cache.fill(second, (), "dropped", 2));

        let in_flight = miss_ticket(cache.lookup(&(), "other"));
        cache.invalidate_where(|key| *key == "dropped");

        assert!(matches!(cache.lookup(&(), "kept"), CacheLookup::Hit(1)));
        miss_ticket(cache.lookup(&(), "dropped"));
        assert!(!cache.fill(in_flight, (), "other", 3));
    }

    #[test]
    fn a_new_tag_drops_values_decoded_under_the_old_one_and_retires_old_tickets() {
        let cache: GenerationCache<&str, u32, u64> = GenerationCache::new(16);
        let ticket = miss_ticket(cache.lookup(&1, "key"));
        assert!(cache.fill(ticket, 1, "key", 10));
        assert!(matches!(cache.lookup(&1, "key"), CacheLookup::Hit(10)));

        let old_tag_ticket = miss_ticket(cache.lookup(&1, "other"));
        let new_tag_ticket = miss_ticket(cache.lookup(&2, "key"));
        assert!(cache.fill(new_tag_ticket, 2, "key", 20));
        assert!(matches!(cache.lookup(&2, "key"), CacheLookup::Hit(20)));
        miss_ticket(cache.lookup(&1, "key"));
        assert!(!cache.fill(old_tag_ticket, 1, "other", 11));
    }

    #[test]
    fn reaching_the_entry_bound_starts_over_instead_of_growing() {
        let cache: GenerationCache<u32, u32> = GenerationCache::new(2);
        for key in 0..2 {
            let ticket = miss_ticket(cache.lookup(&(), &key));
            assert!(cache.fill(ticket, (), key, key));
        }
        assert_eq!(cache.len(), 2);
        let ticket = miss_ticket(cache.lookup(&(), &2));
        assert!(cache.fill(ticket, (), 2, 2));
        assert_eq!(cache.len(), 1);
        assert!(matches!(cache.lookup(&(), &2), CacheLookup::Hit(2)));
    }
}
