//! Lazy, bounded per-title file projections. Cached manifests are instructions,
//! never verification proofs; completed state comes from durable file records.
use super::*;
use crate::location::model::{FileVerificationOutcome, LocationOperationState};
use crate::location::root_move::{RootMoveExecutionPlan, RootMoveFileExecution};
use std::collections::VecDeque;

type Manifest = BTreeMap<String, Vec<RootMoveFileExecution>>;
pub(super) type ManifestCache = VecDeque<(String, Arc<Manifest>)>;

#[derive(Clone, Debug)]
pub struct TransferFile {
    pub reason_code: Option<String>,
    pub original_destination_path: String,
    pub verification_total_bytes: u64,
    pub source_path: String,
    pub destination_path: String,
    pub size_bytes: u64,
    pub state: String,
    pub copy_bytes: u64,
    pub verification_bytes: u64,
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LocationOperationRepository;
    use crate::location::model::{
        AppliedVerificationDepth, FileVerificationRecord, LocationExecutionMode,
        LocationOperationType, VerificationDepth,
    };
    use crate::location::test_support::{InMemoryLocationOperationStore, queued_operation};

    #[tokio::test]
    async fn file_pages_show_all_work_and_only_durable_proofs_survive_restart() {
        let (mut app, user) = crate::lib_tests::bootstrap();
        let store = Arc::new(InMemoryLocationOperationStore::new());
        let mut operation = queued_operation(
            "op",
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
            VerificationDepth::Full,
        );
        operation.state = LocationOperationState::Moving;
        store.insert_operation(operation.clone());
        app.services.library.location_operations = store.clone();
        let hub = &app.runtime.library.location_runners.transfers;
        let files: Vec<_> = (0..120)
            .map(|index| RootMoveFileExecution {
                media_file_id: None,
                source_path: format!("/source/{index}.mkv"),
                destination_path: format!("/destination/{index}.mkv"),
                size_bytes: 100,
            })
            .collect();
        let plan = RootMoveExecutionPlan {
            titles: vec![crate::location::root_move::RootMoveTitleExecution {
                title_id: "title".into(),
                title_name: "Title".into(),
                sequence: 0,
                class: crate::location::classify::TitleLocationClass::RootMove,
                source_library_id: "library".into(),
                source_root_id: "source".into(),
                source_folder_path: Some("/source".into()),
                destination_library_id: "library".into(),
                destination_root_id: "destination".into(),
                destination_folder_path: Some("/destination".into()),
                destination_root_path: Some("/destination".into()),
                source_root_path: Some("/source".into()),
                same_volume: None,
                files,
                deduplicated_sources: vec!["/source/duplicate.srt".into()],
                renamed_destinations: vec![],
                prune_directories: vec![],
                warnings: vec![],
                converted_facet: None,
                dropped_tag_prefixes: vec![],
                merge_target_title_id: None,
            }],
            ..Default::default()
        };
        store
            .create_location_operation(&operation, Some(&serde_json::to_string(&plan).unwrap()))
            .await
            .unwrap();
        hub.file_update(
            "op",
            "title",
            "/destination/0.mkv",
            100,
            scryer_domain::ImportTransferPhase::Copying,
            75,
        );
        hub.file_update(
            "op",
            "title",
            "/destination/1.mkv",
            100,
            scryer_domain::ImportTransferPhase::Verifying,
            25,
        );
        hub.file_failed("op", "title", "/destination/2.mkv", 100);
        store
            .record_location_file_verification(&FileVerificationRecord {
                operation_id: "op".into(),
                title_id: "title".into(),
                media_file_id: None,
                source_path: "/source/3.mkv".into(),
                destination_path: "/destination/3.mkv".into(),
                hashes: None,
                depth: AppliedVerificationDepth::exact(VerificationDepth::Quick),
                outcome: FileVerificationOutcome::Verified,
                detail: None,
                verified_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        hub.file_waiting_for_storage("op", "title", "/destination/4.mkv", 100);
        let first = app
            .location_transfer_files_snapshot(&user, "op", "title", 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.total_count, 121);
        assert_eq!(first.files.len(), 50);
        assert!(first.has_more);
        assert_eq!(
            first
                .files
                .iter()
                .take(5)
                .map(|file| file.state.as_str())
                .collect::<Vec<_>>(),
            [
                "MOVING",
                "VERIFYING",
                "FAILED",
                "DONE",
                "WAITING_FOR_STORAGE"
            ]
        );
        assert_eq!(first.files[0].copy_bytes, 75);
        assert_eq!(first.files[1].verification_bytes, 25);
        let last = app
            .location_transfer_files_snapshot(&user, "op", "title", 100)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(last.files.len(), 21);
        assert_eq!(last.files[20].source_path, "/source/duplicate.srt");
        assert_eq!(last.files[20].state, "DUPLICATE");
        assert_eq!(last.files[19].source_path, "/source/119.mkv");
        assert!(!last.has_more);
        let other = app
            .location_transfer_files_snapshot(&user, "op", "other", 0)
            .await
            .unwrap()
            .unwrap();
        assert!(other.files.is_empty());
        hub.file_update(
            "op",
            "title",
            "/destination/3.mkv",
            100,
            scryer_domain::ImportTransferPhase::Waiting,
            0,
        );
        let retrying = app
            .location_transfer_files_snapshot(&user, "op", "title", 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrying.files[3].state, "QUEUED");
        hub.finish("op");
        hub.manifests.lock().await.clear();
        let restarted = app
            .location_transfer_files_snapshot(&user, "op", "title", 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restarted.files[1].state, "QUEUED");
        assert_eq!(restarted.files[1].verification_bytes, 0);
        assert_eq!(restarted.files[3].state, "DONE");
        operation.state = LocationOperationState::Canceled;
        store.insert_operation(operation);
        let canceled = app
            .location_transfer_files_snapshot(&user, "op", "title", 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(canceled.files[1].state, "CANCELED");
        assert_eq!(canceled.files[3].state, "DONE");
    }
}

impl crate::AppUseCase {
    pub async fn location_transfer_files_snapshot(
        &self,
        actor: &scryer_domain::User,
        operation_id: &str,
        title_id: &str,
        offset: i64,
    ) -> crate::AppResult<Option<TransferSnapshot>> {
        // Performs the same authorization and captures the starting revision
        // used by summary/page subscriptions and their polling fallbacks.
        let Some(mut snapshot) = self
            .location_transfer_snapshot(actor, operation_id, None, 50)
            .await?
        else {
            return Ok(None);
        };
        let store = &self.services.library.location_operations;
        let hub = &self.runtime.library.location_runners.transfers;
        let manifest = {
            let mut cache = hub.manifests.lock().await;
            if let Some(index) = cache.iter().position(|(id, _)| id == operation_id) {
                let entry = cache.remove(index).expect("known manifest");
                let manifest = entry.1.clone();
                cache.push_back(entry);
                manifest
            } else {
                let json = store.get_location_operation_plan_json(operation_id).await?;
                let manifest =
                    tokio::task::spawn_blocking(move || -> crate::AppResult<Manifest> {
                        let Some(json) = json else {
                            return Ok(BTreeMap::new());
                        };
                        let plan: RootMoveExecutionPlan = serde_json::from_str(&json)
                            .map_err(|error| crate::AppError::Repository(error.to_string()))?;
                        Ok(plan
                            .titles
                            .into_iter()
                            .map(|title| {
                                let mut files = title.files;
                                // Duplicates are part of the confirmed file work too,
                                // but have no destination write or read-back phase.
                                files.extend(title.deduplicated_sources.into_iter().map(
                                    |source_path| RootMoveFileExecution {
                                        source_path,
                                        destination_path: String::new(),
                                        size_bytes: 0,
                                        media_file_id: None,
                                    },
                                ));
                                (title.title_id, files)
                            })
                            .collect())
                    })
                    .await
                    .map_err(|error| crate::AppError::Repository(error.to_string()))??;
                let manifest = Arc::new(manifest);
                // Avoid retaining an unbounded history of large plans.
                if cache.len() >= 4 {
                    cache.pop_front();
                }
                cache.push_back((operation_id.to_owned(), manifest.clone()));
                manifest
            }
        };
        let files = manifest.get(title_id).map(Vec::as_slice).unwrap_or(&[]);
        let offset = usize::try_from(offset.max(0)).unwrap_or(usize::MAX);
        let page: Vec<_> = files.iter().skip(offset).take(50).collect();
        let sources: Vec<_> = page.iter().map(|file| file.source_path.clone()).collect();
        let resolutions = store
            .file_resolutions_for_sources(operation_id, title_id, &sources)
            .await?;
        let resolved: BTreeMap<_, _> = resolutions
            .iter()
            .map(|row| (row.source_path.as_str(), row))
            .collect();
        let paths: Vec<_> = page
            .iter()
            .map(|file| {
                resolved.get(file.source_path.as_str()).map_or_else(
                    || file.destination_path.clone(),
                    |row| row.destination_path.clone(),
                )
            })
            .collect();
        let proofs: BTreeMap<_, _> = store
            .location_file_verifications_for_paths(operation_id, title_id, &paths)
            .await?
            .into_iter()
            .map(|record| (record.destination_path.clone(), record))
            .collect();
        let title = store.transfer_title(operation_id, title_id).await?;
        let state = hub.state.lock().unwrap_or_else(|error| error.into_inner());
        let telemetry = state.operations.get(operation_id);
        snapshot.files = page
            .into_iter()
            .map(|file| {
                let key = (title_id.to_owned(), file.destination_path.clone());
                let live = telemetry.and_then(|op| op.files.get(&key));
                let failed = telemetry.is_some_and(|op| op.abandoned_work.contains_key(&key));
                let resolution = resolved.get(file.source_path.as_str());
                let destination = resolution.map_or(file.destination_path.as_str(), |row| {
                    row.destination_path.as_str()
                });
                let proof = proofs.get(destination);
                let active = live.filter(|live| !live.done);
                let verified =
                    proof.is_some_and(|proof| proof.outcome == FileVerificationOutcome::Verified);
                let status = match active.and_then(|file| file.phase) {
                    _ if active.is_some_and(|file| file.waiting_for_storage) => {
                        "WAITING_FOR_STORAGE"
                    }
                    _ if active.is_some_and(|file| file.comparing) => "COMPARING",
                    _ if file.destination_path.is_empty() => "DUPLICATE",
                    Some(scryer_domain::ImportTransferPhase::Verifying) => "VERIFYING",
                    Some(scryer_domain::ImportTransferPhase::Copying) => "MOVING",
                    Some(scryer_domain::ImportTransferPhase::Waiting) => "QUEUED",
                    _ if failed => "FAILED",
                    _ if verified => "DONE",
                    _ if proof.is_some() => "FAILED",
                    _ if title
                        .as_ref()
                        .is_some_and(|title| title.state == TitleCheckpointState::Blocked) =>
                    {
                        "BLOCKED"
                    }
                    _ if snapshot.operation.state == LocationOperationState::Canceled => "CANCELED",
                    _ if snapshot.operation.state.is_terminal() => "NOT_PROCESSED",
                    _ => "QUEUED",
                };
                TransferFile {
                    reason_code: resolution
                        .and_then(|row| row.reason_code.clone())
                        .or_else(|| {
                            (failed
                                || proof
                                    .is_some_and(|proof| !proof.outcome.permits_source_removal()))
                            .then(|| "file_transfer_failed".into())
                        }),
                    original_destination_path: file.destination_path.clone(),
                    verification_total_bytes: active
                        .filter(|file| file.comparing)
                        .map_or(file.size_bytes, |file| file.comparison_total),
                    source_path: file.source_path.clone(),
                    destination_path: destination.to_owned(),
                    size_bytes: file.size_bytes,
                    state: status.to_owned(),
                    copy_bytes: live.map_or(if verified { file.size_bytes } else { 0 }, |file| {
                        file.copied
                    }),
                    verification_bytes: live.map_or(
                        if verified { file.size_bytes } else { 0 },
                        |file| {
                            if file.comparing && !file.done {
                                file.compared
                            } else {
                                file.verified
                            }
                        },
                    ),
                    detail: resolution
                        .and_then(|row| row.warning.clone())
                        .or_else(|| proof.and_then(|proof| proof.detail.clone())),
                }
            })
            .collect();
        snapshot.total_count = files.len() as i64;
        snapshot.has_more = offset.saturating_add(snapshot.files.len()) < files.len();
        Ok(Some(snapshot))
    }
}
