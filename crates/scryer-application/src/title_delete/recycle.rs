use crate::{AppUseCase, domain_events::DomainEventActor, recycle_bin};

/// Recycle cleanup is reported separately from successful catalog deletion.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecyclePurgeOutcome {
    pub purged: usize,
    pub preserved: usize,
    pub busy: usize,
    pub failed: usize,
}

impl RecyclePurgeOutcome {
    pub fn incomplete(&self) -> bool {
        self.preserved != 0 || self.busy != 0 || self.failed != 0
    }
}

impl AppUseCase {
    /// Explicit user deletion has no replacement-presence prerequisite.
    /// Retention and policy deletion continue through housekeeping validation.
    pub(crate) async fn purge_explicit_title_recycle_entries(
        &self,
        title: &scryer_domain::Title,
        actor: DomainEventActor,
    ) -> RecyclePurgeOutcome {
        let mut outcome = RecyclePurgeOutcome::default();
        let roots = match self.all_library_root_folders_for_facet(&title.facet).await {
            Ok(roots) => roots
                .into_iter()
                .filter(|root| root.library_id == title.library_id)
                .map(|root| root.path)
                .collect::<Vec<_>>(),
            Err(error) => {
                tracing::warn!(%error, title_id = %title.id, "cannot resolve recycle roots");
                outcome.failed += 1;
                return outcome;
            }
        };
        // Enumerate at this title's purge boundary: entries created during an
        // earlier batch item must remain visible. Config construction dedupes bins.
        for (_, config) in self.recycle_bin_configs_for_recycling(roots).await {
            if let Some(error) = config.validation_error.as_ref() {
                tracing::warn!(%error, title_id = %title.id, "cannot verify title recycle cleanup for invalid bin");
                outcome.failed += 1;
                continue;
            }
            let entries = match recycle_bin::list_committed_entries(&config).await {
                Ok(entries) => entries,
                Err(error) => {
                    tracing::warn!(%error, title_id = %title.id, "cannot enumerate recycle entries");
                    outcome.failed += 1;
                    continue;
                }
            };
            for entry in entries {
                if entry.manifest.title_id.as_deref() != Some(title.id.as_str()) {
                    continue;
                }
                let Some(id) = entry.manifest.entry_id.as_deref() else {
                    outcome.preserved += 1;
                    continue;
                };
                let Some(_guard) = self
                    .runtime
                    .jobs
                    .interactive_operation_guards
                    .try_acquire(&format!("recycle-entry:{id}"))
                    .await
                else {
                    outcome.busy += 1;
                    continue;
                };
                // Listing is a hint. Re-read provenance and ownership under
                // the same exclusion key used by restore and expiry.
                let fresh = match recycle_bin::find_committed_entry(&config, id).await {
                    Ok(Some(fresh))
                        if fresh.manifest.title_id.as_deref() == Some(title.id.as_str()) =>
                    {
                        fresh
                    }
                    Ok(_) => {
                        outcome.preserved += 1;
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(%error, entry_id = id, "cannot revalidate recycle entry");
                        outcome.failed += 1;
                        continue;
                    }
                };
                match recycle_bin::purge_committed_entry(&config, &fresh).await {
                    Ok(true) => {
                        outcome.purged += 1;
                        self.record_recycle_entry_purged_event(actor.clone(), &fresh.manifest)
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
        outcome
    }
}
