//! Executor-path tests over real temp-directory fixtures.
//!
//! These drive the committed runner ([`LocationOperationRunner`]) through the
//! seams in the parent module against real files, so the assertions are about
//! what ends up on disk and in the catalog rather than about mock call counts.

use super::*;

use std::sync::Mutex;

use crate::location::executor::LocationOperationRunner;
use crate::location::model::{
    LocationExecutionMode, LocationOperationState, LocationOperationType, TitleCheckpointState,
};
use crate::location::root_move::{RootMoveFileExecution, RootMoveTitleExecution};
use crate::location::test_support::{InMemoryLocationOperationStore, queued_operation};
use crate::location::verify::{CacheBypass, ReadBackHandle};

// ── Fixtures ────────────────────────────────────────────────────────────────

/// A fake catalog that records the writes a reconcile made, in order, so
/// "ownership flipped only after verification" is checkable as a sequence.
#[derive(Default)]
struct FakeCatalog {
    state: Mutex<FakeCatalogState>,
}

#[derive(Default)]
struct FakeCatalogState {
    placements: BTreeMap<String, TitlePlacementSnapshot>,
    writes: Vec<String>,
}

impl FakeCatalog {
    fn with_title(title_id: &str, placement: TitlePlacementSnapshot) -> Self {
        let catalog = Self::default();
        catalog
            .state
            .lock()
            .expect("lock")
            .placements
            .insert(title_id.to_string(), placement);
        catalog
    }

    fn writes(&self) -> Vec<String> {
        self.state.lock().expect("lock").writes.clone()
    }

    fn placement(&self, title_id: &str) -> Option<TitlePlacementSnapshot> {
        self.state
            .lock()
            .expect("lock")
            .placements
            .get(title_id)
            .cloned()
    }

    fn forget(&self, title_id: &str) {
        self.state.lock().expect("lock").placements.remove(title_id);
    }
}

#[async_trait]
impl RootMoveCatalog for FakeCatalog {
    async fn title_placement(&self, title_id: &str) -> AppResult<Option<TitlePlacementSnapshot>> {
        Ok(self.placement(title_id))
    }

    async fn set_media_file_path(&self, media_file_id: &str, stored_path: &str) -> AppResult<()> {
        self.state
            .lock()
            .expect("lock")
            .writes
            .push(format!("media:{media_file_id}={stored_path}"));
        Ok(())
    }

    async fn set_title_folder_path(&self, title_id: &str, stored_path: &str) -> AppResult<()> {
        let mut state = self.state.lock().expect("lock");
        state
            .writes
            .push(format!("folder:{title_id}={stored_path}"));
        if let Some(placement) = state.placements.get_mut(title_id) {
            placement.folder_path = Some(stored_path.to_string());
        }
        Ok(())
    }

    async fn set_title_root(&self, title_id: &str, root_folder_id: &str) -> AppResult<()> {
        let mut state = self.state.lock().expect("lock");
        state.writes.push(format!("root:{title_id}={root_folder_id}"));
        if let Some(placement) = state.placements.get_mut(title_id) {
            placement.root_folder_id = root_folder_id.to_string();
        }
        Ok(())
    }

    async fn set_media_file_content_hashes(
        &self,
        media_file_id: &str,
        hashes: &crate::location::model::PersistedContentHashes,
    ) -> AppResult<()> {
        self.state
            .lock()
            .expect("lock")
            .writes
            .push(format!("hashes:{media_file_id}={}", hashes.full_blake3));
        Ok(())
    }
}

/// A recycler that records what it was handed instead of touching a bin.
#[derive(Default)]
struct RecordingRecycler {
    recycled: Mutex<Vec<String>>,
}

impl RecordingRecycler {
    fn recycled(&self) -> Vec<String> {
        self.recycled.lock().expect("lock").clone()
    }
}

#[async_trait]
impl SourceRecycler for RecordingRecycler {
    async fn recycle_source(
        &self,
        _operation_id: &str,
        _title: &RootMoveTitleExecution,
        source: &Path,
        _media_file_id: Option<&str>,
        _size_bytes: u64,
    ) -> AppResult<SourceDisposal> {
        if tokio::fs::symlink_metadata(source).await.is_err() {
            return Ok(SourceDisposal::AlreadyAbsent);
        }
        self.recycled
            .lock()
            .expect("lock")
            .push(path_to_stored_string(source));
        tokio::fs::remove_file(source)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        Ok(SourceDisposal::Recycled)
    }
}

/// Records every path whose permissions were applied, so FR-031's
/// "apply configured permissions at the destination" is observable.
#[derive(Default)]
struct RecordingPermissions {
    files: Mutex<Vec<String>>,
    directories: Mutex<Vec<String>>,
}

#[async_trait]
impl PlacedContentPermissions for RecordingPermissions {
    async fn apply_to_file(&self, path: &Path) -> AppResult<()> {
        self.files
            .lock()
            .expect("lock")
            .push(path_to_stored_string(path));
        Ok(())
    }

    async fn apply_to_directory(&self, path: &Path) -> AppResult<()> {
        self.directories
            .lock()
            .expect("lock")
            .push(path_to_stored_string(path));
        Ok(())
    }
}

fn write_file(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, contents).expect("write fixture");
}

/// One title, one media file, moving from `<root>/a/Movie` to
/// `<root>/b/Movie`.
fn single_title_plan(
    source_root: &Path,
    destination_root: &Path,
    file_name: &str,
    size_bytes: u64,
) -> RootMoveExecutionPlan {
    let source_folder = source_root.join("Movie");
    let destination_folder = destination_root.join("Movie");
    RootMoveExecutionPlan {
        titles: vec![RootMoveTitleExecution {
            title_id: "title-1".to_string(),
            title_name: "Movie".to_string(),
            sequence: 0,
            class: crate::location::classify::TitleLocationClass::RootMove,
            source_library_id: "lib-1".to_string(),
            source_root_id: "root-a".to_string(),
            source_folder_path: Some(path_to_stored_string(&source_folder)),
            destination_library_id: "lib-1".to_string(),
            destination_root_id: "root-b".to_string(),
            destination_folder_path: Some(path_to_stored_string(&destination_folder)),
            destination_root_path: Some(path_to_stored_string(destination_root)),
            source_root_path: Some(path_to_stored_string(source_root)),
            same_volume: None,
            files: vec![RootMoveFileExecution {
                media_file_id: Some("mf-1".to_string()),
                source_path: path_to_stored_string(&source_folder.join(file_name)),
                destination_path: path_to_stored_string(&destination_folder.join(file_name)),
                size_bytes,
            }],
            deduplicated_sources: Vec::new(),
            prune_directories: vec![path_to_stored_string(&source_folder)],
            warnings: Vec::new(),
        }],
    }
}

fn placement_for(plan: &RootMoveExecutionPlan) -> TitlePlacementSnapshot {
    let title = &plan.titles[0];
    TitlePlacementSnapshot {
        root_folder_id: title.source_root_id.clone(),
        library_id: title.source_library_id.clone(),
        folder_path: title.source_folder_path.clone(),
        media_file_paths: title
            .files
            .iter()
            .filter(|file| file.media_file_id.is_some())
            .map(|file| file.source_path.clone())
            .collect(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// FR-032: a move inside one filesystem is a rename. No bytes are copied, so
/// the record carries no hashes and no verification pass ran — and the source
/// is gone because the rename moved it, not because cleanup removed it.
#[tokio::test]
async fn same_filesystem_moves_rename_and_skip_verification() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_root = temp.path().join("a");
    let destination_root = temp.path().join("b");
    let plan = single_title_plan(&source_root, &destination_root, "movie.mkv", 11);
    write_file(&plan.titles[0].files[0].source(), b"hello world");

    let store = InMemoryLocationOperationStore::new();
    store.insert_operation(queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::MoveWithScryer,
        VerificationDepth::Full,
    ));
    let catalog = FakeCatalog::with_title("title-1", placement_for(&plan));
    let recycler = RecordingRecycler::default();
    let mover = RootMoveFileMover::without_permissions();
    let admission = RootMoveAdmission::new(&plan, &catalog);
    let reconciler = RootMoveReconciler::new(&plan, &catalog, &store, &recycler);

    let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
        .run("op-1", &plan.to_work_plan())
        .await
        .expect("run");

    assert_eq!(outcome.state, LocationOperationState::Completed);
    let destination = plan.titles[0].files[0].destination();
    assert_eq!(
        std::fs::read(&destination).expect("destination readable"),
        b"hello world"
    );
    assert!(!plan.titles[0].files[0].source().exists());

    let records = store.verifications();
    assert_eq!(records.len(), 1);
    assert!(
        records[0].hashes.is_none(),
        "a rename copies no bytes, so there is nothing to hash"
    );
    assert!(records[0].outcome.permits_source_removal());
    // Nothing was recycled: the rename already moved the only copy.
    assert!(recycler.recycled().is_empty());
    // The empty source folder was removed (FR-031).
    assert!(!source_root.join("Movie").exists());
}

/// FR-032/FR-041: a same-filesystem rename copies no bytes, so there are no
/// hashes to persist. Writing one anyway would put an unfounded value on the
/// media file and take it off the backfill queue.
#[tokio::test]
async fn a_rename_placement_persists_no_content_hashes() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_root = temp.path().join("a");
    let destination_root = temp.path().join("b");
    let plan = single_title_plan(&source_root, &destination_root, "movie.mkv", 11);
    write_file(&plan.titles[0].files[0].source(), b"hello world");
    std::fs::create_dir_all(
        plan.titles[0].files[0]
            .destination()
            .parent()
            .expect("destination parent"),
    )
    .expect("create destination folder");

    let catalog = Arc::new(FakeCatalog::with_title("title-1", placement_for(&plan)));
    let mover = RootMoveFileMover::without_permissions().with_catalog(catalog.clone());
    let work_plan = plan.to_work_plan();
    let planned_title = &work_plan.titles[0];

    let verified = mover
        .move_file(FileMoveRequest {
            operation_id: "op-1",
            title: planned_title,
            file: &planned_title.files[0],
            depth: VerificationDepth::Full,
        })
        .await
        .expect("rename placement");

    assert!(verified.permits_source_removal());
    assert!(
        verified.hashes.is_none(),
        "a rename streams no bytes through the hashers"
    );
    assert!(
        !catalog
            .writes()
            .iter()
            .any(|write| write.starts_with("hashes:")),
        "a rename must not write content hashes: {:?}",
        catalog.writes()
    );
}

/// US2.5 / FR-031: a cross-filesystem move copies, verifies with the real
/// copier, then flips the catalog, then recycles the source — in that order.
#[tokio::test]
async fn cross_filesystem_moves_copy_verify_then_flip_the_catalog() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_root = temp.path().join("a");
    let destination_root = temp.path().join("b");
    let mut plan = single_title_plan(&source_root, &destination_root, "movie.mkv", 11);
    // Force the copy path: the fixture is one filesystem, so the mover would
    // otherwise rename. Pointing the source at a path the mover must copy from
    // is not possible in a temp dir, so the copier is exercised directly and
    // the runner is driven with a mover that always copies.
    plan.titles[0].same_volume = Some(false);
    write_file(&plan.titles[0].files[0].source(), b"hello world");

    struct AlwaysCopy {
        copier: VerifiedCopier,
        permissions: Arc<RecordingPermissions>,
    }

    #[async_trait]
    impl TitleFileMover for AlwaysCopy {
        async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
            if let Some(parent) = request.file.destination_path.parent() {
                tokio::fs::create_dir_all(parent).await.expect("mkdir");
                self.permissions.apply_to_directory(parent).await?;
            }
            let verified = self
                .copier
                .copy_and_verify(VerifiedCopyRequest {
                    source: request.file.source_path.clone(),
                    destination: request.file.destination_path.clone(),
                    depth: request.depth,
                    claim: DestinationClaim::ClaimHere,
                })
                .await?;
            if verified.permits_source_removal() {
                self.permissions
                    .apply_to_file(&request.file.destination_path)
                    .await?;
            }
            Ok(verified)
        }
    }

    let store = InMemoryLocationOperationStore::new();
    store.insert_operation(queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::MoveWithScryer,
        VerificationDepth::Full,
    ));
    let catalog = FakeCatalog::with_title("title-1", placement_for(&plan));
    let recycler = RecordingRecycler::default();
    let permissions = Arc::new(RecordingPermissions::default());
    let mover = AlwaysCopy {
        copier: VerifiedCopier::new(),
        permissions: permissions.clone(),
    };
    let admission = RootMoveAdmission::new(&plan, &catalog);
    let reconciler = RootMoveReconciler::new(&plan, &catalog, &store, &recycler);

    let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
        .run("op-1", &plan.to_work_plan())
        .await
        .expect("run");

    assert_eq!(outcome.state, LocationOperationState::Completed);
    let records = store.verifications();
    assert_eq!(records.len(), 1);
    let hashes = records[0].hashes.as_ref().expect("a copy streams hashes");
    assert_eq!(hashes.size_bytes, 11);
    assert!(!hashes.full_blake3.is_empty());
    assert!(records[0].outcome.permits_source_removal());
    assert_eq!(records[0].depth.applied, VerificationDepth::Full);

    // Catalog ownership flipped, and only after the file was verified.
    assert_eq!(
        catalog.writes(),
        vec![
            format!(
                "media:mf-1={}",
                plan.titles[0].files[0].destination_path
            ),
            format!(
                "folder:title-1={}",
                plan.titles[0]
                    .destination_folder_path
                    .clone()
                    .expect("destination folder")
            ),
            "root:title-1=root-b".to_string(),
        ]
    );

    // The verified-redundant source copy was recycled, not deleted silently.
    assert_eq!(
        recycler.recycled(),
        vec![plan.titles[0].files[0].source_path.clone()]
    );
    assert!(!source_root.join("Movie").exists(), "empty source dir pruned");

    // FR-031: configured permissions were applied to the placed content.
    assert_eq!(
        permissions.files.lock().expect("lock").len(),
        1,
        "the placed file gets the configured modes"
    );
    assert!(!permissions.directories.lock().expect("lock").is_empty());
}

/// SC-006 at the lib level: a byte flipped after the write is detected at full
/// depth, the operation fails, and the source survives untouched (FR-044).
#[tokio::test]
async fn corruption_at_full_depth_blocks_source_removal() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_root = temp.path().join("a");
    let destination_root = temp.path().join("b");
    let plan = single_title_plan(&source_root, &destination_root, "movie.mkv", 11);
    write_file(&plan.titles[0].files[0].source(), b"hello world");

    /// Copies, then flips a byte at the destination before verification reads
    /// it back — the injection SC-006 describes.
    struct CorruptingMover {
        copier: VerifiedCopier,
    }

    #[async_trait]
    impl TitleFileMover for CorruptingMover {
        async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
            let destination = request.file.destination_path.clone();
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent).await.expect("mkdir");
            }
            let hashes = self
                .copier
                .copy(
                    &request.file.source_path,
                    &destination,
                    DestinationClaim::ClaimHere,
                )
                .await?;
            let mut bytes = std::fs::read(&destination).expect("read back");
            bytes[0] ^= 0xFF;
            std::fs::write(&destination, &bytes).expect("corrupt");

            let assessment = self
                .copier
                .verify(
                    &request.file.source_path,
                    &destination,
                    &hashes,
                    request.depth,
                )
                .await?;
            Ok(VerifiedFile {
                source_path: request.file.source_path.clone(),
                destination_path: destination,
                hashes: Some(hashes),
                depth: assessment.depth,
                outcome: assessment.outcome,
                detail: assessment.detail,
            })
        }
    }

    let store = InMemoryLocationOperationStore::new();
    store.insert_operation(queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::MoveWithScryer,
        VerificationDepth::Full,
    ));
    let catalog = FakeCatalog::with_title("title-1", placement_for(&plan));
    let recycler = RecordingRecycler::default();
    let mover = CorruptingMover {
        copier: VerifiedCopier::new(),
    };
    let admission = RootMoveAdmission::new(&plan, &catalog);
    let reconciler = RootMoveReconciler::new(&plan, &catalog, &store, &recycler);

    let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
        .run("op-1", &plan.to_work_plan())
        .await
        .expect("run");

    assert_eq!(outcome.state, LocationOperationState::Failed);
    let records = store.verifications();
    assert_eq!(records.len(), 1);
    assert!(!records[0].outcome.permits_source_removal());

    // The source is intact and the catalog never moved.
    assert!(plan.titles[0].files[0].source().exists());
    assert!(recycler.recycled().is_empty());
    assert!(catalog.writes().is_empty());
    assert_eq!(
        catalog
            .placement("title-1")
            .expect("title still known")
            .root_folder_id,
        "root-a"
    );
}

/// SC-002's lib-level core: an interrupted run resumes and repeats no verified
/// work. The second run must not copy a file whose destination is already
/// recorded verified (FR-092).
#[tokio::test]
async fn resume_never_recopies_a_verified_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_root = temp.path().join("a");
    let destination_root = temp.path().join("b");
    let plan = single_title_plan(&source_root, &destination_root, "movie.mkv", 11);
    write_file(&plan.titles[0].files[0].source(), b"hello world");

    /// Fails the whole run after the first file is verified, standing in for a
    /// process that died mid-operation.
    struct FailAfterFirstFile {
        inner: RootMoveFileMover,
        moved: Mutex<Vec<String>>,
        fail_after: bool,
    }

    #[async_trait]
    impl TitleFileMover for FailAfterFirstFile {
        async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
            self.moved
                .lock()
                .expect("lock")
                .push(path_to_stored_string(&request.file.source_path));
            let verified = self.inner.move_file(request).await?;
            if self.fail_after {
                // The verification record has not been written yet, so record it
                // the way the runner would have, then abort.
                return Ok(verified);
            }
            Ok(verified)
        }
    }

    let store = InMemoryLocationOperationStore::new();
    store.insert_operation(queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::MoveWithScryer,
        VerificationDepth::Full,
    ));
    let catalog = FakeCatalog::with_title("title-1", placement_for(&plan));
    let recycler = RecordingRecycler::default();

    // First run: a reconciler that fails, so the title never settles even
    // though its file was moved and its verification recorded.
    struct FailingReconciler;

    #[async_trait]
    impl TitleReconciler for FailingReconciler {
        async fn reconcile_title(
            &self,
            _operation: &LocationOperation,
            _title: &PlannedTitle,
        ) -> AppResult<TitleStepOutcome> {
            Err(AppError::Repository("interrupted".to_string()))
        }
    }

    let first_mover = FailAfterFirstFile {
        inner: RootMoveFileMover::without_permissions(),
        moved: Mutex::new(Vec::new()),
        fail_after: true,
    };
    let admission = RootMoveAdmission::new(&plan, &catalog);
    let failing = FailingReconciler;
    let first = LocationOperationRunner::new(&store, &first_mover, &admission, &failing)
        .run("op-1", &plan.to_work_plan())
        .await
        .expect("first run");
    assert_eq!(first.state, LocationOperationState::Failed);
    assert_eq!(first_mover.moved.lock().expect("lock").len(), 1);
    assert_eq!(store.verifications().len(), 1);
    assert_eq!(
        store
            .checkpoint("op-1", "title-1")
            .expect("checkpoint")
            .state,
        TitleCheckpointState::Failed
    );

    // The interrupted operation is resumable: reset it to a non-terminal state
    // the way a restart's reconciliation would.
    let mut resumed = store.operation("op-1").expect("operation");
    resumed.state = LocationOperationState::Queued;
    resumed.completed_at = None;
    store.insert_operation(resumed);

    // Second run: the source is gone (the rename moved it) and the destination
    // is already verified, so the mover must not be called at all.
    let second_mover = FailAfterFirstFile {
        inner: RootMoveFileMover::without_permissions(),
        moved: Mutex::new(Vec::new()),
        fail_after: false,
    };
    let reconciler = RootMoveReconciler::new(&plan, &catalog, &store, &recycler);
    let second = LocationOperationRunner::new(&store, &second_mover, &admission, &reconciler)
        .run("op-1", &plan.to_work_plan())
        .await
        .expect("second run");

    assert_eq!(second.state, LocationOperationState::Completed);
    assert!(
        second_mover.moved.lock().expect("lock").is_empty(),
        "a verified destination is never moved again (FR-092)"
    );
    assert_eq!(store.verifications().len(), 1);
    assert_eq!(catalog.writes().len(), 3);
    assert_eq!(store.open_claim_count(), 0, "a finished operation owns nothing");
}

/// T033: a fileless title completes with no filesystem work at all — no
/// verification records, no recycling, just the root reference.
#[tokio::test]
async fn catalog_only_titles_complete_without_touching_the_filesystem() {
    let plan = RootMoveExecutionPlan {
        titles: vec![RootMoveTitleExecution {
            title_id: "title-1".to_string(),
            title_name: "Fileless".to_string(),
            sequence: 0,
            class: crate::location::classify::TitleLocationClass::CatalogOnly,
            source_library_id: "lib-1".to_string(),
            source_root_id: "root-a".to_string(),
            source_folder_path: None,
            destination_library_id: "lib-1".to_string(),
            destination_root_id: "root-b".to_string(),
            destination_folder_path: None,
            destination_root_path: None,
            source_root_path: None,
            same_volume: None,
            files: Vec::new(),
            deduplicated_sources: Vec::new(),
            prune_directories: Vec::new(),
            warnings: Vec::new(),
        }],
    };

    let store = InMemoryLocationOperationStore::new();
    store.insert_operation(queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::CatalogOnly,
        VerificationDepth::Full,
    ));
    let catalog = FakeCatalog::with_title(
        "title-1",
        TitlePlacementSnapshot {
            root_folder_id: "root-a".to_string(),
            library_id: "lib-1".to_string(),
            folder_path: None,
            media_file_paths: BTreeSet::new(),
        },
    );
    let recycler = RecordingRecycler::default();
    let mover = RootMoveFileMover::without_permissions();
    let admission = RootMoveAdmission::new(&plan, &catalog);
    let reconciler = RootMoveReconciler::new(&plan, &catalog, &store, &recycler);

    let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
        .run("op-1", &plan.to_work_plan())
        .await
        .expect("run");

    assert_eq!(outcome.state, LocationOperationState::Completed);
    assert!(store.verifications().is_empty());
    assert!(recycler.recycled().is_empty());
    assert_eq!(catalog.writes(), vec!["root:title-1=root-b".to_string()]);
    assert_eq!(
        catalog.placement("title-1").expect("known").root_folder_id,
        "root-b"
    );
}

/// FR-031: cleanup removes only *empty* source directories. A directory that
/// still holds unmanaged content is left alone and warned about.
#[tokio::test]
async fn cleanup_leaves_a_non_empty_source_directory_alone() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_root = temp.path().join("a");
    let destination_root = temp.path().join("b");
    let plan = single_title_plan(&source_root, &destination_root, "movie.mkv", 11);
    write_file(&plan.titles[0].files[0].source(), b"hello world");
    // Something the operation never planned for, sitting beside the media file.
    write_file(&source_root.join("Movie").join("notes.txt"), b"mine");

    let store = InMemoryLocationOperationStore::new();
    store.insert_operation(queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::MoveWithScryer,
        VerificationDepth::Full,
    ));
    let catalog = FakeCatalog::with_title("title-1", placement_for(&plan));
    let recycler = RecordingRecycler::default();
    let mover = RootMoveFileMover::without_permissions();
    let admission = RootMoveAdmission::new(&plan, &catalog);
    let reconciler = RootMoveReconciler::new(&plan, &catalog, &store, &recycler);

    let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
        .run("op-1", &plan.to_work_plan())
        .await
        .expect("run");

    assert_eq!(outcome.state, LocationOperationState::CompletedWithWarnings);
    assert!(source_root.join("Movie").exists());
    assert!(source_root.join("Movie").join("notes.txt").exists());
    assert!(
        outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("still holds content")),
        "the user is told why the directory survived: {:?}",
        outcome.warnings
    );
}

/// FR-089: a catalog input that changed underneath an unprocessed title is
/// stale; the operation stops and demands a new preview.
#[tokio::test]
async fn a_changed_catalog_input_stops_the_operation_as_stale() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_root = temp.path().join("a");
    let destination_root = temp.path().join("b");
    let plan = single_title_plan(&source_root, &destination_root, "movie.mkv", 11);
    write_file(&plan.titles[0].files[0].source(), b"hello world");

    let store = InMemoryLocationOperationStore::new();
    store.insert_operation(queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::MoveWithScryer,
        VerificationDepth::Full,
    ));
    let mut placement = placement_for(&plan);
    // Someone moved the title to a third root between preview and start.
    placement.root_folder_id = "root-c".to_string();
    let catalog = FakeCatalog::with_title("title-1", placement);
    let recycler = RecordingRecycler::default();
    let mover = RootMoveFileMover::without_permissions();
    let admission = RootMoveAdmission::new(&plan, &catalog);
    let reconciler = RootMoveReconciler::new(&plan, &catalog, &store, &recycler);

    let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
        .run("op-1", &plan.to_work_plan())
        .await
        .expect("run");

    assert_eq!(outcome.state, LocationOperationState::Failed);
    assert_eq!(
        outcome.stop_reason,
        Some(crate::location::executor::StopReason::StalePlan)
    );
    assert!(plan.titles[0].files[0].source().exists());
}

/// FR-089's carve-out, stated directly: a source that vanished because *this*
/// operation already verified its destination is resumable, not stale.
#[tokio::test]
async fn an_already_verified_destination_is_not_stale() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_root = temp.path().join("a");
    let destination_root = temp.path().join("b");
    let plan = single_title_plan(&source_root, &destination_root, "movie.mkv", 11);
    // The source is gone and the destination is present: exactly the state an
    // interrupted rename leaves behind.
    write_file(&plan.titles[0].files[0].destination(), b"hello world");

    let catalog = FakeCatalog::with_title("title-1", placement_for(&plan));
    let admission = RootMoveAdmission::new(&plan, &catalog);
    let operation = queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::MoveWithScryer,
        VerificationDepth::Full,
    );
    let planned_title = plan.titles[0].to_planned_title();

    let verified: BTreeSet<String> = std::iter::once(plan.titles[0].files[0].destination_path.clone())
        .collect();
    let admitted = admission
        .admit_title(TitleAdmissionContext {
            operation: &operation,
            title: &planned_title,
            verified_destinations: &verified,
        })
        .await
        .expect("admission");
    assert_eq!(admitted, TitleAdmission::Proceed);

    // Without the verification record the same missing source *is* stale.
    let none: BTreeSet<String> = BTreeSet::new();
    let admitted = admission
        .admit_title(TitleAdmissionContext {
            operation: &operation,
            title: &planned_title,
            verified_destinations: &none,
        })
        .await
        .expect("admission");
    assert!(matches!(admitted, TitleAdmission::Stale(_)));
}

/// A title the catalog no longer knows is a stale plan, not a crash.
#[tokio::test]
async fn a_deleted_title_stales_the_plan() {
    let temp = tempfile::tempdir().expect("temp dir");
    let plan = single_title_plan(
        &temp.path().join("a"),
        &temp.path().join("b"),
        "movie.mkv",
        11,
    );
    let catalog = FakeCatalog::with_title("title-1", placement_for(&plan));
    catalog.forget("title-1");

    let admission = RootMoveAdmission::new(&plan, &catalog);
    let operation = queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::MoveWithScryer,
        VerificationDepth::Full,
    );
    let planned_title = plan.titles[0].to_planned_title();
    let none: BTreeSet<String> = BTreeSet::new();

    let admitted = admission
        .admit_title(TitleAdmissionContext {
            operation: &operation,
            title: &planned_title,
            verified_destinations: &none,
        })
        .await
        .expect("admission");

    assert!(matches!(admitted, TitleAdmission::Stale(_)));
}

/// A tracked file that appeared after the preview changes what the plan would
/// move, so the plan is stale.
#[tokio::test]
async fn a_newly_tracked_file_stales_the_plan() {
    let temp = tempfile::tempdir().expect("temp dir");
    let plan = single_title_plan(
        &temp.path().join("a"),
        &temp.path().join("b"),
        "movie.mkv",
        11,
    );
    write_file(&plan.titles[0].files[0].source(), b"hello world");

    let mut placement = placement_for(&plan);
    placement
        .media_file_paths
        .insert("/a/Movie/extra.mkv".to_string());
    let catalog = FakeCatalog::with_title("title-1", placement);

    let admission = RootMoveAdmission::new(&plan, &catalog);
    let operation = queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::MoveWithScryer,
        VerificationDepth::Full,
    );
    let planned_title = plan.titles[0].to_planned_title();
    let none: BTreeSet<String> = BTreeSet::new();

    let admitted = admission
        .admit_title(TitleAdmissionContext {
            operation: &operation,
            title: &planned_title,
            verified_destinations: &none,
        })
        .await
        .expect("admission");

    assert!(matches!(admitted, TitleAdmission::Stale(_)));
}

/// FR-042: when the full read-back cannot run, verification falls back to the
/// quick floor and the reduced guarantee is recorded on the operation.
#[tokio::test]
async fn a_quick_floor_fallback_is_counted_on_the_operation() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_root = temp.path().join("a");
    let destination_root = temp.path().join("b");
    let mut plan = single_title_plan(&source_root, &destination_root, "movie.mkv", 11);
    plan.titles[0].same_volume = Some(false);
    write_file(&plan.titles[0].files[0].source(), b"hello world");

    struct CopyWithoutReadBack {
        copier: VerifiedCopier,
    }

    #[async_trait]
    impl TitleFileMover for CopyWithoutReadBack {
        async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
            if let Some(parent) = request.file.destination_path.parent() {
                tokio::fs::create_dir_all(parent).await.expect("mkdir");
            }
            self.copier
                .copy_and_verify(VerifiedCopyRequest {
                    source: request.file.source_path.clone(),
                    destination: request.file.destination_path.clone(),
                    depth: request.depth,
                    claim: DestinationClaim::ClaimHere,
                })
                .await
        }
    }

    let store = InMemoryLocationOperationStore::new();
    store.insert_operation(queued_operation(
        "op-1",
        LocationOperationType::RootMove,
        LocationExecutionMode::MoveWithScryer,
        VerificationDepth::Full,
    ));
    let catalog = FakeCatalog::with_title("title-1", placement_for(&plan));
    let recycler = RecordingRecycler::default();
    let mover = CopyWithoutReadBack {
        copier: VerifiedCopier::with_read_back_opener(Arc::new(|_path: &Path| {
            ReadBackHandle::Unsupported("this filesystem cannot be read back".to_string())
        })),
    };
    let admission = RootMoveAdmission::new(&plan, &catalog);
    let reconciler = RootMoveReconciler::new(&plan, &catalog, &store, &recycler);

    let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
        .run("op-1", &plan.to_work_plan())
        .await
        .expect("run");

    assert!(outcome.completed());
    let records = store.verifications();
    assert_eq!(records[0].depth.applied, VerificationDepth::Quick);
    assert!(records[0].depth.fell_back);
    assert_eq!(
        store
            .operation("op-1")
            .expect("operation")
            .verification_fallback_count,
        1
    );
    // Even at the quick floor, the source is still recycled: the floor is a
    // pass, just a weaker one (FR-042/044).
    assert_eq!(recycler.recycled().len(), 1);
}

/// The cache-bypass note travels with a full verification rather than
/// downgrading it — a cached read-back is still a full read-back.
#[tokio::test]
async fn a_rejected_cache_bypass_stays_a_full_verification() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source = temp.path().join("source.bin");
    let destination = temp.path().join("destination.bin");
    write_file(&source, b"hello world");

    let copier = VerifiedCopier::with_read_back_opener(Arc::new(|path: &Path| {
        match std::fs::File::open(path) {
            Ok(file) => ReadBackHandle::Ready {
                file,
                bypass: CacheBypass::Rejected("not supported here".to_string()),
            },
            Err(error) => ReadBackHandle::Unavailable(error.to_string()),
        }
    }));

    let verified = copier
        .copy_and_verify(VerifiedCopyRequest {
            source,
            destination,
            depth: VerificationDepth::Full,
            claim: DestinationClaim::ClaimHere,
        })
        .await
        .expect("copy");

    assert_eq!(verified.depth.applied, VerificationDepth::Full);
    assert!(!verified.depth.fell_back);
    assert!(verified.permits_source_removal());
    assert!(
        verified
            .detail
            .as_deref()
            .expect("detail")
            .contains("not cache-bypassed")
    );
}

/// `RemoveVerifiedSource` is the explicit no-recycle-bin fallback: it removes,
/// and it always says so.
#[tokio::test]
async fn removing_a_verified_source_is_always_reported() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source = temp.path().join("movie.mkv");
    write_file(&source, b"bytes");
    let plan = single_title_plan(&temp.path().join("a"), &temp.path().join("b"), "m.mkv", 5);

    let disposal = RemoveVerifiedSource
        .recycle_source("op-1", &plan.titles[0], &source, Some("mf-1"), 5)
        .await
        .expect("dispose");

    assert!(matches!(
        disposal,
        SourceDisposal::RemovedRecycleUnavailable(_)
    ));
    assert!(!source.exists());

    // Running it again on the now-missing source is a no-op, so a resumed
    // cleanup cannot fail.
    let disposal = RemoveVerifiedSource
        .recycle_source("op-1", &plan.titles[0], &source, Some("mf-1"), 5)
        .await
        .expect("dispose");
    assert_eq!(disposal, SourceDisposal::AlreadyAbsent);
}

#[tokio::test]
async fn planned_source_paths_lists_every_source() {
    let temp = tempfile::tempdir().expect("temp dir");
    let plan = single_title_plan(
        &temp.path().join("a"),
        &temp.path().join("b"),
        "movie.mkv",
        11,
    );

    assert_eq!(planned_source_paths(&plan).len(), 1);
}
