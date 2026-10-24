use chrono::Utc;
use scryer_domain::{CompletedDownload, download_identity::DownloadId};

use crate::{
    AppUseCase, ClientJobLocator, DownloadSourceKind, DownloadSubmission,
    DownloadSubmissionIdentity, ObservationResolution, ObservedClientJob, SubmissionScope,
    extract_magnet_info_hash,
};

pub const DOWNLOAD_ID_PARAMETER: &str = "*scryer_download_id";

pub struct AcceptedDownloadIdentityInput<'a> {
    pub initial_download_id: Option<&'a str>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_hint: Option<&'a str>,
    pub info_hash_hint: Option<&'a str>,
    pub client_type: Option<&'a str>,
    pub client_item_id: Option<&'a str>,
    pub accepted_info_hash: Option<&'a str>,
}

pub struct ObservedDownloadIdentityInput<'a> {
    pub download_id: Option<&'a str>,
    pub parameters: &'a [(String, String)],
    pub info_hash_hint: Option<&'a str>,
}

pub(crate) fn coalesce_download_submissions_by_release_attempt(
    submissions: &[DownloadSubmission],
) -> Option<DownloadSubmission> {
    let first = submissions.first()?;
    let first_key = download_submission_release_attempt_key(first);
    submissions
        .iter()
        .all(|submission| download_submission_release_attempt_key(submission) == first_key)
        .then(|| first.clone())
}

pub(crate) fn coalesce_completed_downloads_by_release_observation(
    downloads: &[CompletedDownload],
) -> Option<CompletedDownload> {
    let first = downloads.first()?;
    let first_key = completed_download_release_observation_key(first);
    downloads
        .iter()
        .all(|download| completed_download_release_observation_key(download) == first_key)
        .then(|| first.clone())
}

pub fn observed_download_identity(
    input: ObservedDownloadIdentityInput<'_>,
) -> DownloadSubmissionIdentity {
    let download_id = normalize_token(input.download_id)
        .or_else(|| observed_identity_parameter(input.parameters, DOWNLOAD_ID_PARAMETER))
        .or_else(|| download_id_from_info_hash(input.info_hash_hint));

    DownloadSubmissionIdentity { download_id }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ObservedClientJobResolution {
    Resolved(DownloadId),
    Conflict,
    BindingAlreadyEnded,
    Unavailable,
}

/// Memo of client-observation identity resolutions.
///
/// Every download-client tick re-reads the same queue and history rows — six
/// times a minute at the default 10 s cadence — and
/// [`resolve_observed_client_job`] is a registry *transaction* per row. A
/// client history window full of rows Scryer never submitted therefore cost a
/// couple of hundred transactions through the SQLite writer gate every tick,
/// forever, for an answer that cannot have changed.
///
/// The answer for an unchanged row is a pure function of the observed
/// locator/token/name and the registry's binding state, so entries are held
/// against [`AppRuntimeAcquisitionState::download_registry_generation`] and the
/// whole memo is dropped the moment a binding is created, attached or ended.
/// A resolution that was merely *unavailable* (a registry read error) is never
/// memoized. A `Conflict` is, because it too is a fact about the registry's
/// current generation: only a registry write can heal it, and every registry
/// write bumps the generation.
///
/// Bindings are also created and retired inside the workflow stores' own
/// transactions (a re-add that retires a stale terminal binding and mints its
/// replacement, a title delete, a queue-item delete). The app-layer call sites
/// of those paths bump the generation, but the generation is only as good as
/// that enumeration, so [`OBSERVATION_MEMO_TTL`] is the backstop: no entry is
/// reused past it, whatever the generation says. At a 10 s tick that is one
/// re-resolution per row per 10 minutes instead of six per minute.
///
/// Two instances exist, of this one type: the download-queue poller owns a
/// listing-scoped instance for the completed-history read (it is rebuilt from
/// each listing, so a row the client stops reporting drops out with it), and
/// [`AppRuntimeAcquisitionState::download_observation_resolutions`] holds the
/// process-wide instance the queue-enrichment and tracker paths share, pruned
/// by [`OBSERVATION_MEMO_TTL`] and capped by
/// [`OBSERVATION_MEMO_MAX_ENTRIES`].
#[derive(Default)]
pub(crate) struct ObservationResolutionCache {
    generation: u64,
    entries: std::collections::HashMap<ObservationMemoKey, MemoizedObservationResolution>,
    /// Bindings whose `last_seen_at`/`last_observed_at` refresh came due while
    /// serving a memo hit, waiting to be written as one batch.
    pending_touches: Vec<crate::ports::ObservationTouch>,
}

/// The inputs a client row contributes to its identity resolution, plus the
/// freshness fields that prove the row is the same sighting.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ObservationMemoKey {
    client_id: Option<String>,
    client_type: String,
    item_id: String,
    /// The raw client-reported download id, not the parsed wire token: the
    /// token is a pure function of it, and the by-id submission fallback is
    /// keyed on the raw string.
    download_id: Option<String>,
    observed_name: Option<String>,
    /// A completion timestamp for history rows; `None` for live queue rows.
    freshness: Option<i64>,
}

#[derive(Clone)]
struct MemoizedObservationResolution {
    resolution: ObservedClientJobResolution,
    /// When this row was actually resolved. Carried across ticks unchanged, so
    /// reuse cannot slide the expiry forward indefinitely.
    resolved_at: std::time::Instant,
    /// When this row's binding timestamps were last written. The resolving
    /// transaction writes them itself, so a fresh entry starts here.
    touched_at: std::time::Instant,
    /// The by-id submission fallback ran for this row and found nothing. A
    /// grab that would change that bumps the registry generation (it mints an
    /// unbound binding), which retires this entry.
    submission_lookup_missed: bool,
}

/// Longest a memoized resolution is trusted, regardless of registry generation.
pub(crate) const OBSERVATION_MEMO_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// How often a live binding's `last_seen_at` is refreshed while its row keeps
/// being observed. Mirrors the 60 s throttle the resolving transaction applies
/// (`observation_timestamp_write_required`), so memoizing a row does not stop
/// its freshness being recorded.
pub(crate) const OBSERVATION_TOUCH_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(60);

/// Hard cap on the process-wide memo. Reaching it means far more distinct rows
/// than any client set observes in a TTL window, so the whole memo is dropped
/// rather than grown without bound.
const OBSERVATION_MEMO_MAX_ENTRIES: usize = 8_192;

/// What a memo hit tells the caller.
pub(crate) struct MemoizedObservation {
    pub(crate) resolution: ObservedClientJobResolution,
    /// The by-id submission fallback already ran for this row and found
    /// nothing, so the caller may skip it instead of re-issuing its selects.
    pub(crate) submission_lookup_missed: bool,
}

pub(crate) fn observation_memo_key(
    observation: &ObservedClientJob,
    raw_download_id: Option<&str>,
    freshness: Option<i64>,
) -> ObservationMemoKey {
    ObservationMemoKey {
        client_id: observation.locator.client_id.clone(),
        client_type: observation.locator.client_type.clone(),
        item_id: observation.locator.item_id.clone(),
        download_id: raw_download_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        observed_name: observation
            .observed_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase),
        freshness,
    }
}

/// The identity of a conflict, for warn-once bookkeeping.
///
/// Two conflicts are the same conflict when the same client row names the same
/// token and is blocked by the same binding. Anything else — the row moving to
/// another token, another binding taking the locator — is a new fact and warns
/// again.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ConflictWarningKey {
    client_id: Option<String>,
    client_type: String,
    item_id: String,
    token_id: DownloadId,
    binding_download_id: DownloadId,
}

/// Hard cap on the warn-once set, mirroring [`OBSERVATION_MEMO_MAX_ENTRIES`].
/// Reaching it would mean more distinct *conflicts* than any client set can
/// produce, so the set is dropped rather than grown without bound; the cost of
/// being wrong is one repeated warning, not unbounded memory.
const CONFLICT_WARNING_MAX_ENTRIES: usize = 8_192;

pub(crate) fn observation_resolution_is_memoizable(
    resolution: &ObservedClientJobResolution,
) -> bool {
    match resolution {
        // A resolved binding and an ended binding are both facts about the
        // registry's current generation; a bump retires them.
        ObservedClientJobResolution::Resolved(_)
        | ObservedClientJobResolution::BindingAlreadyEnded => true,
        // So is a conflict. A conflict can only be healed by a registry write —
        // a binding created, attached, rebound or ended — and every registry
        // write bumps `download_registry_generation`, which drops the whole
        // memo. [`OBSERVATION_MEMO_TTL`] is the backstop for a write path that
        // forgets to bump, exactly as it is for `Resolved`.
        //
        // This used to be `false`, on the reasoning that "a later tick may heal
        // it". A later tick heals nothing by itself: it re-runs the same read
        // against the same rows and reaches the same answer. The exclusion
        // therefore bought no healing and cost a `resolve_observation` write
        // transaction, per conflicting row, per call site, per tick, forever —
        // ~1,100 transactions/s and ~1,650 log lines/s on a load-test instance
        // with 961 conflicting rows.
        ObservedClientJobResolution::Conflict => true,
        // An unavailable resolution is a read failure that must be retried.
        ObservedClientJobResolution::Unavailable => false,
    }
}

impl ObservationResolutionCache {
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Backdates every entry, so a test can reach the age backstop without
    /// sleeping.
    #[cfg(test)]
    pub(crate) fn age_entries_for_test(&mut self, by: std::time::Duration) {
        for entry in self.entries.values_mut() {
            if let Some(resolved_at) = entry.resolved_at.checked_sub(by) {
                entry.resolved_at = resolved_at;
            }
            if let Some(touched_at) = entry.touched_at.checked_sub(by) {
                entry.touched_at = touched_at;
            }
        }
    }

    /// Drop everything decided against an older registry generation.
    pub(crate) fn begin_generation(&mut self, generation: u64) {
        if self.generation != generation {
            self.entries.clear();
            self.generation = generation;
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn reset_to_generation(&mut self, generation: u64) {
        self.entries.clear();
        self.generation = generation;
    }

    /// Serve a memoized resolution, enqueueing the binding's periodic
    /// freshness write when it has come due.
    ///
    /// The touch is *claimed* here (the entry's clock moves) and written by
    /// [`flush_observation_touches`] as one batch, so a tick over many rows
    /// costs at most one touch transaction instead of one per row.
    pub(crate) fn hit(
        &mut self,
        key: &ObservationMemoKey,
        observed_at: chrono::DateTime<Utc>,
    ) -> Option<MemoizedObservation> {
        let entry = self.entries.get_mut(key)?;
        if entry.resolved_at.elapsed() >= OBSERVATION_MEMO_TTL {
            self.entries.remove(key);
            return None;
        }
        if let ObservedClientJobResolution::Resolved(download_id) = entry.resolution
            && entry.touched_at.elapsed() >= OBSERVATION_TOUCH_INTERVAL
        {
            entry.touched_at = std::time::Instant::now();
            self.pending_touches.push(crate::ports::ObservationTouch {
                download_id,
                observed_at,
            });
        }
        let entry = &self.entries[key];
        Some(MemoizedObservation {
            resolution: entry.resolution.clone(),
            submission_lookup_missed: entry.submission_lookup_missed,
        })
    }

    pub(crate) fn insert(
        &mut self,
        key: ObservationMemoKey,
        resolution: &ObservedClientJobResolution,
    ) {
        if !observation_resolution_is_memoizable(resolution) {
            self.entries.remove(&key);
            return;
        }
        let now = std::time::Instant::now();
        self.entries.insert(
            key,
            MemoizedObservationResolution {
                resolution: resolution.clone(),
                resolved_at: now,
                // The resolving transaction wrote the timestamps itself.
                touched_at: now,
                submission_lookup_missed: false,
            },
        );
    }

    /// Record that the by-id submission fallback found nothing for this row.
    pub(crate) fn record_submission_lookup_miss(&mut self, key: &ObservationMemoKey) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.submission_lookup_missed = true;
        }
    }

    /// Keep only the rows this listing reported, so a row the client stopped
    /// reporting leaves the memo with it.
    pub(crate) fn retain_keys(&mut self, keys: &std::collections::HashSet<ObservationMemoKey>) {
        self.entries.retain(|key, _| keys.contains(key));
    }

    /// Bound the process-wide memo: expired entries go, and a memo that has
    /// somehow grown past the cap is dropped whole.
    fn prune(&mut self) {
        self.entries
            .retain(|_, entry| entry.resolved_at.elapsed() < OBSERVATION_MEMO_TTL);
        if self.entries.len() > OBSERVATION_MEMO_MAX_ENTRIES {
            self.entries.clear();
        }
    }

    pub(crate) fn take_pending_touches(&mut self) -> Vec<crate::ports::ObservationTouch> {
        std::mem::take(&mut self.pending_touches)
    }
}

/// Resolve an observation through the process-wide memo.
///
/// A row whose locator, token, name and freshness are unchanged, and whose
/// registry generation has not moved, costs no registry transaction and no
/// statements at all.
pub(crate) async fn resolve_observed_client_job_memoized(
    app: &AppUseCase,
    observation: ObservedClientJob,
    raw_download_id: Option<&str>,
) -> MemoizedObservation {
    let key = observation_memo_key(&observation, raw_download_id, None);
    let generation = app.runtime.acquisition.download_registry_generation();
    {
        let mut cache = app
            .runtime
            .acquisition
            .download_observation_resolutions
            .lock()
            .await;
        cache.begin_generation(generation);
        if let Some(hit) = cache.hit(&key, observation.observed_at) {
            return hit;
        }
    }

    let resolution = resolve_observed_client_job(app, observation).await;
    let mut cache = app
        .runtime
        .acquisition
        .download_observation_resolutions
        .lock()
        .await;
    // A live resolution can itself have mutated the registry (it attaches or
    // mints bindings), and a concurrent writer can have moved it while this
    // one ran. Anything decided against the older generation is retired rather
    // than carried forward, this resolution included: it is returned, but the
    // next tick resolves the row again against the current registry.
    let generation_now = app.runtime.acquisition.download_registry_generation();
    if cache.generation() != generation_now {
        cache.reset_to_generation(generation_now);
    }
    cache.prune();
    if generation_now == generation {
        cache.insert(key, &resolution);
    }
    MemoizedObservation {
        resolution,
        submission_lookup_missed: false,
    }
}

/// Note that the by-id submission fallback found nothing for this row, so a
/// later tick over the same unchanged row can skip it.
pub(crate) async fn record_memoized_submission_lookup_miss(
    app: &AppUseCase,
    observation: &ObservedClientJob,
    raw_download_id: Option<&str>,
) {
    let key = observation_memo_key(observation, raw_download_id, None);
    app.runtime
        .acquisition
        .download_observation_resolutions
        .lock()
        .await
        .record_submission_lookup_miss(&key);
}

/// Write every binding freshness refresh that came due this cycle, in one
/// transaction.
pub(crate) async fn flush_observation_touches(
    app: &AppUseCase,
    touches: Vec<crate::ports::ObservationTouch>,
) {
    if touches.is_empty() {
        return;
    }
    if let Err(error) = app
        .services
        .workflow
        .download_registry
        .touch_observations(&touches)
        .await
    {
        tracing::debug!(
            target: "download_identity_resolver",
            error = %error,
            touches = touches.len(),
            "failed to refresh observed download timestamps"
        );
    }
}

/// Drain and write the process-wide memo's due freshness refreshes.
pub(crate) async fn flush_shared_observation_touches(app: &AppUseCase) {
    let touches = app
        .runtime
        .acquisition
        .download_observation_resolutions
        .lock()
        .await
        .take_pending_touches();
    flush_observation_touches(app, touches).await;
}

pub(crate) fn legacy_binding_predates_delete(
    binding: &crate::DownloadClientBindingRecord,
    command_created_at: chrono::DateTime<Utc>,
) -> bool {
    binding.last_seen_at.unwrap_or(binding.created_at) <= command_created_at
}

fn completed_delete_proves_stale_binding(
    command: &crate::DownloadQueueCommandRecord,
    binding: &crate::DownloadClientBindingRecord,
    locator: &ClientJobLocator,
) -> bool {
    if command.status != scryer_domain::DownloadQueueDeleteStatus::Completed {
        return false;
    }
    let Ok(created_at) = chrono::DateTime::parse_from_rfc3339(&command.created_at) else {
        return false;
    };
    let Some(finished_at) = command
        .finished_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return false;
    };
    if finished_at < created_at {
        return false;
    }

    match command.canonical_download_id {
        Some(download_id) => download_id == binding.download_id,
        None => {
            ClientJobLocator::new(
                command.client_id.as_deref(),
                &command.client_type,
                &command.download_client_item_id,
            ) == *locator
                && legacy_binding_predates_delete(binding, created_at.with_timezone(&Utc))
        }
    }
}

async fn heal_binding_after_completed_delete(
    app: &AppUseCase,
    observation: &ObservedClientJob,
    conflicting_download_id: DownloadId,
) -> bool {
    let binding = match app
        .services
        .workflow
        .download_registry
        .find_active_binding_by_locator(&observation.locator)
        .await
    {
        Ok(Some(binding)) if binding.download_id == conflicting_download_id => binding,
        Ok(_) | Err(_) => return false,
    };
    let sources = [
        (
            observation.locator.client_id.clone(),
            observation.locator.client_type.clone(),
            observation.locator.item_id.clone(),
            false,
        ),
        (
            observation.locator.client_id.clone(),
            observation.locator.client_type.clone(),
            observation.locator.item_id.clone(),
            true,
        ),
    ];
    let commands = match app
        .services
        .workflow
        .download_queue_commands
        .list_latest_delete_commands_for_sources(&sources, true)
        .await
    {
        Ok(commands) => commands,
        Err(_) => return false,
    };
    if !commands.iter().any(|command| {
        completed_delete_proves_stale_binding(command, &binding, &observation.locator)
    }) {
        return false;
    }

    if app
        .services
        .workflow
        .download_registry
        .end_binding(&binding.download_id)
        .await
        .is_err()
    {
        return false;
    }
    app.runtime
        .acquisition
        .invalidate_download_registry_observations();

    tracing::info!(
        target: "download_identity_resolver",
        download_id = %binding.download_id,
        config_id = observation.locator.client_id.as_deref().unwrap_or(""),
        client_type = %observation.locator.client_type,
        native_item_id = %observation.locator.item_id,
        "retired stale binding proven by completed delete"
    );
    true
}

/// Whether this exact conflict has not been announced yet, claiming it if so.
async fn conflict_warning_is_first(
    app: &AppUseCase,
    locator: &ClientJobLocator,
    token_id: DownloadId,
    binding_download_id: DownloadId,
) -> bool {
    let key = ConflictWarningKey {
        client_id: locator.client_id.clone(),
        client_type: locator.client_type.clone(),
        item_id: locator.item_id.clone(),
        token_id,
        binding_download_id,
    };
    let mut warned = app
        .runtime
        .acquisition
        .warned_download_identity_conflicts
        .lock()
        .await;
    if warned.len() >= CONFLICT_WARNING_MAX_ENTRIES {
        warned.clear();
    }
    warned.insert(key)
}

/// Resolve an observation to the sole workflow identity.
pub(crate) async fn resolve_observed_client_job(
    app: &AppUseCase,
    observation: ObservedClientJob,
) -> ObservedClientJobResolution {
    let valid_token = observation
        .wire_token
        .as_deref()
        .and_then(scryer_domain::download_identity::DownloadId::from_wire);
    let token = observation.wire_token.as_deref().unwrap_or("");
    let config_id = observation.locator.client_id.as_deref().unwrap_or("");
    let client_type = observation.locator.client_type.as_str();
    let native_item_id = observation.locator.item_id.as_str();

    let mut resolution = app
        .services
        .workflow
        .download_registry
        .resolve_observation(&observation)
        .await;
    if let Ok(ObservationResolution::Conflict {
        binding_download_id,
        ..
    }) = &resolution
        && heal_binding_after_completed_delete(app, &observation, *binding_download_id).await
    {
        resolution = app
            .services
            .workflow
            .download_registry
            .resolve_observation(&observation)
            .await;
    }

    match resolution {
        Ok(ObservationResolution::Resolved {
            download_id,
            newly_foreign,
            attached,
        }) => {
            if valid_token.is_some() && newly_foreign {
                tracing::debug!(
                    target: "download_identity_resolver",
                    token,
                    config_id,
                    client_type,
                    native_item_id,
                    "unknown valid token adopted as foreign"
                );
            }
            // A binding was created or attached, so every memoized resolution
            // taken against the previous registry state is retired.
            if attached || newly_foreign {
                app.runtime
                    .acquisition
                    .invalidate_download_registry_observations();
            }
            if valid_token.is_none() && attached {
                tracing::debug!(
                    target: "download_identity_resolver",
                    config_id,
                    client_type,
                    native_item_id,
                    "ambiguous locator attached"
                );
            }
            ObservedClientJobResolution::Resolved(download_id)
        }
        Ok(ObservationResolution::Rebound { download_id }) => {
            // The binding row moved, so every memoized resolution taken against
            // the previous registry state is retired. That also means this INFO
            // is logged once and not again: the next tick resolves the item
            // normally and memoizes it.
            app.runtime
                .acquisition
                .invalidate_download_registry_observations();
            tracing::info!(
                target: "download_identity_resolver",
                token,
                config_id,
                client_type,
                native_item_id,
                download_id = %download_id,
                "rebound an ended download binding onto the client now reporting it"
            );
            ObservedClientJobResolution::Resolved(download_id)
        }
        Ok(ObservationResolution::Conflict {
            token_id,
            binding_download_id,
        }) => {
            // A conflict is a standing state, so it is announced once and then
            // kept at debug. The memo already stops most repeats; this covers
            // the rest, because any grab bumps the generation and clears it.
            if conflict_warning_is_first(app, &observation.locator, token_id, binding_download_id)
                .await
            {
                tracing::warn!(
                    target: "download_identity_resolver",
                    token,
                    config_id,
                    client_type,
                    native_item_id,
                    token_id = %token_id,
                    binding_download_id = %binding_download_id,
                    "conflicting canonical download identity observation"
                );
            } else {
                tracing::debug!(
                    target: "download_identity_resolver",
                    token,
                    config_id,
                    client_type,
                    native_item_id,
                    token_id = %token_id,
                    binding_download_id = %binding_download_id,
                    "conflicting canonical download identity observation (already reported)"
                );
            }
            ObservedClientJobResolution::Conflict
        }
        Ok(ObservationResolution::BindingAlreadyEnded) => {
            tracing::debug!(
                target: "download_identity_resolver",
                token,
                config_id,
                client_type,
                native_item_id,
                "observed client job belongs to an ended binding; skipping item"
            );
            ObservedClientJobResolution::BindingAlreadyEnded
        }
        Err(error) => {
            tracing::warn!(
                target: "download_identity_resolver",
                token,
                config_id,
                client_type,
                native_item_id,
                error = %error,
                "failed to resolve client observation"
            );
            ObservedClientJobResolution::Unavailable
        }
    }
}

/// Whether a client row carries a Scryer-minted download token. Such a row is
/// Scryer's own work whatever category the client files it under.
pub(crate) fn queue_item_carries_scryer_token(item: &scryer_domain::DownloadQueueItem) -> bool {
    item.download_id
        .as_deref()
        .and_then(DownloadId::from_wire)
        .is_some()
}

/// Whether this client row may be resolved against the download registry.
///
/// Resolution *is* adoption for a row Scryer never submitted: it leaves behind
/// a permanent `downloads` row and an active `download_client_bindings` row.
/// Rows sitting outside the categories their client feeds Scryer are therefore
/// never resolved, so they never become a download, a binding, or a tracked
/// download. Scryer's own rows — a submission joined onto the row, or a
/// Scryer wire token on it — are never filtered.
pub(crate) fn queue_item_is_in_adoption_scope(
    item: &scryer_domain::DownloadQueueItem,
    snapshot: Option<&crate::services::DownloadClientCategoryAdmissionSnapshot>,
) -> bool {
    queue_item_carries_scryer_token(item)
        || crate::services::foreign_observation_is_in_client_scope(
            item.is_scryer_origin,
            item.client_id.as_str(),
            item.category.as_deref(),
            snapshot,
        )
}

pub(crate) fn observed_queue_item_job(
    item: &scryer_domain::DownloadQueueItem,
) -> ObservedClientJob {
    ObservedClientJob {
        locator: ClientJobLocator::new(
            Some(item.client_id.as_str()),
            item.client_type.as_str(),
            item.download_client_item_id.as_str(),
        ),
        wire_token: item
            .download_id
            .as_deref()
            .and_then(DownloadId::from_wire)
            .map(|id| id.to_wire()),
        observed_name: (!item.title_name.trim().is_empty()).then(|| item.title_name.clone()),
        observed_at: Utc::now(),
    }
}

pub(crate) fn observed_completed_job(item: &CompletedDownload) -> ObservedClientJob {
    let observed_name = item
        .release_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| (!item.name.trim().is_empty()).then(|| item.name.clone()));
    ObservedClientJob {
        locator: ClientJobLocator::new(
            Some(item.client_id.as_str()),
            item.client_type.as_str(),
            item.download_client_item_id.as_str(),
        ),
        wire_token: item
            .download_id
            .as_deref()
            .and_then(DownloadId::from_wire)
            .map(|id| id.to_wire()),
        observed_name,
        observed_at: Utc::now(),
    }
}

pub fn download_submission_identity_is_empty(identity: &DownloadSubmissionIdentity) -> bool {
    identity
        .download_id
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
}

pub fn download_id_from_info_hash(info_hash_hint: Option<&str>) -> Option<String> {
    normalize_torrent_info_hash(info_hash_hint)
}

pub fn normalize_torrent_info_hash(raw: Option<&str>) -> Option<String> {
    let mut value = normalize_lower(raw)?;
    if let Some(stripped) = value.strip_prefix("urn:btih:") {
        value = stripped.to_string();
    }
    let normalized = value
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect::<String>();
    matches!(normalized.len(), 40 | 64).then_some(normalized)
}

pub fn accepted_download_submission_identity(
    input: AcceptedDownloadIdentityInput<'_>,
) -> DownloadSubmissionIdentity {
    let expected_info_hash = normalize_torrent_info_hash(input.info_hash_hint)
        .or_else(|| normalize_magnet_info_hash(input.source_hint));
    let actual_info_hash = normalize_torrent_info_hash(input.accepted_info_hash).or_else(|| {
        if source_kind_is_torrent(input.source_kind) || expected_info_hash.is_some() {
            normalize_torrent_info_hash(input.client_item_id)
        } else {
            None
        }
    });

    if let (Some(expected), Some(actual)) = (&expected_info_hash, &actual_info_hash)
        && expected != actual
    {
        tracing::debug!(
            initial_download_id = input.initial_download_id.unwrap_or(""),
            client_type = input.client_type.unwrap_or(""),
            expected_info_hash = expected.as_str(),
            actual_info_hash = actual.as_str(),
            "download_torrent_info_hash_mismatch"
        );
    }

    let download_id = actual_info_hash
        .or(expected_info_hash)
        .or_else(|| {
            client_round_trips_download_id(input.client_type)
                .then(|| normalize_token(input.initial_download_id))
                .flatten()
        })
        .or_else(|| normalize_token(input.client_item_id))
        .or_else(|| normalize_token(input.initial_download_id));

    DownloadSubmissionIdentity { download_id }
}

/// Clients that store the grab's download id with the job and report it back
/// on every queue and completed row. Their identity is that id, so the import
/// finds the grab by what the client reports rather than by its item id.
fn client_round_trips_download_id(client_type: Option<&str>) -> bool {
    matches!(
        normalize_lower(client_type).as_deref(),
        Some("nzbget" | "weaver" | "http")
    )
}

fn normalize_token(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn observed_identity_parameter(parameters: &[(String, String)], key: &str) -> Option<String> {
    parameters
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, value)| normalize_token(Some(value)))
}

fn normalize_lower(raw: Option<&str>) -> Option<String> {
    normalize_token(raw).map(|value| value.to_ascii_lowercase())
}

fn download_submission_release_attempt_key(submission: &DownloadSubmission) -> String {
    let scope = match &submission.scope {
        SubmissionScope::Episode { episode_id } => format!("episode:{}", episode_id.trim()),
        SubmissionScope::EpisodeSet { episode_ids } => {
            let mut ids = episode_ids
                .iter()
                .map(|episode_id| episode_id.trim().to_string())
                .filter(|episode_id| !episode_id.is_empty())
                .collect::<Vec<_>>();
            ids.sort();
            ids.dedup();
            format!("episodes:{}", ids.join(","))
        }
        SubmissionScope::Collection { collection_id } => {
            format!("collection:{}", collection_id.trim())
        }
        SubmissionScope::Title => "title".to_string(),
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => format!("series_movie:{series_movie_link_id}"),
        SubmissionScope::Orphan => "orphan".to_string(),
    };
    format!(
        "{}\u{1f}{}\u{1f}{}",
        submission.title_id.trim(),
        submission.facet.trim().to_ascii_lowercase(),
        scope
    )
}

fn completed_download_release_observation_key(completed: &CompletedDownload) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{:?}",
        normalize_observed_identity_field(&completed.client_type),
        normalize_observed_identity_field(&completed.name),
        normalize_observed_identity_field(&completed.dest_dir),
        completed
            .category
            .as_deref()
            .map(normalize_observed_identity_field)
            .unwrap_or_default(),
        completed.size_bytes
    )
}

fn normalize_observed_identity_field(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_magnet_info_hash(raw: Option<&str>) -> Option<String> {
    raw.and_then(extract_magnet_info_hash)
}

fn source_kind_is_torrent(source_kind: Option<DownloadSourceKind>) -> bool {
    matches!(
        source_kind,
        Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_torrent_hash_is_download_id() {
        let accepted_hash = "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD";
        let identity = accepted_download_submission_identity(AcceptedDownloadIdentityInput {
            initial_download_id: Some("scryer-download:1"),
            source_kind: Some(DownloadSourceKind::TorrentFile),
            source_hint: Some("https://indexer.example/release.torrent"),
            info_hash_hint: Some("0123456789abcdef0123456789abcdef01234567"),
            client_type: Some("qbittorrent"),
            client_item_id: Some("job-1"),
            accepted_info_hash: Some(accepted_hash),
        });

        assert_eq!(
            identity.download_id.as_deref(),
            Some("abcdefabcdefabcdefabcdefabcdefabcdefabcd")
        );
    }

    #[test]
    fn nzbget_keeps_round_trip_download_id() {
        let identity = accepted_download_submission_identity(AcceptedDownloadIdentityInput {
            initial_download_id: Some("scryer-download:1"),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_hint: Some("https://indexer.example/release.nzb"),
            info_hash_hint: None,
            client_type: Some("nzbget"),
            client_item_id: Some("10010"),
            accepted_info_hash: None,
        });

        assert_eq!(identity.download_id.as_deref(), Some("scryer-download:1"));
    }

    #[test]
    fn sab_uses_client_download_id() {
        let identity = accepted_download_submission_identity(AcceptedDownloadIdentityInput {
            initial_download_id: Some("scryer-download:1"),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_hint: Some("https://indexer.example/release.nzb"),
            info_hash_hint: None,
            client_type: Some("sabnzbd"),
            client_item_id: Some("SABnzbd_nzo_abc"),
            accepted_info_hash: None,
        });

        assert_eq!(identity.download_id.as_deref(), Some("SABnzbd_nzo_abc"));
    }

    #[test]
    fn completed_delete_evidence_requires_exact_identity_and_safe_time_ordering() {
        let command_at = chrono::DateTime::parse_from_rfc3339("2026-08-29T12:00:00Z")
            .expect("valid command timestamp")
            .with_timezone(&Utc);
        let locator = ClientJobLocator::new(Some("client-1"), "weaver", "job-1");
        let binding_id = DownloadId::new();
        let binding = crate::DownloadClientBindingRecord {
            download_id: binding_id,
            client_config_id: locator.client_id.clone(),
            client_type_snapshot: Some(locator.client_type.clone()),
            client_name_snapshot: None,
            native_item_id: Some(locator.item_id.clone()),
            created_at: command_at - chrono::Duration::minutes(2),
            last_seen_at: Some(command_at - chrono::Duration::minutes(1)),
            ended_at: None,
        };
        let command = crate::DownloadQueueCommandRecord {
            id: "delete-1".to_string(),
            action: scryer_domain::DownloadQueueCommandAction::Delete,
            canonical_download_id: None,
            client_id: locator.client_id.clone(),
            client_type: locator.client_type.clone(),
            download_client_item_id: locator.item_id.clone(),
            is_history: false,
            status: scryer_domain::DownloadQueueDeleteStatus::Completed,
            error_text: None,
            requested_by_user_id: None,
            started_at: Some(command_at.to_rfc3339()),
            finished_at: Some((command_at + chrono::Duration::seconds(1)).to_rfc3339()),
            created_at: command_at.to_rfc3339(),
            updated_at: command_at.to_rfc3339(),
        };
        assert!(completed_delete_proves_stale_binding(
            &command, &binding, &locator
        ));
        let binding_at_boundary = crate::DownloadClientBindingRecord {
            last_seen_at: Some(command_at),
            ..binding.clone()
        };
        assert!(completed_delete_proves_stale_binding(
            &command,
            &binding_at_boundary,
            &locator
        ));
        let binding_seen_later = crate::DownloadClientBindingRecord {
            last_seen_at: Some(command_at + chrono::Duration::seconds(1)),
            ..binding.clone()
        };
        assert!(!completed_delete_proves_stale_binding(
            &command,
            &binding_seen_later,
            &locator
        ));

        let wrong_locator = ClientJobLocator::new(Some("client-1"), "weaver", "job-2");
        let mut altered = command.clone();
        altered.canonical_download_id = Some(binding_id);
        assert!(completed_delete_proves_stale_binding(
            &altered,
            &binding_seen_later,
            &wrong_locator
        ));
        altered.canonical_download_id = Some(DownloadId::new());
        assert!(!completed_delete_proves_stale_binding(
            &altered, &binding, &locator
        ));
        assert!(!completed_delete_proves_stale_binding(
            &command,
            &binding,
            &wrong_locator
        ));

        altered = command.clone();
        altered.status = scryer_domain::DownloadQueueDeleteStatus::Failed;
        assert!(!completed_delete_proves_stale_binding(
            &altered, &binding, &locator
        ));
        altered = command.clone();
        altered.finished_at = None;
        assert!(!completed_delete_proves_stale_binding(
            &altered, &binding, &locator
        ));
        altered = command.clone();
        altered.created_at = "not-a-timestamp".to_string();
        assert!(!completed_delete_proves_stale_binding(
            &altered, &binding, &locator
        ));
    }
}
