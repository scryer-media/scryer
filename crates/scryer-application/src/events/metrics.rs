//! Prometheus counters derived from the domain-event stream.
//!
//! Every stored domain event passes through exactly one application seam
//! (`AppUseCase::publish_stored_domain_event`, plus the batch fan-out in
//! `AppUseCase::append_domain_events`), so the product loops — search → grab →
//! download failure, and import → upgrade → rejection — are instrumented once
//! here instead of at dozens of call sites that drift apart.
//!
//! Label discipline: every label value is either a `&'static str` from a
//! bounded enum or a small fixed code set. Nothing free-form (titles, paths,
//! ids, error text, release names) ever reaches a label.

use metrics::{Unit, counter, describe_counter};
use scryer_domain::{DomainEvent, DomainEventPayload, MediaFacet, TitleContextSnapshot};

use crate::acquisition::release_search::ReleaseAutoDecisionCode;
use crate::jobs::definitions::JobKey;

const UNKNOWN: &str = "unknown";
const OTHER: &str = "other";
const NONE: &str = "none";

const DOMAIN_EVENTS_TOTAL: &str = "scryer_domain_events_total";
const ACQUISITION_SEARCHES_COMPLETED_TOTAL: &str = "scryer_acquisition_searches_completed_total";
const ACQUISITION_SEARCH_RESULTS_TOTAL: &str = "scryer_acquisition_search_results_total";
const ACQUISITION_CANDIDATES_REJECTED_TOTAL: &str = "scryer_acquisition_candidates_rejected_total";
const DOWNLOADS_FAILED_TOTAL: &str = "scryer_downloads_failed_total";
const RELEASES_BLOCKLISTED_TOTAL: &str = "scryer_releases_blocklisted_total";
const DOWNLOADS_IGNORED_TOTAL: &str = "scryer_downloads_ignored_total";
const IMPORTS_TOTAL: &str = "scryer_imports_total";
const IMPORT_FILES_TOTAL: &str = "scryer_import_files_total";
const IMPORT_BYTES_TOTAL: &str = "scryer_import_bytes_total";
const IMPORT_REJECTIONS_TOTAL: &str = "scryer_import_rejections_total";
const MEDIA_FILE_UPGRADES_TOTAL: &str = "scryer_media_file_upgrades_total";
const MEDIA_FILE_UPGRADE_BYTES_TOTAL: &str = "scryer_media_file_upgrade_bytes_total";
const MEDIA_FILES_DELETED_TOTAL: &str = "scryer_media_files_deleted_total";
const LIBRARY_SCANS_TOTAL: &str = "scryer_library_scans_total";
const LIBRARY_SCAN_ITEMS_TOTAL: &str = "scryer_library_scan_items_total";
const JOB_RUNS_TOTAL: &str = "scryer_job_runs_total";
const SUBTITLES_DOWNLOADED_TOTAL: &str = "scryer_subtitles_downloaded_total";
const SUBTITLE_SEARCH_FAILURES_TOTAL: &str = "scryer_subtitle_search_failures_total";

/// Registers HELP/UNIT metadata for every family this module emits.
///
/// Crate-public because the binary's metrics setup calls it once at startup,
/// before any event has been recorded, so the scrape surface is self-describing
/// even while a family is still empty.
pub fn describe_domain_event_metrics() {
    describe_counter!(
        DOMAIN_EVENTS_TOTAL,
        "Domain events appended to the event stream, by event type."
    );
    describe_counter!(
        ACQUISITION_SEARCHES_COMPLETED_TOTAL,
        "Acquisition searches that ran to completion, by media facet."
    );
    describe_counter!(
        ACQUISITION_SEARCH_RESULTS_TOTAL,
        "Release candidates returned by completed acquisition searches, by media facet."
    );
    describe_counter!(
        ACQUISITION_CANDIDATES_REJECTED_TOTAL,
        "Release candidates rejected by the acquisition decision gate, by media facet and reason code."
    );
    describe_counter!(
        DOWNLOADS_FAILED_TOTAL,
        "Downloads reported as failed by a download client, by media facet and client type."
    );
    describe_counter!(
        RELEASES_BLOCKLISTED_TOTAL,
        "Releases added to the blocklist, by media facet."
    );
    describe_counter!(
        DOWNLOADS_IGNORED_TOTAL,
        "Download-client entries Scryer deliberately ignored, by media facet and client type."
    );
    describe_counter!(
        IMPORTS_TOTAL,
        "Completed imports, by media facet (one per import event, not per file)."
    );
    describe_counter!(
        IMPORT_FILES_TOTAL,
        "Media files brought in by completed imports, by media facet."
    );
    describe_counter!(
        IMPORT_BYTES_TOTAL,
        Unit::Bytes,
        "Bytes brought in by completed imports, by media facet."
    );
    describe_counter!(
        IMPORT_REJECTIONS_TOTAL,
        "Imports rejected before completion, by media facet, terminal status and skip reason."
    );
    describe_counter!(
        MEDIA_FILE_UPGRADES_TOTAL,
        "Media files replaced by a better release, by media facet."
    );
    describe_counter!(
        MEDIA_FILE_UPGRADE_BYTES_TOTAL,
        Unit::Bytes,
        "Bytes of the replacement files written by media-file upgrades, by media facet."
    );
    describe_counter!(
        MEDIA_FILES_DELETED_TOTAL,
        "Media files deleted from the library, by media facet and deletion reason."
    );
    describe_counter!(
        LIBRARY_SCANS_TOTAL,
        "Library scans that reached a terminal state, by outcome."
    );
    describe_counter!(
        LIBRARY_SCAN_ITEMS_TOTAL,
        "Items accounted for by completed library scans, by summary kind."
    );
    describe_counter!(
        JOB_RUNS_TOTAL,
        "Scheduled job runs that reached a terminal state, by job key and outcome."
    );
    describe_counter!(
        SUBTITLES_DOWNLOADED_TOTAL,
        "Subtitle files downloaded, by media facet."
    );
    describe_counter!(
        SUBTITLE_SEARCH_FAILURES_TOTAL,
        "Subtitle searches that failed to produce a usable result, by media facet."
    );
}

/// Records the metric families a single stored domain event contributes to.
///
/// Pure, infallible and allocation-light: the only allocation is the lowercased
/// download-client type used as a label value.
pub(crate) fn record_domain_event_metrics(event: &DomainEvent) {
    let payload = &event.payload;
    counter!(DOMAIN_EVENTS_TOTAL, "event_type" => payload.event_type().as_str()).increment(1);

    let facet = facet_label(event);

    match payload {
        DomainEventPayload::AcquisitionSearchCompleted(data) => {
            counter!(ACQUISITION_SEARCHES_COMPLETED_TOTAL, "facet" => facet).increment(1);
            counter!(ACQUISITION_SEARCH_RESULTS_TOTAL, "facet" => facet)
                .increment(non_negative(data.result_count));
        }
        DomainEventPayload::AcquisitionCandidateRejected(data) => {
            counter!(
                ACQUISITION_CANDIDATES_REJECTED_TOTAL,
                "facet" => facet,
                "reason_code" => rejection_reason_label(&data.reason_code),
            )
            .increment(1);
        }
        DomainEventPayload::DownloadFailed(data) => {
            counter!(
                DOWNLOADS_FAILED_TOTAL,
                "facet" => facet,
                "client_type" => client_type_label(data.client_type.as_deref()),
            )
            .increment(1);
        }
        DomainEventPayload::ReleaseBlocklisted(_) => {
            counter!(RELEASES_BLOCKLISTED_TOTAL, "facet" => facet).increment(1);
        }
        DomainEventPayload::DownloadIgnored(data) => {
            counter!(
                DOWNLOADS_IGNORED_TOTAL,
                "facet" => facet,
                "client_type" => client_type_label(data.client_type.as_deref()),
            )
            .increment(1);
        }
        DomainEventPayload::ImportCompleted(data) => {
            counter!(IMPORTS_TOTAL, "facet" => facet).increment(1);
            counter!(IMPORT_FILES_TOTAL, "facet" => facet)
                .increment(non_negative(i64::from(data.imported_count)));
            if let Some(size_bytes) = data.size_bytes.filter(|bytes| *bytes > 0) {
                counter!(IMPORT_BYTES_TOTAL, "facet" => facet).increment(non_negative(size_bytes));
            }
        }
        DomainEventPayload::ImportRejected(data) => {
            counter!(
                IMPORT_REJECTIONS_TOTAL,
                "facet" => facet,
                "status" => data.status.as_str(),
                "skip_reason" => data.skip_reason.as_ref().map_or(NONE, |reason| reason.as_str()),
            )
            .increment(1);
        }
        DomainEventPayload::MediaFileUpgraded(data) => {
            counter!(MEDIA_FILE_UPGRADES_TOTAL, "facet" => facet).increment(1);
            if let Some(size_bytes) = data.size_bytes.filter(|bytes| *bytes > 0) {
                counter!(MEDIA_FILE_UPGRADE_BYTES_TOTAL, "facet" => facet)
                    .increment(non_negative(size_bytes));
            }
        }
        DomainEventPayload::MediaFileDeleted(data) => {
            counter!(
                MEDIA_FILES_DELETED_TOTAL,
                "facet" => facet,
                "reason" => data.reason.as_str(),
            )
            .increment(1);
        }
        DomainEventPayload::LibraryScanCompleted(data) => {
            counter!(LIBRARY_SCANS_TOTAL, "outcome" => library_scan_status_label(&data.status))
                .increment(1);
            if let Some(summary) = data.summary.as_ref() {
                counter!(LIBRARY_SCAN_ITEMS_TOTAL, "kind" => "scanned")
                    .increment(non_negative(summary.scanned));
                counter!(LIBRARY_SCAN_ITEMS_TOTAL, "kind" => "matched")
                    .increment(non_negative(summary.matched));
                counter!(LIBRARY_SCAN_ITEMS_TOTAL, "kind" => "imported")
                    .increment(non_negative(summary.imported));
                counter!(LIBRARY_SCAN_ITEMS_TOTAL, "kind" => "skipped")
                    .increment(non_negative(summary.skipped));
                counter!(LIBRARY_SCAN_ITEMS_TOTAL, "kind" => "unmatched")
                    .increment(non_negative(summary.unmatched));
            }
        }
        DomainEventPayload::LibraryScanFailed(_) => {
            counter!(LIBRARY_SCANS_TOTAL, "outcome" => "failed").increment(1);
        }
        DomainEventPayload::LibraryScanCanceled(_) => {
            counter!(LIBRARY_SCANS_TOTAL, "outcome" => "canceled").increment(1);
        }
        DomainEventPayload::JobRunCompleted(data) => {
            counter!(
                JOB_RUNS_TOTAL,
                "job_key" => job_key_label(&data.job_key),
                "outcome" => "completed",
            )
            .increment(1);
        }
        DomainEventPayload::JobRunFailed(data) => {
            counter!(
                JOB_RUNS_TOTAL,
                "job_key" => job_key_label(&data.job_key),
                "outcome" => "failed",
            )
            .increment(1);
        }
        DomainEventPayload::SubtitleDownloaded(_) => {
            counter!(SUBTITLES_DOWNLOADED_TOTAL, "facet" => facet).increment(1);
        }
        DomainEventPayload::SubtitleSearchFailed(_) => {
            counter!(SUBTITLE_SEARCH_FAILURES_TOTAL, "facet" => facet).increment(1);
        }
        // Grabs are already counted at their call sites as `scryer_grabs_total`,
        // and the high-frequency queue/scan-progress events would only add churn
        // beyond the generic `scryer_domain_events_total`.
        _ => {}
    }
}

/// The facet a payload is about, preferring the payload's own title snapshot
/// over the envelope's facet so that events appended without an envelope facet
/// still land on the right series.
fn facet_label(event: &DomainEvent) -> &'static str {
    payload_title(&event.payload)
        .map(|title| title.facet.as_str())
        .or_else(|| event.facet.as_ref().map(MediaFacet::as_str))
        .unwrap_or(UNKNOWN)
}

fn payload_title(payload: &DomainEventPayload) -> Option<&TitleContextSnapshot> {
    match payload {
        DomainEventPayload::TitleAdded(data) => Some(&data.title),
        DomainEventPayload::TitleUpdated(data) => Some(&data.title),
        DomainEventPayload::TitleRematched(data) => Some(&data.title),
        DomainEventPayload::TitleDeleted(data) => Some(&data.title),
        DomainEventPayload::MetadataHydrationUpdated(data) => Some(&data.title),
        DomainEventPayload::ReleaseGrabbed(data) => Some(&data.title),
        DomainEventPayload::DownloadFailed(data) => data.title.as_ref(),
        DomainEventPayload::ReleaseBlocklisted(data) => data.title.as_ref(),
        DomainEventPayload::ImportCompleted(data) => Some(&data.title),
        DomainEventPayload::ImportRejected(data) => data.title.as_ref(),
        DomainEventPayload::MediaFileImported(data) => Some(&data.title),
        DomainEventPayload::MediaFileAnalyzed(data) => Some(&data.title),
        DomainEventPayload::MediaFileRenamed(data) => Some(&data.title),
        DomainEventPayload::MediaFileDeleted(data) => Some(&data.title),
        DomainEventPayload::MediaFileUpgraded(data) => Some(&data.title),
        DomainEventPayload::AcquisitionSearchCompleted(data) => Some(&data.title),
        DomainEventPayload::AcquisitionCandidateRejected(data) => Some(&data.title),
        DomainEventPayload::ImportRequested(data) => data.title.as_ref(),
        DomainEventPayload::PostProcessingCompleted(data) => Some(&data.title),
        DomainEventPayload::SubtitleDownloaded(data) => Some(&data.title),
        DomainEventPayload::SubtitleSearchFailed(data) => Some(&data.title),
        DomainEventPayload::DownloadIgnored(data) => data.title.as_ref(),
        DomainEventPayload::SeedingStarted(data) => data.title.as_ref(),
        DomainEventPayload::SeedingCompleted(data) => data.title.as_ref(),
        _ => None,
    }
}

/// Clamps a signed count to the unsigned counter increment a Prometheus counter
/// accepts. A negative count is a bug upstream, never a decrement here.
fn non_negative(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// `reason_code` is written by Scryer itself, always from
/// [`ReleaseAutoDecisionCode::as_str`]. Round-tripping it through `parse`
/// pins the label to that bounded set even if a future call site passes
/// something else.
fn rejection_reason_label(reason_code: &str) -> &'static str {
    ReleaseAutoDecisionCode::parse(reason_code).map_or(OTHER, ReleaseAutoDecisionCode::as_str)
}

/// `job_key` is persisted as the string form of the bounded [`JobKey`]
/// registry enum; anything that does not round-trip collapses to `other`.
fn job_key_label(job_key: &str) -> &'static str {
    JobKey::parse(job_key).map_or(OTHER, JobKey::as_str)
}

/// Library-scan `status` is a `String` on the wire but is always produced from
/// `LibraryScanStatus::as_str`. Pinning it to that allowlist keeps the label
/// bounded no matter what an older persisted event carries.
fn library_scan_status_label(status: &str) -> &'static str {
    match status {
        "discovering" => "discovering",
        "running" => "running",
        "completed" => "completed",
        "canceled" => "canceled",
        "warning" => "warning",
        "failed" => "failed",
        _ => OTHER,
    }
}

/// Download-client types are a small fixed set of client kinds; lowercasing
/// keeps a client that echoes its own casing from splitting the series.
fn client_type_label(client_type: Option<&str>) -> String {
    client_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| UNKNOWN.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use metrics::with_local_recorder;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use scryer_domain::{
        AcquisitionCandidateRejectedEventData, AcquisitionSearchCompletedEventData,
        ConfigurationChangeAction, ConfigurationChangedEventData,
        DiscoverySearchCompletedEventData, DomainEvent, DomainEventActorKind, DomainEventPayload,
        DomainEventStream, DomainExternalIds, DownloadFailedEventData, DownloadIgnoredEventData,
        DownloadQueueCommandAction, DownloadQueueItem, DownloadQueueItemCommandIssuedEventData,
        DownloadQueueItemRemovedEventData, DownloadQueueItemUpsertedEventData, DownloadQueueState,
        ImportCompletedEventData, ImportRecoveryCompletedEventData, ImportRejectedEventData,
        ImportRequestKind, ImportRequestedEventData, ImportSkipReason, ImportStatus,
        JobNextRunUpdatedEventData, JobRunCompletedEventData, JobRunFailedEventData,
        JobRunStartedEventData, LibraryScanCanceledEventData, LibraryScanCompletedEventData,
        LibraryScanDeltaRecordedEventData, LibraryScanFailedEventData,
        LibraryScanProgressedEventData, LibraryScanStartedEventData, LibraryScanSummaryEventData,
        LibraryScanTitleDiscoveredEventData, MediaFacet, MediaFileAnalyzedEventData,
        MediaFileDeletedEventData, MediaFileDeletedReason, MediaFileImportedEventData,
        MediaFileRenamedEventData, MediaFileUpgradedEventData, MediaRequestResolvedEventData,
        MediaRequestSubmittedEventData, MetadataHydrationState, MetadataHydrationUpdatedEventData,
        PostProcessingCompletedEventData, PostProcessingResult, ReleaseBlocklistedEventData,
        ReleaseGrabbedEventData, SeedingCompletedEventData, SeedingStartedEventData,
        SubtitleDownloadedEventData, SubtitleSearchFailedEventData, TitleAddedEventData,
        TitleContextSnapshot, TitleDeletedEventData, TitleRematchedEventData,
        TitleUpdatedEventData,
    };

    use super::*;

    /// One recorded counter series: name, sorted labels, value.
    type CounterSeries = (String, BTreeMap<String, String>, u64);

    fn title_snapshot(facet: MediaFacet) -> TitleContextSnapshot {
        TitleContextSnapshot {
            title_name: "Example".to_string(),
            facet,
            external_ids: DomainExternalIds::default(),
            poster_url: None,
            year: Some(2024),
        }
    }

    fn event_with_facet(payload: DomainEventPayload, facet: Option<MediaFacet>) -> DomainEvent {
        DomainEvent {
            sequence: 1,
            event_id: "evt-1".to_string(),
            occurred_at: Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some("title-1".to_string()),
            facet,
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Global,
            payload,
        }
    }

    fn event(payload: DomainEventPayload) -> DomainEvent {
        event_with_facet(payload, Some(MediaFacet::Series))
    }

    /// Records the given events against a thread-local debugging recorder and
    /// returns every counter series it observed. Never installs a global
    /// recorder, so tests stay independent of each other.
    fn record(events: &[DomainEvent]) -> Vec<CounterSeries> {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, || {
            for event in events {
                record_domain_event_metrics(event);
            }
        });
        snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _unit, _description, value)| match value {
                DebugValue::Counter(count) => {
                    let key = key.key();
                    let labels = key
                        .labels()
                        .map(|label| (label.key().to_string(), label.value().to_string()))
                        .collect::<BTreeMap<_, _>>();
                    Some((key.name().to_string(), labels, count))
                }
                _ => None,
            })
            .collect()
    }

    fn series<'a>(
        recorded: &'a [CounterSeries],
        name: &str,
    ) -> Vec<(&'a BTreeMap<String, String>, u64)> {
        recorded
            .iter()
            .filter(|(series_name, _, _)| series_name == name)
            .map(|(_, labels, value)| (labels, *value))
            .collect()
    }

    fn value_with(recorded: &[CounterSeries], name: &str, labels: &[(&str, &str)]) -> Option<u64> {
        let expected = labels
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect::<BTreeMap<_, _>>();
        recorded
            .iter()
            .find(|(series_name, series_labels, _)| {
                series_name == name && *series_labels == expected
            })
            .map(|(_, _, value)| *value)
    }

    fn import_completed(facet: MediaFacet, size_bytes: Option<i64>) -> ImportCompletedEventData {
        ImportCompletedEventData {
            title: title_snapshot(facet),
            media_updates: Vec::new(),
            imported_count: 2,
            import_id: None,
            source_system: None,
            source_ref: None,
            source_title: None,
            source_path: None,
            dest_path: None,
            quality: None,
            episode_ids: Vec::new(),
            size_bytes,
        }
    }

    fn import_rejected(skip_reason: Option<ImportSkipReason>) -> ImportRejectedEventData {
        ImportRejectedEventData {
            title: Some(title_snapshot(MediaFacet::Series)),
            status: ImportStatus::Skipped,
            import_id: None,
            source_system: None,
            source_ref: None,
            source_title: None,
            source_path: None,
            dest_path: None,
            quality: None,
            reason: None,
            skip_reason,
            episode_ids: Vec::new(),
        }
    }

    fn download_failed(client_type: Option<&str>) -> DownloadFailedEventData {
        DownloadFailedEventData {
            title: Some(title_snapshot(MediaFacet::Movie)),
            source_title: None,
            source_hint: None,
            download_id: None,
            client_id: None,
            client_name: None,
            client_type: client_type.map(str::to_string),
            quality: None,
            reason: None,
            episode_ids: Vec::new(),
            collection_id: None,
        }
    }

    fn media_file_upgraded(size_bytes: Option<i64>) -> MediaFileUpgradedEventData {
        MediaFileUpgradedEventData {
            title: title_snapshot(MediaFacet::Anime),
            media_updates: Vec::new(),
            episode_ids: Vec::new(),
            previous_file_id: None,
            current_file_id: None,
            old_score: Some(10),
            new_score: Some(20),
            size_bytes,
        }
    }

    fn library_scan_summary() -> LibraryScanSummaryEventData {
        LibraryScanSummaryEventData {
            scanned: 10,
            matched: 7,
            imported: 5,
            skipped: 2,
            unmatched: 3,
        }
    }

    fn library_scan_completed(
        status: &str,
        summary: Option<LibraryScanSummaryEventData>,
    ) -> LibraryScanCompletedEventData {
        LibraryScanCompletedEventData {
            session_id: "session-1".to_string(),
            status: status.to_string(),
            found_titles: 10,
            title_match_completed: 10,
            title_match_total_known: true,
            titles_completed: 10,
            titles_total: Some(10),
            files_completed: 10,
            files_total: Some(10),
            summary,
            warning_message: None,
        }
    }

    fn queue_item() -> DownloadQueueItem {
        DownloadQueueItem {
            id: "queue-1".to_string(),
            title_id: None,
            episode_id: None,
            title_name: "Example".to_string(),
            facet: None,
            category: None,
            client_id: "client-1".to_string(),
            client_name: "Weaver".to_string(),
            client_type: "weaver".to_string(),
            state: DownloadQueueState::Downloading,
            progress_percent: 10,
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
            download_client_item_id: "queue-1".to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: true,
            source_provider: None,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
            seeding: None,
        }
    }

    #[test]
    fn facet_label_prefers_the_payload_title_over_the_envelope() {
        let recorded = record(&[event_with_facet(
            DomainEventPayload::ImportCompleted(import_completed(MediaFacet::Anime, None)),
            Some(MediaFacet::Movie),
        )]);

        assert_eq!(
            value_with(&recorded, IMPORTS_TOTAL, &[("facet", "anime")]),
            Some(1)
        );
        assert_eq!(series(&recorded, IMPORTS_TOTAL).len(), 1);
    }

    #[test]
    fn facet_label_falls_back_to_the_envelope_when_the_payload_has_no_title() {
        let recorded = record(&[event_with_facet(
            DomainEventPayload::DownloadFailed(DownloadFailedEventData {
                title: None,
                ..download_failed(Some("sabnzbd"))
            }),
            Some(MediaFacet::Movie),
        )]);

        assert_eq!(
            value_with(
                &recorded,
                DOWNLOADS_FAILED_TOTAL,
                &[("facet", "movie"), ("client_type", "sabnzbd")]
            ),
            Some(1)
        );
    }

    #[test]
    fn facet_label_falls_back_to_unknown_without_a_payload_or_envelope_facet() {
        let recorded = record(&[event_with_facet(
            DomainEventPayload::DownloadFailed(DownloadFailedEventData {
                title: None,
                ..download_failed(Some("nzbget"))
            }),
            None,
        )]);

        assert_eq!(
            value_with(
                &recorded,
                DOWNLOADS_FAILED_TOTAL,
                &[("facet", "unknown"), ("client_type", "nzbget")]
            ),
            Some(1)
        );
    }

    #[test]
    fn download_failed_without_a_client_type_records_unknown() {
        let recorded = record(&[event(DomainEventPayload::DownloadFailed(download_failed(
            None,
        )))]);

        assert_eq!(
            value_with(
                &recorded,
                DOWNLOADS_FAILED_TOTAL,
                &[("facet", "movie"), ("client_type", "unknown")]
            ),
            Some(1)
        );
    }

    #[test]
    fn download_failed_lowercases_the_client_type() {
        let recorded = record(&[event(DomainEventPayload::DownloadFailed(download_failed(
            Some("SABnzbd"),
        )))]);

        assert_eq!(
            value_with(
                &recorded,
                DOWNLOADS_FAILED_TOTAL,
                &[("facet", "movie"), ("client_type", "sabnzbd")]
            ),
            Some(1)
        );
    }

    #[test]
    fn import_rejected_without_a_skip_reason_records_none() {
        let recorded = record(&[event(DomainEventPayload::ImportRejected(import_rejected(
            None,
        )))]);

        assert_eq!(
            value_with(
                &recorded,
                IMPORT_REJECTIONS_TOTAL,
                &[
                    ("facet", "series"),
                    ("status", "skipped"),
                    ("skip_reason", "none"),
                ]
            ),
            Some(1)
        );
    }

    #[test]
    fn import_rejected_records_the_skip_reason_when_present() {
        let recorded = record(&[event(DomainEventPayload::ImportRejected(import_rejected(
            Some(ImportSkipReason::DiskFull),
        )))]);

        assert_eq!(
            value_with(
                &recorded,
                IMPORT_REJECTIONS_TOTAL,
                &[
                    ("facet", "series"),
                    ("status", "skipped"),
                    ("skip_reason", "disk_full"),
                ]
            ),
            Some(1)
        );
    }

    #[test]
    fn import_completed_records_files_and_bytes() {
        let recorded = record(&[event(DomainEventPayload::ImportCompleted(
            import_completed(MediaFacet::Series, Some(4096)),
        ))]);

        assert_eq!(
            value_with(&recorded, IMPORTS_TOTAL, &[("facet", "series")]),
            Some(1)
        );
        assert_eq!(
            value_with(&recorded, IMPORT_FILES_TOTAL, &[("facet", "series")]),
            Some(2)
        );
        assert_eq!(
            value_with(&recorded, IMPORT_BYTES_TOTAL, &[("facet", "series")]),
            Some(4096)
        );
    }

    #[test]
    fn import_completed_without_a_size_leaves_the_bytes_family_absent() {
        let recorded = record(&[event(DomainEventPayload::ImportCompleted(
            import_completed(MediaFacet::Series, None),
        ))]);

        assert_eq!(
            value_with(&recorded, IMPORTS_TOTAL, &[("facet", "series")]),
            Some(1)
        );
        assert!(series(&recorded, IMPORT_BYTES_TOTAL).is_empty());
    }

    #[test]
    fn media_file_upgraded_records_count_and_bytes() {
        let recorded = record(&[event(DomainEventPayload::MediaFileUpgraded(
            media_file_upgraded(Some(8192)),
        ))]);

        assert_eq!(
            value_with(&recorded, MEDIA_FILE_UPGRADES_TOTAL, &[("facet", "anime")]),
            Some(1)
        );
        assert_eq!(
            value_with(
                &recorded,
                MEDIA_FILE_UPGRADE_BYTES_TOTAL,
                &[("facet", "anime")]
            ),
            Some(8192)
        );
    }

    #[test]
    fn media_file_upgraded_without_a_size_leaves_the_bytes_family_absent() {
        let recorded = record(&[event(DomainEventPayload::MediaFileUpgraded(
            media_file_upgraded(None),
        ))]);

        assert_eq!(
            value_with(&recorded, MEDIA_FILE_UPGRADES_TOTAL, &[("facet", "anime")]),
            Some(1)
        );
        assert!(series(&recorded, MEDIA_FILE_UPGRADE_BYTES_TOTAL).is_empty());
    }

    #[test]
    fn acquisition_events_record_results_and_bounded_reason_codes() {
        let recorded = record(&[
            event(DomainEventPayload::AcquisitionSearchCompleted(
                AcquisitionSearchCompletedEventData {
                    title: title_snapshot(MediaFacet::Series),
                    result_count: 12,
                },
            )),
            event(DomainEventPayload::AcquisitionSearchCompleted(
                AcquisitionSearchCompletedEventData {
                    title: title_snapshot(MediaFacet::Series),
                    result_count: -5,
                },
            )),
            event(DomainEventPayload::AcquisitionCandidateRejected(
                AcquisitionCandidateRejectedEventData {
                    title: title_snapshot(MediaFacet::Series),
                    source_title: "Example.S01E01.1080p".to_string(),
                    reason_code: "quality_blocked".to_string(),
                },
            )),
            event(DomainEventPayload::AcquisitionCandidateRejected(
                AcquisitionCandidateRejectedEventData {
                    title: title_snapshot(MediaFacet::Series),
                    source_title: "Example.S01E02.1080p".to_string(),
                    reason_code: "something the gate never produces".to_string(),
                },
            )),
        ]);

        assert_eq!(
            value_with(
                &recorded,
                ACQUISITION_SEARCHES_COMPLETED_TOTAL,
                &[("facet", "series")]
            ),
            Some(2)
        );
        assert_eq!(
            value_with(
                &recorded,
                ACQUISITION_SEARCH_RESULTS_TOTAL,
                &[("facet", "series")]
            ),
            Some(12)
        );
        assert_eq!(
            value_with(
                &recorded,
                ACQUISITION_CANDIDATES_REJECTED_TOTAL,
                &[("facet", "series"), ("reason_code", "quality_blocked")]
            ),
            Some(1)
        );
        assert_eq!(
            value_with(
                &recorded,
                ACQUISITION_CANDIDATES_REJECTED_TOTAL,
                &[("facet", "series"), ("reason_code", "other")]
            ),
            Some(1)
        );
    }

    #[test]
    fn library_scan_completed_records_the_five_summary_kinds() {
        let recorded = record(&[event(DomainEventPayload::LibraryScanCompleted(
            library_scan_completed("completed", Some(library_scan_summary())),
        ))]);

        assert_eq!(
            value_with(&recorded, LIBRARY_SCANS_TOTAL, &[("outcome", "completed")]),
            Some(1)
        );
        for (kind, expected) in [
            ("scanned", 10),
            ("matched", 7),
            ("imported", 5),
            ("skipped", 2),
            ("unmatched", 3),
        ] {
            assert_eq!(
                value_with(&recorded, LIBRARY_SCAN_ITEMS_TOTAL, &[("kind", kind)]),
                Some(expected),
                "kind {kind}"
            );
        }
        assert_eq!(series(&recorded, LIBRARY_SCAN_ITEMS_TOTAL).len(), 5);
    }

    #[test]
    fn library_scan_completed_without_a_summary_emits_no_item_series() {
        let recorded = record(&[event(DomainEventPayload::LibraryScanCompleted(
            library_scan_completed("warning", None),
        ))]);

        assert_eq!(
            value_with(&recorded, LIBRARY_SCANS_TOTAL, &[("outcome", "warning")]),
            Some(1)
        );
        assert!(series(&recorded, LIBRARY_SCAN_ITEMS_TOTAL).is_empty());
    }

    #[test]
    fn library_scan_status_outside_the_allowlist_records_other() {
        let recorded = record(&[event(DomainEventPayload::LibraryScanCompleted(
            library_scan_completed("some-status-that-does-not-exist", None),
        ))]);

        assert_eq!(
            value_with(&recorded, LIBRARY_SCANS_TOTAL, &[("outcome", "other")]),
            Some(1)
        );
    }

    #[test]
    fn terminal_library_scan_events_record_their_outcome() {
        let recorded = record(&[
            event(DomainEventPayload::LibraryScanFailed(
                LibraryScanFailedEventData {
                    session_id: "session-1".to_string(),
                    error_message: "boom".to_string(),
                },
            )),
            event(DomainEventPayload::LibraryScanCanceled(
                LibraryScanCanceledEventData {
                    session_id: "session-1".to_string(),
                    status: "canceled".to_string(),
                    found_titles: 1,
                    title_match_completed: 1,
                    title_match_total_known: true,
                    titles_completed: 1,
                    titles_total: Some(1),
                    files_completed: 0,
                    files_total: Some(0),
                    summary: None,
                },
            )),
        ]);

        assert_eq!(
            value_with(&recorded, LIBRARY_SCANS_TOTAL, &[("outcome", "failed")]),
            Some(1)
        );
        assert_eq!(
            value_with(&recorded, LIBRARY_SCANS_TOTAL, &[("outcome", "canceled")]),
            Some(1)
        );
    }

    #[test]
    fn job_run_outcomes_use_bounded_job_keys() {
        let recorded = record(&[
            event(DomainEventPayload::JobRunCompleted(
                JobRunCompletedEventData {
                    run_id: "run-1".to_string(),
                    job_key: "rss_sync".to_string(),
                    summary_text: None,
                },
            )),
            event(DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: "run-2".to_string(),
                job_key: "rss_sync".to_string(),
                error_text: None,
            })),
            event(DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: "run-3".to_string(),
                job_key: "not-a-registered-job".to_string(),
                error_text: None,
            })),
        ]);

        assert_eq!(
            value_with(
                &recorded,
                JOB_RUNS_TOTAL,
                &[("job_key", "rss_sync"), ("outcome", "completed")]
            ),
            Some(1)
        );
        assert_eq!(
            value_with(
                &recorded,
                JOB_RUNS_TOTAL,
                &[("job_key", "rss_sync"), ("outcome", "failed")]
            ),
            Some(1)
        );
        assert_eq!(
            value_with(
                &recorded,
                JOB_RUNS_TOTAL,
                &[("job_key", "other"), ("outcome", "failed")]
            ),
            Some(1)
        );
    }

    #[test]
    fn subtitle_blocklist_and_ignore_events_record_their_families() {
        let recorded = record(&[
            event(DomainEventPayload::SubtitleDownloaded(
                SubtitleDownloadedEventData {
                    title: title_snapshot(MediaFacet::Series),
                    subtitle_path: None,
                    language: None,
                    provider: None,
                },
            )),
            event(DomainEventPayload::SubtitleSearchFailed(
                SubtitleSearchFailedEventData {
                    title: title_snapshot(MediaFacet::Series),
                    language: None,
                    reason: None,
                },
            )),
            event(DomainEventPayload::ReleaseBlocklisted(
                ReleaseBlocklistedEventData {
                    title: Some(title_snapshot(MediaFacet::Movie)),
                    source_title: None,
                    source_hint: None,
                    download_id: None,
                    client_id: None,
                    client_name: None,
                    client_type: None,
                    quality: None,
                    reason: None,
                    episode_ids: Vec::new(),
                    collection_id: None,
                },
            )),
            event(DomainEventPayload::DownloadIgnored(
                DownloadIgnoredEventData {
                    title: Some(title_snapshot(MediaFacet::Movie)),
                    download_client_item_id: "queue-1".to_string(),
                    client_id: None,
                    client_type: Some("qBittorrent".to_string()),
                    source_provider: None,
                    source_title: None,
                },
            )),
        ]);

        assert_eq!(
            value_with(
                &recorded,
                SUBTITLES_DOWNLOADED_TOTAL,
                &[("facet", "series")]
            ),
            Some(1)
        );
        assert_eq!(
            value_with(
                &recorded,
                SUBTITLE_SEARCH_FAILURES_TOTAL,
                &[("facet", "series")]
            ),
            Some(1)
        );
        assert_eq!(
            value_with(&recorded, RELEASES_BLOCKLISTED_TOTAL, &[("facet", "movie")]),
            Some(1)
        );
        assert_eq!(
            value_with(
                &recorded,
                DOWNLOADS_IGNORED_TOTAL,
                &[("facet", "movie"), ("client_type", "qbittorrent")]
            ),
            Some(1)
        );
    }

    #[test]
    fn media_file_deleted_records_the_bounded_reason() {
        let recorded = record(&[event(DomainEventPayload::MediaFileDeleted(
            MediaFileDeletedEventData {
                title: title_snapshot(MediaFacet::Movie),
                media_updates: Vec::new(),
                file_id: None,
                reason: MediaFileDeletedReason::UpgradeCleanup,
                episode_ids: Vec::new(),
            },
        ))]);

        assert_eq!(
            value_with(
                &recorded,
                MEDIA_FILES_DELETED_TOTAL,
                &[("facet", "movie"), ("reason", "upgrade_cleanup")]
            ),
            Some(1)
        );
    }

    #[test]
    fn release_grabbed_only_contributes_to_the_generic_event_counter() {
        let recorded = record(&[event(DomainEventPayload::ReleaseGrabbed(
            ReleaseGrabbedEventData {
                title: title_snapshot(MediaFacet::Series),
                source_title: None,
                source_hint: None,
                source_provider: None,
                download_id: None,
                episode_ids: Vec::new(),
            },
        ))]);

        assert_eq!(
            value_with(
                &recorded,
                DOMAIN_EVENTS_TOTAL,
                &[("event_type", "release_grabbed")]
            ),
            Some(1)
        );
        assert_eq!(recorded.len(), 1);
    }

    /// One fixture per `DomainEventPayload` variant, paired with the
    /// `event_type` label it must produce.
    fn every_payload_variant() -> Vec<DomainEventPayload> {
        let title = title_snapshot(MediaFacet::Series);
        vec![
            DomainEventPayload::MediaRequestSubmitted(MediaRequestSubmittedEventData {
                requested_lease_days: None,
                request_id: "req-1".to_string(),
                library_id: "lib-1".to_string(),
                facet: MediaFacet::Series,
                title_name: "Example".to_string(),
                external_ids: Vec::new(),
                poster_url: None,
                year: None,
                requested_quality_profile_id: None,
                requested_quality_profile_name: None,
                requested_monitor_type: None,
            }),
            DomainEventPayload::MediaRequestUpdated(MediaRequestSubmittedEventData {
                requested_lease_days: None,
                request_id: "req-1".to_string(),
                library_id: "lib-1".to_string(),
                facet: MediaFacet::Series,
                title_name: "Example".to_string(),
                external_ids: Vec::new(),
                poster_url: None,
                year: None,
                requested_quality_profile_id: None,
                requested_quality_profile_name: None,
                requested_monitor_type: None,
            }),
            DomainEventPayload::MediaRequestApproved(media_request_resolved()),
            DomainEventPayload::MediaRequestRejected(media_request_resolved()),
            DomainEventPayload::MediaRequestCanceled(media_request_resolved()),
            DomainEventPayload::TitleAdded(TitleAddedEventData {
                title: title.clone(),
            }),
            DomainEventPayload::TitleUpdated(TitleUpdatedEventData {
                title: title.clone(),
            }),
            DomainEventPayload::TitleRematched(TitleRematchedEventData {
                title: title.clone(),
                old_tvdb_id: None,
                new_tvdb_id: "123".to_string(),
                smg_id: None,
                tmdb_id: None,
                source: "manual".to_string(),
            }),
            DomainEventPayload::TitleDeleted(TitleDeletedEventData {
                title: title.clone(),
            }),
            DomainEventPayload::ConfigurationChanged(ConfigurationChangedEventData {
                resource_type: "indexer".to_string(),
                resource_id: None,
                action: ConfigurationChangeAction::Saved,
            }),
            DomainEventPayload::DiscoverySearchCompleted(DiscoverySearchCompletedEventData {
                search_type: "series".to_string(),
                query: None,
                result_count: 3,
            }),
            DomainEventPayload::MetadataHydrationUpdated(MetadataHydrationUpdatedEventData {
                title: title.clone(),
                state: MetadataHydrationState::Completed,
                reason: None,
            }),
            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                title: title.clone(),
                source_title: None,
                source_hint: None,
                source_provider: None,
                download_id: None,
                episode_ids: Vec::new(),
            }),
            DomainEventPayload::DownloadFailed(download_failed(Some("sabnzbd"))),
            DomainEventPayload::ReleaseBlocklisted(ReleaseBlocklistedEventData {
                title: Some(title.clone()),
                source_title: None,
                source_hint: None,
                download_id: None,
                client_id: None,
                client_name: None,
                client_type: None,
                quality: None,
                reason: None,
                episode_ids: Vec::new(),
                collection_id: None,
            }),
            DomainEventPayload::ImportCompleted(import_completed(MediaFacet::Series, Some(1))),
            DomainEventPayload::ImportRejected(import_rejected(None)),
            DomainEventPayload::MediaFileImported(MediaFileImportedEventData {
                title: title.clone(),
                media_updates: Vec::new(),
            }),
            DomainEventPayload::MediaFileAnalyzed(MediaFileAnalyzedEventData {
                title: title.clone(),
                media_updates: Vec::new(),
                file_id: "file-1".to_string(),
                analysis_status: "completed".to_string(),
                episode_ids: Vec::new(),
            }),
            DomainEventPayload::MediaFileRenamed(MediaFileRenamedEventData {
                title: title.clone(),
                media_updates: Vec::new(),
                renamed_count: 1,
                episode_ids: Vec::new(),
            }),
            DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                title: title.clone(),
                media_updates: Vec::new(),
                file_id: None,
                reason: MediaFileDeletedReason::Deleted,
                episode_ids: Vec::new(),
            }),
            DomainEventPayload::MediaFileUpgraded(media_file_upgraded(Some(1))),
            DomainEventPayload::AcquisitionSearchCompleted(AcquisitionSearchCompletedEventData {
                title: title.clone(),
                result_count: 1,
            }),
            DomainEventPayload::AcquisitionCandidateRejected(
                AcquisitionCandidateRejectedEventData {
                    title: title.clone(),
                    source_title: "Example.S01E01".to_string(),
                    reason_code: "cutoff_reached".to_string(),
                },
            ),
            DomainEventPayload::ImportRequested(ImportRequestedEventData {
                title: Some(title.clone()),
                client_type: "sabnzbd".to_string(),
                source_ref: "nzo_1".to_string(),
                request_kind: ImportRequestKind::Manual,
            }),
            DomainEventPayload::ImportRecoveryCompleted(ImportRecoveryCompletedEventData {
                recovered_count: 1,
            }),
            DomainEventPayload::DownloadQueueItemCommandIssued(
                DownloadQueueItemCommandIssuedEventData {
                    item_id: "queue-1".to_string(),
                    action: DownloadQueueCommandAction::Pause,
                },
            ),
            DomainEventPayload::PostProcessingCompleted(PostProcessingCompletedEventData {
                title: title.clone(),
                script_name: "notify.sh".to_string(),
                result: PostProcessingResult::Succeeded,
                exit_code: Some(0),
            }),
            DomainEventPayload::SubtitleDownloaded(SubtitleDownloadedEventData {
                title: title.clone(),
                subtitle_path: None,
                language: None,
                provider: None,
            }),
            DomainEventPayload::SubtitleSearchFailed(SubtitleSearchFailedEventData {
                title: title.clone(),
                language: None,
                reason: None,
            }),
            DomainEventPayload::LibraryScanStarted(LibraryScanStartedEventData {
                session_id: "session-1".to_string(),
                library_id: None,
                mode: "full".to_string(),
            }),
            DomainEventPayload::LibraryScanTitleDiscovered(LibraryScanTitleDiscoveredEventData {
                session_id: "session-1".to_string(),
                title_id: "title-1".to_string(),
                title_name: "Example".to_string(),
                facet: MediaFacet::Series,
                discovered_file_count: 1,
                folder_path: None,
            }),
            DomainEventPayload::LibraryScanDeltaRecorded(LibraryScanDeltaRecordedEventData {
                session_id: "session-1".to_string(),
                found_titles_total: Some(1),
                found_titles_delta: 1,
                title_match_completed_delta: 1,
                title_match_failed_delta: 0,
                title_match_total_known: Some(true),
                metadata_total_delta: 0,
                metadata_completed_delta: 0,
                metadata_failed_delta: 0,
                metadata_total_known: Some(true),
                file_total_delta: 0,
                file_completed_delta: 0,
                file_failed_delta: 0,
                file_total_known: Some(true),
                summary: None,
                summary_is_delta: false,
            }),
            DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: "session-1".to_string(),
                status: "running".to_string(),
                found_titles: 1,
                title_match_completed: 1,
                title_match_total_known: true,
                titles_completed: 1,
                titles_total: Some(1),
                files_completed: 0,
                files_total: Some(0),
                warning_message: None,
            }),
            DomainEventPayload::LibraryScanCompleted(library_scan_completed("completed", None)),
            DomainEventPayload::LibraryScanCanceled(LibraryScanCanceledEventData {
                session_id: "session-1".to_string(),
                status: "canceled".to_string(),
                found_titles: 1,
                title_match_completed: 1,
                title_match_total_known: true,
                titles_completed: 1,
                titles_total: Some(1),
                files_completed: 0,
                files_total: Some(0),
                summary: None,
            }),
            DomainEventPayload::LibraryScanFailed(LibraryScanFailedEventData {
                session_id: "session-1".to_string(),
                error_message: "boom".to_string(),
            }),
            DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                run_id: "run-1".to_string(),
                job_key: "rss_sync".to_string(),
                operation_type: "sync".to_string(),
                trigger_source: "schedule".to_string(),
            }),
            DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                run_id: "run-1".to_string(),
                job_key: "rss_sync".to_string(),
                summary_text: None,
            }),
            DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: "run-1".to_string(),
                job_key: "rss_sync".to_string(),
                error_text: None,
            }),
            DomainEventPayload::JobNextRunUpdated(JobNextRunUpdatedEventData {
                job_key: "rss_sync".to_string(),
                next_run_at: None,
            }),
            DomainEventPayload::DownloadQueueItemUpserted(Box::new(
                DownloadQueueItemUpsertedEventData { item: queue_item() },
            )),
            DomainEventPayload::DownloadQueueItemRemoved(DownloadQueueItemRemovedEventData {
                download_client_item_id: "queue-1".to_string(),
                client_id: None,
                client_type: None,
            }),
            DomainEventPayload::DownloadIgnored(DownloadIgnoredEventData {
                title: Some(title.clone()),
                download_client_item_id: "queue-1".to_string(),
                client_id: None,
                client_type: None,
                source_provider: None,
                source_title: None,
            }),
            DomainEventPayload::SeedingStarted(SeedingStartedEventData {
                title: Some(title.clone()),
                download_client_item_id: "queue-1".to_string(),
                client_id: None,
                client_type: None,
                source_provider: None,
                source_title: None,
                reason: "ratio_goal_unmet".to_string(),
                seed_ratio: None,
                seed_time_seconds: None,
            }),
            DomainEventPayload::SeedingCompleted(SeedingCompletedEventData {
                title: Some(title),
                download_client_item_id: "queue-1".to_string(),
                client_id: None,
                client_type: None,
                source_provider: None,
                source_title: None,
                action: "removed".to_string(),
                reason: "ratio_goal_met".to_string(),
                seed_ratio: None,
                seed_time_seconds: None,
            }),
        ]
    }

    fn media_request_resolved() -> MediaRequestResolvedEventData {
        MediaRequestResolvedEventData {
            decided_by_rule_set_ids: Vec::new(),
            decision_reason_codes: Vec::new(),
            approved_lease_days: None,
            policy_tags: Vec::new(),
            request_id: "req-1".to_string(),
            library_id: "lib-1".to_string(),
            facet: MediaFacet::Series,
            title_name: "Example".to_string(),
            external_ids: Vec::new(),
            created_title_id: None,
            requested_quality_profile_id: None,
            requested_quality_profile_name: None,
            requested_monitor_type: None,
            approved_quality_profile_id: None,
            approved_quality_profile_name: None,
        }
    }

    #[test]
    fn every_payload_variant_increments_the_generic_event_counter_exactly_once() {
        let payloads = every_payload_variant();
        let expected_event_types = payloads
            .iter()
            .map(|payload| payload.event_type().as_str())
            .collect::<Vec<_>>();

        // A fixture per variant: if a variant is added without one, this trips.
        let mut deduped = expected_event_types.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(
            deduped.len(),
            expected_event_types.len(),
            "duplicate event types in the fixture set"
        );

        let events = payloads.into_iter().map(event).collect::<Vec<_>>();
        let recorded = record(&events);
        let generic = series(&recorded, DOMAIN_EVENTS_TOTAL);

        assert_eq!(generic.len(), expected_event_types.len());
        for event_type in expected_event_types {
            assert_eq!(
                value_with(
                    &recorded,
                    DOMAIN_EVENTS_TOTAL,
                    &[("event_type", event_type)]
                ),
                Some(1),
                "event type {event_type}"
            );
        }
    }

    #[test]
    fn describe_domain_event_metrics_registers_every_family() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, || {
            describe_domain_event_metrics();
            // Descriptions alone are not "seen" metrics, so touch each family.
            for event in every_payload_variant().into_iter().map(event) {
                record_domain_event_metrics(&event);
            }
            record_domain_event_metrics(&event(DomainEventPayload::LibraryScanCompleted(
                library_scan_completed("completed", Some(library_scan_summary())),
            )));
        });

        let names = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .map(|(key, _, _, _)| key.key().name().to_string())
            .collect::<std::collections::BTreeSet<_>>();

        for family in [
            DOMAIN_EVENTS_TOTAL,
            ACQUISITION_SEARCHES_COMPLETED_TOTAL,
            ACQUISITION_SEARCH_RESULTS_TOTAL,
            ACQUISITION_CANDIDATES_REJECTED_TOTAL,
            DOWNLOADS_FAILED_TOTAL,
            RELEASES_BLOCKLISTED_TOTAL,
            DOWNLOADS_IGNORED_TOTAL,
            IMPORTS_TOTAL,
            IMPORT_FILES_TOTAL,
            IMPORT_BYTES_TOTAL,
            IMPORT_REJECTIONS_TOTAL,
            MEDIA_FILE_UPGRADES_TOTAL,
            MEDIA_FILE_UPGRADE_BYTES_TOTAL,
            MEDIA_FILES_DELETED_TOTAL,
            LIBRARY_SCANS_TOTAL,
            LIBRARY_SCAN_ITEMS_TOTAL,
            JOB_RUNS_TOTAL,
            SUBTITLES_DOWNLOADED_TOTAL,
            SUBTITLE_SEARCH_FAILURES_TOTAL,
        ] {
            assert!(names.contains(family), "missing family {family}");
        }
    }
}
