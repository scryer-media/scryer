//! Bounded stable enrichment shared across polls; mutable state belongs to one pass.
use crate::{AppUseCase, ClientJobLocator, DownloadSubmission, PersistedSeedGoals};
use scryer_domain::download_identity::DownloadId;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

const MAX_ENTRIES: usize = 8_192;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const POSITIVE_TTL: Duration = Duration::from_secs(600);
const NEGATIVE_TTL: Duration = Duration::from_secs(60);

/// Only fields used by the queue projection, never listing payloads or secrets.
#[derive(Clone, Debug)]
pub(crate) struct SubmissionProjection {
    pub download_id: DownloadId,
    pub download_client_id: Option<String>,
    pub download_client_type: String,
    pub title_id: String,
    pub episode_id: Option<String>,
    pub facet: String,
    pub source_title: Option<String>,
    pub source_provider_name: Option<String>,
    pub source_hint: Option<String>,
    pub is_scryer_origin: bool,
}
impl From<&DownloadSubmission> for SubmissionProjection {
    fn from(s: &DownloadSubmission) -> Self {
        Self {
            download_id: s.download_id,
            download_client_id: s.download_client_id.clone(),
            download_client_type: s.download_client_type.clone(),
            title_id: s.title_id.clone(),
            episode_id: s.scope.episode_id().map(str::to_owned),
            facet: s.facet.clone(),
            source_title: s.source_title.clone(),
            source_provider_name: s.source_provider_name.clone(),
            source_hint: s.source_hint.clone(),
            is_scryer_origin: crate::import_parameters::submission_has_scryer_origin(s),
        }
    }
}
impl SubmissionProjection {
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.download_client_type.capacity()
            + self.title_id.capacity()
            + self.facet.capacity()
            + [
                &self.download_client_id,
                &self.episode_id,
                &self.source_title,
                &self.source_provider_name,
                &self.source_hint,
            ]
            .into_iter()
            .map(|s| s.as_ref().map_or(0, String::capacity))
            .sum::<usize>()
    }
}

/// Match the existing queue lookup normalization, without merging clients.
pub(crate) fn locator_key(locator: &ClientJobLocator) -> ClientJobLocator {
    ClientJobLocator::new(
        locator
            .client_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        &locator.client_type,
        &locator.item_id,
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CacheKey {
    Submission(ClientJobLocator),
    Goals(ClientJobLocator),
    Fallback(DownloadId, ClientJobLocator),
}
impl CacheKey {
    fn bytes(&self) -> usize {
        let locator = match self {
            Self::Submission(k) | Self::Goals(k) | Self::Fallback(_, k) => k,
        };
        std::mem::size_of::<Self>()
            + locator.client_id.as_ref().map_or(0, String::capacity)
            + locator.client_type.capacity()
            + locator.item_id.capacity()
    }
}
#[derive(Clone, Debug)]
pub(crate) enum CacheValue {
    Submission(Option<Arc<SubmissionProjection>>),
    Goals(Option<PersistedSeedGoals>),
}
impl CacheValue {
    fn negative(&self) -> bool {
        matches!(self, Self::Submission(None) | Self::Goals(None))
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Submission(Some(s)) => s.bytes(),
                Self::Goals(Some(g)) => {
                    g.seeding_profile_id.as_ref().map_or(0, String::capacity)
                        + g.info_hash.as_ref().map_or(0, String::capacity)
                }
                _ => 0,
            }
    }
}
#[derive(Debug)]
struct Entry {
    value: CacheValue,
    loaded: Instant,
    order: u64,
    bytes: usize,
}
#[derive(Debug, Default)]
pub(crate) struct CacheState {
    generation: u64,
    entries: HashMap<CacheKey, Entry>,
    lru: BTreeMap<u64, CacheKey>,
    order: u64,
    bytes: usize,
}
impl CacheState {
    fn generation(&mut self, generation: u64) {
        if self.generation != generation {
            self.entries.clear();
            self.lru.clear();
            self.bytes = 0;
            self.generation = generation;
            self.gauges();
        }
    }
    fn gauges(&self) {
        metrics::gauge!("scryer_download_enrichment_cache_entries").set(self.entries.len() as f64);
        metrics::gauge!("scryer_download_enrichment_cache_bytes").set(self.bytes as f64);
    }
    fn remove(&mut self, key: &CacheKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.lru.remove(&entry.order);
            self.bytes -= entry.bytes;
        }
    }
    pub(crate) fn get(
        &mut self,
        generation: u64,
        key: &CacheKey,
        now: Instant,
    ) -> Option<CacheValue> {
        self.generation(generation);
        if self.entries.get(key).is_some_and(|e| {
            now.duration_since(e.loaded)
                >= if e.value.negative() {
                    NEGATIVE_TTL
                } else {
                    POSITIVE_TTL
                }
        }) {
            self.remove(key);
            self.gauges();
        }
        let Some(entry) = self.entries.get_mut(key) else {
            metrics::counter!("scryer_download_enrichment_cache_total", "result" => "miss")
                .increment(1);
            return None;
        };
        self.lru.remove(&entry.order);
        self.order += 1;
        entry.order = self.order;
        self.lru.insert(entry.order, key.clone());
        metrics::counter!("scryer_download_enrichment_cache_total", "result" =>
            if entry.value.negative() { "negative_hit" } else { "hit" })
        .increment(1);
        Some(entry.value.clone())
    }
    pub(crate) fn insert(
        &mut self,
        generation: u64,
        key: CacheKey,
        value: CacheValue,
        now: Instant,
    ) {
        self.generation(generation);
        // Both maps retain a key. Include entry/node overhead conservatively.
        let bytes = 2 * key.bytes() + value.bytes() + std::mem::size_of::<Entry>() + 128;
        self.remove(&key);
        if bytes > MAX_BYTES {
            self.gauges();
            return;
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES {
            let Some((_, oldest)) = self.lru.first_key_value() else {
                break;
            };
            let oldest = oldest.clone();
            self.remove(&oldest);
            metrics::counter!("scryer_download_enrichment_cache_total", "result" => "eviction")
                .increment(1);
        }
        self.order += 1;
        self.lru.insert(self.order, key.clone());
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                value,
                loaded: now,
                order: self.order,
                bytes,
            },
        );
        self.gauges();
    }
}
#[derive(Debug, Default)]
pub(crate) struct DownloadEnrichmentCache {
    pub(crate) state: Mutex<CacheState>,
    pub(crate) fill: Mutex<()>,
}

#[derive(Clone)]
pub(crate) struct QueueAttribution {
    client_id: String,
    client_name: String,
    client_type: String,
    title_id: Option<String>,
    episode_id: Option<String>,
    facet: Option<String>,
    title_name: String,
    source_provider: Option<String>,
    is_scryer_origin: bool,
    tracked_state: Option<scryer_domain::TrackedDownloadState>,
    goals: (Option<f64>, Option<i64>, bool),
}
impl QueueAttribution {
    pub(crate) fn capture(item: &crate::DownloadQueueItem) -> Self {
        Self {
            client_id: item.client_id.clone(),
            client_name: item.client_name.clone(),
            client_type: item.client_type.clone(),
            title_id: item.title_id.clone(),
            episode_id: item.episode_id.clone(),
            facet: item.facet.clone(),
            title_name: item.title_name.clone(),
            source_provider: item.source_provider.clone(),
            is_scryer_origin: item.is_scryer_origin,
            tracked_state: item.tracked_state,
            goals: item.seeding.as_ref().map_or((None, None, false), |s| {
                (s.seed_goal_ratio, s.seed_goal_seconds, s.never_remove)
            }),
        }
    }
    fn restore(&self, after: &Self, item: &mut crate::DownloadQueueItem) {
        macro_rules! restore { ($($field:ident),*) => { $(
            if item.$field == after.$field { item.$field.clone_from(&self.$field); }
        )* }; }
        restore!(
            client_id,
            client_name,
            client_type,
            title_id,
            episode_id,
            facet,
            title_name,
            source_provider,
            is_scryer_origin,
            tracked_state
        );
        if let Some(s) = &mut item.seeding {
            if s.seed_goal_ratio == after.goals.0 {
                s.seed_goal_ratio = self.goals.0;
            }
            if s.seed_goal_seconds == after.goals.1 {
                s.seed_goal_seconds = self.goals.1;
            }
            if s.never_remove == after.goals.2 {
                s.never_remove = self.goals.2;
            }
        }
    }
}
struct AppliedAttribution {
    generation: u64,
    before: QueueAttribution,
    after: QueueAttribution,
}
fn applied_key(item: &crate::DownloadQueueItem) -> (ClientJobLocator, Option<String>) {
    (
        locator_key(&ClientJobLocator::new(
            Some(&item.client_id),
            &item.client_type,
            &item.download_client_item_id,
        )),
        item.download_id.clone(),
    )
}

#[derive(Default)]
pub(crate) struct DownloadEnrichmentPass {
    generation: Option<u64>,
    applied: HashMap<(ClientJobLocator, Option<String>), AppliedAttribution>,
    pub(crate) submissions: HashMap<ClientJobLocator, Option<Arc<SubmissionProjection>>>,
    pub(crate) goals: HashMap<ClientJobLocator, Option<PersistedSeedGoals>>,
    pub(crate) states: HashMap<ClientJobLocator, Option<String>>,
}
impl DownloadEnrichmentPass {
    pub(crate) fn restore_invalidated(
        &mut self,
        app: &AppUseCase,
        items: &mut [crate::DownloadQueueItem],
    ) {
        let generation = app.runtime.acquisition.download_registry_generation();
        for item in items {
            let key = applied_key(item);
            if self
                .applied
                .get(&key)
                .is_some_and(|old| old.generation != generation)
                && let Some(old) = self.applied.remove(&key)
            {
                old.before.restore(&old.after, item);
            }
        }
    }
    pub(crate) fn remember(&mut self, item: &crate::DownloadQueueItem, before: QueueAttribution) {
        let generation = self.generation.unwrap_or(0);
        let after = QueueAttribution::capture(item);
        self.applied
            .entry(applied_key(item))
            .and_modify(|old| old.after = after.clone())
            .or_insert(AppliedAttribution {
                generation,
                before,
                after,
            });
    }
    /// Two generation-fenced attempts, then a non-retained read under sustained churn.
    pub(crate) async fn load(&mut self, app: &AppUseCase, locators: &[ClientJobLocator]) {
        let cache = &app.runtime.acquisition.download_enrichment;
        let _fill = cache.fill.lock().await;
        for attempt in 0..3 {
            let generation = app.runtime.acquisition.download_registry_generation();
            if self.generation != Some(generation) || attempt == 2 {
                let applied = std::mem::take(&mut self.applied);
                *self = Self {
                    generation: Some(generation),
                    applied,
                    ..Self::default()
                };
            }
            if attempt < 2 {
                let mut shared = cache.state.lock().await;
                for key in locators {
                    if !self.submissions.contains_key(key)
                        && let Some(CacheValue::Submission(value)) = shared.get(
                            generation,
                            &CacheKey::Submission(key.clone()),
                            Instant::now(),
                        )
                    {
                        self.submissions.insert(key.clone(), value);
                    }
                    if !self.goals.contains_key(key)
                        && let Some(CacheValue::Goals(value)) =
                            shared.get(generation, &CacheKey::Goals(key.clone()), Instant::now())
                    {
                        self.goals.insert(key.clone(), value);
                    }
                }
            }
            let repo = &app.services.workflow.download_submissions;
            let mut fills = Vec::new();
            let missing: Vec<_> = locators
                .iter()
                .filter(|k| !self.submissions.contains_key(*k))
                .cloned()
                .collect();
            if !missing.is_empty() {
                metrics::counter!("scryer_download_enrichment_batch_total", "family" => "submission").increment(1);
                match repo.list_for_client_items(&missing).await {
                    Ok(rows) => {
                        let mut found: HashMap<_, _> = rows
                            .iter()
                            .filter(|s| !s.title_id.trim().is_empty())
                            .map(|s| {
                                (
                                    locator_key(&ClientJobLocator::from_submission(s)),
                                    Arc::new(SubmissionProjection::from(s)),
                                )
                            })
                            .collect();
                        for key in missing {
                            let value = found.remove(&key);
                            fills.push((
                                CacheKey::Submission(key.clone()),
                                CacheValue::Submission(value.clone()),
                            ));
                            self.submissions.insert(key, value);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to batch-load download submissions for queue enrichment")
                    }
                }
            }
            let missing: Vec<_> = locators
                .iter()
                .filter(|k| !self.goals.contains_key(*k))
                .cloned()
                .collect();
            if !missing.is_empty() {
                metrics::counter!("scryer_download_enrichment_batch_total", "family" => "goals")
                    .increment(1);
                match repo.list_seed_goals_for_client_items(&missing).await {
                    Ok(rows) => {
                        let mut found: HashMap<_, _> = rows
                            .into_iter()
                            .map(|(k, mut v)| {
                                // Queue enrichment consumes frozen policy, not profile
                                // attribution or the hash used by other lookup paths.
                                v.seeding_profile_id = None;
                                v.info_hash = None;
                                (locator_key(&k), v)
                            })
                            .collect();
                        for key in missing {
                            let value = found.remove(&key);
                            fills.push((
                                CacheKey::Goals(key.clone()),
                                CacheValue::Goals(value.clone()),
                            ));
                            self.goals.insert(key, value);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to batch-load seeding goals for queue enrichment")
                    }
                }
            }
            let missing: Vec<_> = locators
                .iter()
                .filter(|k| !self.states.contains_key(*k))
                .cloned()
                .collect();
            if !missing.is_empty() {
                metrics::counter!("scryer_download_enrichment_batch_total", "family" => "state")
                    .increment(1);
                match repo
                    .list_identity_tracked_states_for_client_items(&missing)
                    .await
                {
                    Ok(rows) => {
                        let mut found: HashMap<_, _> = rows
                            .into_iter()
                            .map(|(k, v)| (locator_key(&k), v))
                            .collect();
                        for key in missing {
                            self.states.insert(key.clone(), found.remove(&key));
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to batch-load durable tracked states for queue enrichment")
                    }
                }
            }
            if attempt == 2 {
                self.generation = None;
                return;
            }
            let mut shared = cache.state.lock().await;
            if app.runtime.acquisition.download_registry_generation() != generation {
                self.generation = None;
                continue;
            }
            // Keep the two families adjacent in lookup order. Inserting all
            // submissions before all goals would let a large batch evict one
            // entire family, even though those rows were observed together.
            let mut fills: HashMap<_, _> = fills.into_iter().collect();
            for locator in locators {
                for key in [
                    CacheKey::Submission(locator.clone()),
                    CacheKey::Goals(locator.clone()),
                ] {
                    if let Some(value) = fills.remove(&key) {
                        shared.insert(generation, key, value, Instant::now());
                    }
                }
            }
            return;
        }
    }
}

#[cfg(test)]
mod download_enrichment_tests {
    use super::*;
    fn key(n: usize) -> CacheKey {
        CacheKey::Submission(ClientJobLocator::new(
            Some("client"),
            "qbittorrent",
            format!("item-{n}"),
        ))
    }
    fn positive() -> CacheValue {
        CacheValue::Submission(Some(Arc::new(SubmissionProjection {
            download_id: DownloadId::new(),
            download_client_id: Some("client".into()),
            download_client_type: "qbittorrent".into(),
            title_id: "title".into(),
            episode_id: None,
            facet: "anime".into(),
            source_title: None,
            source_provider_name: None,
            source_hint: None,
            is_scryer_origin: true,
        })))
    }
    #[test]
    fn download_enrichment_expiration_is_from_fill_not_last_hit() {
        let now = Instant::now();
        let mut cache = CacheState::default();
        cache.insert(1, key(0), positive(), now);
        cache.insert(1, key(1), CacheValue::Submission(None), now);
        assert!(
            cache
                .get(1, &key(1), now + Duration::from_secs(59))
                .is_some()
        );
        assert!(cache.get(1, &key(1), now + NEGATIVE_TTL).is_none());
        assert!(
            cache
                .get(1, &key(0), now + Duration::from_secs(599))
                .is_some()
        );
        assert!(cache.get(1, &key(0), now + POSITIVE_TTL).is_none());
        assert_eq!(cache.bytes, 0);
    }
    #[test]
    fn download_enrichment_capacity_evicts_least_recently_used_only() {
        let now = Instant::now();
        let mut cache = CacheState::default();
        for n in 0..MAX_ENTRIES {
            cache.insert(1, key(n), CacheValue::Submission(None), now);
        }
        assert!(cache.get(1, &key(0), now).is_some());
        cache.insert(1, key(MAX_ENTRIES), positive(), now);
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.bytes <= MAX_BYTES);
        assert!(cache.get(1, &key(0), now).is_some());
        assert!(cache.get(1, &key(1), now).is_none());
        assert!(cache.get(1, &key(2), now).is_some());
        assert_eq!(cache.entries.len(), cache.lru.len());
    }
    #[test]
    fn download_enrichment_byte_limit_and_oversized_values_are_bounded() {
        let now = Instant::now();
        let mut cache = CacheState::default();
        for n in 0..40 {
            let CacheValue::Submission(Some(mut projection)) = positive() else {
                unreachable!()
            };
            Arc::make_mut(&mut projection).source_title = Some("x".repeat(1024 * 1024));
            cache.insert(1, key(n), CacheValue::Submission(Some(projection)), now);
            assert!(cache.bytes <= MAX_BYTES);
        }
        assert!(cache.entries.len() < 40);
        let CacheValue::Submission(Some(mut projection)) = positive() else {
            unreachable!()
        };
        Arc::make_mut(&mut projection).source_title = Some("x".repeat(MAX_BYTES));
        cache.insert(1, key(50), CacheValue::Submission(Some(projection)), now);
        assert!(cache.get(1, &key(50), now).is_none());
        assert!(cache.get(1, &key(39), now).is_some());
    }
    #[test]
    fn download_enrichment_generation_and_restart_drop_all_old_results() {
        let now = Instant::now();
        let mut cache = CacheState::default();
        cache.insert(1, key(0), positive(), now);
        cache.insert(1, key(1), CacheValue::Submission(None), now);
        assert!(cache.get(2, &key(0), now).is_none());
        assert!(cache.get(2, &key(1), now).is_none());
        assert_eq!(cache.bytes, 0);
        assert!(cache.lru.is_empty());
        assert!(CacheState::default().get(2, &key(0), now).is_none());
    }
}
