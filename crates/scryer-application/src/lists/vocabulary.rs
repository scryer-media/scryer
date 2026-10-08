use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OnceCell, RwLock};
use tokio::time::{Duration, Instant};

use super::ListSubscriptionRepository;
use crate::{AppError, AppResult, AppUseCase, MetadataGateway};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VocabularyEntry {
    pub key: String,
    pub category: String,
    pub name: String,
    pub aliases: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VocabularyReply {
    pub version: String,
    pub unchanged: bool,
    pub entries: Vec<VocabularyEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VocabularySnapshot {
    pub version: String,
    pub entries: Vec<VocabularyEntry>,
    pub checked_at: DateTime<Utc>,
    pub jitter_seconds: i64,
}

pub fn validate_vocabulary(reply: &VocabularyReply) -> AppResult<()> {
    let invalid = || AppError::Repository("invalid canonical tag vocabulary".into());
    if reply.version.is_empty()
        || reply.version.len() > 128
        || reply.entries.len() > 10_000
        || serde_json::to_vec(reply).map_err(|_| invalid())?.len() > 2 * 1024 * 1024
    {
        return Err(invalid());
    }
    let mut keys = HashSet::new();
    for entry in &reply.entries {
        if !matches!(entry.category.as_str(), "genre" | "theme")
            || !entry
                .key
                .starts_with(&format!("canonical:{}:", entry.category))
            || entry.key.len() > 256
            || entry.name.trim().is_empty()
            || entry.name.len() > 256
            || entry.aliases.len() > 256
            || entry.aliases.iter().any(|alias| alias.len() > 256)
            || !keys.insert(&entry.key)
        {
            return Err(invalid());
        }
    }
    if reply.unchanged != reply.entries.is_empty() {
        return Err(invalid());
    }
    Ok(())
}

#[derive(Default)]
pub struct VocabularyRuntime {
    generation: std::sync::atomic::AtomicU64,
    failed: std::sync::atomic::AtomicBool,
    initialized: OnceCell<()>,
    snapshot: RwLock<Option<VocabularySnapshot>>,
    refresh: Arc<Mutex<()>>,
    retry_at: Mutex<Option<Instant>>,
}

impl VocabularyRuntime {
    pub async fn get(
        self: &Arc<Self>,
        store: Arc<dyn ListSubscriptionRepository>,
        gateway: Arc<dyn MetadataGateway>,
        force: bool,
    ) -> AppResult<VocabularySnapshot> {
        self.initialized
            .get_or_try_init(|| async {
                let cached = store.vocabulary_cache().await?.filter(|snapshot| {
                    validate_vocabulary(&VocabularyReply {
                        version: snapshot.version.clone(),
                        unchanged: false,
                        entries: snapshot.entries.clone(),
                    })
                    .is_ok()
                });
                *self.snapshot.write().await = cached;
                Ok::<_, AppError>(())
            })
            .await?;
        let generation = self.generation.load(std::sync::atomic::Ordering::Acquire);
        let snapshot = self.snapshot.read().await.clone();
        let stale = snapshot.as_ref().is_none_or(|value| {
            Utc::now()
                .signed_duration_since(value.checked_at)
                .num_seconds()
                >= 86400 + value.jitter_seconds.clamp(0, 21600)
        });
        if !force
            && (!stale
                || self
                    .retry_at
                    .lock()
                    .await
                    .is_some_and(|at| Instant::now() < at))
        {
            return snapshot.ok_or_else(unavailable);
        }
        // Only the winner refreshes. Cold concurrent readers join its result.
        let guard = match self.refresh.clone().try_lock_owned() {
            Ok(guard) => guard,
            Err(_) if snapshot.is_some() && !force => return Ok(snapshot.unwrap()),
            Err(_) => {
                let _guard = self.refresh.lock().await;
                if self.failed.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(unavailable());
                }
                return self.snapshot.read().await.clone().ok_or_else(unavailable);
            }
        };
        if self.generation.load(std::sync::atomic::Ordering::Acquire) != generation {
            if self.failed.load(std::sync::atomic::Ordering::Acquire)
                && (force || snapshot.is_none())
            {
                return Err(unavailable());
            }
            return self.snapshot.read().await.clone().ok_or_else(unavailable);
        }
        let runtime = self.clone();
        let task = tokio::spawn(async move {
            let _guard = guard;
            let result = runtime.refresh_now(store.as_ref(), gateway.as_ref()).await;
            if result.is_err() {
                *runtime.retry_at.lock().await = Some(Instant::now() + Duration::from_secs(900));
            }
            runtime
                .failed
                .store(result.is_err(), std::sync::atomic::Ordering::Release);
            runtime
                .generation
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            result
        });
        if let Some(snapshot) = snapshot.filter(|_| !force) {
            return Ok(snapshot);
        }
        task.await.map_err(|_| unavailable())?
    }

    async fn refresh_now(
        &self,
        store: &dyn ListSubscriptionRepository,
        gateway: &dyn MetadataGateway,
    ) -> AppResult<VocabularySnapshot> {
        let old = self.snapshot.read().await.clone();
        let reply = tokio::time::timeout(
            Duration::from_secs(30),
            gateway.canonical_tag_vocabulary(old.as_ref().map(|v| v.version.as_str())),
        )
        .await
        .map_err(|_| unavailable())??;
        validate_vocabulary(&reply)?;
        let unchanged = reply.unchanged;
        let snapshot = if unchanged {
            let mut old = old
                .filter(|old| old.version == reply.version)
                .ok_or_else(unavailable)?;
            old.checked_at = Utc::now();
            old
        } else {
            let jitter_seconds = old.as_ref().map(|v| v.jitter_seconds).unwrap_or_else(|| {
                scryer_domain::Id::new().0.bytes().fold(0u64, |hash, byte| {
                    hash.wrapping_mul(31).wrapping_add(u64::from(byte))
                }) as i64
                    & 0x7fff_ffff
            }) % 21601;
            VocabularySnapshot {
                version: reply.version,
                entries: reply.entries,
                checked_at: Utc::now(),
                jitter_seconds,
            }
        };
        store.save_vocabulary_cache(&snapshot, unchanged).await?;
        *self.snapshot.write().await = Some(snapshot.clone());
        *self.retry_at.lock().await = None;
        Ok(snapshot)
    }
}

pub fn normalize_filters(
    filters: &[scryer_domain::ListFilter],
    facets: &[scryer_domain::MediaFacet],
    entries: &[VocabularyEntry],
) -> Vec<scryer_domain::ListFilter> {
    use scryer_domain::{ListFilter, ListRatingMinimum};
    let mut result = Vec::new();
    for filter in filters {
        match filter {
            ListFilter::RatingAtLeast { scale, value } => {
                for facet in facets {
                    result.push(ListFilter::Ratings {
                        facet: facet.clone(),
                        match_any: false,
                        minimums: vec![ListRatingMinimum {
                            source: scale.clone(),
                            value: *value,
                        }],
                    });
                }
            }
            ListFilter::ExcludeGenres { genres } => {
                let mut keys = Vec::new();
                let mut unresolved_labels = Vec::new();
                for label in genres {
                    let matches = entries
                        .iter()
                        .filter(|entry| {
                            entry.name.eq_ignore_ascii_case(label.trim())
                                || entry
                                    .aliases
                                    .iter()
                                    .any(|alias| alias.eq_ignore_ascii_case(label.trim()))
                        })
                        .collect::<Vec<_>>();
                    if let [entry] = matches.as_slice() {
                        keys.push(entry.key.clone());
                    } else {
                        unresolved_labels.push(label.clone());
                    }
                }
                keys.sort();
                keys.dedup();
                for facet in facets {
                    result.push(ListFilter::ExcludeCanonicalTags {
                        facet: facet.clone(),
                        keys: keys.clone(),
                        unresolved_labels: unresolved_labels.clone(),
                    });
                }
            }
            other => result.push(other.clone()),
        }
    }
    result
}

fn unavailable() -> AppError {
    AppError::temporary_unavailable("canonical tag vocabulary is unavailable; retry", None)
}

#[cfg(test)]
#[path = "vocabulary_tests.rs"]
mod tests;

impl AppUseCase {
    pub(super) async fn normalize_list_filters(
        &self,
        filters: &[scryer_domain::ListFilter],
        facets: &[scryer_domain::MediaFacet],
    ) -> AppResult<Vec<scryer_domain::ListFilter>> {
        let snapshot = if filters.iter().any(|filter| matches!(filter, scryer_domain::ListFilter::ExcludeGenres { genres } if !genres.is_empty())) {
            Some(self.canonical_tag_vocabulary(false).await?)
        } else { None };
        Ok(normalize_filters(
            filters,
            facets,
            snapshot
                .as_ref()
                .map(|snapshot| snapshot.entries.as_slice())
                .unwrap_or_default(),
        ))
    }

    pub async fn canonical_tag_vocabulary(&self, force: bool) -> AppResult<VocabularySnapshot> {
        self.services
            .lists
            .vocabulary
            .get(
                self.services.lists.subscriptions.clone(),
                self.services.library.metadata_gateway.clone(),
                force,
            )
            .await
    }
}
