//! Client page walks use the same registry and generation as interactive raw search.

use super::*;
use crate::error_reasons::raw_search as reasons;
use std::mem::size_of;

fn full() -> AppError {
    AppError::validation_refused(
        reasons::RETAINED_STATE_FULL,
        "Raw search retained-state capacity is full; retry the search",
    )
}

fn matching_generation(
    entries: &InteractiveReleaseSearchRegistryEntries,
    principal: &SearchPrincipal,
    scope_key: &str,
    offset: usize,
) -> Option<(String, Arc<raw::RawSearchGeneration>)> {
    entries.iter().find_map(|(id, entry)| {
        (entry.principal == *principal
            && entry.scope_key == scope_key
            && !entry.cancel.is_cancelled()
            && (offset != 0
                || (entry.raw_starts_at_zero
                    && entry.snapshot.state == InteractiveReleaseSearchState::Running)))
            .then(|| entry.raw.as_ref().map(|raw| (id.clone(), raw.clone())))
            .flatten()
    })
}

type InteractiveReleaseSearchRegistryEntries = HashMap<String, InteractiveReleaseSearchJobEntry>;

impl AppUseCase {
    /// The caller resolves authorization and source routing before entering the
    /// core, then checks current authorization again before delivering the page.
    /// `scope_key` is a credential-free identity of the effective request and
    /// source configuration. Offset, page size and output format are not keys.
    pub(crate) async fn search_scoped_raw_page(
        &self,
        principal: SearchPrincipal,
        scope_key: String,
        sources: Vec<RawSearchSource>,
        offset: usize,
        limit: usize,
    ) -> AppResult<RawSearchPage> {
        let task_charge = raw::RawSearchGeneration::reserve_reclaiming(
            &self.runtime.acquisition.raw_search_budget,
            2048,
            Some(&Arc::downgrade(
                &self.runtime.acquisition.interactive_release_searches,
            )),
        )
        .await
        .ok_or_else(full)?;
        let (id, generation) = self
            .scoped_raw_generation(principal, scope_key, sources, offset)
            .await?;
        // The driver outlives a disconnected HTTP consumer. In particular, its
        // completion must still end the initial-read join window; otherwise
        // every subsequent offset-zero request would join an abandoned walk.
        let app = self.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        self.restore_coordinator().spawn_task(async move {
            let response = generation.page_or_rebuild(offset, limit).await;
            let mut entries = app
                .runtime
                .acquisition
                .interactive_release_searches
                .lock()
                .await;
            if let Some(entry) = entries.get_mut(&id) {
                if let Ok((rebuilt, _)) = &response
                    && entry
                        .raw
                        .as_ref()
                        .is_some_and(|raw| Arc::ptr_eq(raw, &generation))
                {
                    entry.raw = Some(rebuilt.clone());
                }
                entry.snapshot.state = InteractiveReleaseSearchState::Completed;
                entry.snapshot.completed_at = Some(app.runtime.environment.now());
            }
            drop(entries);
            let _ = send.send(response.map(|(_, page)| page));
            drop(task_charge);
        });
        receive.await.map_err(|_| {
            AppError::Repository("Raw search driver stopped before completing".into())
        })?
    }

    async fn scoped_raw_generation(
        &self,
        principal: SearchPrincipal,
        scope_key: String,
        sources: Vec<RawSearchSource>,
        offset: usize,
    ) -> AppResult<(String, Arc<raw::RawSearchGeneration>)> {
        let registry = &self.runtime.acquisition.interactive_release_searches;
        {
            let mut entries = registry.lock().await;
            evict_stale_entries(&mut entries, self.runtime.environment.now());
            if let Some(found) = matching_generation(&entries, &principal, &scope_key, offset) {
                found.1.touch();
                return Ok(found);
            }
        }

        // Reserve outside the registry lock: reclamation may evict idle entries.
        // Recheck under the lock afterward so racing initial requests install
        // only one generation, with no duplicate source work.
        let id = Id::new().0;
        let principal_bytes = match &principal {
            SearchPrincipal::User { user_id } => user_id.capacity(),
            SearchPrincipal::IndexerClient {
                client_id,
                grants_generation,
                ..
            } => client_id.capacity() + grants_generation.capacity(),
        };
        let registry_charge = raw::RawSearchGeneration::reserve_reclaiming(
            &self.runtime.acquisition.raw_search_budget,
            size_of::<InteractiveReleaseSearchJobEntry>()
                + 1024
                + id.capacity() * 3
                + scope_key.capacity()
                + principal_bytes,
            Some(&Arc::downgrade(registry)),
        )
        .await
        .ok_or_else(full)?;
        let cancel = self.restore_coordinator().shutdown_workers.child_token();
        let generation = raw::RawSearchGeneration::new(
            sources,
            self.services.integrations.indexer_client.clone(),
            self.runtime.acquisition.raw_search_budget.clone(),
            cancel.clone(),
            Some(Arc::downgrade(registry)),
            self.restore_coordinator().clone(),
        )
        .await?;
        let mut entries = registry.lock().await;
        evict_stale_entries(&mut entries, self.runtime.environment.now());
        if let Some(found) = matching_generation(&entries, &principal, &scope_key, offset) {
            found.1.touch();
            return Ok(found);
        }
        // Standard Nab offsets name the current matching walk. Outstanding
        // readers retain their previous Arc; replacing the lookup never cancels
        // source reads or releases their retained-state reservations early.
        entries.retain(|_, entry| entry.principal != principal || entry.scope_key != scope_key);
        entries.shrink_to_fit();
        entries.insert(
            id.clone(),
            InteractiveReleaseSearchJobEntry {
                snapshot: InteractiveReleaseSearchSnapshot {
                    id: id.clone(),
                    state: InteractiveReleaseSearchState::Running,
                    results: Vec::new(),
                    raw_page: None,
                    raw_failure_reason: None,
                    indexers: Vec::new(),
                    started_at: self.runtime.environment.now(),
                    completed_at: None,
                    limit: None,
                },
                principal,
                scope_key,
                cancel,
                raw: Some(generation.clone()),
                raw_page_request: 0,
                raw_starts_at_zero: offset == 0,
                _raw_registry_charge: Some(registry_charge),
            },
        );
        Ok((id, generation))
    }
}

#[cfg(test)]
mod tests;
