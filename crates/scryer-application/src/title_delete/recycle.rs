use std::collections::{HashMap, HashSet};

use crate::{AppUseCase, domain_events::DomainEventActor, recycle_bin};

/// How many recorded deletions one housekeeping pass reads. The rest wait for
/// the next pass.
const RECORDED_PURGES_PER_PASS: usize = 10_000;

/// What happened to the listed recycle entries of one deleted title.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecyclePurgeOutcome {
    pub purged: usize,
    pub preserved: usize,
    pub busy: usize,
    pub failed: usize,
}

impl RecyclePurgeOutcome {
    pub(crate) fn incomplete(&self) -> bool {
        self.preserved != 0 || self.busy != 0 || self.failed != 0
    }
}

/// The deleted titles whose recycle entries a housekeeping pass purges, read
/// once at the start of the pass.
///
/// A title is finished, and its record cleared, once the pass has gone through
/// every bin of its library without leaving one of its entries behind. An
/// entry that is busy, fails, is withheld from listing (pending, quarantined
/// or untrusted), or sits in a bin that could not be read keeps the record for
/// a later pass.
#[derive(Debug, Default)]
pub(crate) struct RecordedTitleRecyclePurges {
    /// Recorded title id to the library it was deleted from.
    libraries: HashMap<String, String>,
    /// Recorded titles that must stay recorded after this pass.
    keep: HashSet<String>,
    /// Records whose title exists again. They are cleared without claiming
    /// any entry.
    stale: Vec<String>,
    /// The bins of every library were resolved, so a title with no entries
    /// left really has none.
    bins_resolved: bool,
}

impl RecordedTitleRecyclePurges {
    pub(crate) fn is_empty(&self) -> bool {
        self.libraries.is_empty() && self.stale.is_empty()
    }

    pub(crate) fn set_bins_resolved(&mut self, resolved: bool) {
        self.bins_resolved = resolved;
    }

    /// Whether `title_id` is recorded and this bin belongs to the library it
    /// was deleted from: the bins its delete would have purged.
    pub(crate) fn claims(&self, title_id: &str, bin_libraries: &HashSet<String>) -> bool {
        self.libraries
            .get(title_id)
            .is_some_and(|library_id| bin_libraries.contains(library_id))
    }

    pub(crate) fn keep(&mut self, title_id: &str) {
        self.keep.insert(title_id.to_string());
    }

    /// Keep every recorded title of these libraries, for a bin that could not
    /// be read or verified.
    pub(crate) fn keep_libraries(&mut self, bin_libraries: &HashSet<String>) {
        for (title_id, library_id) in &self.libraries {
            if bin_libraries.contains(library_id) {
                self.keep.insert(title_id.clone());
            }
        }
    }

    fn finished(self) -> Vec<String> {
        let mut finished = self.stale;
        if self.bins_resolved {
            finished.extend(
                self.libraries
                    .into_keys()
                    .filter(|title_id| !self.keep.contains(title_id)),
            );
        }
        finished.sort();
        finished
    }
}

impl AppUseCase {
    /// The recorded deletions this pass purges. With nothing recorded this is
    /// one read.
    pub(crate) async fn load_recorded_title_recycle_purges(&self) -> RecordedTitleRecyclePurges {
        let titles = &self.services.catalog.titles;
        let recorded = match titles.list_recycle_purges(RECORDED_PURGES_PER_PASS).await {
            Ok(recorded) => recorded,
            Err(error) => {
                tracing::warn!(%error, "cannot read deleted titles awaiting recycle cleanup");
                return RecordedTitleRecyclePurges::default();
            }
        };
        if recorded.is_empty() {
            return RecordedTitleRecyclePurges::default();
        }
        let ids = recorded
            .iter()
            .map(|purge| purge.title_id.clone())
            .collect::<Vec<_>>();
        // A record never outlives its title's delete, but a title that exists
        // again keeps its entries.
        let existing = match titles.list_library_ids_for_titles(&ids).await {
            Ok(existing) => existing
                .into_iter()
                .map(|(title_id, _)| title_id)
                .collect::<HashSet<_>>(),
            Err(error) => {
                tracing::warn!(%error, "cannot check deleted titles awaiting recycle cleanup");
                return RecordedTitleRecyclePurges::default();
            }
        };
        let mut purges = RecordedTitleRecyclePurges::default();
        for purge in recorded {
            if existing.contains(&purge.title_id) {
                purges.stale.push(purge.title_id);
            } else {
                purges.libraries.insert(purge.title_id, purge.library_id);
            }
        }
        purges
    }

    /// Drop the records older than `retention_days`, oldest first, at most
    /// one pass's worth. Their remaining entries are left to the bin's own
    /// retention; nothing on disk is touched. With none that old this is one
    /// read and no write.
    pub(crate) async fn drop_expired_title_recycle_purges(&self, retention_days: i64) {
        let cutoff = self.runtime.environment.now() - chrono::Duration::days(retention_days);
        match self
            .services
            .catalog
            .titles
            .drop_recycle_purges_recorded_before(cutoff, RECORDED_PURGES_PER_PASS)
            .await
        {
            Ok(0) => {}
            Ok(dropped) => tracing::info!(
                dropped,
                "dropped recycle cleanup records older than the history retention period"
            ),
            Err(error) => {
                tracing::warn!(%error, "cannot drop expired recycle cleanup records")
            }
        }
    }

    /// Clear the records this pass finished, in one write. Nothing is written
    /// when none finished.
    pub(crate) async fn finish_recorded_title_recycle_purges(
        &self,
        purges: RecordedTitleRecyclePurges,
    ) {
        let finished = purges.finished();
        if finished.is_empty() {
            return;
        }
        if let Err(error) = self
            .services
            .catalog
            .titles
            .clear_recycle_purges(&finished)
            .await
        {
            tracing::warn!(%error, "cannot clear finished recycle cleanup records");
        }
    }

    /// Purge one listed entry of a deleted title. Deletion of a title has no
    /// replacement-presence prerequisite, so this skips the replacement check
    /// retention and policy deletion apply. The listing is a hint: the entry
    /// is re-read under the exclusion key restore and expiry use, and purged
    /// only if it still belongs to the title.
    pub(crate) async fn purge_listed_title_recycle_entry(
        &self,
        config: &recycle_bin::RecycleBinConfig,
        title_id: &str,
        entry: recycle_bin::CommittedRecycleEntry,
        actor: DomainEventActor,
        outcome: &mut RecyclePurgeOutcome,
    ) {
        let Some(id) = entry.manifest.entry_id.as_deref() else {
            outcome.preserved += 1;
            return;
        };
        let Some(_guard) = self
            .runtime
            .jobs
            .interactive_operation_guards
            .try_acquire(&format!("recycle-entry:{id}"))
            .await
        else {
            outcome.busy += 1;
            return;
        };
        let fresh = match recycle_bin::find_committed_entry(config, id).await {
            Ok(Some(fresh)) if fresh.manifest.title_id.as_deref() == Some(title_id) => fresh,
            Ok(_) => {
                outcome.preserved += 1;
                return;
            }
            Err(error) => {
                tracing::warn!(%error, entry_id = id, "cannot revalidate recycle entry");
                outcome.failed += 1;
                return;
            }
        };
        match recycle_bin::purge_committed_entry(config, &fresh).await {
            Ok(true) => {
                outcome.purged += 1;
                self.record_recycle_entry_purged_event(actor, &fresh.manifest)
                    .await;
            }
            Ok(false) => outcome.preserved += 1,
            Err(error) => {
                tracing::warn!(%error, entry_id = id, "cannot purge title recycle entry");
                outcome.failed += 1;
            }
        }
    }
}
