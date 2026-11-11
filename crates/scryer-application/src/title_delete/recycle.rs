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
        recycle_index: &mut recycle_bin::TitleRecycleEntryIndex,
    ) -> RecyclePurgeOutcome {
        let configs = self.title_recycle_bin_configs(title).await;
        self.purge_explicit_title_recycle_entries_in(
            title,
            actor,
            configs.as_deref(),
            recycle_index,
        )
        .await
    }

    /// The bins a title's recycle entries can be in: one per root of its
    /// library, deduplicated. `None` when the roots cannot be read.
    pub(crate) async fn title_recycle_bin_configs(
        &self,
        title: &scryer_domain::Title,
    ) -> Option<Vec<recycle_bin::RecycleBinConfig>> {
        let roots = match self.all_library_root_folders_for_facet(&title.facet).await {
            Ok(roots) => roots
                .into_iter()
                .filter(|root| root.library_id == title.library_id)
                .map(|root| root.path)
                .collect::<Vec<_>>(),
            Err(error) => {
                tracing::warn!(%error, title_id = %title.id, "cannot resolve recycle roots");
                return None;
            }
        };
        Some(
            self.recycle_bin_configs_for_recycling(roots)
                .await
                .into_iter()
                .map(|(_, config)| config)
                .collect(),
        )
    }

    /// [`Self::purge_explicit_title_recycle_entries`] with the title's bins
    /// already resolved by [`Self::title_recycle_bin_configs`].
    pub(crate) async fn purge_explicit_title_recycle_entries_in(
        &self,
        title: &scryer_domain::Title,
        actor: DomainEventActor,
        configs: Option<&[recycle_bin::RecycleBinConfig]>,
        recycle_index: &mut recycle_bin::TitleRecycleEntryIndex,
    ) -> RecyclePurgeOutcome {
        let mut outcome = RecyclePurgeOutcome::default();
        let Some(configs) = configs else {
            outcome.failed += 1;
            return outcome;
        };
        // Enumerate at this title's purge boundary: entries created during an
        // earlier batch item must remain visible. Config construction dedupes bins.
        for config in configs {
            if let Some(error) = config.validation_error.as_ref() {
                tracing::warn!(%error, title_id = %title.id, "cannot verify title recycle cleanup for invalid bin");
                outcome.failed += 1;
                continue;
            }
            let entries = match recycle_bin::list_committed_entries_for_title(
                &config,
                &title.id,
                recycle_index,
            )
            .await
            {
                Ok(entries) => entries,
                Err(error) => {
                    tracing::warn!(%error, title_id = %title.id, "cannot enumerate recycle entries");
                    outcome.failed += 1;
                    continue;
                }
            };
            for entry in entries {
                self.purge_listed_title_recycle_entry(
                    config,
                    &title.id,
                    entry,
                    actor.clone(),
                    &mut outcome,
                )
                .await;
            }
        }
        outcome
    }

    /// Purge one listed entry of `title_id`. The listing is a hint: the entry
    /// is re-read under the exclusion key restore and expiry use, and purged
    /// only if it still belongs to the title.
    async fn purge_listed_title_recycle_entry(
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

    /// After a chunk of a deletion job has deleted its titles, purge the
    /// recycle entries of `titles` that appeared in their bins after the
    /// chunk's per-title purge listed them, as a purge at each title's own
    /// turn would have seen. Each bin is listed once for all its titles, and
    /// only directories the chunk's earlier listings did not see are
    /// inspected. Counts are added to the owning title's outcome.
    pub(crate) async fn purge_late_title_recycle_entries(
        &self,
        titles: &[(&str, &[recycle_bin::RecycleBinConfig])],
        actor: DomainEventActor,
        recycle_index: &mut recycle_bin::TitleRecycleEntryIndex,
    ) -> std::collections::HashMap<String, RecyclePurgeOutcome> {
        let mut outcomes = std::collections::HashMap::<String, RecyclePurgeOutcome>::new();
        // Each valid bin, in first-seen order, with the titles it holds.
        let mut bins: Vec<(
            &recycle_bin::RecycleBinConfig,
            std::collections::HashSet<&str>,
        )> = Vec::new();
        for (title_id, configs) in titles {
            for config in configs.iter() {
                // An invalid bin was already reported by the per-title purge.
                if config.validation_error.is_some() {
                    continue;
                }
                match bins
                    .iter_mut()
                    .find(|(known, _)| known.base_path == config.base_path)
                {
                    Some((_, ids)) => {
                        ids.insert(*title_id);
                    }
                    None => bins.push((config, [*title_id].into_iter().collect())),
                }
            }
        }
        for (config, title_ids) in bins {
            let entries = match recycle_bin::list_new_committed_entries_for_titles(
                config,
                &title_ids,
                recycle_index,
            )
            .await
            {
                Ok(entries) => entries,
                Err(error) => {
                    tracing::warn!(%error, path = %config.base_path.display(), "cannot enumerate recycle entries");
                    for title_id in &title_ids {
                        outcomes.entry((*title_id).to_string()).or_default().failed += 1;
                    }
                    continue;
                }
            };
            for entry in entries {
                let Some(title_id) = entry.manifest.title_id.clone() else {
                    continue;
                };
                let outcome = outcomes.entry(title_id.clone()).or_default();
                self.purge_listed_title_recycle_entry(
                    config,
                    &title_id,
                    entry,
                    actor.clone(),
                    outcome,
                )
                .await;
            }
        }
        outcomes
    }
}
