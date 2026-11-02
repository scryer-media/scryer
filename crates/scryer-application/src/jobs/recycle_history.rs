use crate::{AppResult, AppUseCase, RecycledItemHistory, recycle_bin::RecycleManifest};
use scryer_domain::{DomainEvent, DomainEventFilter, DomainEventPayload, DomainEventType};
use std::collections::HashSet;

const CANDIDATE_LIMIT: usize = 5_000;
const SUMMARY_LIMIT: usize = 50;

pub(super) async fn load_summary(
    app: &AppUseCase,
    manifest: &RecycleManifest,
    title_id: &str,
) -> AppResult<RecycledItemHistory> {
    let mut candidates = Vec::new();
    let mut filter = DomainEventFilter {
        title_id: Some(title_id.to_string()),
        event_types: Some(vec![
            DomainEventType::ReleaseGrabbed,
            DomainEventType::ImportCompleted,
            DomainEventType::MediaFileUpgraded,
            DomainEventType::MediaFileDeleted,
        ]),
        ..Default::default()
    };
    // Bound both datastore pages and memory. An extra candidate distinguishes
    // a complete scan from one that reached the interactive summary's ceiling.
    while candidates.len() <= CANDIDATE_LIMIT {
        filter.limit = (CANDIDATE_LIMIT + 1 - candidates.len()).min(500);
        let page = app.services.events.domain_events.list(&filter).await?;
        if page.is_empty() {
            break;
        }
        filter.before_sequence = page.last().map(|event| event.sequence);
        candidates.extend(page);
    }
    let scan_limited = candidates.len() > CANDIDATE_LIMIT;
    candidates.truncate(CANDIDATE_LIMIT);
    let mut matched = correlated_events(manifest, &candidates);
    matched.sort_by_key(|event| std::cmp::Reverse((event.occurred_at, event.sequence)));
    let truncated = scan_limited || matched.len() > SUMMARY_LIMIT;
    matched.truncate(SUMMARY_LIMIT);
    matched.reverse();
    Ok(RecycledItemHistory {
        records: matched
            .into_iter()
            .filter_map(crate::event_views::title_history_record_from_domain_event)
            .collect(),
        truncated,
    })
}

/// Correlate by retained file/import/download identities, never by a title's
/// name, a reused destination path, or a guessed time window.
fn correlated_events<'a>(
    manifest: &RecycleManifest,
    events: &'a [DomainEvent],
) -> Vec<&'a DomainEvent> {
    let mut selected = HashSet::new();
    let mut imports = HashSet::new();
    let mut downloads = HashSet::new();
    let same_id = |expected: Option<&String>, actual: Option<&String>| {
        expected.is_some() && expected == actual
    };
    let scoped = |event: &&DomainEvent| event.title_id == manifest.title_id;
    for event in events.iter().filter(scoped) {
        match &event.payload {
            DomainEventPayload::MediaFileUpgraded(data)
                if same_id(
                    manifest.original_file_id.as_ref(),
                    data.previous_file_id.as_ref(),
                ) && manifest
                    .replacement_file_id
                    .as_ref()
                    .is_none_or(|id| data.current_file_id.as_ref() == Some(id)) =>
            {
                selected.insert(event.event_id.as_str());
                imports.extend(data.import_id.as_deref());
                downloads.extend(data.source_ref.as_deref());
            }
            DomainEventPayload::MediaFileDeleted(data)
                if same_id(manifest.original_file_id.as_ref(), data.file_id.as_ref()) =>
            {
                selected.insert(event.event_id.as_str());
            }
            _ => {}
        }
    }
    // Resolve both directions between the upgrade's import and download IDs.
    // A grab merely comparing itself to the old file does not prove it was
    // the replacement that committed, so scoring comparisons never seed links.
    for _ in 0..2 {
        for event in events.iter().filter(scoped) {
            if let DomainEventPayload::ImportCompleted(data) = &event.payload
                && (data
                    .import_id
                    .as_deref()
                    .is_some_and(|id| imports.contains(id))
                    || data
                        .source_ref
                        .as_deref()
                        .is_some_and(|id| downloads.contains(id)))
            {
                selected.insert(event.event_id.as_str());
                imports.extend(data.import_id.as_deref());
                downloads.extend(data.source_ref.as_deref());
            }
        }
    }
    for event in events.iter().filter(scoped) {
        if let DomainEventPayload::ReleaseGrabbed(data) = &event.payload
            && data
                .download_id
                .as_deref()
                .is_some_and(|id| downloads.contains(id))
        {
            selected.insert(event.event_id.as_str());
        }
    }
    events
        .iter()
        .filter(|event| selected.contains(event.event_id.as_str()))
        .collect()
}
