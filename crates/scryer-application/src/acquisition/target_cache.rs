//! The background walk's target set, held in memory between walks.
//!
//! Deriving the target set reads every monitored scope in the catalog, so the
//! walk no longer derives it per pass. The set is built on the first walk and
//! from then on patched: a change that names its title marks that title, and
//! the next walk re-derives only the marked titles. A change that cannot name
//! its titles (a library scan, a settings change) marks the whole missing set,
//! which the next walk rebuilds once.
//!
//! Time changes a derivation in a few known ways: an episode's search window
//! opens, a movie reaches its availability date, a scope leaves the hot lane.
//! Each derivation records, per title, the soonest such instant, and the walk
//! re-derives a title once its instant passes. Nothing is re-derived because
//! a timer fired.
//!
//! The cutoff half is the expensive half (a quality sweep over every occupied
//! scope, and format scoring) and changes only with files, profiles and rules.
//! It is patched per marked title like the missing half, refreshed in full on
//! a profile or rules change, and otherwise refreshed in full only every
//! [`CUTOFF_REFRESH_INTERVAL`], checked when a walk starts.
//!
//! Marking is a `std` mutex around a set, so the event paths that mark never
//! wait on a walk. The cached set itself is only ever touched by the walk.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::*;
use crate::acquisition::targets::{AcquisitionTarget, TargetTransitions};
use crate::acquisition::walk_metrics;

/// How long a full cutoff refresh stands when nothing marks the cutoff set.
/// Per-title changes are patched as they come; this only bounds how long a
/// time-driven cutoff change (a movie reaching availability while it already
/// has a file) waits.
pub(crate) const CUTOFF_REFRESH_INTERVAL: chrono::Duration = chrono::Duration::hours(6);

/// Above this many marked titles, one full derivation is cheaper than that
/// many per-title ones.
const PER_TITLE_PATCH_LIMIT: usize = 64;

#[derive(Debug, Default)]
struct PendingChanges {
    /// The whole missing set must be re-derived.
    all_missing: bool,
    /// The whole cutoff set must be re-derived.
    all_cutoff: bool,
    titles: HashSet<String>,
}

impl PendingChanges {
    fn merge(&mut self, other: PendingChanges) {
        self.all_missing |= other.all_missing;
        self.all_cutoff |= other.all_cutoff;
        self.titles.extend(other.titles);
    }
}

#[derive(Default)]
struct CachedTargets {
    built: bool,
    /// Missing targets first, in the order a full derivation returns them,
    /// then cutoff targets. `missing_len` is the boundary.
    targets: Arc<Vec<AcquisitionTarget>>,
    missing_len: usize,
    /// Per title, when time alone next changes its missing targets.
    recheck_at: HashMap<String, DateTime<Utc>>,
    recheck_queue: BTreeSet<(DateTime<Utc>, String)>,
    cutoff_refreshed_at: Option<DateTime<Utc>>,
}

/// How a walk obtained its target set, as the `source` label of
/// [`walk_metrics::TARGET_CACHE_READS_TOTAL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetCacheRead {
    /// Derived in full: the first walk, or a change marked the whole set.
    Built,
    /// Some titles (or the cutoff half) were re-derived.
    Patched,
    /// Nothing had changed; no derivation read ran.
    Reused,
}

impl TargetCacheRead {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Built => "built",
            Self::Patched => "patched",
            Self::Reused => "reused",
        }
    }
}

/// What the cache holds, for the benchmark and the size gauges.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AcquisitionTargetCacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub titles_with_recheck: usize,
}

#[derive(Default)]
pub(crate) struct AcquisitionTargetCache {
    pending: std::sync::Mutex<PendingChanges>,
    cached: tokio::sync::Mutex<CachedTargets>,
}

impl AcquisitionTargetCache {
    fn pending(&self) -> std::sync::MutexGuard<'_, PendingChanges> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// One title's monitoring, episodes, files or metadata changed.
    pub(crate) fn mark_title(&self, title_id: &str) {
        self.pending().titles.insert(title_id.to_string());
    }

    /// Something changed that no single title names: re-derive the missing
    /// set in full on the next walk.
    pub(crate) fn mark_all_missing(&self) {
        self.pending().all_missing = true;
    }

    /// Profiles or rules changed: re-derive the cutoff set in full on the
    /// next walk.
    pub(crate) fn mark_cutoff(&self) {
        self.pending().all_cutoff = true;
    }

    /// Both halves, for changes (settings) that can move either.
    pub(crate) fn mark_everything(&self) {
        let mut pending = self.pending();
        pending.all_missing = true;
        pending.all_cutoff = true;
    }

    pub(crate) async fn stats(&self) -> AcquisitionTargetCacheStats {
        let cached = self.cached.lock().await;
        cached.stats()
    }
}

impl CachedTargets {
    fn stats(&self) -> AcquisitionTargetCacheStats {
        let target_bytes: usize = self
            .targets
            .iter()
            .map(AcquisitionTarget::estimated_bytes)
            .sum::<usize>()
            + (self.targets.capacity() - self.targets.len())
                * std::mem::size_of::<AcquisitionTarget>();
        // Each recheck entry is a title id in the map and again in the queue.
        let recheck_bytes: usize = self
            .recheck_at
            .keys()
            .map(|title_id| {
                2 * (title_id.len()
                    + std::mem::size_of::<String>()
                    + std::mem::size_of::<DateTime<Utc>>())
            })
            .sum();
        AcquisitionTargetCacheStats {
            entries: self.targets.len(),
            bytes: target_bytes + recheck_bytes,
            titles_with_recheck: self.recheck_at.len(),
        }
    }

    fn set_recheck(&mut self, title_id: &str, at: Option<DateTime<Utc>>) {
        if let Some(previous) = self.recheck_at.remove(title_id) {
            self.recheck_queue.remove(&(previous, title_id.to_string()));
        }
        if let Some(at) = at {
            self.recheck_at.insert(title_id.to_string(), at);
            self.recheck_queue.insert((at, title_id.to_string()));
        }
    }

    /// Titles whose recheck instant has passed, removed from the queue.
    fn take_due(&mut self, now: &DateTime<Utc>) -> Vec<String> {
        let mut due = Vec::new();
        while let Some((at, _)) = self.recheck_queue.first() {
            if at > now {
                break;
            }
            let (_, title_id) = self.recheck_queue.pop_first().expect("first entry exists");
            self.recheck_at.remove(&title_id);
            due.push(title_id);
        }
        due
    }
}

/// The order a full missing derivation returns: episodes, then movies, then
/// series-movie links, each by id.
fn missing_order_key(target: &AcquisitionTarget) -> (u8, &str) {
    match target.media_type.as_str() {
        "episode" => (0, target.episode_id.as_deref().unwrap_or_default()),
        "movie" => (1, target.title_id.as_str()),
        _ => (
            2,
            target.series_movie_link_id.as_deref().unwrap_or_default(),
        ),
    }
}

/// Whether a settings change can move the target set. The walk's own
/// settings (`acquisition.*`: cadence, intensity, thresholds) decide how the
/// set is walked, never what is in it; any other key may.
pub(crate) fn settings_keys_may_move_targets(keys: &[String]) -> bool {
    keys.iter().any(|key| !key.starts_with("acquisition."))
}

/// [`settings_keys_may_move_targets`] for a broadcast receive; a lagged
/// receiver missed keys it cannot name, so it assumes they did.
pub(crate) fn settings_change_may_move_targets(
    changed: &Result<Vec<String>, tokio::sync::broadcast::error::RecvError>,
) -> bool {
    match changed {
        Ok(keys) => settings_keys_may_move_targets(keys),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
        Err(tokio::sync::broadcast::error::RecvError::Closed) => false,
    }
}

/// Re-derived targets for a set of titles, held until every read succeeded.
struct TitlePatch {
    title_id: String,
    missing: Vec<AcquisitionTarget>,
    cutoff: Option<Vec<AcquisitionTarget>>,
    recheck_at: Option<DateTime<Utc>>,
}

impl AppUseCase {
    /// The target set the background walk rotates over, minus nothing: the
    /// walk applies location locks, paused scopes and blocked facets itself.
    pub(crate) async fn cached_acquisition_targets(
        &self,
        now: &DateTime<Utc>,
    ) -> AppResult<(Arc<Vec<AcquisitionTarget>>, TargetCacheRead)> {
        let cache = &self.runtime.acquisition.target_cache;
        let mut cached = cache.cached.lock().await;
        let pending = std::mem::take(&mut *cache.pending());
        let due = if cached.built {
            cached.take_due(now)
        } else {
            Vec::new()
        };
        match self
            .refresh_cached_targets(&mut cached, &pending, &due, now)
            .await
        {
            Ok(read) => {
                metrics::counter!(walk_metrics::TARGET_CACHE_READS_TOTAL, "source" => read.as_str())
                    .increment(1);
                if read != TargetCacheRead::Reused {
                    let stats = cached.stats();
                    metrics::gauge!(walk_metrics::TARGET_CACHE_ENTRIES).set(stats.entries as f64);
                    metrics::gauge!(walk_metrics::TARGET_CACHE_BYTES).set(stats.bytes as f64);
                }
                Ok((cached.targets.clone(), read))
            }
            Err(err) => {
                // Nothing was applied: put back what this walk took so the
                // next one tries again.
                let mut restore = pending;
                restore.titles.extend(due);
                cache.pending().merge(restore);
                Err(err)
            }
        }
    }

    async fn refresh_cached_targets(
        &self,
        cached: &mut CachedTargets,
        pending: &PendingChanges,
        due: &[String],
        now: &DateTime<Utc>,
    ) -> AppResult<TargetCacheRead> {
        let mut titles: Vec<&str> = pending
            .titles
            .iter()
            .map(String::as_str)
            .chain(due.iter().map(String::as_str))
            .collect();
        titles.sort_unstable();
        titles.dedup();
        let too_many = titles.len() > PER_TITLE_PATCH_LIMIT;
        let full_missing = !cached.built || pending.all_missing || too_many;
        let full_cutoff = !cached.built
            || pending.all_cutoff
            || too_many
            || cached
                .cutoff_refreshed_at
                .is_none_or(|at| *now >= at + CUTOFF_REFRESH_INTERVAL);
        if !full_missing && !full_cutoff && titles.is_empty() {
            return Ok(TargetCacheRead::Reused);
        }

        // Every read first; the cache changes only once all of them worked.
        let mut transitions = TargetTransitions::default();
        let rebuilt_missing = if full_missing {
            Some(
                self.derive_missing_targets_timed(now, &mut transitions)
                    .await?,
            )
        } else {
            None
        };
        let rebuilt_cutoff = if full_cutoff {
            let quality = self
                .derive_cutoff_quality_targets_for_search(now, None, true)
                .await?;
            let format = self
                .derive_format_cutoff_targets_for_search(now, None, true, &quality)
                .await?;
            let mut cutoff = quality;
            cutoff.extend(format);
            Some(cutoff)
        } else {
            None
        };
        let mut patches = Vec::new();
        if !(full_missing && full_cutoff) {
            for title_id in &titles {
                let mut title_transitions = TargetTransitions::default();
                let missing = if full_missing {
                    Vec::new()
                } else {
                    self.derive_missing_targets_with_transitions(
                        now,
                        Some(title_id),
                        true,
                        &mut title_transitions,
                    )
                    .await?
                };
                let cutoff = if full_cutoff {
                    None
                } else {
                    let quality = self
                        .derive_cutoff_quality_targets_for_search(now, Some(title_id), true)
                        .await?;
                    let format = self
                        .derive_format_cutoff_targets_for_search(
                            now,
                            Some(title_id),
                            true,
                            &quality,
                        )
                        .await?;
                    let mut cutoff = quality;
                    cutoff.extend(format);
                    Some(cutoff)
                };
                patches.push(TitlePatch {
                    title_id: (*title_id).to_string(),
                    missing,
                    cutoff,
                    recheck_at: title_transitions.soonest_for(title_id),
                });
            }
        }

        let targets = Arc::make_mut(&mut cached.targets);
        let mut cutoff = targets.split_off(cached.missing_len);
        let mut missing = std::mem::take(targets);
        if let Some(rebuilt) = rebuilt_missing {
            missing = rebuilt;
            cached.recheck_at.clear();
            cached.recheck_queue.clear();
            for (title_id, at) in transitions.into_soonest() {
                cached.recheck_queue.insert((at, title_id.clone()));
                cached.recheck_at.insert(title_id, at);
            }
        }
        if let Some(rebuilt) = rebuilt_cutoff {
            cutoff = rebuilt;
            cached.cutoff_refreshed_at = Some(*now);
        }
        if !patches.is_empty() {
            let patched: HashSet<&str> = patches
                .iter()
                .map(|patch| patch.title_id.as_str())
                .collect();
            if !full_missing {
                missing.retain(|target| !patched.contains(target.title_id.as_str()));
            }
            for patch in &mut patches {
                if !full_missing {
                    missing.append(&mut patch.missing);
                    let recheck_at = patch.recheck_at;
                    cached.set_recheck(&patch.title_id, recheck_at);
                }
                if let Some(fresh) = patch.cutoff.take() {
                    replace_title_targets(&mut cutoff, &patch.title_id, fresh);
                }
            }
            if !full_missing {
                missing
                    .sort_by(|left, right| missing_order_key(left).cmp(&missing_order_key(right)));
            }
        }

        // A scope both halves name is walked once, as missing.
        let missing_keys: HashSet<&str> = missing
            .iter()
            .map(|target| target.scope_key.as_str())
            .collect();
        let mut seen = HashSet::new();
        cutoff.retain(|target| {
            !missing_keys.contains(target.scope_key.as_str())
                && seen.insert(target.scope_key.clone())
        });
        drop(missing_keys);
        cached.missing_len = missing.len();
        let targets = Arc::make_mut(&mut cached.targets);
        *targets = missing;
        targets.append(&mut cutoff);
        targets.shrink_to_fit();

        let read = if !cached.built || (full_missing && full_cutoff) {
            TargetCacheRead::Built
        } else {
            TargetCacheRead::Patched
        };
        cached.built = true;
        Ok(read)
    }

    /// The targets one walk rotates over: the cached set minus titles a
    /// location operation holds, paused scopes, and facets a library scan is
    /// blocking. Those three are read per walk; each read returns only the
    /// held, paused or blocked rows. The cached set is shared, not copied,
    /// unless one of them removes something.
    pub(crate) async fn walk_targets(
        &self,
        now: &DateTime<Utc>,
        blocked_facets: &[scryer_domain::MediaFacet],
    ) -> AppResult<WalkTargets> {
        let (all, read) = self.cached_acquisition_targets(now).await?;
        let started = std::time::Instant::now();
        let locked_titles = self.location_owned_title_ids().await?;
        let paused_keys = self.paused_scope_keys(None).await?;
        let excluded = |target: &AcquisitionTarget| {
            locked_titles.contains(&target.title_id)
                || paused_keys.contains(&target.scope_key)
                || blocked_facets.contains(&target.facet)
        };
        let filtered = all.iter().any(excluded).then(|| {
            all.iter()
                .filter(|target| !excluded(target))
                .cloned()
                .collect::<Vec<_>>()
        });
        walk_metrics::record_derivation(walk_metrics::DerivationPass::Filters, started.elapsed());
        Ok(WalkTargets {
            all,
            filtered,
            read,
        })
    }

    /// What the cache holds now. Benchmark support.
    #[doc(hidden)]
    pub async fn acquisition_target_cache_stats(&self) -> AcquisitionTargetCacheStats {
        self.runtime.acquisition.target_cache.stats().await
    }

    /// Runs the walk's target read once, as the walk would, and reports how it
    /// was served. Benchmark and test support.
    #[doc(hidden)]
    pub async fn refresh_acquisition_target_cache(&self) -> AppResult<(usize, &'static str)> {
        let (targets, read) = self.cached_acquisition_targets(&Utc::now()).await?;
        Ok((targets.len(), read.as_str()))
    }

    /// Marks one title changed, as an event naming it would. Benchmark and
    /// test support.
    #[doc(hidden)]
    pub fn mark_acquisition_title_changed(&self, title_id: &str) {
        self.runtime.acquisition.target_cache.mark_title(title_id);
    }
}

/// One walk's target set. See [`AppUseCase::walk_targets`].
pub(crate) struct WalkTargets {
    all: Arc<Vec<AcquisitionTarget>>,
    filtered: Option<Vec<AcquisitionTarget>>,
    pub(crate) read: TargetCacheRead,
}

impl WalkTargets {
    pub(crate) fn as_slice(&self) -> &[AcquisitionTarget] {
        self.filtered.as_deref().unwrap_or(self.all.as_slice())
    }
}

/// Replaces `title_id`'s targets in `targets` with `fresh`, where the old
/// ones stood, so the rest of the rotation keeps its order.
fn replace_title_targets(
    targets: &mut Vec<AcquisitionTarget>,
    title_id: &str,
    fresh: Vec<AcquisitionTarget>,
) {
    let at = targets
        .iter()
        .position(|target| target.title_id == title_id)
        .unwrap_or(targets.len());
    targets.retain(|target| target.title_id != title_id);
    let at = at.min(targets.len());
    targets.splice(at..at, fresh);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(media_type: &str, title_id: &str, entity: &str) -> AcquisitionTarget {
        AcquisitionTarget {
            scope_key: format!("{media_type}:{entity}"),
            title_id: title_id.to_string(),
            library_id: "synthetic-library".to_string(),
            facet: scryer_domain::MediaFacet::Series,
            media_type: media_type.to_string(),
            episode_id: (media_type == "episode").then(|| entity.to_string()),
            collection_id: None,
            series_movie_link_id: (media_type == "series_movie").then(|| entity.to_string()),
            season_number: None,
            episode_number: None,
            is_hot: false,
            hot_by_air_date: false,
            occupied: false,
        }
    }

    #[test]
    fn missing_order_matches_the_full_derivation_order() {
        let mut targets = vec![
            target("series_movie", "title-b", "link-a"),
            target("movie", "title-b", "title-b"),
            target("episode", "title-a", "episode-b"),
            target("movie", "title-a", "title-a"),
            target("episode", "title-c", "episode-a"),
        ];
        targets.sort_by(|left, right| missing_order_key(left).cmp(&missing_order_key(right)));
        let keys: Vec<&str> = targets
            .iter()
            .map(|target| target.scope_key.as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "episode:episode-a",
                "episode:episode-b",
                "movie:title-a",
                "movie:title-b",
                "series_movie:link-a"
            ]
        );
    }

    #[test]
    fn a_patched_title_keeps_its_place_in_the_cutoff_rotation() {
        let mut targets = vec![
            target("episode", "title-a", "a1"),
            target("episode", "title-b", "b1"),
            target("episode", "title-b", "b2"),
            target("episode", "title-c", "c1"),
        ];
        replace_title_targets(
            &mut targets,
            "title-b",
            vec![target("episode", "title-b", "b3")],
        );
        let keys: Vec<&str> = targets
            .iter()
            .map(|target| target.scope_key.as_str())
            .collect();
        assert_eq!(keys, ["episode:a1", "episode:b3", "episode:c1"]);

        replace_title_targets(
            &mut targets,
            "title-d",
            vec![target("episode", "title-d", "d1")],
        );
        assert_eq!(
            targets.last().map(|target| target.scope_key.as_str()),
            Some("episode:d1")
        );

        replace_title_targets(&mut targets, "title-a", Vec::new());
        assert_eq!(
            targets.first().map(|target| target.scope_key.as_str()),
            Some("episode:b3")
        );
    }

    #[test]
    fn recheck_queue_yields_only_due_titles_once() {
        let now = Utc::now();
        let mut cached = CachedTargets::default();
        cached.set_recheck("title-a", Some(now - chrono::Duration::minutes(1)));
        cached.set_recheck("title-b", Some(now + chrono::Duration::hours(1)));
        cached.set_recheck("title-c", Some(now + chrono::Duration::hours(2)));
        // A later derivation moves title-c's instant into the past.
        cached.set_recheck("title-c", Some(now - chrono::Duration::seconds(1)));
        let mut due = cached.take_due(&now);
        due.sort();
        assert_eq!(due, ["title-a", "title-c"]);
        assert!(cached.take_due(&now).is_empty());
        assert_eq!(cached.recheck_at.len(), 1);
        assert_eq!(cached.recheck_queue.len(), 1);
    }
}
