use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum CompletedDownloadLookupCoverage {
    Full,
    #[default]
    Recent,
}

#[derive(Clone, Default)]
pub(crate) struct CompletedDownloadLookup {
    coverage: CompletedDownloadLookupCoverage,
    pub(super) by_source: HashMap<(String, String, String), CompletedDownload>,
    by_download_id: HashMap<(String, String, String), Vec<CompletedDownload>>,
    pub(super) by_canonical:
        HashMap<scryer_domain::download_identity::DownloadId, CompletedDownload>,
}

impl CompletedDownloadLookup {
    pub(super) fn empty_recent() -> Self {
        Self::default()
    }

    pub(crate) fn from_recent_downloads(downloads: Vec<CompletedDownload>) -> Self {
        index_completed_downloads(downloads, CompletedDownloadLookupCoverage::Recent)
    }

    pub(crate) fn matches_tracked_download(&self, td: &TrackedDownload) -> bool {
        find_completed_download_in_lookup(self, td).is_some()
    }

    /// Fold a second listing into this one, keeping the STRONGER coverage.
    ///
    /// Used when a widened re-read backfills rows a narrower one truncated
    /// away. Coverage may only improve, never regress: a `Full` lookup merged
    /// with a `Recent` one stays `Full`, so a later exhaustiveness check is not
    /// weakened by having taken on extra rows.
    pub(crate) fn merge(&mut self, other: Self) {
        let coverage = if self.is_exhaustive() || other.is_exhaustive() {
            CompletedDownloadLookupCoverage::Full
        } else {
            self.coverage
        };
        for completed in other.by_source.into_values() {
            index_completed_download_into(self, completed);
        }
        self.by_canonical.extend(other.by_canonical);
        self.coverage = coverage;
    }

    #[cfg(test)]
    pub(super) fn empty_full() -> Self {
        Self {
            coverage: CompletedDownloadLookupCoverage::Full,
            ..Self::default()
        }
    }

    pub(super) fn is_exhaustive(&self) -> bool {
        self.coverage == CompletedDownloadLookupCoverage::Full
    }
}

/// Per-poller memo of completed-history identity resolutions.
///
/// This is a listing-scoped instance of the shared
/// [`crate::download_identity::ObservationResolutionCache`]: it is rebuilt from
/// each completed listing, so a row that falls out of the client's history
/// window drops out of the memo with it. The resolution semantics (registry
/// generation, TTL backstop, which resolutions are memoizable) all live on the
/// shared type.
pub(crate) type CompletedDownloadResolutionCache =
    crate::download_identity::ObservationResolutionCache;

/// Per-tick reuse handed to the completed-download lookup loader.
///
/// `uncached` is the behaviour every caller had before: resolve every row
/// against the registry, and fetch the completed history from the client.
pub(crate) struct CompletedDownloadLookupCycle<'a> {
    pub(crate) resolutions: Option<&'a mut CompletedDownloadResolutionCache>,
    /// Completed rows already read from the clients this tick, when the
    /// snapshot read produced them from the same response.
    pub(crate) prefetched: Option<&'a crate::ports::PrefetchedCompletedDownloads>,
}

impl CompletedDownloadLookupCycle<'_> {
    pub(crate) fn uncached() -> Self {
        Self {
            resolutions: None,
            prefetched: None,
        }
    }
}

pub(crate) async fn load_completed_download_lookup(
    app: &AppUseCase,
) -> AppResult<CompletedDownloadLookup> {
    let completed_downloads = app
        .services
        .integrations
        .download_client
        .list_completed_downloads()
        .await?;
    let canonical_download_ids =
        resolve_completed_download_observations(app, &completed_downloads).await;
    Ok(index_completed_download_observations(
        completed_downloads,
        canonical_download_ids,
        CompletedDownloadLookupCoverage::Full,
    ))
}

async fn load_recent_completed_download_lookup_for_items_or_default_excluding_client_types(
    app: &AppUseCase,
    items: &[DownloadQueueItem],
    limit: usize,
    excluded_client_types: &[&str],
    cycle: CompletedDownloadLookupCycle<'_>,
) -> CompletedDownloadLookup {
    let (client_ids, client_types) = completed_download_client_scope(items, true);
    load_recent_completed_download_lookup_for_client_scope_or_default_excluding_client_types(
        app,
        limit,
        &client_ids,
        &client_types,
        excluded_client_types,
        cycle,
    )
    .await
}

/// How much of the requested scope this cycle's own snapshot read already
/// covers, split per client id.
///
/// Coverage is per client, not all-or-nothing: on a mixed fleet the clients
/// that produced their completed rows from this tick's history read are served
/// from those rows, and only the clients that did not are asked again. The
/// split is by client id alone, so the two halves can never both answer for
/// the same client. A scope that falls back on client *types* (rows that
/// arrived without a configured client id) cannot be split that way — a type
/// can name a covered client — so it issues the read it always did.
struct PrefetchedScopeCoverage {
    rows: Vec<CompletedDownload>,
    /// Scope the client still has to be asked about. Empty means this cycle
    /// answers the whole scope and no client read happens at all.
    uncovered_client_ids: Vec<String>,
}

fn prefetched_scope_coverage(
    cycle: &CompletedDownloadLookupCycle<'_>,
    limit: usize,
    client_ids: &[String],
    client_types: &[String],
    excluded_client_types: &[&str],
) -> Option<PrefetchedScopeCoverage> {
    let prefetched = cycle.prefetched?;
    if limit == 0 || !client_types.is_empty() || client_ids.is_empty() {
        return None;
    }

    let (covered_client_ids, uncovered_client_ids): (Vec<String>, Vec<String>) = client_ids
        .iter()
        .cloned()
        .partition(|client_id| prefetched.client_ids.contains(client_id.trim()));
    if covered_client_ids.is_empty() {
        return None;
    }

    let mut rows = prefetched
        .rows
        .iter()
        .filter(|completed| {
            crate::ports::completed_download_matches_client_scope(
                completed,
                &covered_client_ids,
                &[],
                excluded_client_types,
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    // Stable, so rows keep the order the snapshot read put them in within a
    // completion timestamp.
    rows.sort_by_key(|completed| std::cmp::Reverse(completed.completed_at));
    truncate_completed_downloads_per_client(&mut rows, limit);
    Some(PrefetchedScopeCoverage {
        rows,
        uncovered_client_ids,
    })
}

/// Keep each client's newest `limit` rows.
///
/// The limit is per client everywhere else — the router reads `limit` rows
/// from every client and never trims the merged listing — so trimming the
/// merged rows to `limit` here would drop a client's newest completions
/// whenever a busier client happened to fill the budget, while coverage still
/// claimed the whole scope and no fallback read made up for it.
///
/// `rows` must already be sorted newest-first; the retained rows keep that
/// order.
fn truncate_completed_downloads_per_client(rows: &mut Vec<CompletedDownload>, limit: usize) {
    let mut kept_per_client: HashMap<&str, usize> = HashMap::new();
    let mut keep = Vec::with_capacity(rows.len());
    for completed in rows.iter() {
        let kept = kept_per_client
            .entry(completed.client_id.trim())
            .or_insert(0);
        keep.push(*kept < limit);
        *kept += 1;
    }

    let mut keep = keep.into_iter();
    rows.retain(|_| keep.next().unwrap_or(false));
}

async fn load_recent_completed_download_lookup_for_client_scope_or_default_excluding_client_types(
    app: &AppUseCase,
    limit: usize,
    client_ids: &[String],
    client_types: &[String],
    excluded_client_types: &[&str],
    mut cycle: CompletedDownloadLookupCycle<'_>,
) -> CompletedDownloadLookup {
    if client_ids.is_empty() && client_types.is_empty() {
        return CompletedDownloadLookup::empty_recent();
    }

    let coverage = prefetched_scope_coverage(
        &cycle,
        limit,
        client_ids,
        client_types,
        excluded_client_types,
    );
    let completed_downloads = match coverage {
        // This cycle's own read answers every client in the scope.
        Some(coverage) if coverage.uncovered_client_ids.is_empty() => Ok(coverage.rows),
        // Part of the fleet produced its rows with the snapshot; the rest is
        // read as before and the two halves are merged.
        Some(coverage) => {
            match app
                .services
                .integrations
                .download_client
                .list_recent_completed_downloads_for_client_scope(
                    limit,
                    &coverage.uncovered_client_ids,
                    client_types,
                    excluded_client_types,
                )
                .await
            {
                Ok(mut rows) => {
                    rows.extend(coverage.rows);
                    rows.sort_by_key(|completed| std::cmp::Reverse(completed.completed_at));
                    truncate_completed_downloads_per_client(&mut rows, limit);
                    Ok(rows)
                }
                Err(error) => Err(error),
            }
        }
        None => {
            app.services
                .integrations
                .download_client
                .list_recent_completed_downloads_for_client_scope(
                    limit,
                    client_ids,
                    client_types,
                    excluded_client_types,
                )
                .await
        }
    };

    match completed_downloads {
        Ok(completed_downloads) => {
            let canonical_download_ids = resolve_completed_download_observations_with_cache(
                app,
                &completed_downloads,
                cycle.resolutions.as_deref_mut(),
            )
            .await;
            index_completed_download_observations(
                completed_downloads,
                canonical_download_ids,
                CompletedDownloadLookupCoverage::Recent,
            )
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "download queue poller: failed to load completed download snapshot for this cycle"
            );
            CompletedDownloadLookup::empty_recent()
        }
    }
}

#[cfg(test)]
pub(crate) async fn load_completed_download_lookup_for_items(
    app: &AppUseCase,
    items: &[DownloadQueueItem],
    limit: usize,
) -> Option<CompletedDownloadLookup> {
    load_completed_download_lookup_for_items_excluding_client_types(
        app,
        items,
        limit,
        &[],
        CompletedDownloadLookupCycle::uncached(),
    )
    .await
}

pub(crate) async fn load_completed_download_lookup_for_items_excluding_client_types(
    app: &AppUseCase,
    items: &[DownloadQueueItem],
    limit: usize,
    excluded_client_types: &[&str],
    cycle: CompletedDownloadLookupCycle<'_>,
) -> Option<CompletedDownloadLookup> {
    if !items.iter().any(download_queue_item_needs_completed_lookup) {
        return None;
    }

    Some(
        load_recent_completed_download_lookup_for_items_or_default_excluding_client_types(
            app,
            items,
            limit,
            excluded_client_types,
            cycle,
        )
        .await,
    )
}

pub(crate) async fn load_completed_download_lookup_for_tracked_client_items_excluding_client_types(
    app: &AppUseCase,
    items: &[DownloadQueueItem],
    limit: usize,
    excluded_client_types: &[&str],
) -> Option<CompletedDownloadLookup> {
    let cycle = CompletedDownloadLookupCycle::uncached();
    if items.is_empty() {
        return None;
    }

    let (client_ids, client_types) = completed_download_client_scope(items, false);
    if client_ids.is_empty() && client_types.is_empty() {
        return None;
    }

    Some(
        load_recent_completed_download_lookup_for_client_scope_or_default_excluding_client_types(
            app,
            limit,
            &client_ids,
            &client_types,
            excluded_client_types,
            cycle,
        )
        .await,
    )
}

/// Resolve every row, reusing memoized answers for rows this poller already
/// resolved against the current registry generation.
///
/// Without a cache this is exactly [`resolve_completed_download_observations`].
pub(super) async fn resolve_completed_download_observations_with_cache(
    app: &AppUseCase,
    completed_downloads: &[CompletedDownload],
    cache: Option<&mut CompletedDownloadResolutionCache>,
) -> Vec<crate::download_identity::ObservedClientJobResolution> {
    let Some(cache) = cache else {
        return resolve_completed_download_observations(app, completed_downloads).await;
    };

    let generation = app.runtime.acquisition.download_registry_generation();
    cache.begin_generation(generation);

    let mut resolutions = Vec::with_capacity(completed_downloads.len());
    // Rebuilt from this listing so a row the client no longer reports drops
    // out of the memo with it.
    let mut listed = HashSet::with_capacity(completed_downloads.len());
    for completed in completed_downloads {
        let observation = crate::download_identity::observed_completed_job(completed);
        let key = crate::download_identity::observation_memo_key(
            &observation,
            completed.download_id.as_deref(),
            completed.completed_at.map(|at| at.timestamp()),
        );
        listed.insert(key.clone());
        // A hit costs no registry transaction; it may enqueue the binding's
        // 60 s freshness refresh, which is flushed as one batch below.
        if let Some(memoized) = cache.hit(&key, observation.observed_at) {
            resolutions.push(memoized.resolution);
            continue;
        }

        let resolution =
            crate::download_identity::resolve_observed_client_job(app, observation).await;
        cache.insert(key, &resolution);
        resolutions.push(resolution);
    }

    // A live resolution can itself have mutated the registry (it attaches or
    // mints bindings). Anything decided against the older generation is
    // retired rather than carried forward.
    let generation_now = app.runtime.acquisition.download_registry_generation();
    if generation_now == generation {
        cache.retain_keys(&listed);
    } else {
        cache.reset_to_generation(generation_now);
    }
    let touches = cache.take_pending_touches();
    crate::download_identity::flush_observation_touches(app, touches).await;
    resolutions
}

pub(super) async fn resolve_completed_download_observations(
    app: &AppUseCase,
    completed_downloads: &[CompletedDownload],
) -> Vec<crate::download_identity::ObservedClientJobResolution> {
    let mut resolutions = Vec::with_capacity(completed_downloads.len());
    for completed in completed_downloads {
        resolutions.push(
            crate::download_identity::resolve_observed_client_job(
                app,
                crate::download_identity::observed_completed_job(completed),
            )
            .await,
        );
    }
    resolutions
}

fn index_completed_download_observations(
    downloads: Vec<CompletedDownload>,
    resolutions: Vec<crate::download_identity::ObservedClientJobResolution>,
    coverage: CompletedDownloadLookupCoverage,
) -> CompletedDownloadLookup {
    debug_assert_eq!(downloads.len(), resolutions.len());
    let (downloads, canonical_download_ids): (Vec<_>, Vec<_>) = downloads
        .into_iter()
        .zip(resolutions)
        .filter_map(|(completed, resolution)| match resolution {
            crate::download_identity::ObservedClientJobResolution::Resolved(download_id) => {
                Some((completed, Some(download_id)))
            }
            crate::download_identity::ObservedClientJobResolution::Conflict
            | crate::download_identity::ObservedClientJobResolution::BindingAlreadyEnded => None,
            crate::download_identity::ObservedClientJobResolution::Unavailable => {
                Some((completed, None))
            }
        })
        .unzip();
    index_completed_downloads_with_canonical_download_ids(
        downloads,
        canonical_download_ids,
        coverage,
    )
}

fn download_queue_item_needs_completed_lookup(item: &DownloadQueueItem) -> bool {
    matches!(
        item.state,
        DownloadQueueState::Completed | DownloadQueueState::ImportPending
    )
}

fn completed_download_client_scope(
    items: &[DownloadQueueItem],
    require_lookup_state: bool,
) -> (Vec<String>, Vec<String>) {
    let mut client_ids: Vec<String> = Vec::new();
    let mut fallback_client_types: Vec<String> = Vec::new();

    for item in items
        .iter()
        .filter(|item| !require_lookup_state || download_queue_item_needs_completed_lookup(item))
    {
        let client_id = item.client_id.trim();
        if !client_id.is_empty() {
            if !client_ids
                .iter()
                .any(|existing| existing.as_str() == client_id)
            {
                client_ids.push(client_id.to_string());
            }
            continue;
        }

        let client_type = item.client_type.trim();
        if !client_type.is_empty()
            && !fallback_client_types
                .iter()
                .any(|existing| existing.as_str().eq_ignore_ascii_case(client_type))
        {
            fallback_client_types.push(client_type.to_string());
        }
    }

    (client_ids, fallback_client_types)
}

pub(super) fn index_completed_downloads(
    downloads: Vec<CompletedDownload>,
    coverage: CompletedDownloadLookupCoverage,
) -> CompletedDownloadLookup {
    let mut lookup = CompletedDownloadLookup {
        coverage,
        ..CompletedDownloadLookup::default()
    };
    for completed in downloads {
        index_completed_download_into(&mut lookup, completed);
    }
    lookup
}

pub(super) fn index_completed_downloads_with_canonical_download_ids(
    downloads: Vec<CompletedDownload>,
    canonical_download_ids: Vec<Option<scryer_domain::download_identity::DownloadId>>,
    coverage: CompletedDownloadLookupCoverage,
) -> CompletedDownloadLookup {
    debug_assert_eq!(downloads.len(), canonical_download_ids.len());
    let mut lookup = CompletedDownloadLookup {
        coverage,
        ..CompletedDownloadLookup::default()
    };
    for (completed, canonical_download_id) in downloads.into_iter().zip(canonical_download_ids) {
        index_completed_download_into_with_canonical_download_id(
            &mut lookup,
            completed,
            canonical_download_id,
        );
    }
    lookup
}

fn index_completed_download_into(
    lookup: &mut CompletedDownloadLookup,
    completed: CompletedDownload,
) {
    index_completed_download_into_with_canonical_download_id(lookup, completed, None);
}

fn index_completed_download_into_with_canonical_download_id(
    lookup: &mut CompletedDownloadLookup,
    completed: CompletedDownload,
    canonical_download_id: Option<scryer_domain::download_identity::DownloadId>,
) {
    if let Some(canonical_download_id) = canonical_download_id {
        lookup
            .by_canonical
            .insert(canonical_download_id, completed.clone());
    }
    let observed_identity = observed_completed_download_identity(&completed);
    if let Some(download_id) = observed_identity
        .download_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lookup
            .by_download_id
            .entry(completed_download_lookup_key(
                Some(&completed.client_id),
                &completed.client_type,
                download_id,
            ))
            .or_default()
            .push(completed.clone());
    }
    lookup.by_source.insert(
        completed_download_lookup_key(
            Some(&completed.client_id),
            &completed.client_type,
            &completed.download_client_item_id,
        ),
        completed,
    );
}

pub(super) fn observed_queue_item_identity(
    item: &DownloadQueueItem,
) -> crate::DownloadSubmissionIdentity {
    crate::observed_download_identity(crate::ObservedDownloadIdentityInput {
        download_id: item.download_id.as_deref(),
        parameters: &[],
        info_hash_hint: None,
    })
}

pub(super) fn queue_item_source_identity(item: &DownloadQueueItem) -> ClientJobLocator {
    ClientJobLocator::new(
        Some(item.client_id.as_str()),
        item.client_type.as_str(),
        item.download_client_item_id.as_str(),
    )
}

pub(super) fn observed_completed_download_identity(
    completed: &CompletedDownload,
) -> crate::DownloadSubmissionIdentity {
    crate::observed_download_identity(crate::ObservedDownloadIdentityInput {
        download_id: completed.download_id.as_deref(),
        parameters: &completed.parameters,
        info_hash_hint: None,
    })
}

pub(super) fn completed_download_source_identity(
    completed: &CompletedDownload,
) -> ClientJobLocator {
    ClientJobLocator::new(
        Some(completed.client_id.as_str()),
        completed.client_type.as_str(),
        completed.download_client_item_id.as_str(),
    )
}

pub(super) async fn download_id_tracked_state(
    app: &AppUseCase,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    identity: &crate::DownloadSubmissionIdentity,
    source_identity: Option<&ClientJobLocator>,
) -> Option<TrackedDownloadState> {
    if crate::download_submission_identity_is_empty(identity) {
        return None;
    }
    app.services
        .workflow
        .download_submissions
        .get_identity_tracked_state_for_download(canonical_download_id, identity, source_identity)
        .await
        .ok()
        .flatten()
        .and_then(|state| TrackedDownloadState::from_str_opt(&state))
}

pub(super) fn apply_download_id_state(td: &mut TrackedDownload, state: TrackedDownloadState) {
    td.state = state;
    td.waiting_for_completed_history = false;
    match state {
        TrackedDownloadState::Imported | TrackedDownloadState::ImportedSeeding => {
            td.status = TrackedDownloadStatus::Ok;
            td.status_messages.clear();
        }
        TrackedDownloadState::Failed => {
            td.status = TrackedDownloadStatus::Error;
        }
        _ => {}
    }
}

fn single_completed_download_identity_match(
    matches: Option<&Vec<CompletedDownload>>,
    label: &str,
    value: &str,
) -> Option<CompletedDownload> {
    let matches = matches?;
    if matches.len() == 1 {
        return matches.first().cloned();
    }
    if let Some(completed) =
        crate::download_identity::coalesce_completed_downloads_by_release_observation(matches)
    {
        return Some(completed);
    }
    tracing::warn!(
        identity_kind = label,
        identity_value = value,
        matches = matches.len(),
        "find_completed_download: DownloadId matched multiple completed downloads"
    );
    None
}

fn source_match_identity_is_compatible(
    queue_identity: &crate::DownloadSubmissionIdentity,
    completed: &CompletedDownload,
) -> bool {
    let completed_identity = observed_completed_download_identity(completed);
    crate::download_submission_identity_is_empty(&completed_identity)
        || completed_identity == *queue_identity
}

pub(super) fn completed_download_lookup_key(
    client_id: Option<&str>,
    client_type: &str,
    item_id: &str,
) -> (String, String, String) {
    (
        client_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("")
            .to_string(),
        client_type.trim().to_ascii_lowercase(),
        item_id.to_string(),
    )
}

pub(super) async fn remap_completed_download_for_client(
    app: &AppUseCase,
    completed: &mut CompletedDownload,
) {
    let client_id = completed.client_id.trim();
    if client_id.is_empty() {
        return;
    }

    let config = match app
        .services
        .integrations
        .download_client_configs
        .get_by_id(client_id)
        .await
    {
        Ok(Some(config)) => config,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(
                client_id,
                error = %error,
                "find_completed_download: failed to load download client config for remote path mapping"
            );
            return;
        }
    };

    match parse_download_client_remote_path_mappings(&config.config_json) {
        Ok(mappings) => apply_remote_path_mappings_to_completed_download(completed, &mappings),
        Err(error) => {
            tracing::warn!(
                client_id,
                error = %error,
                "find_completed_download: failed to parse remote path mappings"
            );
        }
    }
}

pub(super) async fn find_completed_download(
    app: &AppUseCase,
    td: &TrackedDownload,
    completed_lookup: Option<&CompletedDownloadLookup>,
) -> Option<CompletedDownload> {
    let lookup = match completed_lookup {
        Some(_) => None,
        None => match super::load_completed_download_lookup(app).await {
            Ok(lookup) => Some(lookup),
            Err(error) => {
                tracing::warn!(error = %error, "find_completed_download: failed to fetch from client");
                return None;
            }
        },
    };
    let completed = match completed_lookup {
        Some(lookup) => find_completed_download_in_lookup(lookup, td),
        None => lookup
            .as_ref()
            .and_then(|indexed| find_completed_download_in_lookup(indexed, td)),
    };
    match completed {
        Some(completed) => {
            let mut completed = with_tracked_metadata(td, completed);
            remap_completed_download_for_client(app, &mut completed).await;
            Some(completed)
        }
        None => {
            tracing::debug!(
                id = %td.id,
                item_id = %td.client_item.download_client_item_id,
                client_type = %td.client_type,
                "find_completed_download: no matching item in client history"
            );
            None
        }
    }
}

pub(super) fn find_completed_download_in_lookup(
    lookup: &CompletedDownloadLookup,
    td: &TrackedDownload,
) -> Option<CompletedDownload> {
    let canonical_download_id = td.canonical_download_id();
    let canonical = canonical_download_id
        .and_then(|download_id| lookup.by_canonical.get(download_id))
        .cloned();
    let legacy = find_completed_download_in_lookup_legacy(lookup, td);

    match (canonical_download_id, canonical, legacy) {
        (Some(canonical_download_id), Some(canonical), Some(legacy)) => {
            if serde_json::to_vec(&canonical).ok() != serde_json::to_vec(&legacy).ok() {
                tracing::warn!(
                    target: "download_identity_resolver",
                    canonical_download_id = %canonical_download_id,
                    canonical_client_id = canonical.client_id.as_str(),
                    canonical_client_type = canonical.client_type.as_str(),
                    canonical_item_id = canonical.download_client_item_id.as_str(),
                    legacy_client_id = legacy.client_id.as_str(),
                    legacy_client_type = legacy.client_type.as_str(),
                    legacy_item_id = legacy.download_client_item_id.as_str(),
                    "completed download lookup canonical and legacy routes disagree; using legacy result"
                );
            }
            Some(legacy)
        }
        (None, Some(_), Some(legacy)) => Some(legacy),
        (_, Some(canonical), None) => Some(canonical),
        (_, None, legacy) => legacy,
    }
}

fn find_completed_download_in_lookup_legacy(
    lookup: &CompletedDownloadLookup,
    td: &TrackedDownload,
) -> Option<CompletedDownload> {
    let observed_identity = observed_queue_item_identity(&td.client_item);
    if let Some(download_id) = observed_identity
        .download_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let identity_key =
            completed_download_lookup_key(Some(&td.client_id), &td.client_type, download_id);
        if let Some(matches) = lookup.by_download_id.get(&identity_key) {
            return single_completed_download_identity_match(
                Some(matches),
                "download_id",
                download_id,
            );
        }

        let source_key = completed_download_lookup_key(
            Some(&td.client_id),
            &td.client_type,
            &td.client_item.download_client_item_id,
        );
        return lookup
            .by_source
            .get(&source_key)
            .filter(|completed| source_match_identity_is_compatible(&observed_identity, completed))
            .cloned();
    }

    let key = completed_download_lookup_key(
        Some(&td.client_id),
        &td.client_type,
        &td.client_item.download_client_item_id,
    );
    if let Some(completed) = lookup.by_source.get(&key) {
        return Some(completed.clone());
    }

    None
}

pub(super) async fn maybe_resolve_title_from_completed_download(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    completed: &CompletedDownload,
) {
    if !matches!(
        td.match_type,
        TitleMatchType::Unmatched | TitleMatchType::IdOnly | TitleMatchType::TitleParse
    ) {
        return;
    }

    clear_id_only_conflict(td);

    let Ok(matcher) = app.monitored_title_matcher().await else {
        return;
    };

    // The client-reported release name, else the media file names (non-sample,
    // largest first). Never a display label or destination folder.
    for release_title in crate::import_workflow::completed_download_release_claims(completed) {
        let parsed = crate::parse_release_metadata(&release_title);
        let resolved = if parsed.episode.is_some() {
            matcher
                .resolve_episode(
                    &parsed,
                    td.client_item.facet.as_deref().or(td.facet.as_deref()),
                )
                .await
        } else {
            matcher.resolve_movie(&parsed).await
        }
        .unwrap_or_default();

        if let Some(resolved) = resolved {
            if td.match_type == TitleMatchType::IdOnly
                && let Some(existing_title_id) = td.title_id.as_deref()
                && existing_title_id != resolved.title.id
            {
                td.status = TrackedDownloadStatus::Warning;
                td.status_messages.retain(|message| {
                    !matches!(
                        message.kind,
                        crate::tracked_downloads::TrackedStatusMessageKind::IdOnlyConfirmation
                            | crate::tracked_downloads::TrackedStatusMessageKind::IdOnlyConflict
                    )
                });
                td.warn_kind(
                    crate::tracked_downloads::TrackedStatusMessageKind::IdOnlyConflict,
                    ID_ONLY_CONFLICT_MESSAGE,
                );
                return;
            }

            td.title_id = Some(resolved.title.id.clone());
            td.facet = Some(resolved.title.facet.as_str().to_string());
            td.source_title = Some(release_title);
            if td.match_type != TitleMatchType::IdOnly {
                td.match_type = resolved.match_type;
            }
            return;
        }
    }
}

fn clear_id_only_conflict(td: &mut TrackedDownload) {
    td.status_messages.retain(|message| {
        message.kind != crate::tracked_downloads::TrackedStatusMessageKind::IdOnlyConflict
    });
    if td.status_messages.is_empty() && td.status == TrackedDownloadStatus::Warning {
        td.status = TrackedDownloadStatus::Ok;
    }
}

pub(super) fn with_tracked_metadata(
    td: &TrackedDownload,
    mut completed: CompletedDownload,
) -> CompletedDownload {
    upsert_parameter(
        &mut completed.parameters,
        "*scryer_title_id",
        td.title_id.clone().unwrap_or_default(),
    );
    upsert_parameter(
        &mut completed.parameters,
        "*scryer_facet",
        td.facet.clone().unwrap_or_default(),
    );
    completed
}

fn upsert_parameter(params: &mut Vec<(String, String)>, key: &str, value: String) {
    if value.trim().is_empty() {
        return;
    }

    if let Some((_, existing)) = params
        .iter_mut()
        .find(|(existing_key, _)| existing_key == key)
    {
        *existing = value;
    } else {
        params.push((key.to_string(), value));
    }
}
