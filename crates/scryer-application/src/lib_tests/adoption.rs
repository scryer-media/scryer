//! US3 — "the files are already there" — at the story level.
//!
//! Everything here drives the use-case API (`preview_adoption`,
//! `start_adoption`, `run_root_move`, `resume_location_operation`) against real
//! directories, with the content moved by the test the way a user moves it with
//! Finder or `rsync`: outside Scryer, before Scryer is told. The assertions are
//! about what the catalog ends up pointing at, what was proven before it did,
//! and what was left alone on the way (FR-050–053).
//!
//! The matcher's own rules live in `location::adoption::tests`, and the
//! plan-shape rules in `location::root_move::tests`; this file is the seam
//! where a preview, a confirmation, a runner, and a filesystem meet.

use super::*;

use crate::location::classify::DestinationRequest;
use crate::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationState, LocationOperationType,
    PersistedContentHashes, VerificationDepth,
};
use crate::location::operations::{RootMovePreviewRequest, StartRootMoveRequest};
use crate::location::preview::{PlanConfirmationRequest, PlanItemKind};
use crate::location::test_support::{
    InMemoryLocationOperationStore, queued_operation, title_boundary_cancel_check,
};

/// Two roots in one movie library. Titles are seeded on root A and then moved
/// to root B behind Scryer's back, which is the whole premise of US3.
struct AdoptionFixture {
    app: AppUseCase,
    user: User,
    operations: Arc<InMemoryLocationOperationStore>,
    temp: tempfile::TempDir,
    root_a_id: String,
    root_b_id: String,
}

impl AdoptionFixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().expect("adoption tempdir");
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

    /// A monitored movie owning `folder_name` under root A, with `files`
    /// (name, size) written into it. Every entry gets a tracked media-file row:
    /// adoption is about tracked media, and a companion asset at the
    /// destination shows up as an additional file instead.
    async fn seed_title(&self, name: &str, year: i32, files: &[(&str, usize)]) -> Title {
        let folder = self.root_a().join(format!("{name} ({year})"));
        std::fs::create_dir_all(&folder).expect("create title folder");

        let title = self
            .app
            .add_title(
                &self.user,
                NewTitle {
                    name: name.to_string(),
                    facet: MediaFacet::Movie,
                    monitored: true,
                    year: Some(year),
                    root_folder_id: Some(self.root_a_id.clone()),
                    ..Default::default()
                },
            )
            .await
            .expect("create movie title");
        self.app
            .services
            .catalog
            .titles
            .set_folder_path(&title.id, folder.to_string_lossy().as_ref())
            .await
            .expect("set title folder");

        for (file_name, size) in files {
            let path = folder.join(file_name);
            std::fs::write(&path, content_of(file_name, *size)).expect("write fixture file");
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

    /// The user's own move: the whole title folder, at its existing name, onto
    /// root B. Scryer is told nothing.
    fn move_folder_externally(&self, title: &Title) -> PathBuf {
        let source = PathBuf::from(title.folder_path.clone().expect("seeded folder"));
        let destination = self
            .root_b()
            .join(source.file_name().expect("folder name"));
        std::fs::rename(&source, &destination).expect("external move");
        destination
    }

    /// The user's own *copy*: the source stays where it is, which is what makes
    /// the FR-053 cleanup question live.
    fn copy_folder_externally(&self, title: &Title) -> PathBuf {
        let source = PathBuf::from(title.folder_path.clone().expect("seeded folder"));
        let destination = self
            .root_b()
            .join(source.file_name().expect("folder name"));
        std::fs::create_dir_all(&destination).expect("create destination folder");
        for entry in std::fs::read_dir(&source).expect("read source folder") {
            let entry = entry.expect("source entry");
            std::fs::copy(entry.path(), destination.join(entry.file_name())).expect("copy file");
        }
        destination
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

    /// Stamp the full BLAKE3 the catalog would hold once the backfill job has
    /// seen a file (D4, FR-047) — the proof adoption needs before it may touch
    /// a source copy the user kept (FR-053).
    async fn record_full_hash(&self, title_id: &str, file_name: &str, bytes: &[u8]) {
        let files = self
            .app
            .services
            .library
            .media_files
            .list_media_files_for_title(title_id)
            .await
            .expect("list media files");
        let media_file = files
            .iter()
            .find(|file| file.file_path.ends_with(file_name))
            .expect("the seeded media file");
        self.app
            .services
            .library
            .media_files
            .update_media_file_content_hashes(
                &media_file.id,
                &PersistedContentHashes {
                    full_blake3: blake3::hash(bytes).to_hex().to_string(),
                    move_crc: None,
                    crc_algorithm: None,
                    hash_computed_at: Some(chrono::Utc::now()),
                },
            )
            .await
            .expect("record the persisted full hash");
    }

    async fn preview(&self, title_ids: &[&str]) -> crate::location::operations::RootMovePreview {
        self.app
            .preview_adoption(
                &self.user,
                RootMovePreviewRequest {
                    title_ids: title_ids.iter().map(|id| (*id).to_string()).collect(),
                    destination: DestinationRequest::to_root(self.root_b_id.clone()),
                },
            )
            .await
            .expect("preview adoption")
    }

    async fn start(
        &self,
        title_ids: &[&str],
    ) -> AppResult<crate::location::operations::LocationOperationAccepted> {
        let preview = self.preview(title_ids).await;
        self.app
            .start_adoption(
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
    }

    async fn start_and_settle(&self, title_ids: &[&str]) -> LocationOperation {
        let accepted = self.start(title_ids).await.expect("start adoption");
        self.settle(&accepted.operation.id).await
    }

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

/// Distinct bytes per file name, so two same-sized fixtures are not accidentally
/// identical content — an adoption test that cannot tell them apart proves
/// nothing about a matcher whose job is telling them apart.
fn content_of(file_name: &str, size: usize) -> Vec<u8> {
    let seed = file_name.bytes().fold(7_u8, |acc, byte| acc.wrapping_add(byte));
    (0..size).map(|index| seed.wrapping_add(index as u8)).collect()
}

// ── US3.1 ────────────────────────────────────────────────────────────────────

/// US3.1: an externally moved title is matched from stored identity, size, and
/// stored content signatures, and previewed with the managed-move model.
#[tokio::test]
async fn us3_1_an_externally_moved_title_is_matched_and_previewed_like_a_managed_move() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("Moved By Hand", 2024, &[("Moved.By.Hand.2024.mkv", 2048)])
        .await;
    fixture.move_folder_externally(&title);

    let preview = fixture.preview(&[&title.id]).await;

    assert_eq!(
        preview.plan.header.operation_type,
        LocationOperationType::Adoption
    );
    assert_eq!(
        preview.plan.header.mode,
        LocationExecutionMode::FilesAlreadyThere
    );
    assert!(!preview.plan.blocks_start());
    let moves = preview
        .plan
        .section(PlanItemKind::Move)
        .expect("the adopted file is previewed as a move");
    assert_eq!(moves.items.total, 1);
    assert_eq!(
        moves.items.items[0].reason_code.as_deref(),
        Some(crate::location::adoption::plan_reasons::ADOPTED_AT_DESTINATION)
    );
    // The preview still states the depth that will apply to the adopted bytes
    // (FR-080), even though nothing is written.
    assert_eq!(preview.plan.verification.files, 1);
}

/// US3.1, end to end: the catalog ends up pointing at the new location.
#[tokio::test]
async fn us3_1_an_adoption_repoints_the_catalog_at_the_new_location() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("Adopted Whole", 2023, &[("Adopted.Whole.2023.mkv", 4096)])
        .await;
    let destination = fixture.move_folder_externally(&title);

    let operation = fixture.start_and_settle(&[&title.id]).await;

    assert!(
        matches!(
            operation.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "adoption should finish: {:?} {:?}",
        operation.state,
        operation.detail
    );
    let adopted = fixture.title(&title.id).await;
    assert_eq!(adopted.root_folder_id, fixture.root_b_id);
    assert_eq!(
        adopted.folder_path.as_deref(),
        Some(destination.to_string_lossy().as_ref())
    );
    assert_eq!(
        fixture.media_paths(&title.id).await,
        vec![
            destination
                .join("Adopted.Whole.2023.mkv")
                .to_string_lossy()
                .to_string()
        ]
    );
}

/// US3.1: nothing is copied. The destination file the user placed is the file
/// the catalog adopts — same inode, untouched.
#[tokio::test]
async fn us3_1_adoption_writes_no_bytes_of_its_own() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("Untouched", 2022, &[("Untouched.2022.mkv", 3000)])
        .await;
    let destination = fixture
        .move_folder_externally(&title)
        .join("Untouched.2022.mkv");
    let before = std::fs::metadata(&destination).expect("destination metadata");

    fixture.start_and_settle(&[&title.id]).await;

    let after = std::fs::metadata(&destination).expect("destination metadata");
    assert_eq!(before.len(), after.len());
    assert_eq!(
        before.modified().ok(),
        after.modified().ok(),
        "adoption must not rewrite the file it adopts"
    );
}

// ── US3.2 ────────────────────────────────────────────────────────────────────

/// US3.2: a tracked file that is not at the destination blocks the
/// confirmation — a refusal with a named unresolved state, never a guess.
#[tokio::test]
async fn us3_2_a_tracked_file_missing_at_the_destination_refuses_the_confirmation() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title(
            "Half Moved",
            2021,
            &[("Half.Moved.2021.mkv", 2048), ("Half.Moved.2021.extra.mkv", 1024)],
        )
        .await;
    let destination = fixture.move_folder_externally(&title);
    // The user only moved half of it.
    std::fs::remove_file(destination.join("Half.Moved.2021.extra.mkv")).expect("remove one file");

    let preview = fixture.preview(&[&title.id]).await;
    assert!(preview.plan.blocks_start());
    let blocked = preview
        .plan
        .section(PlanItemKind::Blocked)
        .expect("blocked section");
    assert!(blocked.items.items.iter().any(|item| {
        item.reason_code.as_deref()
            == Some(crate::location::adoption::plan_reasons::ADOPTION_MEDIA_MISSING)
    }));

    let refused = fixture
        .start(&[&title.id])
        .await
        .expect_err("a title with unaccounted media must not start");
    assert!(
        matches!(
            refused,
            AppError::LocationPlanRefused {
                code: crate::location::preview::PlanConfirmationError::Blocked,
                ..
            }
        ),
        "the refusal names the blocked plan: {refused}"
    );

    // Nothing moved in the catalog either.
    let untouched = fixture.title(&title.id).await;
    assert_eq!(untouched.root_folder_id, fixture.root_a_id);
}

/// US3.2, the other unresolved shape: two destination files are equally
/// plausible and the stored proof cannot choose, so adoption refuses rather
/// than picking one.
#[tokio::test]
async fn us3_2_an_ambiguous_destination_refuses_the_confirmation() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("Two Ways", 2020, &[("Two.Ways.2020.mkv", 2048)])
        .await;
    let destination = fixture.move_folder_externally(&title);
    // The user left two same-sized files behind and renamed both, so neither
    // name nor layout can break the tie and no full hash is stored.
    std::fs::rename(
        destination.join("Two.Ways.2020.mkv"),
        destination.join("candidate-a.mkv"),
    )
    .expect("rename to a name the catalog does not know");
    std::fs::write(destination.join("candidate-b.mkv"), vec![b'z'; 2048])
        .expect("write a same-sized decoy");

    let preview = fixture.preview(&[&title.id]).await;

    assert!(preview.plan.blocks_start());
    let blocked = preview
        .plan
        .section(PlanItemKind::Blocked)
        .expect("blocked section");
    assert!(blocked.items.items.iter().any(|item| {
        item.reason_code.as_deref()
            == Some(crate::location::adoption::plan_reasons::ADOPTION_MEDIA_AMBIGUOUS)
    }));
}

/// FR-051: a destination file no tracked media claims is surfaced and left
/// exactly as it is. It is not a reason to refuse.
#[tokio::test]
async fn additional_destination_files_are_surfaced_and_left_alone() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("With Extras", 2019, &[("With.Extras.2019.mkv", 1500)])
        .await;
    let destination = fixture.move_folder_externally(&title);
    let stray = destination.join("the-users-own-notes.txt");
    std::fs::write(&stray, b"mine").expect("write an untracked file");

    let preview = fixture.preview(&[&title.id]).await;
    assert!(!preview.plan.blocks_start());
    let unmanaged = preview
        .plan
        .section(PlanItemKind::UnmanagedContent)
        .expect("the additional file is surfaced");
    assert_eq!(unmanaged.items.total, 1);

    fixture.start_and_settle(&[&title.id]).await;
    assert!(stray.exists(), "adoption never removes what it did not adopt");
}

// ── US3.3 ────────────────────────────────────────────────────────────────────

/// US3.3: the source is gone entirely — the mount went away, or the user moved
/// the content off a disk that is no longer attached. The destination is
/// provable from stored catalog information, so adoption proceeds.
#[tokio::test]
async fn us3_3_a_stale_source_mount_does_not_block_adoption() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("Off A Dead Disk", 2018, &[("Off.A.Dead.Disk.2018.mkv", 2048)])
        .await;
    let destination = fixture.move_folder_externally(&title);
    // The source root itself is gone, which is what an unmounted share looks
    // like from here.
    std::fs::remove_dir_all(fixture.root_a()).expect("drop the source root");

    let operation = fixture.start_and_settle(&[&title.id]).await;

    assert!(
        matches!(
            operation.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "a stale source mount must not stop an adoption: {:?} {:?}",
        operation.state,
        operation.detail
    );
    assert_eq!(fixture.title(&title.id).await.root_folder_id, fixture.root_b_id);
    assert_eq!(
        fixture.media_paths(&title.id).await,
        vec![
            destination
                .join("Off.A.Dead.Disk.2018.mkv")
                .to_string_lossy()
                .to_string()
        ]
    );
}

/// US3.3 across a restart: a resume must make the same allowance the start did,
/// or the FR-053 exemption would evaporate the first time the process bounced.
#[tokio::test]
async fn us3_3_a_stale_source_mount_does_not_block_a_resume_either() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("Resumed Off A Dead Disk", 2017, &[("Resumed.2017.mkv", 1024)])
        .await;
    fixture.move_folder_externally(&title);

    let preview = fixture.preview(&[&title.id]).await;
    let operation_id = "operation-adoption-stale-source";
    let operation = queued_operation(
        operation_id,
        LocationOperationType::Adoption,
        LocationExecutionMode::FilesAlreadyThere,
        preview.plan.verification.depth,
    );
    let plan_json = serde_json::to_string(&preview.execution).expect("serialize plan");
    fixture
        .app
        .services
        .library
        .location_operations
        .create_location_operation(&operation, Some(&plan_json))
        .await
        .expect("persist the operation");

    std::fs::remove_dir_all(fixture.root_a()).expect("drop the source root");

    let decision = fixture
        .app
        .resume_location_operation(operation_id)
        .await
        .expect("resume decision");
    assert!(
        decision.plan().is_some(),
        "an adoption resumes without its source root"
    );
}

// ── US3.4 ────────────────────────────────────────────────────────────────────

/// US3.4, first half: ownership updates only after verification. Every adopted
/// file has a persisted verification record by the time the catalog points at
/// it.
#[tokio::test]
async fn us3_4_catalog_ownership_changes_only_after_the_destination_is_verified() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("Proven First", 2016, &[("Proven.First.2016.mkv", 2048)])
        .await;
    let destination = fixture.move_folder_externally(&title);

    fixture.start_and_settle(&[&title.id]).await;

    let records = fixture.verifications();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].destination_path,
        destination
            .join("Proven.First.2016.mkv")
            .to_string_lossy()
            .to_string()
    );
    assert!(records[0].outcome.permits_source_removal());
    assert_eq!(fixture.title(&title.id).await.root_folder_id, fixture.root_b_id);
}

/// US3.4, second half: Scryer recycles a redundant source copy **only** when it
/// can prove redundancy. A persisted full hash plus a full-depth read-back is
/// that proof (FR-053).
#[tokio::test]
async fn us3_4_a_full_hash_proven_redundant_source_copy_is_recycled() {
    let fixture = AdoptionFixture::new().await;
    let bytes = content_of("Kept.Both.2015.mkv", 2048);
    let title = fixture
        .seed_title("Kept Both", 2015, &[("Kept.Both.2015.mkv", 2048)])
        .await;
    fixture
        .record_full_hash(&title.id, "Kept.Both.2015.mkv", &bytes)
        .await;
    // The user copied rather than moved, so both copies exist.
    fixture.copy_folder_externally(&title);
    let source = PathBuf::from(title.folder_path.clone().expect("source folder"))
        .join("Kept.Both.2015.mkv");
    assert!(source.exists());

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert!(
        !matches!(operation.state, LocationOperationState::Failed),
        "adoption failed: {:?}",
        operation.detail
    );

    let records = fixture.verifications();
    assert!(
        records[0].hashes.is_some(),
        "a full-hash proof records the hashes it proved against"
    );
    assert!(
        !source.exists(),
        "a source copy proven identical by full hash is recycled (FR-053)"
    );
}

/// US3.4, the default: without full-hash proof the source copy is the user's,
/// and adoption leaves it exactly where they put it.
#[tokio::test]
async fn us3_4_a_source_copy_without_full_hash_proof_is_left_for_the_user() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("Unproven Twin", 2014, &[("Unproven.Twin.2014.mkv", 2048)])
        .await;
    fixture.copy_folder_externally(&title);
    let source_folder = PathBuf::from(title.folder_path.clone().expect("source folder"));
    let source = source_folder.join("Unproven.Twin.2014.mkv");

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert!(
        !matches!(operation.state, LocationOperationState::Failed),
        "adoption failed: {:?}",
        operation.detail
    );

    let records = fixture.verifications();
    assert!(
        records[0].hashes.is_none(),
        "no full-hash proof means no licence to touch the source"
    );
    assert!(source.exists(), "source cleanup is left to the user (FR-053)");
    assert!(
        source_folder.exists(),
        "adoption never removes the user's source directories"
    );
    assert_eq!(fixture.title(&title.id).await.root_folder_id, fixture.root_b_id);
}

/// FR-042/043 for adoption: with no persisted hash to compare against, full
/// depth falls back to the quick floor and says so, rather than claiming a
/// guarantee it did not give.
#[tokio::test]
async fn an_adoption_without_a_persisted_hash_records_the_quick_floor_fallback() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("No Stored Hash", 2013, &[("No.Stored.Hash.2013.mkv", 1024)])
        .await;
    fixture.move_folder_externally(&title);

    fixture.start_and_settle(&[&title.id]).await;

    let records = fixture.verifications();
    assert_eq!(records[0].depth.requested, VerificationDepth::Full);
    assert_eq!(records[0].depth.applied, VerificationDepth::Quick);
    assert!(records[0].depth.fell_back);
    assert!(
        records[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("full-file hash")),
        "the record says why the guarantee is weaker: {:?}",
        records[0].detail
    );
}

/// FR-089: destination content that is not there any more is not something an
/// adoption may quietly proceed past.
#[tokio::test]
async fn an_adoption_stops_when_its_destination_content_disappears_before_it_runs() {
    let fixture = AdoptionFixture::new().await;
    let title = fixture
        .seed_title("Vanishing Act", 2012, &[("Vanishing.Act.2012.mkv", 1024)])
        .await;
    let destination = fixture.move_folder_externally(&title);

    let preview = fixture.preview(&[&title.id]).await;
    let operation_id = "operation-adoption-vanished";
    let operation = queued_operation(
        operation_id,
        LocationOperationType::Adoption,
        LocationExecutionMode::FilesAlreadyThere,
        preview.plan.verification.depth,
    );
    let plan_json = serde_json::to_string(&preview.execution).expect("serialize plan");
    fixture
        .app
        .services
        .library
        .location_operations
        .create_location_operation(&operation, Some(&plan_json))
        .await
        .expect("persist the operation");

    std::fs::remove_file(destination.join("Vanishing.Act.2012.mkv")).expect("remove destination");

    let outcome = fixture
        .app
        .run_root_move(operation_id, &preview.execution)
        .await
        .expect("the runner reports rather than panics");
    assert_eq!(outcome.state, LocationOperationState::Failed);
    assert_eq!(fixture.title(&title.id).await.root_folder_id, fixture.root_a_id);
}

// ── Cancel and resume (FR-092, FR-033) ───────────────────────────────────────

/// FR-092: a cancel stops at the next title checkpoint; the title that already
/// finished stays adopted and consistent.
#[tokio::test]
async fn an_adoption_cancels_at_a_title_boundary_and_leaves_finished_titles_alone() {
    let fixture = AdoptionFixture::new().await;
    let first = fixture
        .seed_title("First Adopted", 2011, &[("First.Adopted.2011.mkv", 1500)])
        .await;
    let second = fixture
        .seed_title("Second Adopted", 2010, &[("Second.Adopted.2010.mkv", 2500)])
        .await;
    let first_destination = fixture.move_folder_externally(&first);
    fixture.move_folder_externally(&second);

    let preview = fixture.preview(&[&first.id, &second.id]).await;
    assert_eq!(preview.execution.titles.len(), 2);

    let operation_id = "operation-adoption-cancel";
    let operation = queued_operation(
        operation_id,
        LocationOperationType::Adoption,
        LocationExecutionMode::FilesAlreadyThere,
        preview.plan.verification.depth,
    );
    let plan_json = serde_json::to_string(&preview.execution).expect("serialize plan");
    fixture
        .app
        .services
        .library
        .location_operations
        .create_location_operation(&operation, Some(&plan_json))
        .await
        .expect("persist the operation");

    fixture
        .operations
        .cancel_at_cancel_check(title_boundary_cancel_check(2, 1));
    let outcome = fixture
        .app
        .run_root_move(operation_id, &preview.execution)
        .await
        .expect("the runner stops cleanly");

    assert_eq!(outcome.state, LocationOperationState::Canceled);
    assert_eq!(
        fixture.media_paths(&first.id).await,
        vec![
            first_destination
                .join("First.Adopted.2011.mkv")
                .to_string_lossy()
                .to_string()
        ],
        "the title that finished before the cancel stays adopted"
    );
    assert_eq!(
        fixture.title(&second.id).await.root_folder_id,
        fixture.root_a_id,
        "the title the cancel stopped short of is untouched"
    );
}

/// FR-033/FR-092: an interrupted adoption picks up from its last checkpoint and
/// never re-verifies a file it already proved.
#[tokio::test]
async fn an_interrupted_adoption_resumes_without_reproving_settled_titles() {
    let fixture = AdoptionFixture::new().await;
    let first = fixture
        .seed_title("Settled Adoption", 2009, &[("Settled.Adoption.2009.mkv", 1500)])
        .await;
    let second = fixture
        .seed_title("Interrupted Adoption", 2008, &[("Interrupted.Adoption.2008.mkv", 2500)])
        .await;
    fixture.move_folder_externally(&first);
    let second_destination = fixture.move_folder_externally(&second);

    let preview = fixture.preview(&[&first.id, &second.id]).await;
    let operation_id = "operation-adoption-resume";
    let operation = LocationOperation {
        counters: crate::location::model::LocationOperationCounters {
            titles_total: 2,
            files_total: 2,
            bytes_total: 4000,
            ..Default::default()
        },
        ..queued_operation(
            operation_id,
            LocationOperationType::Adoption,
            LocationExecutionMode::FilesAlreadyThere,
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

    fixture
        .operations
        .crash_on_cancel_check(title_boundary_cancel_check(2, 1));
    let crashed = fixture.app.run_root_move(operation_id, &preview.execution).await;
    assert!(crashed.is_err(), "the armed store failure interrupts the run");

    let verifications_before = fixture.verifications().len();
    assert_eq!(verifications_before, 1);

    let plan = fixture
        .app
        .resume_location_operation(operation_id)
        .await
        .expect("resume decision")
        .plan()
        .expect("an interrupted adoption resumes");
    let outcome = fixture
        .app
        .run_root_move(operation_id, &plan)
        .await
        .expect("the resumed run finishes");

    assert!(
        matches!(
            outcome.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "the resumed adoption finishes: {:?} {:?}",
        outcome.state,
        outcome.detail
    );
    assert_eq!(
        fixture.verifications().len(),
        2,
        "the settled title's file is not proven a second time"
    );
    assert_eq!(
        fixture.media_paths(&second.id).await,
        vec![
            second_destination
                .join("Interrupted.Adoption.2008.mkv")
                .to_string_lossy()
                .to_string()
        ]
    );
}
