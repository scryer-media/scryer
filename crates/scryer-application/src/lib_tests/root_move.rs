//! US2 — "move titles to another root in the same library" — at the story
//! level, plus the verification-depth half of US9 that only shows up once a
//! preference has travelled all the way to a persisted per-file record.
//!
//! Everything here drives the use-case API (`preview_root_move`,
//! `start_root_move`, `resume_location_operation`, `run_root_move`) against real
//! directories and real files, so the assertions are about what ends up on disk
//! and in the catalog. The seam-level tests in `location::execution::tests` own
//! the cases that need an injected mover — a cross-filesystem copy cannot be
//! staged inside one temp directory, because the mover decides rename-vs-copy
//! from the actual device ids.

use super::*;

use crate::location::classify::DestinationRequest;
use crate::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationState, LocationOperationType,
    VerificationDepth,
};
use crate::location::operations::{
    RootMovePreviewRequest, StartRootMoveRequest, is_catalog_only,
};
use crate::location::preview::{PlanConfirmationRequest, PlanItemKind};
use crate::location::test_support::{InMemoryLocationOperationStore, queued_operation};

/// Two roots in one movie library, both real directories, with the operation
/// store the runner checkpoints through.
struct RootMoveFixture {
    app: AppUseCase,
    user: User,
    operations: Arc<InMemoryLocationOperationStore>,
    temp: tempfile::TempDir,
    root_a_id: String,
    root_b_id: String,
}

impl RootMoveFixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().expect("root move tempdir");
        let root_a = temp.path().join("root-a");
        let root_b = temp.path().join("root-b");
        std::fs::create_dir_all(&root_a).expect("create root a");
        std::fs::create_dir_all(&root_b).expect("create root b");

        let (app, user, _) =
            bootstrap_movie_scan_app(&root_a, Vec::new(), Arc::new(EmptySearchMetadataGateway))
                .await;
        let operations = Arc::new(InMemoryLocationOperationStore::new());
        let app = app.with_test_overrides({
            let operations = operations.clone();
            move |services| services.with_location_operation_repository(operations)
        });

        let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
        let library = app
            .services
            .catalog
            .libraries
            .update(
                &library_id,
                "Movies".to_string(),
                "movies".to_string(),
                vec![
                    LibraryRootDraft {
                        path: root_a.to_string_lossy().to_string(),
                        is_default: true,
                    },
                    LibraryRootDraft {
                        path: root_b.to_string_lossy().to_string(),
                        is_default: false,
                    },
                ],
            )
            .await
            .expect("configure two roots");

        Self {
            root_a_id: library.roots[0].id.clone(),
            root_b_id: library.roots[1].id.clone(),
            app,
            user,
            operations,
            temp,
        }
    }

    fn root_a(&self) -> PathBuf {
        self.temp.path().join("root-a")
    }

    fn root_b(&self) -> PathBuf {
        self.temp.path().join("root-b")
    }

    /// A monitored movie owning `folder_name` under `root`, with `files`
    /// (name, size) written into that folder. The first entry gets the tracked
    /// media-file row; the rest are companion assets the folder walk picks up
    /// (FR-027).
    async fn seed_title(
        &self,
        name: &str,
        year: i32,
        root_id: &str,
        root: &Path,
        folder_name: &str,
        files: &[(&str, usize)],
    ) -> Title {
        let folder = root.join(folder_name);
        std::fs::create_dir_all(&folder).expect("create title folder");

        let title = self.create_title(name, year, root_id).await;
        self.app
            .services
            .catalog
            .titles
            .set_folder_path(&title.id, folder.to_string_lossy().as_ref())
            .await
            .expect("set title folder");

        for (index, (file_name, size)) in files.iter().enumerate() {
            let path = folder.join(file_name);
            std::fs::write(&path, vec![b'x'; *size]).expect("write fixture file");
            if index > 0 {
                continue;
            }
            self.app
                .services
                .library
                .media_files
                .insert_media_file(&InsertMediaFileInput {
                    title_id: title.id.clone(),
                    file_path: path.to_string_lossy().to_string(),
                    size_bytes: *size as i64,
                    role: MediaFileRole::Primary,
                    ..Default::default()
                })
                .await
                .expect("seed media file row");
        }

        self.title(&title.id).await
    }

    async fn create_title(&self, name: &str, year: i32, root_id: &str) -> Title {
        let title = self
            .app
            .add_title(
                &self.user,
                NewTitle {
                    name: name.to_string(),
                    facet: MediaFacet::Movie,
                    monitored: true,
                    year: Some(year),
                    root_folder_id: Some(root_id.to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("create movie title");
        // `add_title` places the title in its library's default root; a fixture
        // that seeds content on the second root has to say so explicitly.
        self.app
            .services
            .catalog
            .titles
            .update_metadata(&title.id, None, None, None, Some(root_id.to_string()))
            .await
            .expect("set title root");
        self.title(&title.id).await
    }

    async fn title(&self, title_id: &str) -> Title {
        self.app
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .expect("load title")
            .expect("title exists")
    }

    async fn media_paths(&self, title_id: &str) -> Vec<String> {
        let mut paths = self
            .app
            .services
            .library
            .media_files
            .list_media_files_for_title(title_id)
            .await
            .expect("list media files")
            .into_iter()
            .map(|file| file.file_path)
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }

    async fn preview(&self, title_ids: &[&str]) -> crate::location::operations::RootMovePreview {
        self.app
            .preview_root_move(
                &self.user,
                RootMovePreviewRequest {
                    title_ids: title_ids.iter().map(|id| (*id).to_string()).collect(),
                    destination: DestinationRequest::to_root(self.root_b_id.clone()),
                },
            )
            .await
            .expect("preview root move")
    }

    /// Preview, confirm with the fingerprint the preview returned, and wait for
    /// the spawned runner to reach a terminal state — the whole client-visible
    /// sequence in one call.
    async fn start_and_settle(&self, title_ids: &[&str]) -> LocationOperation {
        let preview = self.preview(title_ids).await;
        let accepted = self
            .app
            .start_root_move(
                &self.user,
                StartRootMoveRequest {
                    title_ids: title_ids.iter().map(|id| (*id).to_string()).collect(),
                    destination: DestinationRequest::to_root(self.root_b_id.clone()),
                    confirmation: PlanConfirmationRequest {
                        fingerprint: preview.plan.fingerprint.clone(),
                        typed_confirmation: None,
                    },
                },
            )
            .await
            .expect("start root move");
        self.settle(&accepted.operation.id).await
    }

    /// `start_root_move` is asynchronous by contract (FR-030): it hands back an
    /// id and the runner works in the background, so a story test watches the
    /// operation the way Activity does.
    async fn settle(&self, operation_id: &str) -> LocationOperation {
        timeout(Duration::from_secs(10), async {
            loop {
                let operation = self
                    .app
                    .location_operation(operation_id)
                    .await
                    .expect("read operation")
                    .expect("operation row exists");
                if operation.state.is_terminal() {
                    return operation;
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the operation reached a terminal state")
    }

    fn verifications(&self) -> Vec<crate::location::model::FileVerificationRecord> {
        self.operations.verifications()
    }
}

fn move_items(preview: &crate::location::operations::RootMovePreview) -> Vec<&crate::location::preview::PlanItem> {
    preview
        .plan
        .section(PlanItemKind::Move)
        .map(|section| section.items.items.iter().collect())
        .unwrap_or_default()
}

// ── US2.1 ────────────────────────────────────────────────────────────────────

/// US2.1, first half: the preview states current and destination folders, the
/// file count, and the total size before anything is confirmed.
#[tokio::test]
async fn the_preview_states_current_and_destination_folders_with_file_count_and_size() {
    let fixture = RootMoveFixture::new().await;
    let title = fixture
        .seed_title(
            "Preview Subject",
            2024,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Preview Subject (2024)",
            &[
                ("Preview.Subject.2024.1080p.mkv", 2048),
                ("Preview.Subject.2024.1080p.en.srt", 64),
            ],
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;

    assert_eq!(preview.plan.classification.root_move, 1);
    assert_eq!(preview.plan.counts.titles_total, 1);
    assert_eq!(
        preview.plan.counts.files_total, 2,
        "the sidecar travels with its title, so the count the user sees includes it (FR-027)"
    );
    assert_eq!(preview.plan.counts.bytes_total, 2048 + 64);

    // Current and destination folders, both named outright.
    let planned = &preview.execution.titles[0];
    assert_eq!(
        planned.source_folder_path.as_deref(),
        Some(
            fixture
                .root_a()
                .join("Preview Subject (2024)")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(
        planned.destination_folder_path.as_deref(),
        Some(
            fixture
                .root_b()
                .join("Preview Subject (2024)")
                .to_string_lossy()
                .as_ref()
        )
    );

    // Every file is shown with where it comes from and where it lands.
    let items = move_items(&preview);
    assert_eq!(items.len(), 2);
    for item in items {
        let source = item.source_path.as_deref().expect("a move has a source");
        let destination = item
            .destination_path
            .as_deref()
            .expect("a move has a destination");
        assert!(source.starts_with(fixture.root_a().to_string_lossy().as_ref()));
        assert!(destination.starts_with(fixture.root_b().to_string_lossy().as_ref()));
    }

    // Nothing has happened yet: a preview is a read.
    assert!(fixture.root_a().join("Preview Subject (2024)").exists());
    assert!(!fixture.root_b().join("Preview Subject (2024)").exists());
    assert_eq!(fixture.title(&title.id).await.root_folder_id, fixture.root_a_id);
}

/// US2.1, second half: on confirm the operation completes and the catalog is
/// pointed at the destination — folder, root, and every media-file path — with
/// the source folder cleaned up behind it.
#[tokio::test]
async fn confirming_a_move_relocates_the_files_and_then_the_catalog() {
    let fixture = RootMoveFixture::new().await;
    let title = fixture
        .seed_title(
            "Confirmed Move",
            2023,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Confirmed Move (2023)",
            &[("Confirmed.Move.2023.1080p.mkv", 4096)],
        )
        .await;

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert_eq!(operation.state, LocationOperationState::Completed);
    assert_eq!(operation.counters.titles_processed, 1);
    assert_eq!(operation.counters.files_processed, 1);

    let destination_folder = fixture.root_b().join("Confirmed Move (2023)");
    assert!(destination_folder.join("Confirmed.Move.2023.1080p.mkv").exists());
    assert!(!fixture.root_a().join("Confirmed Move (2023)").exists());

    let moved = fixture.title(&title.id).await;
    assert_eq!(moved.root_folder_id, fixture.root_b_id);
    assert_eq!(
        moved.folder_path.as_deref(),
        Some(destination_folder.to_string_lossy().as_ref())
    );
    assert_eq!(fixture.media_paths(&title.id).await, vec![
        destination_folder
            .join("Confirmed.Move.2023.1080p.mkv")
            .to_string_lossy()
            .to_string()
    ]);

    // Every file has a verification record, and the record is what unblocked
    // touching the source (FR-044).
    let records = fixture.verifications();
    assert_eq!(records.len(), 1);
    assert!(records[0].outcome.permits_source_removal());
}

/// US2.1's ordering rule from the failing side: when a title's content cannot
/// be placed, the catalog is not touched at all. The complementary case — a
/// *verification* that fails after a successful copy — is asserted against the
/// real copier in `location::execution::tests`, which is the only layer that can
/// stage a cross-filesystem copy.
#[tokio::test]
async fn a_failed_placement_leaves_the_catalog_pointing_at_the_source() {
    let fixture = RootMoveFixture::new().await;
    let title = fixture
        .seed_title(
            "Blocked Placement",
            2022,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Blocked Placement (2022)",
            &[("Blocked.Placement.2022.1080p.mkv", 1024)],
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;
    let destination_folder = fixture.root_b().join("Blocked Placement (2022)");

    // A plain file where the destination folder has to go: creating the
    // directory fails, so the placement cannot happen.
    std::fs::write(&destination_folder, b"not a directory").expect("occupy the destination path");

    let accepted = fixture
        .app
        .start_root_move(
            &fixture.user,
            StartRootMoveRequest {
                title_ids: vec![title.id.clone()],
                destination: DestinationRequest::to_root(fixture.root_b_id.clone()),
                confirmation: PlanConfirmationRequest {
                    fingerprint: preview.plan.fingerprint.clone(),
                    typed_confirmation: None,
                },
            },
        )
        .await
        .expect("start root move");
    let operation = fixture.settle(&accepted.operation.id).await;

    assert_eq!(operation.state, LocationOperationState::Failed);

    let unmoved = fixture.title(&title.id).await;
    assert_eq!(
        unmoved.root_folder_id, fixture.root_a_id,
        "no ownership flip without a placed, verified destination"
    );
    assert_eq!(
        unmoved.folder_path.as_deref(),
        Some(
            fixture
                .root_a()
                .join("Blocked Placement (2022)")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert!(
        fixture
            .root_a()
            .join("Blocked Placement (2022)")
            .join("Blocked.Placement.2022.1080p.mkv")
            .exists(),
        "the source content is untouched"
    );
    assert!(fixture.verifications().is_empty());
}

// ── US2.2 ────────────────────────────────────────────────────────────────────

/// US2.2: the destination folder is calculated from the destination library's
/// naming policy, so a stale folder name is repaired by the move — and the
/// preview showed the repaired name before the user confirmed.
#[tokio::test]
async fn a_stale_folder_name_is_repaired_by_the_destination_naming_policy() {
    let fixture = RootMoveFixture::new().await;
    let title = fixture
        .seed_title(
            "Stale Name",
            2019,
            &fixture.root_a_id,
            &fixture.root_a(),
            // What is on disk today: no year, wrong case, trailing junk.
            "stale.name.DVDRip",
            &[("Stale.Name.2019.1080p.mkv", 512)],
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;
    let repaired = fixture.root_b().join("Stale Name (2019)");
    assert_eq!(
        preview.execution.titles[0].destination_folder_path.as_deref(),
        Some(repaired.to_string_lossy().as_ref()),
        "the preview shows the repaired folder name before confirmation (FR-013)"
    );
    assert!(
        move_items(&preview)
            .iter()
            .all(|item| item
                .destination_path
                .as_deref()
                .is_some_and(|path| path.starts_with(repaired.to_string_lossy().as_ref()))),
        "every previewed destination sits under the repaired folder"
    );

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert_eq!(operation.state, LocationOperationState::Completed);
    assert!(repaired.join("Stale.Name.2019.1080p.mkv").exists());
    assert!(!fixture.root_a().join("stale.name.DVDRip").exists());
    assert_eq!(
        fixture.title(&title.id).await.folder_path.as_deref(),
        Some(repaired.to_string_lossy().as_ref())
    );
}

// ── US2.3 ────────────────────────────────────────────────────────────────────

/// US2.3: a selection mixing both roots classifies A-titles as moves and
/// B-titles as no-ops, and nothing is dropped on the floor — the no-op is a
/// counted plan item at preview time and a counted operation counter afterwards
/// (FR-091).
#[tokio::test]
async fn a_bulk_selection_across_both_roots_classifies_every_title_and_omits_none() {
    let fixture = RootMoveFixture::new().await;
    let first = fixture
        .seed_title(
            "Source One",
            2001,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Source One (2001)",
            &[("Source.One.2001.mkv", 100)],
        )
        .await;
    let second = fixture
        .seed_title(
            "Source Two",
            2002,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Source Two (2002)",
            &[("Source.Two.2002.mkv", 200)],
        )
        .await;
    let already_there = fixture
        .seed_title(
            "Already There",
            2003,
            &fixture.root_b_id,
            &fixture.root_b(),
            "Already There (2003)",
            &[("Already.There.2003.mkv", 300)],
        )
        .await;

    let selection = [
        first.id.as_str(),
        already_there.id.as_str(),
        second.id.as_str(),
    ];
    let preview = fixture.preview(&selection).await;

    assert_eq!(preview.plan.classification.root_move, 2);
    assert_eq!(preview.plan.classification.no_op, 1);
    assert_eq!(
        preview.plan.classification.total(),
        3,
        "every selected title is classified into exactly one class (SC-005)"
    );
    assert_eq!(
        preview.plan.counts.for_kind(PlanItemKind::NoOp),
        1,
        "the no-op is visible in the plan, not silently omitted"
    );
    assert_eq!(preview.execution.titles.len(), 2);
    assert_eq!(preview.execution.no_op_titles, 1);
    assert_eq!(preview.execution.unresolved_titles, 0);

    let operation = fixture.start_and_settle(&selection).await;
    assert_eq!(operation.state, LocationOperationState::Completed);
    assert_eq!(operation.counters.titles_total, 2);
    assert_eq!(operation.counters.titles_processed, 2);
    assert_eq!(
        operation.counters.no_ops, 1,
        "the title that needed nothing is still reported (FR-091)"
    );
    assert_eq!(operation.counters.unresolved, 0);

    assert_eq!(fixture.title(&first.id).await.root_folder_id, fixture.root_b_id);
    assert_eq!(fixture.title(&second.id).await.root_folder_id, fixture.root_b_id);

    // The B-title was never entered into the operation, and its content sits
    // exactly where it always did.
    let untouched = fixture.title(&already_there.id).await;
    assert_eq!(untouched.root_folder_id, fixture.root_b_id);
    assert_eq!(
        untouched.folder_path.as_deref(),
        Some(
            fixture
                .root_b()
                .join("Already There (2003)")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert!(
        fixture
            .verifications()
            .iter()
            .all(|record| record.title_id != already_there.id)
    );
}

// ── US2.4 ────────────────────────────────────────────────────────────────────

/// US2.4 / FR-076: a monitored title with no files on disk is a catalog-only
/// reassignment. The plan says so — the mode never enters move-mode selection —
/// and the run touches no filesystem at all.
#[tokio::test]
async fn a_monitored_title_with_no_files_takes_the_catalog_only_fast_path() {
    let fixture = RootMoveFixture::new().await;
    let title = fixture.create_title("Nothing On Disk", 2020, &fixture.root_a_id).await;
    assert!(title.monitored);
    assert!(fixture.media_paths(&title.id).await.is_empty());

    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(preview.plan.classification.catalog_only, 1);
    assert_eq!(preview.plan.classification.root_move, 0);
    assert_eq!(
        preview.plan.header.mode,
        LocationExecutionMode::CatalogOnly,
        "FR-076: no move-mode selection for a title with nothing to move"
    );
    assert!(is_catalog_only(&preview.plan));
    assert_eq!(preview.plan.counts.files_total, 0);
    assert_eq!(preview.plan.counts.bytes_total, 0);
    assert_eq!(preview.plan.counts.for_kind(PlanItemKind::CatalogChange), 1);

    let before: Vec<_> = std::fs::read_dir(fixture.root_b())
        .expect("read destination root")
        .map(|entry| entry.expect("entry").path())
        .collect();

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert_eq!(operation.state, LocationOperationState::Completed);
    assert_eq!(operation.mode, LocationExecutionMode::CatalogOnly);

    assert_eq!(fixture.title(&title.id).await.root_folder_id, fixture.root_b_id);
    assert!(
        fixture.verifications().is_empty(),
        "a catalog-only reassignment verifies nothing because it moves nothing"
    );
    let after: Vec<_> = std::fs::read_dir(fixture.root_b())
        .expect("read destination root")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(before, after, "the destination root is untouched");
}

// ── SC-002 ───────────────────────────────────────────────────────────────────

/// SC-002 / FR-033: a process that dies mid-operation leaves a non-terminal row
/// with checkpoints for the titles that settled. Resume reads the *persisted*
/// plan back, skips what already settled, and finishes — counting each title
/// once.
///
/// The crash is injected at the store, which is where a dying process actually
/// shows up from the runner's point of view: a read fails and `run` returns
/// without ever writing a terminal state.
#[tokio::test]
async fn an_interrupted_operation_resumes_from_its_persisted_plan_without_redoing_settled_titles() {
    let fixture = RootMoveFixture::new().await;
    let first = fixture
        .seed_title(
            "Settled First",
            2011,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Settled First (2011)",
            &[("Settled.First.2011.mkv", 1500)],
        )
        .await;
    let second = fixture
        .seed_title(
            "Interrupted Second",
            2012,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Interrupted Second (2012)",
            &[("Interrupted.Second.2012.mkv", 2500)],
        )
        .await;

    let preview = fixture.preview(&[&first.id, &second.id]).await;
    assert_eq!(preview.execution.titles.len(), 2);

    // Persist the row and the plan exactly as `start_root_move` does, but drive
    // the runner inline so the interruption is deterministic.
    let operation_id = "operation-crash-resume";
    let operation = LocationOperation {
        counters: crate::location::model::LocationOperationCounters {
            titles_total: 2,
            files_total: 2,
            bytes_total: 4000,
            ..Default::default()
        },
        ..queued_operation(
            operation_id,
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
            preview.plan.verification.depth,
        )
    };
    let plan_json = serde_json::to_string(&preview.execution).expect("serialize plan");
    fixture
        .app
        .services
        .library
        .location_operations
        .create_location_operation(&operation, Some(&plan_json))
        .await
        .expect("persist the operation");

    // The runner checks the cancel flag once per unprocessed title, so the
    // second check is the boundary right after the first title settled.
    fixture.operations.crash_on_cancel_check(2);
    let crashed = fixture
        .app
        .run_root_move(operation_id, &preview.execution)
        .await;
    assert!(crashed.is_err(), "the injected store failure aborts the run");

    let interrupted = fixture
        .app
        .location_operation(operation_id)
        .await
        .expect("read operation")
        .expect("operation row");
    assert!(
        interrupted.state.is_active(),
        "a crash never writes a terminal state, got {:?}",
        interrupted.state
    );
    let settled_checkpoint = fixture
        .operations
        .checkpoint(operation_id, &first.id)
        .expect("the first title settled before the crash");
    assert_eq!(
        settled_checkpoint.state,
        crate::location::model::TitleCheckpointState::Completed
    );
    assert!(
        fixture
            .operations
            .checkpoint(operation_id, &second.id)
            .is_none_or(|checkpoint| !checkpoint.state.is_settled()),
        "the second title never got its turn"
    );
    assert_eq!(fixture.verifications().len(), 1);
    assert_eq!(
        fixture.title(&first.id).await.root_folder_id,
        fixture.root_b_id
    );
    assert_eq!(
        fixture.title(&second.id).await.root_folder_id,
        fixture.root_a_id
    );

    // Resume: the plan comes back out of the store, not out of the caller.
    let resumed_plan = fixture
        .app
        .resume_location_operation(operation_id)
        .await
        .expect("resume")
        .expect("an interrupted root move is resumable");
    assert_eq!(resumed_plan, preview.execution);

    let outcome = fixture
        .app
        .run_root_move(operation_id, &resumed_plan)
        .await
        .expect("resumed run");
    assert_eq!(outcome.state, LocationOperationState::Completed);

    // No repeated work: the settled title's checkpoint and its verification
    // record are the ones the first run wrote.
    assert_eq!(
        fixture
            .operations
            .checkpoint(operation_id, &first.id)
            .expect("checkpoint"),
        settled_checkpoint
    );
    assert_eq!(
        fixture.verifications().len(),
        2,
        "one record per file, not one per attempt"
    );
    assert_eq!(outcome.counters.titles_total, 2);
    assert_eq!(outcome.counters.titles_processed, 2);
    assert_eq!(outcome.counters.files_processed, 2);
    assert_eq!(outcome.counters.bytes_processed, 4000);

    assert_eq!(
        fixture.title(&second.id).await.root_folder_id,
        fixture.root_b_id
    );
    assert!(
        fixture
            .root_b()
            .join("Interrupted Second (2012)")
            .join("Interrupted.Second.2012.mkv")
            .exists()
    );
    assert_eq!(
        fixture.operations.open_claim_count(),
        0,
        "a finished operation owns nothing (FR-084)"
    );
}

// ── US9.1–9.2 end-to-end, and SC-007 against a live operation ───────────────

/// US9.1: with no preference set, the plan, the persisted operation, and every
/// per-file record all say full depth. The default has to survive the whole
/// path, not just the settings reader.
#[tokio::test]
async fn the_default_verification_depth_reaches_the_operation_and_every_record() {
    let fixture = RootMoveFixture::new().await;
    let title = fixture
        .seed_title(
            "Full Depth",
            2024,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Full Depth (2024)",
            &[("Full.Depth.2024.mkv", 777)],
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(preview.plan.verification.depth, VerificationDepth::Full);

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert_eq!(operation.verification_depth, VerificationDepth::Full);
    assert_eq!(operation.verification_fallback_count, 0);

    let records = fixture.verifications();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].depth.applied, VerificationDepth::Full);
    assert!(!records[0].depth.fell_back);
}

/// US9.2: flipping the preference to quick changes what the preview promises,
/// what the operation row records, and what each file's record is stamped with —
/// so the reduced guarantee is auditable after the fact (FR-043).
#[tokio::test]
async fn the_quick_verification_preference_is_recorded_end_to_end() {
    let fixture = RootMoveFixture::new().await;
    fixture
        .app
        .update_verification_settings(
            &fixture.user,
            UpdateVerificationSettings {
                depth: VerificationDepth::Quick,
            },
        )
        .await
        .expect("choose quick verification");

    let title = fixture
        .seed_title(
            "Quick Depth",
            2024,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Quick Depth (2024)",
            &[("Quick.Depth.2024.mkv", 777)],
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(preview.plan.verification.depth, VerificationDepth::Quick);

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert_eq!(operation.state, LocationOperationState::Completed);
    assert_eq!(operation.verification_depth, VerificationDepth::Quick);

    let records = fixture.verifications();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].depth.applied,
        VerificationDepth::Quick,
        "the per-file stamp is what makes 'verified (quick)' auditable"
    );
    assert!(
        !records[0].depth.fell_back,
        "quick was chosen, not fallen back to"
    );
}

/// SC-007, against a live operation rather than a hand-placed claim: while an
/// interrupted root move still holds its titles, the backfill job skips their
/// files entirely — and picks them up once the operation finishes.
#[tokio::test]
async fn the_backfill_job_skips_files_owned_by_an_in_flight_operation() {
    use crate::location::backfill::FullHashBackfillOptions;

    let fixture = RootMoveFixture::new().await;
    let first = fixture
        .seed_title(
            "Owned One",
            2014,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Owned One (2014)",
            &[("Owned.One.2014.mkv", 640)],
        )
        .await;
    let second = fixture
        .seed_title(
            "Owned Two",
            2015,
            &fixture.root_a_id,
            &fixture.root_a(),
            "Owned Two (2015)",
            &[("Owned.Two.2015.mkv", 960)],
        )
        .await;

    let preview = fixture.preview(&[&first.id, &second.id]).await;
    let operation_id = "operation-backfill-noninterference";
    let plan_json = serde_json::to_string(&preview.execution).expect("serialize plan");
    fixture
        .app
        .services
        .library
        .location_operations
        .create_location_operation(
            &queued_operation(
                operation_id,
                LocationOperationType::RootMove,
                LocationExecutionMode::MoveWithScryer,
                preview.plan.verification.depth,
            ),
            Some(&plan_json),
        )
        .await
        .expect("persist the operation");

    // Interrupt after the first title, leaving the operation active and still
    // holding both titles.
    fixture.operations.crash_on_cancel_check(2);
    assert!(
        fixture
            .app
            .run_root_move(operation_id, &preview.execution)
            .await
            .is_err()
    );
    assert!(
        fixture.operations.open_claim_count() > 0,
        "an interrupted operation keeps its claims"
    );

    let during = fixture
        .app
        .run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("backfill during the operation");
    assert_eq!(during.hashed, 0);
    assert_eq!(
        during.skipped_owned, 2,
        "both titles are owned by the operation, so neither file is read"
    );

    // Finish the operation; its claims are released and the same files converge.
    let resumed_plan = fixture
        .app
        .resume_location_operation(operation_id)
        .await
        .expect("resume")
        .expect("resumable");
    let outcome = fixture
        .app
        .run_root_move(operation_id, &resumed_plan)
        .await
        .expect("resumed run");
    assert_eq!(outcome.state, LocationOperationState::Completed);
    assert_eq!(fixture.operations.open_claim_count(), 0);

    let after = fixture
        .app
        .run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("backfill after the operation");
    assert_eq!(after.skipped_owned, 0);
    assert_eq!(after.hashed, 2);
}
