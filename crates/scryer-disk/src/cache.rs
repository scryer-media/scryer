//! The short-lived stat cache for configured roots.
//!
//! Readings are kept in memory only, never persisted, never written to the
//! database, and only for paths under a configured root that is cacheable.
//! A reading that says what is there, or that nothing is, is trusted for
//! [`DISK_STAT_REUSE`] by a probing read and a cache-only read alike; past
//! that, a cache-only read reports nothing and a probing read probes. A read
//! that failed or did not answer in time is never kept: it forgets what was
//! held for the path instead.
//!
//! Every mutation made through the owner evicts what it can have changed.
//! A probe that was out while an eviction happened does not store its answer,
//! so a reading taken before a change can never land after it.
//!
//! Spellings of one path are matched by lexical rules only, never by asking
//! the disk. Repeated and trailing separators and `.` components are one
//! spelling. Spellings that differ only in letter case share one slot: a
//! reading is answered only to the spelling it was taken under, and a
//! mutation through either spelling evicts it. A path with a `..` component
//! is never cached, and a mutation through one forgets every reading. A path
//! reached through a link to one of its parents, or spelled in another
//! Unicode form, is not recognised as the same path.

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
    /// The reading and the spelling it was taken under.
    reading: Option<(PathBuf, PathReading)>,
    /// Held while a probe of this path is out; callers that find it held wait,
    /// then answer from the reading the probe left.
    flight: Arc<Mutex<()>>,
}

#[derive(Debug, Default)]
pub(crate) struct StatCache {
    /// Keyed by [`slot_of`] the spelling.
    entries: RwLock<BTreeMap<PathBuf, Entry>>,
    /// Advanced by every eviction. A probe stores only when it has not moved
    /// since the probe began.
    epoch: AtomicU64,
}

/// The cache key: the path's components, so `a//b/` and `a/b` are one entry.
pub(crate) fn key_of(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Whether `path` names a parent through `..`, which lexical rules cannot
/// tell apart from another spelling of the same path.
pub(crate) fn climbs(path: &Path) -> bool {
    path.components()
        .any(|component| component == std::path::Component::ParentDir)
}

/// The slot a key's readings live in: the key with its letter case folded,
/// so a mutation through any case of a name evicts the others.
fn slot_of(key: &Path) -> PathBuf {
    key.components()
        .map(|component| {
            let part = component.as_os_str();
            part.to_str()
                .map_or_else(|| part.to_os_string(), |text| text.to_lowercase().into())
        })
        .collect()
}

/// The reading held for exactly `key`, if any.
fn held(entries: &BTreeMap<PathBuf, Entry>, key: &Path) -> Option<PathReading> {
    let (spelling, reading) = entries.get(&slot_of(key))?.reading.as_ref()?;
    (spelling == key).then_some(*reading)
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
        let reading = held(&read(&self.entries), key)?;
        age_under(reading.observed_at()?, now, DISK_STAT_REUSE).then_some(reading)
    }

    /// A complete reading taken after `asked_at` or under [`DISK_STAT_REUSE`]
    /// before it, which a probing read may answer from.
    pub(crate) fn reusable(&self, key: &Path, asked_at: DateTime<Utc>) -> Option<PathReading> {
        let reading = held(&read(&self.entries), key)?;
        let observed_at = reading.observed_at()?;
        (reading.is_complete() && super::reusable(observed_at, asked_at, DISK_STAT_REUSE))
            .then_some(reading)
    }

    pub(crate) fn flight(&self, key: &Path) -> Arc<Mutex<()>> {
        let slot = slot_of(key);
        if let Some(entry) = read(&self.entries).get(&slot) {
            return entry.flight.clone();
        }
        let mut entries = write(&self.entries);
        let flight = entries.entry(slot).or_default().flight.clone();
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
        let dir = slot_of(dir);
        let listed: HashSet<PathBuf> = children.iter().map(|(path, _)| slot_of(path)).collect();
        let gone: Vec<PathBuf> = subtree(&entries, &dir)
            .filter(|path| path.parent() == Some(dir.as_path()) && !listed.contains(path))
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
        self.forget(key, |entries, slot| {
            entries.remove(slot);
        });
    }

    /// Forget `key`, everything under it, and its parent.
    pub(crate) fn evict_subtree(&self, key: &Path) {
        self.forget(key, |entries, slot| {
            let doomed: Vec<PathBuf> = subtree(entries, slot).collect();
            for path in doomed {
                entries.remove(&path);
            }
        });
    }

    /// Forget `key` and every ancestor, for a change that may have created
    /// any of them.
    pub(crate) fn evict_ancestors(&self, key: &Path) {
        self.forget(key, |entries, slot| {
            for ancestor in slot.ancestors() {
                entries.remove(ancestor);
            }
        });
    }

    /// Advance the epoch, apply `evict` to the slot of `key`, and forget its
    /// parent. A key that climbs through `..` can name any path, so it
    /// forgets every reading.
    fn forget(&self, key: &Path, evict: impl FnOnce(&mut BTreeMap<PathBuf, Entry>, &Path)) {
        let mut entries = write(&self.entries);
        self.epoch.fetch_add(1, Ordering::SeqCst);
        if climbs(key) {
            for entry in entries.values_mut() {
                entry.reading = None;
            }
            return;
        }
        let slot = slot_of(key);
        evict(&mut entries, &slot);
        if let Some(parent) = slot.parent() {
            entries.remove(parent);
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
    let entry = entries.entry(slot_of(key)).or_default();
    if entry
        .reading
        .as_ref()
        .and_then(|(_, held)| held.observed_at())
        .zip(reading.observed_at())
        .is_some_and(|(existing, new)| existing > new)
    {
        return;
    }
    // A failed or unanswered read is not knowledge of the path; it only
    // stops an older reading from being answered.
    entry.reading = reading
        .exists()
        .is_some()
        .then(|| (key.to_path_buf(), reading));
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
                entry
                    .reading
                    .as_ref()
                    .and_then(|(_, reading)| reading.observed_at()),
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
    use crate::reading::EntryKind;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_700_000_000 + seconds, 0).expect("instant")
    }

    fn known(seconds: i64) -> PathReading {
        PathReading::Known {
            kind: EntryKind::File,
            len: Some(1),
            modified: None,
            identity: None,
            links: None,
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
    fn a_failed_read_is_not_kept_and_forgets_the_older_reading() {
        let cache = StatCache::default();
        cache.store(&key("/srv/m/a"), known(0), 0);
        let failed = PathReading::Unreachable {
            observed_at: at(0),
            cause: crate::reason::DISK_TIMED_OUT,
        };
        cache.store(&key("/srv/m/a"), failed, 0);
        assert!(cache.current(&key("/srv/m/a"), at(0)).is_none());
        assert!(cache.reusable(&key("/srv/m/a"), at(0)).is_none());
    }

    #[test]
    fn a_reading_is_answered_only_to_its_spelling_and_evicted_through_any_case() {
        let cache = StatCache::default();
        cache.store(&key("/srv/m/Show/a.mkv"), known(0), 0);
        cache.store(&key("/srv/m/Show/b.mkv"), known(0), 0);
        assert!(cache.current(&key("/srv/m/show/a.mkv"), at(0)).is_none());
        assert!(cache.current(&key("/srv/m/Show/a.mkv"), at(0)).is_some());
        cache.evict(&key("/srv/m/SHOW/A.MKV"));
        assert!(cache.current(&key("/srv/m/Show/a.mkv"), at(0)).is_none());
        cache.evict_subtree(&key("/srv/m/show"));
        assert!(cache.current(&key("/srv/m/Show/b.mkv"), at(0)).is_none());
    }

    #[test]
    fn a_mutation_through_a_parent_forgets_every_reading() {
        let cache = StatCache::default();
        cache.store(&key("/srv/m/a"), known(0), 0);
        cache.store(&key("/srv/other/b"), known(0), 0);
        cache.evict(&key("/srv/m/Show/../a"));
        assert!(cache.current(&key("/srv/m/a"), at(0)).is_none());
        assert!(cache.current(&key("/srv/other/b"), at(0)).is_none());
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
