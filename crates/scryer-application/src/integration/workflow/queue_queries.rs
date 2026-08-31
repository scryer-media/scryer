const DOWNLOAD_QUEUE_RECENT_ACTIVITY_LIMIT: usize = 300;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackedDownloadBackgroundWorkKind {
    Import,
    Failed,
}
impl TrackedDownloadBackgroundWorkKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Failed => "failed",
        }
    }
}
#[derive(Debug)]
struct TrackedDownloadBackgroundWorkResult {
    id: String,
    kind: TrackedDownloadBackgroundWorkKind,
    outcome: Result<TrackedDownload, String>,
    elapsed: Duration,
}
#[derive(Clone, Debug)]
pub(crate) enum ManualImportSourceResolution {
    Eligible {
        completed: Option<Box<CompletedDownload>>,
    },
    NotEligible {
        message: String,
    },
}
fn is_post_processing_reason(reason: Option<&str>) -> bool {
    let Some(reason) = reason else {
        return false;
    };
    let normalized = reason.trim().to_ascii_uppercase();
    normalized.contains("PP_QUEUED")
        || normalized.contains("POSTPROCESSING")
        || normalized.contains("UNPACKING")
        || normalized.contains("REPAIRING")
        || normalized.contains("VERIFYING")
        || normalized.contains("RENAMING")
        || normalized.contains("MOVING")
        || normalized.contains("EXECUTING_SCRIPT")
}
fn normalize_routing_categories(categories: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for category in categories {
        let category = category.trim().to_string();
        if category.is_empty() || !seen.insert(category.clone()) {
            continue;
        }
        normalized.push(category);
    }
    normalized
}
fn compare_case_insensitive(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
}
fn download_history_title(item: &DownloadQueueItem) -> &str {
    let title = item.title_name.trim();
    if title.is_empty() {
        item.download_client_item_id.as_str()
    } else {
        title
    }
}
fn download_history_client_label(item: &DownloadQueueItem) -> &str {
    let client_name = item.client_name.trim();
    if client_name.is_empty() {
        item.client_type.as_str()
    } else {
        client_name
    }
}
fn compare_download_history_items(
    left: &DownloadQueueItem,
    right: &DownloadQueueItem,
    sort: DownloadHistorySort,
) -> std::cmp::Ordering {
    let ordering = match sort.key {
        DownloadHistorySortKey::Title => {
            compare_case_insensitive(download_history_title(left), download_history_title(right))
        }
        DownloadHistorySortKey::Client => compare_case_insensitive(
            download_history_client_label(left),
            download_history_client_label(right),
        )
        .then_with(|| compare_case_insensitive(&left.client_type, &right.client_type)),
        DownloadHistorySortKey::Status => {
            download_history_status_rank(left).cmp(&download_history_status_rank(right))
        }
        DownloadHistorySortKey::Progress => left.progress_percent.cmp(&right.progress_percent),
        DownloadHistorySortKey::Size => left
            .size_bytes
            .unwrap_or(0)
            .cmp(&right.size_bytes.unwrap_or(0)),
    };

    let ordering = match sort.direction {
        SortDirection::Asc => ordering,
        SortDirection::Desc => ordering.reverse(),
    };

    ordering
        .then_with(|| {
            compare_case_insensitive(
                &download_queue_client_filter_key(left),
                &download_queue_client_filter_key(right),
            )
        })
        .then_with(|| {
            compare_case_insensitive(
                &left.download_client_item_id,
                &right.download_client_item_id,
            )
        })
}
fn sort_download_history_items(items: &mut [DownloadQueueItem], sort: DownloadHistorySort) {
    items.sort_by(|left, right| compare_download_history_items(left, right, sort));
}
fn download_queue_client_filter_key(item: &DownloadQueueItem) -> String {
    let client_id = item.client_id.trim();
    if !client_id.is_empty() {
        return client_id.to_string();
    }

    let client_type = item.client_type.trim();
    if !client_type.is_empty() {
        return client_type.to_ascii_lowercase();
    }

    item.id.clone()
}
fn collect_download_client_filter_options(
    items: &[DownloadQueueItem],
) -> Vec<DownloadClientFilterOption> {
    let mut seen = HashSet::new();
    let mut clients = Vec::new();

    for item in items {
        let client_id = download_queue_client_filter_key(item);
        if !seen.insert(client_id.clone()) {
            continue;
        }

        let client_name = item.client_name.trim();
        let client_type = item.client_type.trim();
        clients.push(DownloadClientFilterOption {
            client_id,
            client_name: if client_name.is_empty() {
                client_type.to_string()
            } else {
                client_name.to_string()
            },
            client_type: client_type.to_string(),
        });
    }

    clients.sort_by(|left, right| {
        left.client_name
            .to_ascii_lowercase()
            .cmp(&right.client_name.to_ascii_lowercase())
            .then_with(|| {
                left.client_type
                    .to_ascii_lowercase()
                    .cmp(&right.client_type.to_ascii_lowercase())
            })
            .then_with(|| left.client_id.cmp(&right.client_id))
    });
    clients
}
fn unique_download_client_config_for_type<'a>(
    configs: &'a [DownloadClientConfig],
    client_type: &str,
) -> Option<&'a DownloadClientConfig> {
    let normalized_client_type = client_type.trim().to_ascii_lowercase();
    if normalized_client_type.is_empty() {
        return None;
    }

    let mut matches = configs.iter().filter(|config| {
        config
            .client_type
            .trim()
            .eq_ignore_ascii_case(&normalized_client_type)
    });
    let matched = matches.next()?;
    matches.next().is_none().then_some(matched)
}
fn canonical_download_client_config_for_item<'a>(
    item: &DownloadQueueItem,
    configs: &'a [DownloadClientConfig],
) -> Option<&'a DownloadClientConfig> {
    let client_id = item.client_id.trim();
    if !client_id.is_empty()
        && let Some(config) = configs.iter().find(|config| config.id == client_id)
    {
        return Some(config);
    }

    let client_type = item.client_type.trim();
    if client_type.is_empty() {
        return None;
    }

    let has_type_fallback_id = client_id.is_empty() || client_id.eq_ignore_ascii_case(client_type);
    has_type_fallback_id
        .then(|| unique_download_client_config_for_type(configs, client_type))
        .flatten()
}
fn canonicalize_download_queue_item_client(
    item: &mut DownloadQueueItem,
    configs: &[DownloadClientConfig],
) {
    let Some(config) = canonical_download_client_config_for_item(item, configs) else {
        return;
    };

    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = config.client_type.clone();
}
fn canonicalize_download_queue_item_clients(
    items: &mut [DownloadQueueItem],
    configs: &[DownloadClientConfig],
) {
    for item in items {
        canonicalize_download_queue_item_client(item, configs);
    }
}
fn matches_download_history_client_ids(
    item: &DownloadQueueItem,
    client_ids: Option<&HashSet<String>>,
) -> bool {
    match client_ids {
        None => true,
        Some(ids) if ids.is_empty() => false,
        Some(ids) => ids.contains(&download_queue_client_filter_key(item)),
    }
}
pub(crate) fn extract_url_origin(raw: &str) -> Option<String> {
    let trimmed = raw.trim().strip_prefix("nzb_url|").unwrap_or(raw.trim());
    let (_, remainder) = trimmed.split_once("://")?;
    let authority = remainder.split(['/', '?', '#']).next()?.trim();
    let host = authority.rsplit('@').next()?.trim();
    if host.is_empty() {
        return None;
    }

    if let Some(bracketed) = host.strip_prefix('[') {
        return bracketed
            .split_once(']')
            .map(|(address, _)| address.to_string())
            .filter(|address| !address.is_empty());
    }

    host.split(':')
        .next()
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_string)
}

pub(crate) fn source_provider_label(
    provider_name: Option<&str>,
    source_hint: Option<&str>,
) -> Option<String> {
    provider_name
        .and_then(safe_source_provider_name)
        .or_else(|| source_hint.and_then(extract_url_origin))
        .or_else(|| source_hint.and_then(safe_source_provider_name))
}

fn safe_source_provider_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty() && !trimmed.contains([':', '/', '\\', '?', '#', '@', '|', '=']))
        .then(|| trimmed.to_string())
}
fn apply_import_record_overlay_to_queue_item(item: &mut DownloadQueueItem, record: &ImportRecord) {
    item.import_status = Some(record.status);
    item.import_transfer_phase = record.import_transfer_phase;
    item.import_transfer_bytes = record.import_transfer_bytes;
    item.import_transfer_total_bytes = record.import_transfer_total_bytes;
    item.import_transfer_started_at = record.import_transfer_started_at.clone();
    item.import_transfer_updated_at = record.import_transfer_updated_at.clone();
    item.imported_at = record
        .finished_at
        .clone()
        .or(Some(record.updated_at.clone()));
}

pub(crate) fn import_record_error_overlay(
    record: &ImportRecord,
) -> (Option<scryer_domain::ImportErrorCode>, Option<String>) {
    if record.import_type == ImportType::ManualImport {
        return record
            .result_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<crate::ManualImportExecutionResult>(json).ok())
            .map_or((None, None), |result| {
                (result.error_code, result.error_message)
            });
    }

    let error_message = record
        .result_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<scryer_domain::ImportResult>(json).ok())
        .and_then(|result| result.error_message);
    (None, error_message)
}

fn apply_import_record_to_queue_item(item: &mut DownloadQueueItem, record: &ImportRecord) {
    apply_import_record_overlay_to_queue_item(item, record);
    let (error_code, error_message) = import_record_error_overlay(record);
    item.import_error_code = error_code;
    item.import_error_message = error_message.clone();
    if let Some(error_message) = error_message {
        item.attention_reason = Some(error_message);
    }
}
fn apply_delete_command_to_queue_item(
    item: &mut DownloadQueueItem,
    command: &crate::DownloadQueueCommandRecord,
) {
    item.delete_status = Some(command.status);
    item.delete_error_message = command.error_text.clone();
    if let Some(error_text) = command.error_text.as_ref() {
        item.attention_reason = Some(error_text.clone());
    }
}
fn queue_item_import_state_eligible(item: &DownloadQueueItem) -> bool {
    matches!(
        item.state,
        DownloadQueueState::Completed | DownloadQueueState::ImportPending
    )
}
fn download_queue_identity_key(
    client_id: Option<&str>,
    client_type: &str,
    download_client_item_id: &str,
) -> (String, String, String) {
    (
        normalized_download_client_id(client_id),
        client_type.to_string(),
        download_client_item_id.to_string(),
    )
}
fn download_queue_item_source_identity(item: &DownloadQueueItem) -> ClientJobLocator {
    ClientJobLocator::new(
        Some(item.client_id.as_str()).filter(|value| !value.trim().is_empty()),
        &item.client_type,
        &item.download_client_item_id,
    )
}
fn push_source_identity_candidate(
    identities: &mut Vec<ClientJobLocator>,
    seen: &mut HashSet<(String, String, String)>,
    identity: ClientJobLocator,
) {
    if identity.client_type.is_empty() || identity.item_id.is_empty() {
        return;
    }

    let key = download_queue_identity_key(
        identity.client_id.as_deref(),
        &identity.client_type,
        &identity.item_id,
    );
    if seen.insert(key) {
        identities.push(identity);
    }
}
fn push_submission_lookup_key(
    keys: &mut Vec<(String, String, String)>,
    seen: &mut HashSet<(String, String, String)>,
    identity: &ClientJobLocator,
) {
    if identity.client_type.is_empty() || identity.item_id.is_empty() {
        return;
    }

    let key = download_queue_identity_key(
        identity.client_id.as_deref(),
        &identity.client_type,
        &identity.item_id,
    );
    if seen.insert(key.clone()) {
        keys.push(key);
    }
}
fn apply_submission_to_queue_item(item: &mut DownloadQueueItem, submission: &DownloadSubmission) {
    if crate::import_parameters::submission_has_scryer_origin(submission) {
        item.is_scryer_origin = true;
    }
    // The submission remembers exactly which configured client the grab was
    // routed to. A client-observed row often carries only a provider-type
    // identity ("qbittorrent" as both id and type), and the unique-type
    // canonicalization above it cannot pick a config when two clients share the
    // type — so the row rendered the provider type where the operator expects
    // the configured name. Restore the real id here and let the canonicalize
    // pass that follows enrichment resolve it to the configured name.
    let item_client_id = item.client_id.trim();
    let item_has_type_fallback_id =
        item_client_id.is_empty() || item_client_id.eq_ignore_ascii_case(item.client_type.trim());
    if item_has_type_fallback_id
        && let Some(submission_client_id) = submission
            .download_client_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        item.client_id = submission_client_id.to_string();
        if item.client_type.trim().is_empty() {
            item.client_type = submission.download_client_type.clone();
        }
        // The name belongs to the config, not the submission; clear a
        // type-derived stand-in so canonicalization fills the configured name.
        if item.client_name.trim().eq_ignore_ascii_case(item.client_type.trim()) {
            item.client_name = String::new();
        }
    }
    if let Some(source_title) = submission
        .source_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        item.title_name = source_title.to_string();
    }
    if item.source_provider.is_none() {
        item.source_provider = source_provider_label(
            submission.source_provider_name.as_deref(),
            submission.source_hint.as_deref(),
        );
    }
    if item.title_id.is_none() && !submission.title_id.trim().is_empty() {
        item.title_id = Some(submission.title_id.clone());
    }
    if item.episode_id.is_none() {
        item.episode_id = submission.scope.episode_id().map(ToString::to_string);
    }
    if item.facet.is_none() && !submission.facet.trim().is_empty() {
        item.facet = Some(submission.facet.clone());
    }
}
pub async fn enrich_download_queue_items_from_submissions(
    app: &AppUseCase,
    items: &mut [DownloadQueueItem],
) {
    enrich_download_queue_items_from_submissions_with_original_identities(app, items, None).await;
}
async fn enrich_download_queue_items_from_submissions_with_original_identities(
    app: &AppUseCase,
    items: &mut [DownloadQueueItem],
    original_source_identities: Option<&[ClientJobLocator]>,
) {
    let mut client_items = Vec::new();
    let mut seen_client_items = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let current = download_queue_item_source_identity(item);
        push_source_identity_candidate(&mut client_items, &mut seen_client_items, current.clone());
        if current.client_id.is_none() {
            push_source_identity_candidate(
                &mut client_items,
                &mut seen_client_items,
                ClientJobLocator::new(None, &current.client_type, &current.item_id),
            );
        }

        if let Some(original) =
            original_source_identities.and_then(|identities| identities.get(index))
        {
            push_source_identity_candidate(
                &mut client_items,
                &mut seen_client_items,
                original.clone(),
            );
            if original.client_id.is_none() {
                push_source_identity_candidate(
                    &mut client_items,
                    &mut seen_client_items,
                    ClientJobLocator::new(None, &original.client_type, &original.item_id),
                );
            }
        }
    }

    let submission_map = if client_items.is_empty() {
        HashMap::new()
    } else {
        let submissions = match app
            .services
            .workflow
            .download_submissions
            .list_for_client_items(&client_items)
            .await
        {
            Ok(submissions) => submissions,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to batch-load download submissions for queue enrichment"
                );
                Vec::new()
            }
        };

        submissions
            .into_iter()
            .filter(|submission| !submission.title_id.trim().is_empty())
            .map(|submission| {
                (
                    download_queue_identity_key(
                        submission.download_client_id.as_deref(),
                        &submission.download_client_type,
                        &submission.download_client_item_id,
                    ),
                    submission,
                )
            })
            .collect::<HashMap<_, _>>()
    };

    // Seeding goals are frozen on the submission row at grab time; the queue
    // needs them beside the observed ratio/seed time so a row can show progress
    // against what it was actually grabbed under.
    let seed_goal_map = if client_items.is_empty() {
        HashMap::new()
    } else {
        match app
            .services
            .workflow
            .download_submissions
            .list_seed_goals_for_client_items(&client_items)
            .await
        {
            Ok(goals) => goals
                .into_iter()
                .map(|(identity, goals)| {
                    (
                        download_queue_identity_key(
                            identity.client_id.as_deref(),
                            &identity.client_type,
                            &identity.item_id,
                        ),
                        goals,
                    )
                })
                .collect::<HashMap<_, _>>(),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to batch-load seeding goals for queue enrichment"
                );
                HashMap::new()
            }
        }
    };

    let identity_tracked_state_map = if client_items.is_empty() {
        HashMap::new()
    } else {
        match app
            .services
            .workflow
            .download_submissions
            .list_identity_tracked_states_for_client_items(&client_items)
            .await
        {
            Ok(states) => states
                .into_iter()
                .map(|(identity, state)| {
                    (
                        download_queue_identity_key(
                            identity.client_id.as_deref(),
                            &identity.client_type,
                            &identity.item_id,
                        ),
                        state,
                    )
                })
                .collect::<HashMap<_, _>>(),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to batch-load durable tracked states for queue enrichment"
                );
                HashMap::new()
            }
        }
    };

    for (index, item) in items.iter_mut().enumerate() {
        let current = download_queue_item_source_identity(item);
        let original = original_source_identities.and_then(|identities| identities.get(index));
        let mut lookup_keys = Vec::new();
        let mut seen_lookup_keys = HashSet::new();
        push_submission_lookup_key(&mut lookup_keys, &mut seen_lookup_keys, &current);
        if let Some(original) = original {
            push_submission_lookup_key(&mut lookup_keys, &mut seen_lookup_keys, original);
        }
        if current.client_id.is_none() {
            push_submission_lookup_key(
                &mut lookup_keys,
                &mut seen_lookup_keys,
                &ClientJobLocator::new(None, &current.client_type, &current.item_id),
            );
        }
        if let Some(original) = original
            && original.client_id.is_none()
        {
            push_submission_lookup_key(
                &mut lookup_keys,
                &mut seen_lookup_keys,
                &ClientJobLocator::new(None, &original.client_type, &original.item_id),
            );
        }

        // Durable identity rows are matched by client triple, which can be
        // shared across grabs (a re-added torrent reuses its hash as the item
        // id). Only the latest row's `ignored` marker may be stamped, and only
        // onto history-state items, so a live re-grab of a previously ignored
        // identity is never hidden and runtime state is otherwise
        // authoritative.
        if is_history_download_state(&item.state)
            && lookup_keys
                .iter()
                .find_map(|key| identity_tracked_state_map.get(key))
                .and_then(|state| TrackedDownloadState::from_str_opt(state))
                == Some(TrackedDownloadState::Ignored)
        {
            item.tracked_state = Some(TrackedDownloadState::Ignored);
        }

        if let Some(goals) = lookup_keys.iter().find_map(|key| seed_goal_map.get(key)) {
            apply_seed_goals_to_queue_item(item, goals);
        }

        if let Some(submission) = lookup_keys
            .iter()
            .find_map(|key| submission_map.get(key))
            .cloned()
        {
            apply_submission_to_queue_item(item, &submission);
            continue;
        }

        if let Some(submission) =
            find_submission_for_queue_item_by_download_id(app, item, original).await
        {
            apply_submission_to_queue_item(item, &submission);
        }
    }
}
/// Join the goals a torrent was grabbed under onto its queue row.
///
/// These are policy, not client state, so they live beside the observation
/// rather than inside it: the queue shows "ratio 1.4 of 2.0" only because both
/// halves reach the same row. A row with goals but no client observation still
/// gets a snapshot, so a held torrent whose client says nothing still shows
/// what it is waiting for.
fn apply_seed_goals_to_queue_item(item: &mut DownloadQueueItem, goals: &crate::PersistedSeedGoals) {
    let snapshot = item.seeding.get_or_insert_with(Default::default);
    snapshot.seed_goal_ratio = goals.seed_goal_ratio;
    snapshot.seed_goal_seconds = goals.seed_goal_seconds;
    snapshot.never_remove = goals.never_remove;
}
async fn find_submission_for_queue_item_by_download_id(
    app: &AppUseCase,
    item: &DownloadQueueItem,
    original: Option<&ClientJobLocator>,
) -> Option<DownloadSubmission> {
    let download_id = item.download_id.as_deref().map(str::trim)?;
    if download_id.is_empty() {
        return None;
    }

    let canonical_download_id = match crate::download_identity::resolve_observed_client_job(
        app,
        crate::download_identity::observed_queue_item_job(item),
    )
    .await
    {
        crate::download_identity::ObservedClientJobResolution::Resolved(download_id) => {
            Some(download_id)
        }
        crate::download_identity::ObservedClientJobResolution::Conflict => return None,
        crate::download_identity::ObservedClientJobResolution::Unavailable => None,
    };

    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let mut push_candidate = |client_id: Option<&str>, client_type: &str| {
        let client_type = client_type.trim();
        if client_type.is_empty() {
            return;
        }
        let key = (
            normalized_download_client_id(client_id),
            client_type.to_string(),
        );
        if seen.insert(key.clone()) {
            candidates.push(key);
        }
    };

    push_candidate(
        Some(item.client_id.as_str()).filter(|value| !value.trim().is_empty()),
        &item.client_type,
    );
    if let Some(original) = original {
        push_candidate(original.client_id.as_deref(), &original.client_type);
        if original.client_id.is_none() {
            push_candidate(None, &original.client_type);
        }
    }
    if item.client_id.trim().is_empty() {
        push_candidate(None, &item.client_type);
    }

    for (client_id, client_type) in candidates {
        match app
            .services
            .workflow
            .download_submissions
            .list_by_download_id_for_download(
                canonical_download_id.as_ref(),
                Some(client_id.as_str()).filter(|value| !value.trim().is_empty()),
                &client_type,
                download_id,
            )
            .await
        {
            Ok(submissions) => {
                if let Some(submission) = submissions
                    .into_iter()
                    .find(|submission| !submission.title_id.trim().is_empty())
                {
                    return Some(submission);
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    client_id = %client_id,
                    client_type = %client_type,
                    "failed to load download submission by download id for queue enrichment"
                );
            }
        }
    }

    None
}
fn config_value_is_empty(value: Option<&serde_json::Value>) -> bool {
    match value {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(value)) => value.trim().is_empty(),
        _ => false,
    }
}
fn config_value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.trim().to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
    .filter(|value| !value.is_empty())
}
fn synthetic_tracked_snapshot_queue_item(
    tracked: &TrackedDownloadQueueMetadata,
    primary_client: Option<&DownloadClientConfig>,
) -> Option<DownloadQueueItem> {
    // Visibility allowlist for tracked rows that have no live client row of
    // their own. `ImportedSeeding` MUST be here: a state that is held back for
    // seeding but hidden from the queue is a download that silently
    // disappeared as far as the operator is concerned.
    match tracked.state {
        TrackedDownloadState::Imported
        | TrackedDownloadState::ImportedSeeding
        | TrackedDownloadState::Failed
        | TrackedDownloadState::ImportPending
        | TrackedDownloadState::Importing
        | TrackedDownloadState::ImportBlocked => {}
        _ => return None,
    }

    let mut item = tracked.client_item.clone();
    apply_tracked_download_activity_projection(&mut item, tracked);

    if item.client_id.trim().is_empty() && !tracked.client_id.trim().is_empty() {
        item.client_id = tracked.client_id.clone();
    }
    if item.client_type.trim().is_empty() && !tracked.client_type.trim().is_empty() {
        item.client_type = tracked.client_type.clone();
    }

    if let Some(primary_client) = primary_client {
        if item.client_id.trim().is_empty() {
            item.client_id = primary_client.id.clone();
        }
        if item.client_name.trim().is_empty() {
            item.client_name = primary_client.name.clone();
        }
        if item.client_type.trim().is_empty() {
            item.client_type = primary_client.client_type.clone();
        }
    }

    Some(item)
}
impl AppUseCase {
    async fn enrich_download_queue_items(
        &self,
        enabled_clients: &[DownloadClientConfig],
        mut items: Vec<DownloadQueueItem>,
        use_tracked_runtime_snapshot: bool,
    ) -> Vec<DownloadQueueItem> {
        let original_source_identities = items
            .iter()
            .map(download_queue_item_source_identity)
            .collect::<Vec<_>>();
        canonicalize_download_queue_item_clients(&mut items, enabled_clients);
        enrich_download_queue_items_from_submissions_with_original_identities(
            self,
            &mut items,
            Some(&original_source_identities),
        )
        .await;
        let primary_client = enabled_clients.first();

        if use_tracked_runtime_snapshot {
            match tokio::time::timeout(
                TRACKED_DOWNLOAD_SNAPSHOT_READ_BUDGET,
                self.runtime.acquisition.tracked_download_snapshot.read(),
            )
            .await
            {
                Ok(snapshot) => {
                    let existing_ids = items
                        .iter()
                        .map(tracked_download_id_for_item)
                        .collect::<HashSet<_>>();
                    for item in &mut items {
                        let tracked_id = tracked_download_id_for_item(item);
                        if let Some(metadata) = snapshot.get(&tracked_id) {
                            apply_tracked_download_queue_metadata(item, metadata);
                        }
                    }
                    items.extend(snapshot.iter().filter_map(|(tracked_id, metadata)| {
                        if existing_ids.contains(tracked_id) {
                            return None;
                        }
                        synthetic_tracked_snapshot_queue_item(metadata, primary_client).map(
                            |mut item| {
                                if item.download_client_item_id.trim().is_empty() {
                                    item.download_client_item_id = tracked_id.to_string();
                                }
                                apply_tracked_download_queue_metadata(&mut item, metadata);
                                item
                            },
                        )
                    }));
                }
                Err(_) => {
                    tracing::warn!(
                        budget_ms = TRACKED_DOWNLOAD_SNAPSHOT_READ_BUDGET.as_millis() as u64,
                        item_count = items.len(),
                        "download queue enrichment timed out reading tracked snapshot; returning degraded client/persisted state"
                    );
                }
            }
        }

        canonicalize_download_queue_item_clients(&mut items, enabled_clients);
        enrich_download_queue_items_from_submissions(self, &mut items).await;
        // Submission enrichment can have just restored a row's real configured
        // client id (a client-observed row only knows its provider type, and
        // with two same-type clients the unique-type pass above cannot pick
        // one). Canonicalize once more so that id resolves to the configured
        // name before anything renders it.
        canonicalize_download_queue_item_clients(&mut items, enabled_clients);

        let mut items = dedupe_download_queue_items(items)
            .into_iter()
            .map(|item| {
                let mut mapped = item;
                if let Some(primary_client) = primary_client {
                    if mapped.client_id.is_empty() {
                        mapped.client_id = primary_client.id.clone();
                    }
                    if mapped.client_name.is_empty() {
                        mapped.client_name = primary_client.name.clone();
                    }
                    if mapped.client_type.is_empty() {
                        mapped.client_type = primary_client.client_type.clone();
                    }
                }
                mapped
            })
            .collect::<Vec<_>>();

        enrich_queue_item_import_states(self, &mut items).await;

        items
            .into_iter()
            .map(|item| annotate_download_queue_item(item, primary_client))
            .collect()
    }
}
impl AppUseCase {
    async fn filter_ineligible_download_queue_items(
        &self,
        items: Vec<DownloadQueueItem>,
    ) -> Vec<DownloadQueueItem> {
        let classifications = match tokio::time::timeout(
            TRACKED_DOWNLOAD_SNAPSHOT_READ_BUDGET,
            self.runtime.acquisition.tracked_download_snapshot.read(),
        )
        .await
        {
            Ok(snapshot) => snapshot
                .iter()
                .filter_map(|(tracked_id, metadata)| {
                    metadata.import_hold.map(|hold| (tracked_id.clone(), hold))
                })
                .collect::<HashMap<_, _>>(),
            Err(_) => HashMap::new(),
        };

        let category_admission = self.download_client_category_admission_snapshot().await;
        items
            .into_iter()
            .filter(|item| {
                let tracked_id = tracked_download_id_for_item(item);
                if matches!(
                    classifications.get(&tracked_id),
                    Some(
                        crate::tracked_downloads::ImportHold::NoImportableVideo
                            | crate::tracked_downloads::ImportHold::ExternalManager
                    )
                ) {
                    return false;
                }
                crate::services::download_observation_is_admitted(
                    item.is_scryer_origin,
                    item.category.as_deref(),
                    category_admission.as_deref(),
                )
            })
            .collect()
    }
}
impl AppUseCase {
    pub(crate) async fn collect_download_snapshot_items_excluding_client_types(
        &self,
        include_queue: bool,
        include_recent_history: bool,
        use_tracked_runtime_snapshot: bool,
        excluded_client_types: &[&str],
    ) -> AppResult<crate::ports::DownloadClientSnapshotOutcome> {
        let enabled_clients = self.enabled_download_clients_by_priority().await?;
        if enabled_clients.is_empty() {
            return Ok(crate::ports::DownloadClientSnapshotOutcome {
                any_client_read_succeeded: true,
                ..Default::default()
            });
        }

        let snapshot = if include_queue && include_recent_history {
            self.services
                .integrations
                .download_client
                .list_snapshot_outcome_excluding_client_types(
                    DOWNLOAD_QUEUE_RECENT_ACTIVITY_LIMIT,
                    excluded_client_types,
                )
                .await?
        } else {
            let queue_items = if include_queue {
                self.services
                    .integrations
                    .download_client
                    .list_queue_excluding_client_types(excluded_client_types)
                    .await?
            } else {
                Vec::new()
            };
            let history_items = if include_recent_history {
                self.services
                    .integrations
                    .download_client
                    .list_recent_activity_excluding_client_types(
                        DOWNLOAD_QUEUE_RECENT_ACTIVITY_LIMIT,
                        excluded_client_types,
                    )
                    .await?
            } else {
                Vec::new()
            };
            let mut items = queue_items;
            items.extend(history_items);
            crate::ports::DownloadClientSnapshotOutcome {
                items,
                any_client_read_succeeded: true,
                ..Default::default()
            }
        };

        let items = self
            .enrich_download_queue_items(
                &enabled_clients,
                snapshot.items,
                use_tracked_runtime_snapshot,
            )
            .await;
        Ok(crate::ports::DownloadClientSnapshotOutcome {
            items: self.filter_ineligible_download_queue_items(items).await,
            authoritative_client_ids: snapshot.authoritative_client_ids,
            any_client_read_succeeded: snapshot.any_client_read_succeeded,
        })
    }
}
impl AppUseCase {
    async fn current_download_queue_read_model(
        &self,
    ) -> AppResult<(
        crate::services::DownloadQueueSnapshot,
        std::sync::Arc<crate::services::DownloadQueueReadModel>,
    )> {
        let cache = &self.runtime.acquisition.download_queue_read_model;
        let mut snapshot = self
            .runtime
            .acquisition
            .download_queue_snapshot
            .snapshot()
            .await;
        if let Some(model) = cache
            .current
            .read()
            .await
            .as_ref()
            .filter(|model| model.revision == snapshot.revision)
            .cloned()
        {
            metrics::counter!("scryer_download_queue_read_model_total", "result" => "hit")
                .increment(1);
            return Ok((snapshot, model));
        }

        let _build_guard = cache.build_lock.lock().await;
        snapshot = self
            .runtime
            .acquisition
            .download_queue_snapshot
            .snapshot()
            .await;
        if let Some(model) = cache
            .current
            .read()
            .await
            .as_ref()
            .filter(|model| model.revision == snapshot.revision)
            .cloned()
        {
            metrics::counter!("scryer_download_queue_read_model_total", "result" => "hit")
                .increment(1);
            return Ok((snapshot, model));
        }

        metrics::counter!("scryer_download_queue_read_model_total", "result" => "miss")
            .increment(1);
        let title_ids = snapshot
            .items
            .iter()
            .filter_map(|item| item.title_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let title_library_ids = self
            .services
            .catalog
            .titles
            .get_by_ids(&title_ids)
            .await?
            .into_iter()
            .map(|title| (title.id, title.library_id))
            .collect::<HashMap<_, _>>();
        let model = std::sync::Arc::new(crate::services::DownloadQueueReadModel::new(
            snapshot.revision,
            std::sync::Arc::clone(&snapshot.items),
            title_library_ids,
        ));
        cache.current.write().await.replace(model.clone());
        Ok((snapshot, model))
    }

    async fn download_queue_ordering(
        model: &std::sync::Arc<crate::services::DownloadQueueReadModel>,
        sort: DownloadHistorySort,
    ) -> std::sync::Arc<[usize]> {
        if let Some(ordering) = model.orderings.read().await.get(&sort).cloned() {
            return ordering;
        }
        let mut orderings = model.orderings.write().await;
        if let Some(ordering) = orderings.get(&sort).cloned() {
            return ordering;
        }
        let mut ordering = (0..model.items.len()).collect::<Vec<_>>();
        ordering.sort_by(|left, right| {
            compare_download_history_items(&model.items[*left], &model.items[*right], sort)
        });
        let ordering = std::sync::Arc::from(ordering);
        orderings.insert(sort, std::sync::Arc::clone(&ordering));
        ordering
    }

    async fn legacy_download_queue_ordering(
        model: &std::sync::Arc<crate::services::DownloadQueueReadModel>,
    ) -> std::sync::Arc<[usize]> {
        std::sync::Arc::clone(
            model
                .legacy_ordering
                .get_or_init(|| async {
                    let mut ordering = (0..model.items.len()).collect::<Vec<_>>();
                    ordering.sort_by(|left, right| {
                        compare_download_queue_items(&model.items[*left], &model.items[*right])
                    });
                    std::sync::Arc::from(ordering)
                })
                .await,
        )
    }

    pub async fn list_download_queue(
        &self,
        actor: &User,
        include_all_activity: bool,
        include_history_only: bool,
        include_import_activity: bool,
        activity_filter: DownloadActivityFilter,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.require_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?;
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let can_view_operational_history = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (_, model) = self.current_download_queue_read_model().await?;
        let ordering = Self::legacy_download_queue_ordering(&model).await;
        Ok(ordering
            .iter()
            .map(|index| &model.items[*index])
            .filter(|item| {
                item.title_id
                    .as_deref()
                    .map_or(can_view_operational_history, |title_id| {
                        model
                            .title_library_ids
                            .get(title_id)
                            .map_or(can_view_operational_history, |library_id| {
                                allowed_library_ids.contains(library_id)
                            })
                    })
            })
            .filter(|item| include_all_activity || item.is_scryer_origin)
            .filter(|item| {
                matches_download_queue_filter(
                    item,
                    include_history_only,
                    include_import_activity,
                    activity_filter,
                )
            })
            .take(if include_history_only { 50 } else { usize::MAX })
            .cloned()
            .collect())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "paged queue API keeps its filter and sort contract explicit"
    )]
    pub async fn list_download_queue_page(
        &self,
        actor: &User,
        limit: usize,
        offset: usize,
        filters: Option<Vec<DownloadActivityFilter>>,
        client_ids: Option<Vec<String>>,
        scryer_submitted_only: bool,
        title_id: Option<&str>,
        sort: DownloadHistorySort,
    ) -> AppResult<DownloadQueuePage> {
        let started_at = Instant::now();
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }
        let can_view_operational_history = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let limit = limit.clamp(1, 200);
        let (snapshot, model) = self.current_download_queue_read_model().await?;
        let stale = snapshot.stale_at(Utc::now());
        metrics::counter!(
            "scryer_download_queue_cache_total",
            "result" => if snapshot.ready { "hit" } else { "miss" }
        )
        .increment(1);
        let normalized_client_ids = client_ids.map(|ids| {
            ids.into_iter()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect::<HashSet<_>>()
        });
        let ordering = Self::download_queue_ordering(&model, sort).await;
        let mut available_clients_by_id = HashMap::new();
        let mut page_items = Vec::with_capacity(limit);
        let mut total_count = 0usize;
        for index in ordering.iter().copied() {
            let item = &model.items[index];
            let authorized =
                item.title_id
                    .as_deref()
                    .map_or(can_view_operational_history, |title_id| {
                        model
                            .title_library_ids
                            .get(title_id)
                            .map_or(can_view_operational_history, |library_id| {
                                allowed_library_ids.contains(library_id)
                            })
                    });
            if !authorized
                || !matches_download_activity_filter(item, DownloadActivityFilter::All)
                || title_id.is_some_and(|title_id| item.title_id.as_deref() != Some(title_id))
                || (scryer_submitted_only && !item.is_scryer_origin)
                || filters.as_ref().is_some_and(|filters| {
                    !filters
                        .iter()
                        .any(|filter| matches_download_activity_filter(item, *filter))
                })
            {
                continue;
            }

            let client_id = download_queue_client_filter_key(item);
            available_clients_by_id
                .entry(client_id.clone())
                .or_insert_with(|| DownloadClientFilterOption {
                    client_id,
                    client_name: if item.client_name.trim().is_empty() {
                        item.client_type.trim().to_string()
                    } else {
                        item.client_name.trim().to_string()
                    },
                    client_type: item.client_type.trim().to_string(),
                });
            if !matches_download_history_client_ids(item, normalized_client_ids.as_ref()) {
                continue;
            }
            if total_count >= offset && page_items.len() < limit {
                page_items.push(item.clone());
            }
            total_count = total_count.saturating_add(1);
        }
        let mut available_clients = available_clients_by_id.into_values().collect::<Vec<_>>();
        available_clients.sort_by(|left, right| {
            compare_case_insensitive(&left.client_name, &right.client_name)
                .then_with(|| compare_case_insensitive(&left.client_type, &right.client_type))
                .then_with(|| left.client_id.cmp(&right.client_id))
        });
        let has_more = offset.saturating_add(page_items.len()) < total_count;
        metrics::histogram!("scryer_download_queue_page_duration_seconds")
            .record(started_at.elapsed().as_secs_f64());
        metrics::gauge!("scryer_download_queue_snapshot_age_seconds").set(
            snapshot
                .updated_at
                .map(|updated_at| {
                    Utc::now()
                        .signed_duration_since(updated_at)
                        .num_milliseconds()
                })
                .unwrap_or_default()
                .max(0) as f64
                / 1_000.0,
        );

        Ok(DownloadQueuePage {
            items: page_items,
            has_more,
            total_count,
            available_clients,
            revision: snapshot.revision,
            updated_at: snapshot.updated_at,
            ready: snapshot.ready,
            stale,
        })
    }
}
impl AppUseCase {
    async fn find_download_queue_item_raw(
        &self,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadQueueItem>> {
        let target_download_client_item_id = download_client_item_id.trim();
        if target_download_client_item_id.is_empty() {
            return Err(AppError::Validation(
                "download client item id is required".to_string(),
            ));
        }

        let normalized_client_type = client_type
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        let normalized_client_id = client_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let snapshot = self
            .runtime
            .acquisition
            .download_queue_snapshot
            .snapshot()
            .await;
        Ok(snapshot
            .items
            .iter()
            .find(|item| {
                item.download_client_item_id == target_download_client_item_id
                    && normalized_client_id
                        .as_ref()
                        .is_none_or(|client_id| item.client_id == *client_id)
                    && normalized_client_type.as_ref().is_none_or(|client_type| {
                        item.client_type.eq_ignore_ascii_case(client_type)
                    })
            })
            .cloned())
    }
}
impl AppUseCase {
    pub async fn list_download_queue_for_title(
        &self,
        actor: &User,
        title_id: &str,
        include_all_activity: bool,
        include_history_only: bool,
        include_import_activity: bool,
        activity_filter: DownloadActivityFilter,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.require_title_library_permission(
            actor,
            title_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        let (_, model) = self.current_download_queue_read_model().await?;
        let ordering = Self::legacy_download_queue_ordering(&model).await;
        Ok(ordering
            .iter()
            .map(|index| &model.items[*index])
            .filter(|item| item.title_id.as_deref() == Some(title_id))
            .filter(|item| include_all_activity || item.is_scryer_origin)
            .filter(|item| {
                matches_download_queue_filter(
                    item,
                    include_history_only,
                    include_import_activity,
                    activity_filter,
                )
            })
            .take(if include_history_only { 50 } else { usize::MAX })
            .cloned()
            .collect())
    }
}
impl AppUseCase {
    pub async fn list_download_import_page(
        &self,
        actor: &User,
        limit: usize,
        offset: usize,
        filter: DownloadImportFilter,
    ) -> AppResult<DownloadImportPage> {
        self.require_any_library_permission(
            actor,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;

        let limit = limit.clamp(1, 100);
        let items = self
            .collect_download_history_items_for_actor(
                actor,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?
            .into_iter()
            // `is_scryer_origin` is not an ownership filter: hand-added
            // downloads remain eligible for manual assignment. The shared
            // collector has already excluded only runtime-classified or
            // non-owned-category sources.
            .filter(|item| matches_download_import_filter(item, filter))
            .collect::<Vec<_>>();

        let mut items = items;
        items.sort_by(|left, right| {
            let left_rank = match classify_download_queue_item(left).import_filter {
                Some(DownloadImportFilter::Importing) => 0,
                Some(DownloadImportFilter::Pending) => 1,
                Some(DownloadImportFilter::Blocked) => 2,
                Some(DownloadImportFilter::Failed) => 3,
                _ => 4,
            };
            let right_rank = match classify_download_queue_item(right).import_filter {
                Some(DownloadImportFilter::Importing) => 0,
                Some(DownloadImportFilter::Pending) => 1,
                Some(DownloadImportFilter::Blocked) => 2,
                Some(DownloadImportFilter::Failed) => 3,
                _ => 4,
            };
            left_rank.cmp(&right_rank).then_with(|| {
                parse_sort_value(
                    right.last_updated_at.as_deref(),
                    left.last_updated_at.as_deref(),
                )
            })
        });

        let total_count = items.len();
        let page_items = items
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let has_more = offset.saturating_add(page_items.len()) < total_count;

        Ok(DownloadImportPage {
            items: page_items,
            has_more,
            total_count,
        })
    }
}
impl AppUseCase {
    pub async fn count_download_import_items(
        &self,
        actor: &User,
        filter: DownloadImportFilter,
    ) -> AppResult<i64> {
        self.require_any_library_permission(
            actor,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;

        let count = self
            .collect_download_history_items_for_actor(
                actor,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?
            .into_iter()
            // Must mirror list_download_import_page exactly (see the note
            // there); if these two disagree the import badge count drifts from
            // the rows the tab actually shows.
            .filter(|item| matches_download_import_filter(item, filter))
            .count();

        Ok(count as i64)
    }
}
impl AppUseCase {
    #[expect(
        clippy::too_many_arguments,
        reason = "download-history queries mirror the user-visible filter surface explicitly"
    )]
    pub async fn list_download_history_page(
        &self,
        actor: &User,
        limit: usize,
        offset: usize,
        filters: Option<Vec<DownloadHistoryFilter>>,
        client_ids: Option<Vec<String>>,
        scryer_submitted_only: bool,
        sort: Option<DownloadHistorySort>,
    ) -> AppResult<DownloadHistoryPage> {
        self.require_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?;

        let limit = limit.clamp(1, 50);
        let normalized_client_ids = client_ids.map(|ids| {
            ids.into_iter()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect::<HashSet<_>>()
        });
        let mut items = self
            .collect_download_history_items_for_actor(actor, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .filter(|item| {
                matches!(
                    classify_download_queue_item(item).bucket,
                    DownloadQueueBucket::HistorySuccess | DownloadQueueBucket::HistoryFailed
                )
            })
            .collect::<Vec<_>>();
        items.retain(|item| matches_download_history_filters(item, filters.as_deref()));
        if scryer_submitted_only {
            items.retain(|item| item.is_scryer_origin);
        }
        let available_clients = collect_download_client_filter_options(&items);
        items.retain(|item| {
            matches_download_history_client_ids(item, normalized_client_ids.as_ref())
        });
        if let Some(sort) = sort {
            sort_download_history_items(&mut items, sort);
        } else {
            items.sort_by(|left, right| {
                parse_sort_value(
                    right.last_updated_at.as_deref(),
                    left.last_updated_at.as_deref(),
                )
            });
        }

        let total_count = items.len();
        let page_items = items
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let has_more = offset.saturating_add(page_items.len()) < total_count;

        Ok(DownloadHistoryPage {
            items: page_items,
            has_more,
            total_count,
            available_clients,
        })
    }
}
impl AppUseCase {
    pub async fn list_download_queue_snapshot(
        &self,
        actor: &User,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }
        let can_view_operational_history = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (_, model) = self.current_download_queue_read_model().await?;
        let ordering = Self::legacy_download_queue_ordering(&model).await;
        Ok(ordering
            .iter()
            .map(|index| &model.items[*index])
            .filter(|item| {
                item.title_id
                    .as_deref()
                    .map_or(can_view_operational_history, |title_id| {
                        model
                            .title_library_ids
                            .get(title_id)
                            .map_or(can_view_operational_history, |library_id| {
                                allowed_library_ids.contains(library_id)
                            })
                    })
            })
            .cloned()
            .collect())
    }
}
impl AppUseCase {
    pub async fn find_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadQueueItem>> {
        self.require_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?;
        let item = self
            .find_download_queue_item_raw(client_id, client_type, download_client_item_id)
            .await?;
        let Some(item) = item else {
            return Ok(None);
        };
        let visible = self
            .filter_download_queue_items_for_permission(
                actor,
                vec![item],
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        Ok(visible.into_iter().next())
    }

    pub async fn update_import_transfer_progress_and_notify(
        &self,
        import_id: &str,
        phase: scryer_domain::ImportTransferPhase,
        bytes: u64,
        total_bytes: u64,
    ) -> AppResult<()> {
        let bytes = i64::try_from(bytes).unwrap_or(i64::MAX);
        let total_bytes = i64::try_from(total_bytes).unwrap_or(i64::MAX);
        self.services
            .workflow
            .imports
            .update_import_transfer_progress(import_id, phase, bytes, total_bytes)
            .await?;
        self.refresh_import_record_queue_snapshot(import_id).await;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn find_download_queue_scope(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<SubmissionScope>> {
        self.require_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?;

        let submission = self
            .services
            .workflow
            .download_submissions
            .find_by_client_item_id(&ClientJobLocator::new(
                client_id,
                client_type,
                download_client_item_id,
            ))
            .await?;
        if let Some(submission) = submission.as_ref() {
            if matches!(submission.scope, SubmissionScope::Orphan) {
                return Ok(Some(SubmissionScope::Orphan));
            }
            let Some(title) = self
                .services
                .catalog
                .titles
                .get_by_id(&submission.title_id)
                .await?
            else {
                tracing::warn!(
                    title_id = %submission.title_id,
                    client_type,
                    download_client_item_id,
                    "download submission scope refers to a missing title; ignoring stale scope"
                );
                return Ok(None);
            };
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(submission.map(|submission| submission.scope))
    }
}
fn parse_sort_value(left: Option<&str>, right: Option<&str>) -> std::cmp::Ordering {
    fn parse(value: Option<&str>) -> i64 {
        value
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0)
    }

    parse(left).cmp(&parse(right))
}
fn dedupe_download_queue_items(items: Vec<DownloadQueueItem>) -> Vec<DownloadQueueItem> {
    let mut deduped: Vec<DownloadQueueItem> = Vec::with_capacity(items.len());
    let mut key_to_index: HashMap<String, usize> = HashMap::with_capacity(items.len());

    for item in items {
        let key = download_queue_item_key(&item);
        if let Some(index) = key_to_index.get(&key).copied() {
            merge_download_queue_item(&mut deduped[index], item);
            continue;
        }

        key_to_index.insert(key, deduped.len());
        deduped.push(item);
    }

    deduped
}
fn download_queue_item_key(item: &DownloadQueueItem) -> String {
    if item.client_type.is_empty() && item.download_client_item_id.is_empty() {
        return item.id.clone();
    }

    if !item.client_id.trim().is_empty() {
        return format!("{}:{}", item.client_id, item.download_client_item_id);
    }

    format!("{}:{}", item.client_type, item.download_client_item_id)
}
fn merge_download_queue_item(existing: &mut DownloadQueueItem, incoming: DownloadQueueItem) {
    if existing.title_id.is_none() {
        existing.title_id = incoming.title_id.clone();
    }
    if existing.episode_id.is_none() {
        existing.episode_id = incoming.episode_id.clone();
    }
    if existing.title_name.trim().is_empty() || existing.title_name == "Unnamed download" {
        existing.title_name = incoming.title_name.clone();
    }
    if existing.facet.is_none() {
        existing.facet = incoming.facet.clone();
    }
    if existing.client_id.is_empty() {
        existing.client_id = incoming.client_id.clone();
    }
    if existing.client_name.is_empty() {
        existing.client_name = incoming.client_name.clone();
    }
    if existing.client_type.is_empty() {
        existing.client_type = incoming.client_type.clone();
    }

    if let Some(size_bytes) = incoming.size_bytes {
        existing.size_bytes = Some(existing.size_bytes.unwrap_or(size_bytes).max(size_bytes));
    }
    if existing.remaining_seconds.is_none() {
        existing.remaining_seconds = incoming.remaining_seconds;
    }
    if existing.queued_at.is_none() {
        existing.queued_at = incoming.queued_at.clone();
    }
    if existing.last_updated_at.is_none() {
        existing.last_updated_at = incoming.last_updated_at.clone();
    }

    if queue_state_merge_rank(&incoming.state) > queue_state_merge_rank(&existing.state)
        || (incoming.progress_percent > existing.progress_percent
            && queue_state_merge_rank(&incoming.state) == queue_state_merge_rank(&existing.state))
    {
        existing.state = incoming.state;
        existing.progress_percent = incoming.progress_percent;
    } else {
        existing.progress_percent = existing.progress_percent.max(incoming.progress_percent);
    }

    existing.attention_required |= incoming.attention_required;
    if existing.attention_reason.is_none() {
        existing.attention_reason = incoming.attention_reason.clone();
    }
    if incoming.import_status.is_some() {
        existing.import_status = incoming.import_status;
        existing.import_transfer_phase = incoming.import_transfer_phase;
        existing.import_transfer_bytes = incoming.import_transfer_bytes;
        existing.import_transfer_total_bytes = incoming.import_transfer_total_bytes;
        existing.import_transfer_started_at = incoming.import_transfer_started_at.clone();
        existing.import_transfer_updated_at = incoming.import_transfer_updated_at.clone();
    }
    if incoming.import_error_message.is_some() {
        existing.import_error_message = incoming.import_error_message.clone();
    }
    if incoming.imported_at.is_some() {
        existing.imported_at = incoming.imported_at.clone();
    }
    existing.is_scryer_origin |= incoming.is_scryer_origin;
}
fn queue_state_merge_rank(state: &DownloadQueueState) -> u8 {
    match state {
        DownloadQueueState::Paused => 0,
        DownloadQueueState::Queued => 1,
        DownloadQueueState::Downloading => 2,
        DownloadQueueState::Verifying
        | DownloadQueueState::Repairing
        | DownloadQueueState::Extracting => 3,
        DownloadQueueState::Completed => 4,
        DownloadQueueState::ImportPending => 5,
        DownloadQueueState::Warning => 6,
        // A terminal failure outranks a recoverable warning: if two
        // observations of the same item disagree, the one that ends the grab is
        // the one to keep.
        DownloadQueueState::Failed => 7,
    }
}
/// `Warning` is deliberately absent: the download is still live in the client,
/// so the row is activity that needs a look, not a finished grab.
fn is_history_download_state(state: &DownloadQueueState) -> bool {
    matches!(
        state,
        DownloadQueueState::Completed
            | DownloadQueueState::ImportPending
            | DownloadQueueState::Failed
    )
}

#[cfg(test)]
mod queue_query_unit_tests {
    use super::*;
    use crate::{DownloadSubmission, DownloadSubmissionPurpose, SubmissionScope};

    fn queue_item() -> DownloadQueueItem {
        DownloadQueueItem {
            id: "item-1".to_string(),
            title_id: None,
            episode_id: None,
            title_name: "Observed.Release.2026".to_string(),
            facet: None,
            category: None,
            client_id: "client-1".to_string(),
            client_name: "Client".to_string(),
            client_type: "weaver".to_string(),
            state: DownloadQueueState::Completed,
            progress_percent: 100,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "item-1".to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: false,
            source_provider: None,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
            seeding: None,
        }
    }

    fn submission(scope: SubmissionScope) -> DownloadSubmission {
        DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: "title-1".to_string(),
            purpose: DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: "item-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: None,
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope,
        }
    }

    #[test]
    fn orphan_submission_enriches_metadata_without_claiming_scryer_origin() {
        let mut item = queue_item();
        apply_submission_to_queue_item(&mut item, &submission(SubmissionScope::Orphan));

        assert_eq!(item.title_id.as_deref(), Some("title-1"));
        assert_eq!(item.facet.as_deref(), Some("movie"));
        assert!(!item.is_scryer_origin);

        apply_submission_to_queue_item(&mut item, &submission(SubmissionScope::Title));
        assert!(item.is_scryer_origin);
    }

    #[test]
    fn submission_enrichment_prefers_raw_release_title() {
        let mut item = queue_item();
        let mut submitted = submission(SubmissionScope::Title);
        submitted.source_title = Some("GroupTag.Quiet.Meridian.252-279.BD-GROUP".to_string());

        apply_submission_to_queue_item(&mut item, &submitted);

        assert_eq!(item.title_name, "GroupTag.Quiet.Meridian.252-279.BD-GROUP");

        item.title_name = "Client Display Name".to_string();
        submitted.source_title = Some("  ".to_string());
        apply_submission_to_queue_item(&mut item, &submitted);
        assert_eq!(item.title_name, "Client Display Name");
    }
}
