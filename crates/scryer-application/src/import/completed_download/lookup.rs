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
        self.coverage = coverage;
    }

    /// Completion timestamp for a client queue item, when this lookup holds a
    /// matching row keyed by its client-scoped source reference.
    pub(crate) fn completed_at_for_item(
        &self,
        item: &DownloadQueueItem,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        self.by_source
            .get(&completed_download_lookup_key(
                Some(&item.client_id),
                &item.client_type,
                &item.download_client_item_id,
            ))
            .and_then(|completed| completed.completed_at)
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

pub(crate) async fn load_completed_download_lookup(
    app: &AppUseCase,
) -> AppResult<CompletedDownloadLookup> {
    let completed_downloads = app
        .services
        .integrations
        .download_client
        .list_completed_downloads()
        .await?;
    Ok(index_completed_downloads(
        completed_downloads,
        CompletedDownloadLookupCoverage::Full,
    ))
}

async fn load_recent_completed_download_lookup_for_items_or_default_excluding_client_types(
    app: &AppUseCase,
    items: &[DownloadQueueItem],
    limit: usize,
    excluded_client_types: &[&str],
) -> CompletedDownloadLookup {
    let (client_ids, client_types) = completed_download_client_scope(items, true);
    load_recent_completed_download_lookup_for_client_scope_or_default_excluding_client_types(
        app,
        limit,
        &client_ids,
        &client_types,
        excluded_client_types,
    )
    .await
}

async fn load_recent_completed_download_lookup_for_client_scope_or_default_excluding_client_types(
    app: &AppUseCase,
    limit: usize,
    client_ids: &[String],
    client_types: &[String],
    excluded_client_types: &[&str],
) -> CompletedDownloadLookup {
    if client_ids.is_empty() && client_types.is_empty() {
        return CompletedDownloadLookup::empty_recent();
    }

    match app
        .services
        .integrations
        .download_client
        .list_recent_completed_downloads_for_client_scope(
            limit,
            client_ids,
            client_types,
            excluded_client_types,
        )
        .await
    {
        Ok(completed_downloads) => {
            index_completed_downloads(completed_downloads, CompletedDownloadLookupCoverage::Recent)
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
    load_completed_download_lookup_for_items_excluding_client_types(app, items, limit, &[]).await
}

pub(crate) async fn load_completed_download_lookup_for_items_excluding_client_types(
    app: &AppUseCase,
    items: &[DownloadQueueItem],
    limit: usize,
    excluded_client_types: &[&str],
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
        )
        .await,
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

fn index_completed_download_into(
    lookup: &mut CompletedDownloadLookup,
    completed: CompletedDownload,
) {
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

pub(super) fn queue_item_source_identity(item: &DownloadQueueItem) -> DownloadSourceIdentity {
    DownloadSourceIdentity::new(
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
) -> DownloadSourceIdentity {
    DownloadSourceIdentity::new(
        Some(completed.client_id.as_str()),
        completed.client_type.as_str(),
        completed.download_client_item_id.as_str(),
    )
}

pub(super) async fn download_id_tracked_state(
    app: &AppUseCase,
    identity: &crate::DownloadSubmissionIdentity,
    source_identity: Option<&DownloadSourceIdentity>,
) -> Option<TrackedDownloadState> {
    if crate::download_submission_identity_is_empty(identity) {
        return None;
    }
    app.services
        .workflow
        .download_submissions
        .get_identity_tracked_state(identity, source_identity)
        .await
        .ok()
        .flatten()
        .and_then(|state| TrackedDownloadState::from_str_opt(&state))
}

pub(super) fn apply_download_id_state(td: &mut TrackedDownload, state: TrackedDownloadState) {
    td.state = state;
    td.waiting_for_completed_history = false;
    match state {
        TrackedDownloadState::Imported => {
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
        TitleMatchType::Unmatched | TitleMatchType::IdOnly
    ) {
        return;
    }

    clear_id_only_conflict(td);

    let Ok(matcher) = app.monitored_title_matcher().await else {
        return;
    };

    let folder_name = Path::new(&completed.dest_dir)
        .file_name()
        .and_then(|value| value.to_str());
    let release_candidates = [Some(completed.name.as_str()), folder_name];

    for release_title in release_candidates.into_iter().flatten() {
        let release_title = release_title.trim();
        if release_title.is_empty() {
            continue;
        }

        let parsed = crate::parse_release_metadata(release_title);
        let resolved = if parsed.episode.is_some() {
            matcher.resolve_episode(
                &parsed,
                td.client_item
                    .facet
                    .as_deref()
                    .or(td.facet.as_deref())
                    .or(completed.category.as_deref()),
            )
        } else {
            matcher.resolve_movie(&parsed)
        };

        if let Some(resolved) = resolved {
            if td.match_type == TitleMatchType::IdOnly
                && let Some(existing_title_id) = td.title_id.as_deref()
                && existing_title_id != resolved.title.id
            {
                td.status = TrackedDownloadStatus::Warning;
                td.status_messages.retain(|message| {
                    !message.contains("matched by ID only")
                        && !message.contains(ID_ONLY_CONFLICT_MESSAGE)
                });
                td.warn(ID_ONLY_CONFLICT_MESSAGE);
                return;
            }

            td.title_id = Some(resolved.title.id.clone());
            td.facet = Some(resolved.title.facet.as_str().to_string());
            td.source_title = Some(release_title.to_string());
            if td.match_type != TitleMatchType::IdOnly {
                td.match_type = resolved.match_type;
            }
            return;
        }
    }
}

fn clear_id_only_conflict(td: &mut TrackedDownload) {
    td.status_messages
        .retain(|message| message != ID_ONLY_CONFLICT_MESSAGE);
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
