//! A short-lived, bounded store of previewed plans, so confirming a preview
//! does not walk every title folder a second time.
//!
//! An entry is only a candidate. The caller re-reads the catalog facts the
//! plan was built from and reuses the entry only when they digest to the same
//! value the preview recorded; anything else — an expired entry, another
//! actor, another fingerprint, a changed fact — falls back to planning again,
//! which then confirms against the fingerprint exactly as before.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

/// How long a previewed plan stays reusable.
pub const LOCATION_PLAN_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

/// How many previewed plans are held at once, across actors. A plan for a
/// large selection carries every file it moves, so this stays small.
pub const LOCATION_PLAN_CACHE_CAPACITY: usize = 4;

struct Entry<T> {
    actor_id: String,
    fingerprint: String,
    stored_at: Instant,
    value: T,
}

pub struct LocationPlanCache<T> {
    entries: Mutex<VecDeque<Entry<T>>>,
}

impl<T> Default for LocationPlanCache<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
        }
    }
}

impl<T> LocationPlanCache<T> {
    /// Hold `value` for `actor_id` under `fingerprint`, replacing any entry
    /// with the same key and evicting expired and oldest entries.
    pub fn store(&self, actor_id: &str, fingerprint: &str, value: T) {
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|entry| {
            live(entry.stored_at, now)
                && !(entry.actor_id == actor_id && entry.fingerprint == fingerprint)
        });
        while entries.len() >= LOCATION_PLAN_CACHE_CAPACITY {
            entries.pop_front();
        }
        entries.push_back(Entry {
            actor_id: actor_id.to_owned(),
            fingerprint: fingerprint.to_owned(),
            stored_at: now,
            value,
        });
    }

    /// Remove and return the live entry for this actor and fingerprint. An
    /// entry is handed out at most once.
    pub fn take(&self, actor_id: &str, fingerprint: &str) -> Option<T> {
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|entry| live(entry.stored_at, now));
        let index = entries
            .iter()
            .position(|entry| entry.actor_id == actor_id && entry.fingerprint == fingerprint)?;
        entries.remove(index).map(|entry| entry.value)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

fn live(stored_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(stored_at) < LOCATION_PLAN_CACHE_TTL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn an_entry_is_handed_out_once_to_its_own_actor_and_expires() {
        let cache = LocationPlanCache::default();
        cache.store("actor-a", "fp-1", 1);
        assert_eq!(cache.take("actor-b", "fp-1"), None);
        assert_eq!(cache.take("actor-a", "fp-2"), None);
        assert_eq!(cache.take("actor-a", "fp-1"), Some(1));
        assert_eq!(cache.take("actor-a", "fp-1"), None);

        cache.store("actor-a", "fp-1", 2);
        tokio::time::advance(LOCATION_PLAN_CACHE_TTL).await;
        assert_eq!(cache.take("actor-a", "fp-1"), None);
    }

    #[tokio::test]
    async fn the_cache_holds_a_bounded_number_of_plans() {
        let cache = LocationPlanCache::default();
        for index in 0..LOCATION_PLAN_CACHE_CAPACITY + 2 {
            cache.store("actor", &format!("fp-{index}"), index);
        }
        assert_eq!(cache.len(), LOCATION_PLAN_CACHE_CAPACITY);
        assert_eq!(cache.take("actor", "fp-0"), None, "the oldest is evicted");
        assert_eq!(
            cache.take("actor", &format!("fp-{}", LOCATION_PLAN_CACHE_CAPACITY + 1)),
            Some(LOCATION_PLAN_CACHE_CAPACITY + 1)
        );
        // The same key is replaced, not duplicated.
        cache.store("actor", "fp-x", 1);
        cache.store("actor", "fp-x", 2);
        assert_eq!(cache.take("actor", "fp-x"), Some(2));
        assert_eq!(cache.take("actor", "fp-x"), None);
    }
}
