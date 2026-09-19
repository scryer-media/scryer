use super::*;

const CACHED_SUBMISSION_STATE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30);

/// Per-title exclusion for the staged acquisition walk.
///
/// Two walkers can reach one title: the background convergence cycle and an
/// operator's title-scoped acquisition-search job. Both run the same stages —
/// title pack, season packs, then episode scopes — and both hold their grab
/// proposals until the end of the walk, arbitrating against the claims made
/// inside that walk. Those claim sets are per-walk, so two concurrent walks over
/// one title could each conclude that its pack is unopposed and grab both.
///
/// A title lock is the boundary that keeps the arbitration honest: only one walk
/// over a title exists at a time, so the claims that matter are always the ones
/// the current walk made. The two callers take it differently on purpose — the
/// background cycle *tries* and moves on to the next title if an operator holds
/// it (the cursor comes back next cycle), while the interactive job *waits*,
/// because the operator asked for this title.
///
/// That wait is bounded only in the sense that a cycle's pass finishes: a pass
/// over a several-hundred-episode title that spends an inline query per episode
/// scope can run for minutes. The job reports the wait as its own progress step
/// so its UI says why nothing is happening, rather than the wait being written
/// off as brief.
///
/// The table mirrors `DownloadSubmissionGuardTable`: weak handles so an
/// unlocked title drops out of the map instead of accumulating.
#[derive(Clone, Default)]
pub struct AcquisitionTitleWalkLocks {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl AcquisitionTitleWalkLocks {
    async fn handle(&self, title_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(existing) = locks.get(title_id).and_then(std::sync::Weak::upgrade) {
            return existing;
        }
        let created = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(title_id.to_string(), Arc::downgrade(&created));
        created
    }

    /// Wait for the title's walk lock. Used by the interactive job.
    pub(crate) async fn acquire(&self, title_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.handle(title_id).await.lock_owned().await
    }

    /// Take the title's walk lock if it is free, else `None`. Used by the
    /// background cycle, which skips a title rather than blocking its whole
    /// pass behind one operator-driven walk.
    pub(crate) async fn try_acquire(
        &self,
        title_id: &str,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.handle(title_id).await.try_lock_owned().ok()
    }
}

/// Per-title exclusion for metadata hydration.
///
/// A title can be hydrated from more than one lane at once — a library scan's
/// bulk pass and an interactive refresh of the same brand-new title, most
/// visibly. Each lane fetches metadata from the gateway holding the `Title`
/// struct it queued, then persists. Without exclusion the two persists
/// interleave with each other and with the scan's own title upsert, and the
/// loser's re-read finds a row that no longer says `metadata_fetched_at` — the
/// hydration then reports "metadata could not be persisted" for a title that
/// was, in fact, hydrated.
///
/// The lock covers persist-and-read-back, not the gateway call, so a slow
/// metadata fetch never blocks another lane; the winner's fresh row is what the
/// waiter applies its own result onto, and a waiter that only wanted an
/// unhydrated title hydrated takes the winner's answer and stops.
///
/// The table mirrors [`AcquisitionTitleWalkLocks`]: weak handles so an unlocked
/// title drops out of the map instead of accumulating.
#[derive(Clone, Default)]
pub struct TitleHydrationLocks {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl TitleHydrationLocks {
    pub(crate) async fn acquire(&self, title_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(title_id).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(title_id.to_string(), Arc::downgrade(&created));
                created
            }
        };

        lock.lock_owned().await
    }
}

/// In-process guard table for download-submission dedupe and scope ownership.
///
/// Scryer is intentionally single-instance, so the database lookup remains the
/// authoritative duplicate check while this table serializes same-process races.
#[derive(Clone, Default)]
pub struct DownloadSubmissionGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
    uncertain_titles: Arc<std::sync::Mutex<HashMap<String, UncertainDownloadSubmissionClaim>>>,
    title_states: Arc<std::sync::Mutex<HashMap<String, CanonicalSubmissionTitleState>>>,
    client_snapshot: Arc<std::sync::Mutex<Option<CachedDownloadClientSnapshot>>>,
    acquisition_snapshot: Arc<std::sync::Mutex<Option<CachedAcquisitionClientSnapshot>>>,
}

#[derive(Clone)]
pub(crate) struct CanonicalSubmissionTitleState {
    refreshed_at: std::time::Instant,
    pub(crate) submissions: Vec<DownloadSubmission>,
    pub(crate) episodes: Vec<scryer_domain::Episode>,
    pub(crate) accepted_download_ids: HashSet<scryer_domain::download_identity::DownloadId>,
}

impl CanonicalSubmissionTitleState {
    pub(crate) fn new(
        submissions: Vec<DownloadSubmission>,
        episodes: Vec<scryer_domain::Episode>,
    ) -> Self {
        Self {
            refreshed_at: std::time::Instant::now(),
            submissions,
            episodes,
            accepted_download_ids: HashSet::new(),
        }
    }

    pub(crate) fn forget(&mut self, download_id: scryer_domain::download_identity::DownloadId) {
        self.submissions
            .retain(|submission| submission.download_id != download_id);
        self.accepted_download_ids.remove(&download_id);
    }

    pub(crate) fn remember(&mut self, submission: DownloadSubmission) {
        self.forget(submission.download_id);
        self.accepted_download_ids.insert(submission.download_id);
        self.submissions.push(submission);
        self.refreshed_at = std::time::Instant::now();
    }
}

#[derive(Clone)]
struct CachedDownloadClientSnapshot {
    refreshed_at: std::time::Instant,
    snapshot: DownloadClientSnapshotOutcome,
}

/// The derived acquisition view of the same clients: every queue and history
/// page folded into the sets the double-submit guard asks about.
///
/// Held behind an `Arc` because a walk hands one out per search subject and a
/// thousand-item history is not worth copying each time.
#[derive(Clone)]
struct CachedAcquisitionClientSnapshot {
    refreshed_at: std::time::Instant,
    snapshot: Arc<crate::acquisition_workflow::DownloadClientSnapshot>,
}

#[derive(Clone)]
pub(crate) enum UncertainDownloadSubmissionClaim {
    Accepted {
        submission: DownloadSubmission,
        accepted_identity: DownloadSubmissionIdentity,
        seed_goals: Option<PersistedSeedGoals>,
    },
    Ambiguous {
        download_id: scryer_domain::download_identity::DownloadId,
        submission: Option<DownloadSubmission>,
    },
}

impl UncertainDownloadSubmissionClaim {
    pub(crate) fn accepted(
        submission: DownloadSubmission,
        accepted_identity: DownloadSubmissionIdentity,
        seed_goals: Option<PersistedSeedGoals>,
    ) -> Self {
        Self::Accepted {
            submission,
            accepted_identity,
            seed_goals,
        }
    }

    pub(crate) fn ambiguous(
        download_id: scryer_domain::download_identity::DownloadId,
        submission: Option<DownloadSubmission>,
    ) -> Self {
        Self::Ambiguous {
            download_id,
            submission,
        }
    }
}

impl DownloadSubmissionGuardTable {
    async fn acquire_key(&self, key: String) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&created));
                created
            }
        };

        lock.lock_owned().await
    }

    pub async fn acquire_title(&self, title_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.acquire_key(title_id.to_string()).await
    }

    /// Test seam: how many callers hold or are parked on a title's submission
    /// lock, so a test can tell when a second submission reached contention.
    #[cfg(test)]
    pub(crate) async fn title_lock_participants(&self, title_id: &str) -> usize {
        self.locks
            .lock()
            .await
            .get(title_id)
            .map_or(0, std::sync::Weak::strong_count)
    }

    pub(crate) async fn acquire_client_snapshot(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.acquire_key("download-client-snapshot".to_string())
            .await
    }

    /// Serializes the *derived* snapshot build, so the subjects of one walk
    /// queue behind a single set of client reads instead of each starting its
    /// own. Deliberately a different key from `acquire_client_snapshot`: the
    /// two builds read different endpoints, and neither should be able to
    /// stall behind the other.
    pub(crate) async fn acquire_acquisition_snapshot(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.acquire_key("acquisition-client-snapshot".to_string())
            .await
    }

    pub(crate) fn cached_title_state(
        &self,
        title_id: &str,
    ) -> Option<CanonicalSubmissionTitleState> {
        let mut states = self
            .title_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        states.retain(|_, state| state.refreshed_at.elapsed() <= CACHED_SUBMISSION_STATE_MAX_AGE);
        states.get(title_id).cloned()
    }

    pub(crate) fn store_title_state(&self, title_id: &str, state: CanonicalSubmissionTitleState) {
        self.title_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(title_id.to_string(), state);
    }

    pub(crate) fn clear_title_state(&self, title_id: &str) {
        self.title_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(title_id);
    }

    /// A download for `title_id` reached a terminal state (imported, failed,
    /// removed). Both caches may still describe it as in flight — the accepted
    /// set marks it queued and the shared snapshot predates the transition —
    /// and a stale entry turns the next submission for an overlapping scope
    /// (an upgrade, most visibly) into a phantom non-replaceable conflict.
    /// Drop both so the next attempt re-reads authoritative state; terminal
    /// transitions are rare next to searches, so the bounded-work intent of
    /// the caches survives.
    pub(crate) fn forget_settled_download(&self, title_id: &str) {
        self.clear_title_state(title_id);
        self.invalidate_client_snapshots();
    }

    /// Both client-state caches stop describing the clients: a grab was just
    /// submitted, or a download settled. The next reader re-asks.
    ///
    /// This is what keeps snapshot reuse exactly as strict as a fetch per
    /// subject was. A cached snapshot may only answer questions about a queue
    /// nothing has touched since it was read; the moment this process hands a
    /// client something — or learns a client finished something — the cached
    /// answer could say "free" about a scope that is now claimed, and that is
    /// precisely the double-submit the guard exists to prevent.
    pub(crate) fn invalidate_client_snapshots(&self) {
        *self
            .client_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        *self
            .acquisition_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    pub(crate) fn prime_title_state(
        &self,
        title_id: &str,
        submissions: Vec<DownloadSubmission>,
        episodes: Vec<scryer_domain::Episode>,
    ) {
        self.store_title_state(
            title_id,
            CanonicalSubmissionTitleState::new(submissions, episodes),
        );
    }

    pub(crate) fn cached_client_snapshot(&self) -> Option<DownloadClientSnapshotOutcome> {
        self.client_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .filter(|cached| cached.refreshed_at.elapsed() <= CACHED_SUBMISSION_STATE_MAX_AGE)
            .map(|cached| cached.snapshot.clone())
    }

    /// The derived snapshot a walk may still reuse, or `None` when there is
    /// none or it has aged past `CACHED_SUBMISSION_STATE_MAX_AGE` — the same
    /// bound the submission path's cache already honours.
    pub(crate) fn cached_acquisition_snapshot(
        &self,
    ) -> Option<Arc<crate::acquisition_workflow::DownloadClientSnapshot>> {
        self.acquisition_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .filter(|cached| cached.refreshed_at.elapsed() <= CACHED_SUBMISSION_STATE_MAX_AGE)
            .map(|cached| Arc::clone(&cached.snapshot))
    }

    pub(crate) fn store_acquisition_snapshot(
        &self,
        snapshot: Arc<crate::acquisition_workflow::DownloadClientSnapshot>,
    ) {
        *self
            .acquisition_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(CachedAcquisitionClientSnapshot {
                refreshed_at: std::time::Instant::now(),
                snapshot,
            });
    }

    pub(crate) fn store_client_snapshot(&self, snapshot: DownloadClientSnapshotOutcome) {
        *self
            .client_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(CachedDownloadClientSnapshot {
                refreshed_at: std::time::Instant::now(),
                snapshot,
            });
    }

    pub(crate) fn mark_uncertain(&self, title_id: &str, claim: UncertainDownloadSubmissionClaim) {
        self.uncertain_titles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(title_id.to_string(), claim);
    }

    pub(crate) fn clear_uncertain(&self, title_id: &str) {
        self.uncertain_titles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(title_id);
    }

    pub(crate) fn uncertain_claim(
        &self,
        title_id: &str,
    ) -> Option<UncertainDownloadSubmissionClaim> {
        self.uncertain_titles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(title_id)
            .cloned()
    }
}

/// In-process guard table for failed-download handling dedupe.
///
/// This serializes same-process races between the grabbed-item failure sweep and
/// tracked-download failure processing while the persisted blocklist row remains
/// the authoritative record of whether failure side effects already ran.
#[derive(Clone, Default)]
pub struct DownloadFailureGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl DownloadFailureGuardTable {
    async fn acquire_key(&self, key: String) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&created));
                created
            }
        };

        lock.lock_owned().await
    }

    pub async fn acquire(
        &self,
        title_id: Option<&str>,
        client_id: &str,
        client_type: &str,
        client_item_id: &str,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let title_id = title_id.map(str::trim).filter(|value| !value.is_empty())?;
        let key = format!(
            "{title_id}:{}:{}:{}",
            client_id.trim(),
            client_type.trim().to_ascii_lowercase(),
            client_item_id.trim()
        );
        Some(self.acquire_key(key).await)
    }

    pub async fn acquire_release_or_client_item(
        &self,
        title_id: Option<&str>,
        source_title: Option<&str>,
        client_id: &str,
        client_type: &str,
        client_item_id: &str,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let title_id = title_id.map(str::trim).filter(|value| !value.is_empty())?;
        if let Some(source_title) = source_title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
        {
            return Some(
                self.acquire_key(format!("release:{title_id}:{source_title}"))
                    .await,
            );
        }

        self.acquire(Some(title_id), client_id, client_type, client_item_id)
            .await
    }
}

#[derive(Clone, Default)]
pub struct BackupExecutionGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

pub type InteractiveOperationGuardTable = BackupExecutionGuardTable;

impl BackupExecutionGuardTable {
    async fn lock_for_key(&self, key: String) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
            existing
        } else {
            let created = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(key, Arc::downgrade(&created));
            created
        }
    }

    pub async fn try_acquire(&self, key: &str) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let lock = self.lock_for_key(key.to_string()).await;
        lock.try_lock_owned().ok()
    }
}

#[derive(Clone, Default)]
pub struct PluginOperationGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl PluginOperationGuardTable {
    pub async fn acquire(&self, plugin_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let key = plugin_id.trim().to_ascii_lowercase();
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&created));
                created
            }
        };

        lock.lock_owned().await
    }
}
