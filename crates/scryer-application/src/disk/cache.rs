//! The short-lived stat cache for configured roots.
//!
//! Readings are kept in memory only, never persisted, never written to the
//! database, and only for paths under a configured root that is cacheable.
//! A reading is trusted for [`DISK_STAT_REUSE`], by a probing read and a
//! cache-only read alike, whatever it says; past that, a cache-only read
//! reports nothing and a probing read probes.
//!
//! Every mutation made through the owner evicts what it can have changed.
//! A probe that was out while an eviction happened does not store its answer,
//! so a reading taken before a change can never land after it.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Utc};

use super::reading::PathReading;
use super::{read, write};

/// A probing read answers from a reading this recent instead of calling the
/// disk again, so a burst of callers costs at most one call per path per
/// second: the same rule capacity readings follow.
pub const DISK_STAT_REUSE: std::time::Duration = super::capacity::STORAGE_CAPACITY_REFRESH_REUSE;

/// Paths kept at most. Past that, the oldest readings with no probe out are
/// dropped down to three quarters of this, so trimming is not paid per insert.
pub(crate) const DISK_CACHE_MAX: usize = 8192;

#[derive(Debug, Default)]
struct Entry {
    reading: Option<PathReading>,
    /// Held while a probe of this path is out; callers that find it held wait,
    /// then answer from the reading the probe left.
    flight: Arc<Mutex<()>>,
}

#[derive(Debug, Default)]
pub(crate) struct StatCache {
    entries: RwLock<BTreeMap<PathBuf, Entry>>,
    /// Advanced by every eviction. A probe stores only when it has not moved
    /// since the probe began.
    epoch: AtomicU64,
}

/// The cache key: the path's components, so `a//b/` and `a/b` are one entry.
pub(crate) fn key_of(path: &Path) -> PathBuf {
    path.components().collect()
}

fn age_under(observed_at: DateTime<Utc>, now: DateTime<Utc>, bound: std::time::Duration) -> bool {
    chrono::Duration::from_std(bound).is_ok_and(|bound| now - observed_at < bound)
}

impl StatCache {
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// The reading a cache-only read may report at `now`.
    pub(crate) fn current(&self, key: &Path, now: DateTime<Utc>) -> Option<PathReading> {
        let reading = read(&self.entries)
            .get(key)
            .and_then(|entry| entry.reading)?;
        age_under(reading.observed_at()?, now, DISK_STAT_REUSE).then_some(reading)
    }

    /// A complete reading taken after `asked_at` or under [`DISK_STAT_REUSE`]
    /// before it, which a probing read may answer from.
    pub(crate) fn reusable(&self, key: &Path, asked_at: DateTime<Utc>) -> Option<PathReading> {
        let reading = read(&self.entries)
            .get(key)
            .and_then(|entry| entry.reading)?;
        let observed_at = reading.observed_at()?;
        (reading.is_complete() && super::reusable(observed_at, asked_at, DISK_STAT_REUSE))
            .then_some(reading)
    }

    pub(crate) fn flight(&self, key: &Path) -> Arc<Mutex<()>> {
        if let Some(entry) = read(&self.entries).get(key) {
            return entry.flight.clone();
        }
        let mut entries = write(&self.entries);
        let flight = entries.entry(key.to_path_buf()).or_default().flight.clone();
        trim(&mut entries);
        flight
    }

    /// Store `reading` unless an eviction happened since `epoch` or the path
    /// already holds a newer reading.
    pub(crate) fn store(&self, key: &Path, reading: PathReading, epoch: u64) {
        let mut entries = write(&self.entries);
        if self.epoch() != epoch {
            return;
        }
        put(&mut entries, key, reading);
        trim(&mut entries);
    }

    /// Store what a listing of `dir` said about its children, and forget any
    /// cached child the listing did not return.
    pub(crate) fn seed_listing(&self, dir: &Path, children: &[(PathBuf, PathReading)], epoch: u64) {
        let mut entries = write(&self.entries);
        if self.epoch() != epoch {
            return;
        }
        let listed: HashSet<&Path> = children.iter().map(|(path, _)| path.as_path()).collect();
        let gone: Vec<PathBuf> = subtree(&entries, dir)
            .filter(|path| path.parent() == Some(dir) && !listed.contains(path.as_path()))
            .collect();
        for path in gone {
            entries.remove(&path);
        }
        for (path, reading) in children {
            if *reading != PathReading::NotObserved {
                put(&mut entries, path, *reading);
            }
        }
        trim(&mut entries);
    }

    /// Forget `key` and its parent.
    pub(crate) fn evict(&self, key: &Path) {
        let mut entries = write(&self.entries);
        self.epoch.fetch_add(1, Ordering::SeqCst);
        entries.remove(key);
        if let Some(parent) = key.parent() {
            entries.remove(parent);
        }
    }

    /// Forget `key`, everything under it, and its parent.
    pub(crate) fn evict_subtree(&self, key: &Path) {
        let mut entries = write(&self.entries);
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let doomed: Vec<PathBuf> = subtree(&entries, key).collect();
        for path in doomed {
            entries.remove(&path);
        }
        if let Some(parent) = key.parent() {
            entries.remove(parent);
        }
    }

    /// Forget `key` and every ancestor, for a change that may have created
    /// any of them.
    pub(crate) fn evict_ancestors(&self, key: &Path) {
        let mut entries = write(&self.entries);
        self.epoch.fetch_add(1, Ordering::SeqCst);
        for ancestor in key.ancestors() {
            entries.remove(ancestor);
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        read(&self.entries).len()
    }
}

/// `key` and every cached path under it. Paths order by component, so a
/// directory's descendants follow it contiguously.
fn subtree<'a>(
    entries: &'a BTreeMap<PathBuf, Entry>,
    key: &'a Path,
) -> impl Iterator<Item = PathBuf> + 'a {
    entries
        .range::<Path, _>((std::ops::Bound::Included(key), std::ops::Bound::Unbounded))
        .map(|(path, _)| path)
        .take_while(move |path| path.starts_with(key))
        .cloned()
}

fn put(entries: &mut BTreeMap<PathBuf, Entry>, key: &Path, reading: PathReading) {
    let entry = entries.entry(key.to_path_buf()).or_default();
    if entry
        .reading
        .and_then(PathReading::observed_at)
        .zip(reading.observed_at())
        .is_some_and(|(existing, new)| existing > new)
    {
        return;
    }
    entry.reading = Some(reading);
}

fn trim(entries: &mut BTreeMap<PathBuf, Entry>) {
    if entries.len() <= DISK_CACHE_MAX {
        return;
    }
    let keep = DISK_CACHE_MAX * 3 / 4;
    let mut idle: Vec<(Option<DateTime<Utc>>, PathBuf)> = entries
        .iter()
        .filter(|(_, entry)| Arc::strong_count(&entry.flight) == 1)
        .map(|(path, entry)| {
            (
                entry.reading.and_then(PathReading::observed_at),
                path.clone(),
            )
        })
        .collect();
    idle.sort();
    let excess = entries.len().saturating_sub(keep);
    for (_, path) in idle.into_iter().take(excess) {
        entries.remove(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::reading::EntryKind;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_700_000_000 + seconds, 0).expect("instant")
    }

    fn known(seconds: i64) -> PathReading {
        PathReading::Known {
            kind: EntryKind::File,
            len: Some(1),
            modified: None,
            identity: None,
            observed_at: at(seconds),
        }
    }

    fn key(path: &str) -> PathBuf {
        key_of(Path::new(path))
    }

    #[test]
    fn keys_ignore_repeated_and_trailing_separators() {
        assert_eq!(key("/srv/fixture-media//a/"), key("/srv/fixture-media/a"));
    }

    #[test]
    fn every_reading_is_believed_no_longer_than_the_reuse_window() {
        let cache = StatCache::default();
        cache.store(
            &key("/srv/m/gone"),
            PathReading::Missing { observed_at: at(0) },
            0,
        );
        cache.store(&key("/srv/m/here"), known(0), 0);
        assert!(cache.current(&key("/srv/m/gone"), at(0)).is_some());
        assert!(cache.current(&key("/srv/m/gone"), at(1)).is_none());
        assert!(cache.current(&key("/srv/m/here"), at(0)).is_some());
        assert!(cache.current(&key("/srv/m/here"), at(1)).is_none());
    }

    #[test]
    fn a_probe_out_during_an_eviction_does_not_store() {
        let cache = StatCache::default();
        let epoch = cache.epoch();
        cache.evict(&key("/srv/m/a"));
        cache.store(&key("/srv/m/a"), known(0), epoch);
        assert!(cache.current(&key("/srv/m/a"), at(0)).is_none());
    }

    #[test]
    fn an_older_reading_never_replaces_a_newer_one() {
        let cache = StatCache::default();
        cache.store(&key("/srv/m/a"), known(5), 0);
        cache.store(
            &key("/srv/m/a"),
            PathReading::Missing { observed_at: at(3) },
            0,
        );
        assert_eq!(cache.current(&key("/srv/m/a"), at(5)), Some(known(5)));
    }

    #[test]
    fn subtree_eviction_spares_siblings_that_share_a_prefix() {
        let cache = StatCache::default();
        for path in [
            "/srv/m",
            "/srv/m/a",
            "/srv/m/a/b",
            "/srv/m/a/b/c",
            "/srv/m/a0",
            "/srv/m/b",
        ] {
            cache.store(&key(path), known(0), 0);
        }
        cache.evict_subtree(&key("/srv/m/a"));
        for gone in ["/srv/m", "/srv/m/a", "/srv/m/a/b", "/srv/m/a/b/c"] {
            assert!(cache.current(&key(gone), at(0)).is_none(), "{gone}");
        }
        for kept in ["/srv/m/a0", "/srv/m/b"] {
            assert!(cache.current(&key(kept), at(0)).is_some(), "{kept}");
        }
    }

    #[test]
    fn the_cache_is_bounded_and_keeps_paths_with_a_probe_out() {
        let cache = StatCache::default();
        let held = cache.flight(&key("/srv/m/held"));
        for index in 0..(DISK_CACHE_MAX + 10) {
            cache.store(&key(&format!("/srv/m/{index}")), known(index as i64), 0);
        }
        assert!(cache.len() <= DISK_CACHE_MAX);
        assert!(cache.len() >= DISK_CACHE_MAX * 3 / 4);
        assert!(read(&cache.entries).contains_key(&key("/srv/m/held")));
        // The newest readings are the ones kept.
        let newest = key(&format!("/srv/m/{}", DISK_CACHE_MAX + 9));
        assert!(read(&cache.entries).contains_key(&newest));
        drop(held);
    }
}
