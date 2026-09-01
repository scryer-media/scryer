//! US4 — "change a root to a new path" — at the story level (T065).
//!
//! Everything here drives the use-case API (`preview_root_change`,
//! `start_root_change`, `resume_location_operation`, `run_root_move`) against
//! real directories and real files, so the assertions are about what ends up on
//! disk, in the catalog, and in the configured root.
//!
//! # What is deliberately not staged here
//!
//! A genuinely **cross-filesystem** copy cannot be staged inside one temp
//! directory: the mover decides rename-vs-copy from the actual device ids, and
//! two paths under one `tempdir` always share a device. A root change adds no
//! code to that path — it reuses `RootMoveFileMover`, `VerifiedCopier` and the
//! same reconciler as a root move, byte for byte — so the copy/verify/recycle
//! sequence is owned by `location::execution::tests` and `location::verify`.
//!
//! What a root change *does* add on a cross-device move is the recycle bin
//! filling up under the source root and then having to travel with the
//! operation. That is staged directly, by recycling a file into the source
//! root's bin before the operation runs — which is precisely the case the
//! decision is about: an entry recycled *before* the configuration flip.

use super::*;

use crate::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationState, LocationOperationType,
    VerificationDepth,
};
use crate::location::operations::LOCATION_OPERATION_VERIFICATION_DEPTH;
use crate::location::ownership_guard::OwnedEntity;
use crate::location::preview::{
    LOCATION_TYPED_CONFIRMATION_PHRASE, PlanConfirmationRequest, PlanItemKind,
};
use crate::location::root_change::{plan_reasons, refusal_codes, retirement_blockers};
use crate::location::root_change_execution::{RootChangePreview, RootChangePreviewRequest, StartRootChangeRequest};
use crate::location::test_support::{InMemoryLocationOperationStore, title_boundary_cancel_check};

/// One movie library with two configured roots. The first is the one being
/// changed; the second exists so the "that is consolidation, not a root change"
/// refusal has something to point at (FR-020).
struct RootChangeFixture {
    app: AppUseCase,
    user: User,
    operations: Arc<InMemoryLocationOperationStore>,
    temp: tempfile::TempDir,
    library_id: String,
    root_id: String,
    other_root_id: String,
}

impl RootChangeFixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().expect("root change tempdir");
        let source = temp.path().join("old-disk");
        let other = temp.path().join("other-root");
        std::fs::create_dir_all(&source).expect("create source root");
        std::fs::create_dir_all(&other).expect("create other root");

        let (app, user, _) =
            bootstrap_movie_scan_app(&source, Vec::new(), Arc::new(EmptySearchMetadataGateway))
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
                        path: source.to_string_lossy().to_string(),
                        is_default: true,
                    },
                    LibraryRootDraft {
                        path: other.to_string_lossy().to_string(),
                        is_default: false,
                    },
                ],
            )
            .await
            .expect("configure two roots");

        Self {
            root_id: library.roots[0].id.clone(),
            other_root_id: library.roots[1].id.clone(),
            library_id,
            app,
            user,
            operations,
            temp,
        }
    }

    fn source(&self) -> PathBuf {
        self.temp.path().join("old-disk")
    }

    fn other_root(&self) -> PathBuf {
        self.temp.path().join("other-root")
    }

    /// The new path. Deliberately not created: FR-020's destination is "a new
    /// unconfigured path", and a not-yet-existing directory with an existing
    /// parent is the ordinary shape of "I bought a new disk".
    fn destination(&self) -> PathBuf {
        self.temp.path().join("new-disk")
    }

    async fn seed_title(
        &self,
        name: &str,
        year: i32,
        folder_name: &str,
        files: &[(&str, usize)],
    ) -> Title {
        let folder = self.source().join(folder_name);
        std::fs::create_dir_all(&folder).expect("create title folder");
        let title = self.create_title(name, year).await;
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

    /// A monitored title on the root that owns no folder: FR-076's catalog-only
    /// case, which FR-023 still insists on accounting for.
    async fn seed_fileless_title(&self, name: &str, year: i32) -> Title {
        self.create_title(name, year).await
    }

    async fn create_title(&self, name: &str, year: i32) -> Title {
        let title = self
            .app
            .add_title(
                &self.user,
                NewTitle {
                    name: name.to_string(),
                    facet: MediaFacet::Movie,
                    monitored: true,
                    year: Some(year),
                    root_folder_id: Some(self.root_id.clone()),
                    ..Default::default()
                },
            )
            .await
            .expect("create movie title");
        self.app
            .services
            .catalog
            .titles
            .update_metadata(&title.id, None, None, None, Some(self.root_id.clone()))
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

    fn request(&self) -> RootChangePreviewRequest {
        RootChangePreviewRequest {
            library_id: self.library_id.clone(),
            root_id: self.root_id.clone(),
            destination_path: self.destination().to_string_lossy().to_string(),
            mode: LocationExecutionMode::MoveWithScryer,
        }
    }

    async fn preview(&self) -> RootChangePreview {
        self.app
            .preview_root_change(&self.user, self.request())
            .await
            .expect("preview root change")
    }

    async fn start_and_settle(&self) -> LocationOperation {
        let preview = self.preview().await;
        let accepted = self
            .app
            .start_root_change(
                &self.user,
                StartRootChangeRequest {
                    library_id: self.library_id.clone(),
                    root_id: self.root_id.clone(),
                    destination_path: self.destination().to_string_lossy().to_string(),
                    mode: LocationExecutionMode::MoveWithScryer,
                    confirmation: PlanConfirmationRequest {
                        fingerprint: preview.plan.fingerprint.clone(),
                        typed_confirmation: Some(
                            LOCATION_TYPED_CONFIRMATION_PHRASE.to_string(),
                        ),
                    },
                },
            )
            .await
            .expect("start root change");
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

    /// The root as the catalog holds it right now.
    async fn root(&self) -> scryer_domain::LibraryRoot {
        self.app
            .services
            .catalog
            .libraries
            .get_by_id(&self.library_id)
            .await
            .expect("load library")
            .expect("library exists")
            .roots
            .into_iter()
            .find(|root| root.id == self.root_id)
            .expect("the changed root still exists")
    }

    /// Recycle a file that lives under the source root, the way a cross-device
    /// copy would once its destination verified (FR-073).
    async fn recycle_under_source(&self, title_id: &str, relative: &str) -> PathBuf {
        let path = self.source().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create recycle fixture parent");
        }
        std::fs::write(&path, b"recycled-before-the-flip").expect("write recycled fixture");

        let config = self
            .app
            .recycle_bin_config_for_media_root(Some(self.source().to_string_lossy().as_ref()))
            .await;
        assert!(
            config.enabled,
            "the recycle bin has to be on for this story to mean anything"
        );
        crate::recycle_bin::recycle_file(
            &config,
            &path,
            crate::recycle_bin::RecycleManifest {
                schema: None,
                entry_id: None,
                source_operation_id: None,
                recycled_at: Utc::now().to_rfc3339(),
                original_path: path.to_string_lossy().to_string(),
                original_file_id: None,
                size_bytes: 24,
                title_id: Some(title_id.to_string()),
                media_root: Some(self.source().to_string_lossy().to_string()),
                reason: "location_operation_source".to_string(),
                status: None,
                replacement_file_id: None,
                replacement_path: None,
            },
        )
        .await
        .expect("recycle the fixture file")
        .expect("the bin took the file");
        path
    }
}

fn plan_items(
    preview: &RootChangePreview,
    kind: PlanItemKind,
) -> Vec<&crate::location::preview::PlanItem> {
    preview
        .plan
        .section(kind)
        .map(|section| section.items.items.iter().collect())
        .unwrap_or_default()
}

// ── US4.1 ────────────────────────────────────────────────────────────────────

/// US4.1, FR-023: every title assigned to the root is accounted for, and there
/// is no way to leave one out — the request has no selection to filter.
#[tokio::test]
async fn every_title_assigned_to_the_root_is_accounted_for_with_no_way_to_exclude_one() {
    let fixture = RootChangeFixture::new().await;
    let with_files = fixture
        .seed_title(
            "Accounted One",
            2021,
            "Accounted One (2021)",
            &[("Accounted.One.2021.mkv", 512), ("Accounted.One.2021.nfo", 32)],
        )
        .await;
    let also_with_files = fixture
        .seed_title(
            "Accounted Two",
            2022,
            "Accounted Two (2022)",
            &[("Accounted.Two.2022.mkv", 256)],
        )
        .await;
    let fileless = fixture.seed_fileless_title("Accounted Three", 2023).await;

    let preview = fixture.preview().await;

    assert_eq!(preview.accounting.assigned_total, 3);
    assert_eq!(preview.accounting.relocating, 2);
    assert_eq!(preview.accounting.catalog_only, 1);
    assert_eq!(preview.accounting.blocked, 0);
    assert!(
        preview.accounting.accounts_for_every_title(),
        "the ledger has to close: assigned == relocating + catalog-only + blocked"
    );

    // Every title reaches the instruction set, including the one with nothing
    // to move — its stored root path still has to change.
    let planned: Vec<&str> = preview
        .execution
        .titles
        .iter()
        .map(|title| title.title_id.as_str())
        .collect();
    for title_id in [&with_files.id, &also_with_files.id, &fileless.id] {
        assert!(
            planned.contains(&title_id.as_str()),
            "title {title_id} is missing from the confirmed plan"
        );
    }

    // The sidecar travels with its title (FR-027), so the file count the user
    // confirms includes it.
    assert_eq!(preview.plan.counts.files_total, 3);
    assert_eq!(preview.plan.counts.bytes_total, 512 + 32 + 256);

    // Nothing has happened yet: a preview is a read.
    assert!(!fixture.destination().exists());
    assert_eq!(fixture.root().await.path, fixture.source().to_string_lossy());
}

/// US4.1, second half + FR-023/FR-086: a blocked title is named in the preview
/// and stops the operation, because a root change cannot drop it either.
#[tokio::test]
async fn a_blocked_title_is_named_and_stops_the_root_change_until_it_is_repaired() {
    let fixture = RootChangeFixture::new().await;
    let moving = fixture
        .seed_title("Free Title", 2020, "Free Title (2020)", &[("Free.mkv", 128)])
        .await;
    let blocked = fixture
        .seed_title(
            "Held Title",
            2019,
            "Held Title (2019)",
            &[("Held.mkv", 128)],
        )
        .await;

    // FR-084: another location operation already owns it.
    fixture
        .app
        .services
        .library
        .location_operations
        .claim_location_operation_ownership(
            "some-other-operation",
            &[OwnedEntity::Title(blocked.id.clone())],
        )
        .await
        .expect("claim the title for another operation");

    let preview = fixture.preview().await;

    assert_eq!(preview.accounting.assigned_total, 2);
    assert_eq!(preview.accounting.blocked, 1);
    let named = preview
        .accounting
        .blocked_titles
        .first()
        .expect("the blocked title is named, not counted");
    assert_eq!(named.title_id, blocked.id);
    assert!(named.reason.contains("some-other-operation"));

    // It is a start blocker…
    assert!(preview.plan.blocks_start());
    let error = fixture
        .app
        .start_root_change(
            &fixture.user,
            StartRootChangeRequest {
                library_id: fixture.library_id.clone(),
                root_id: fixture.root_id.clone(),
                destination_path: fixture.destination().to_string_lossy().to_string(),
                mode: LocationExecutionMode::MoveWithScryer,
                confirmation: PlanConfirmationRequest {
                    fingerprint: preview.plan.fingerprint.clone(),
                    typed_confirmation: Some(LOCATION_TYPED_CONFIRMATION_PHRASE.to_string()),
                },
            },
        )
        .await
        .expect_err("a root change holding a blocked title cannot start");
    assert!(
        matches!(
            error,
            AppError::LocationPlanRefused {
                code: crate::location::preview::PlanConfirmationError::Blocked,
                ..
            }
        ),
        "got {error:?}"
    );

    // …and a retirement blocker, which is the same rule seen from the far end.
    assert!(
        preview
            .retirement
            .blocker(retirement_blockers::BLOCKED_TITLES)
            .is_some()
    );
    assert!(!preview.retirement.permits_source_removal());

    // The free title is still fully planned: FR-023 accounts for it, it just
    // cannot run yet.
    assert!(
        preview
            .execution
            .titles
            .iter()
            .any(|title| title.title_id == moving.id)
    );
    assert!(fixture.source().join("Free Title (2020)").exists());
}

// ── US4.2 ────────────────────────────────────────────────────────────────────

/// US4.2, FR-021/FR-078: the root keeps its id and its default status; only its
/// path changes, and every title still points at it.
#[tokio::test]
async fn the_root_keeps_its_identity_and_default_status_when_its_path_changes() {
    let fixture = RootChangeFixture::new().await;
    let title = fixture
        .seed_title(
            "Relocated",
            2024,
            "Relocated (2024)",
            &[("Relocated.2024.mkv", 1024), ("Relocated.2024.en.srt", 16)],
        )
        .await;
    let fileless = fixture.seed_fileless_title("Relocated Fileless", 2024).await;

    let before = fixture.root().await;
    assert!(before.is_default);

    let operation = fixture.start_and_settle().await;
    assert_eq!(
        operation.state,
        LocationOperationState::Completed,
        "detail: {:?}",
        operation.detail
    );
    assert_eq!(operation.operation_type, LocationOperationType::RootChange);

    let after = fixture.root().await;
    assert_eq!(after.id, before.id, "the synthetic root id is path-independent");
    assert_eq!(after.path, fixture.destination().to_string_lossy());
    assert!(after.is_default, "a path change never moves the default");

    // Both titles still point at the same root, and the one with files points
    // at the new paths.
    assert_eq!(fixture.title(&title.id).await.root_folder_id, fixture.root_id);
    assert_eq!(
        fixture.title(&fileless.id).await.root_folder_id,
        fixture.root_id
    );
    let relocated = fixture.title(&title.id).await;
    assert_eq!(
        relocated.folder_path.as_deref(),
        Some(
            fixture
                .destination()
                .join("Relocated (2024)")
                .to_string_lossy()
                .as_ref()
        )
    );
    for path in fixture.media_paths(&title.id).await {
        assert!(
            path.starts_with(&*fixture.destination().to_string_lossy()),
            "{path} was not re-anchored onto the new root"
        );
    }

    // FR-026: the relative layout is preserved, sidecar included.
    let destination_folder = fixture.destination().join("Relocated (2024)");
    assert!(destination_folder.join("Relocated.2024.mkv").exists());
    assert!(destination_folder.join("Relocated.2024.en.srt").exists());
    assert!(!fixture.source().join("Relocated (2024)").exists());

    // The legacy per-facet root-folder settings mirror the default library's
    // roots and nothing else in this subsystem changes a root path, so they are
    // updated here or they go stale.
    let mirrored = fixture
        .app
        .read_setting_string_value_for_scope(
            crate::SETTINGS_SCOPE_MEDIA,
            "movies.root_folders",
            None,
        )
        .await
        .expect("read the mirrored setting")
        .expect("the mirror was written");
    assert!(
        mirrored.contains(&*fixture.destination().to_string_lossy()),
        "the legacy mirror still names the old path: {mirrored}"
    );
    assert!(!mirrored.contains("old-disk"));
}

// ── US4.3 ────────────────────────────────────────────────────────────────────

/// US4.3, FR-027/FR-028: unknown content is listed separately, is never removed,
/// and keeps the old location standing — without stopping the titles moving.
#[tokio::test]
async fn unknown_content_is_listed_separately_and_keeps_the_old_location_standing() {
    let fixture = RootChangeFixture::new().await;
    let title = fixture
        .seed_title("Tidy", 2018, "Tidy (2018)", &[("Tidy.2018.mkv", 200)])
        .await;
    let stray = fixture.source().join("someone-elses-notes.txt");
    std::fs::write(&stray, b"not Scryer's").expect("write stray file");

    let preview = fixture.preview().await;

    assert_eq!(preview.content.unknown.len(), 1);
    assert_eq!(
        preview.content.unknown[0].path,
        stray.to_string_lossy().to_string()
    );
    let listed = plan_items(&preview, PlanItemKind::UnmanagedContent);
    assert_eq!(listed.len(), 1, "unknown content gets its own section");
    assert_eq!(
        listed[0].reason_code.as_deref(),
        Some(plan_reasons::UNKNOWN_ROOT_CONTENT)
    );
    assert!(
        preview
            .retirement
            .blocker(retirement_blockers::UNEXPLAINED_SOURCE_CONTENT)
            .is_some()
    );
    assert!(
        !preview.plan.blocks_start(),
        "unknown content blocks the removal, not the move"
    );

    let operation = fixture.start_and_settle().await;
    assert_eq!(
        operation.state,
        LocationOperationState::CompletedWithWarnings,
        "the user is told why the old location survived: {:?}",
        operation.detail
    );
    assert!(
        operation
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("not explained by the catalog")),
        "detail was {:?}",
        operation.detail
    );

    // The title moved…
    assert!(
        fixture
            .destination()
            .join("Tidy (2018)")
            .join("Tidy.2018.mkv")
            .exists()
    );
    assert_eq!(fixture.title(&title.id).await.root_folder_id, fixture.root_id);
    // …and the unexplained file is exactly where it was, in a source location
    // that was not taken away underneath it.
    assert!(stray.exists(), "unknown content was never deleted");
    assert!(fixture.source().exists());
}

// ── US4.4 ────────────────────────────────────────────────────────────────────

/// US4.4, FR-028/FR-031: only empty source directories are removed, and only
/// after every file's destination was verified.
#[tokio::test]
async fn only_empty_source_directories_are_removed_and_only_after_verification() {
    let fixture = RootChangeFixture::new().await;
    let title = fixture
        .seed_title(
            "Nested",
            2017,
            "Nested (2017)",
            &[("Nested.2017.mkv", 300)],
        )
        .await;
    // A root-level directory holding nothing unexplained: cleanup may take it
    // once it is empty.
    let empty_root_directory = fixture.source().join("empty-shelf");
    std::fs::create_dir_all(&empty_root_directory).expect("create empty root directory");

    let preview = fixture.preview().await;
    assert!(preview.retirement.empty_directories_only);
    assert!(preview.retirement.requires_verification_before_source_removal);
    assert!(preview.retirement.permits_source_removal());
    assert!(
        preview
            .retirement
            .removable_directories
            .contains(&empty_root_directory.to_string_lossy().to_string())
    );

    let operation = fixture.start_and_settle().await;
    assert_eq!(operation.state, LocationOperationState::Completed);

    // Every file was proven before its source was touched.
    let records = fixture.operations.verifications();
    assert_eq!(records.len(), 1);
    assert!(records[0].outcome.permits_source_removal());

    assert!(!empty_root_directory.exists(), "an empty directory is removable");
    assert!(!fixture.source().join("Nested (2017)").exists());
    assert!(
        !fixture.source().exists(),
        "with nothing unexplained left, the old location goes too"
    );
    assert!(
        fixture
            .destination()
            .join("Nested (2017)")
            .join("Nested.2017.mkv")
            .exists()
    );
    assert_eq!(fixture.title(&title.id).await.root_folder_id, fixture.root_id);
}

// ── US4.5 ────────────────────────────────────────────────────────────────────

/// US4.5, FR-029: a root-wide operation requires the stronger typed
/// confirmation, and the phrase is the shared one.
#[tokio::test]
async fn a_root_change_requires_the_stronger_typed_confirmation() {
    let fixture = RootChangeFixture::new().await;
    fixture
        .seed_title("Typed", 2016, "Typed (2016)", &[("Typed.mkv", 64)])
        .await;

    let preview = fixture.preview().await;
    assert!(preview.plan.confirmation.requires_typed_confirmation());
    assert_eq!(
        preview.plan.confirmation.typed_phrase.as_deref(),
        Some(LOCATION_TYPED_CONFIRMATION_PHRASE)
    );

    let start = async |typed: Option<&str>| {
        fixture
            .app
            .start_root_change(
                &fixture.user,
                StartRootChangeRequest {
                    library_id: fixture.library_id.clone(),
                    root_id: fixture.root_id.clone(),
                    destination_path: fixture.destination().to_string_lossy().to_string(),
                    mode: LocationExecutionMode::MoveWithScryer,
                    confirmation: PlanConfirmationRequest {
                        fingerprint: preview.plan.fingerprint.clone(),
                        typed_confirmation: typed.map(str::to_string),
                    },
                },
            )
            .await
    };

    assert!(
        start(None).await.is_err(),
        "a root-wide operation is not confirmed by pressing a button"
    );
    assert!(start(Some("move")).await.is_err(), "the phrase is exact");
    let accepted = start(Some(LOCATION_TYPED_CONFIRMATION_PHRASE))
        .await
        .expect("the typed phrase confirms the operation");
    fixture.settle(&accepted.operation.id).await;
}

// ── FR-020: the other branch is consolidation ────────────────────────────────

/// FR-020: "a new unconfigured path, **or** another existing root in the same
/// library — the latter being consolidation". The second branch is a different
/// planner (US5), so it is refused by name rather than run as a root change
/// over a destination that already holds content.
#[tokio::test]
async fn a_destination_that_is_already_a_configured_root_is_refused_as_consolidation() {
    let fixture = RootChangeFixture::new().await;
    fixture
        .seed_title("Folded", 2015, "Folded (2015)", &[("Folded.mkv", 64)])
        .await;

    let error = fixture
        .app
        .preview_root_change(
            &fixture.user,
            RootChangePreviewRequest {
                library_id: fixture.library_id.clone(),
                root_id: fixture.root_id.clone(),
                destination_path: fixture.other_root().to_string_lossy().to_string(),
                mode: LocationExecutionMode::MoveWithScryer,
            },
        )
        .await
        .expect_err("folding one root into another is not a root change");
    let message = error.to_string();
    assert!(
        message.contains(refusal_codes::DESTINATION_IS_CONFIGURED_ROOT),
        "the refusal has to carry a code the client can route on: {message}"
    );
    assert!(
        message.contains("consolidation"),
        "and it has to say which workflow this is: {message}"
    );
    assert!(
        message.contains(&fixture.other_root_id),
        "naming the root it collided with: {message}"
    );
}

/// FR-020: a destination that already holds content is refused too — a root
/// change writes into an empty or not-yet-created place, full stop.
#[tokio::test]
async fn a_destination_that_already_holds_content_is_refused() {
    let fixture = RootChangeFixture::new().await;
    let occupied = fixture.temp.path().join("new-disk");
    std::fs::create_dir_all(occupied.join("something")).expect("occupy the destination");

    let error = fixture
        .app
        .preview_root_change(&fixture.user, fixture.request())
        .await
        .expect_err("a non-empty destination is not a root-change destination");
    assert!(
        error.to_string().contains(refusal_codes::DESTINATION_NOT_EMPTY),
        "got {error}"
    );
}

// ── The recycle bin travels with the operation ───────────────────────────────

/// The bin that lived under the source root moves with the operation, so
/// housekeeping — which finds bins by enumerating *configured* library roots —
/// still sees it after the flip.
///
/// Without this, every file the operation recycled would sit at a path that is
/// no longer a configured root: never purged by retention, never listed, never
/// restorable. Silently, and forever.
#[tokio::test]
async fn the_recycle_bin_under_the_source_root_travels_with_the_operation() {
    let fixture = RootChangeFixture::new().await;
    let title = fixture
        .seed_title("Binned", 2014, "Binned (2014)", &[("Binned.mkv", 96)])
        .await;
    fixture
        .recycle_under_source(&title.id, "Binned (2014)/superseded.mkv")
        .await;

    let source_bin = fixture.source().join(".scryer-recycle");
    assert!(source_bin.exists(), "the fixture put an entry in the bin");

    // The bin is Scryer's own storage, not content the catalog failed to
    // explain, so it must not show up as unknown and must not block the
    // retirement of the old location.
    let preview = fixture.preview().await;
    assert!(
        preview.content.unknown.is_empty(),
        "the recycle bin was mistaken for unexplained content: {:?}",
        preview.content.unknown
    );
    assert!(preview.retirement.permits_source_removal());

    let operation = fixture.start_and_settle().await;
    assert_eq!(
        operation.state,
        LocationOperationState::Completed,
        "detail: {:?}",
        operation.detail
    );

    let destination_bin = fixture.destination().join(".scryer-recycle");
    assert!(destination_bin.exists(), "the bin travelled with the root");
    assert!(!source_bin.exists(), "and it did not stay behind as well");

    // Housekeeping's own enumeration is the proof that matters: it lists bins
    // by walking the configured library roots, which now means the new path.
    let listed = fixture
        .app
        .list_recycled_items(&fixture.user, None)
        .await
        .expect("list recycled items");
    assert_eq!(
        listed.len(),
        1,
        "the relocated bin is invisible to housekeeping"
    );
    assert!(
        listed[0]
            .original_path
            .starts_with(&*fixture.destination().to_string_lossy()),
        "the entry still records the retired path: {}",
        listed[0].original_path
    );
}

/// A file recycled *before* the configuration flipped restores onto the new
/// root path, because its recorded original path is re-anchored by the same
/// rebase the planner applied to the content.
#[tokio::test]
async fn a_file_recycled_before_the_flip_restores_onto_the_new_root_path() {
    let fixture = RootChangeFixture::new().await;
    let title = fixture
        .seed_title(
            "Restorable",
            2013,
            "Restorable (2013)",
            &[("Restorable.mkv", 96)],
        )
        .await;
    let recycled_from = fixture
        .recycle_under_source(&title.id, "Restorable (2013)/old-cut.mkv")
        .await;
    assert!(!recycled_from.exists(), "the bin took the file");

    let operation = fixture.start_and_settle().await;
    assert_eq!(operation.state, LocationOperationState::Completed);

    let listed = fixture
        .app
        .list_recycled_items(&fixture.user, None)
        .await
        .expect("list recycled items");
    let entry = listed.first().expect("the pre-flip entry survived the change");

    assert!(
        fixture
            .app
            .restore_recycled_item(&fixture.user, &entry.id)
            .await
            .expect("restore the pre-flip entry"),
    );

    let restored_to = fixture
        .destination()
        .join("Restorable (2013)")
        .join("old-cut.mkv");
    assert!(
        restored_to.exists(),
        "a pre-flip entry has to restore onto the new root, not the retired one"
    );
    assert!(
        !fixture.source().join("Restorable (2013)").exists(),
        "and certainly not back onto the location the change retired"
    );
}

/// A bin that cannot be moved is named, not lost.
///
/// The content is at its destination and verified by the time the tail runs, so
/// abandoning the whole operation over the bin would be the wrong trade — but
/// so would completing silently while every file the operation recycled sits at
/// a path nothing will ever look at again. The operation completes with a
/// warning that names the path the bin was left at, and the bin is still there.
#[tokio::test]
async fn a_recycle_bin_that_cannot_be_moved_is_named_rather_than_lost() {
    let fixture = RootChangeFixture::new().await;
    let title = fixture
        .seed_title("Stranded", 2007, "Stranded (2007)", &[("Stranded.mkv", 64)])
        .await;
    fixture
        .recycle_under_source(&title.id, "Stranded (2007)/superseded.mkv")
        .await;

    let preview = fixture.preview().await;
    let operation_id = "operation-stranded-bin";
    fixture
        .app
        .services
        .library
        .location_operations
        .create_location_operation(
            &crate::location::test_support::queued_operation(
                operation_id,
                LocationOperationType::RootChange,
                LocationExecutionMode::MoveWithScryer,
                preview.plan.verification.depth,
            ),
            Some(&serde_json::to_string(&preview.execution).expect("serialize plan")),
        )
        .await
        .expect("persist the operation");

    // Something is already sitting where the bin has to land, and it is not a
    // directory. The run is driven directly so this can be staged after the
    // plan was confirmed — which is the only moment it could happen in
    // production too.
    std::fs::create_dir_all(fixture.destination()).expect("create the destination");
    std::fs::write(fixture.destination().join(".scryer-recycle"), b"in the way")
        .expect("block the bin's landing spot");

    let outcome = fixture
        .app
        .run_root_move(operation_id, &preview.execution)
        .await
        .expect("the run itself does not fail over the bin");

    assert_eq!(outcome.state, LocationOperationState::CompletedWithWarnings);
    let detail = outcome.detail.clone().unwrap_or_default();
    assert!(
        detail.contains(&*fixture.source().join(".scryer-recycle").to_string_lossy()),
        "the warning has to name the path the bin was left at: {detail}"
    );
    assert!(
        fixture.source().join(".scryer-recycle").exists(),
        "the bin was lost rather than left behind"
    );

    // The content still moved and the root still flipped: the bin is the one
    // thing that did not.
    assert_eq!(fixture.root().await.path, fixture.destination().to_string_lossy());
    assert!(
        fixture
            .destination()
            .join("Stranded (2007)")
            .join("Stranded.mkv")
            .exists()
    );
}

/// The operator's carve-out: a bin configured to a **custom** path outside the
/// source root belongs to the whole installation, not to this root, so it stays
/// where it is. Its entries are re-anchored in place, which is what keeps a
/// pre-flip entry restorable.
#[tokio::test]
async fn a_custom_recycle_bin_outside_the_source_root_stays_where_it_is() {
    let fixture = RootChangeFixture::new().await;
    let custom_bin = fixture.temp.path().join("shared-bin");
    fixture
        .app
        .services
        .config
        .settings
        .upsert_setting_json(
            crate::SETTINGS_SCOPE_MEDIA,
            crate::RECYCLE_BIN_PATH_KEY,
            None,
            serde_json::to_string(&custom_bin.to_string_lossy().to_string())
                .expect("encode the custom bin path"),
            crate::SETTINGS_SOURCE_TYPED_GRAPHQL,
            None,
        )
        .await
        .expect("point the bin at a shared path");

    let title = fixture
        .seed_title("Shared Bin", 2006, "Shared Bin (2006)", &[("Shared.mkv", 64)])
        .await;
    fixture
        .recycle_under_source(&title.id, "Shared Bin (2006)/superseded.mkv")
        .await;
    assert!(custom_bin.exists(), "the fixture used the custom bin");

    let operation = fixture.start_and_settle().await;
    assert_eq!(
        operation.state,
        LocationOperationState::Completed,
        "detail: {:?}",
        operation.detail
    );

    assert!(custom_bin.exists(), "a shared bin does not travel with one root");
    assert!(
        !fixture.destination().join("shared-bin").exists(),
        "and it certainly does not get copied into the new root"
    );

    // Its entries were re-anchored in place, so a pre-flip entry still restores
    // onto the root's new path rather than being refused as outside it.
    let listed = fixture
        .app
        .list_recycled_items(&fixture.user, None)
        .await
        .expect("list recycled items");
    let entry = listed.first().expect("the pre-flip entry is still listed");
    assert!(
        entry
            .original_path
            .starts_with(&*fixture.destination().to_string_lossy()),
        "the entry still records the retired path: {}",
        entry.original_path
    );
}

// ── Resume ───────────────────────────────────────────────────────────────────

/// FR-033/FR-087: a restart picks the root change back up, finishes the
/// remaining titles, and then runs the tail — once, and idempotently, however
/// many times it is re-entered.
#[tokio::test]
async fn a_restart_resumes_a_root_change_and_finishes_the_retirement_exactly_once() {
    let fixture = RootChangeFixture::new().await;
    let first = fixture
        .seed_title("Resume One", 2012, "Resume One (2012)", &[("One.mkv", 64)])
        .await;
    let second = fixture
        .seed_title("Resume Two", 2011, "Resume Two (2011)", &[("Two.mkv", 64)])
        .await;

    let preview = fixture.preview().await;
    let operation_id = "operation-root-change-resume";
    let plan_json = serde_json::to_string(&preview.execution).expect("serialize plan");
    fixture
        .app
        .services
        .library
        .location_operations
        .create_location_operation(
            &crate::location::test_support::queued_operation(
                operation_id,
                LocationOperationType::RootChange,
                LocationExecutionMode::MoveWithScryer,
                preview.plan.verification.depth,
            ),
            Some(&plan_json),
        )
        .await
        .expect("persist the operation");

    // Die after the first title has settled: the operation is left
    // non-terminal, the second title untouched, and the tail never reached.
    fixture
        .operations
        .crash_on_cancel_check(title_boundary_cancel_check(2, 1));
    assert!(
        fixture
            .app
            .run_root_move(operation_id, &preview.execution)
            .await
            .is_err()
    );
    assert_eq!(
        fixture.root().await.path,
        fixture.source().to_string_lossy(),
        "the configuration must not move while titles are still on the old root"
    );

    // The resume reads the persisted plan — tail included — and carries on.
    let resumed = fixture
        .app
        .resume_location_operation(operation_id)
        .await
        .expect("resume decision");
    let plan = resumed.plan().expect("a root change resumes through the runner");
    assert!(
        plan.root_change.is_some(),
        "the root-scoped tail has to survive the round trip through the plan JSON"
    );
    let outcome = fixture
        .app
        .run_root_move(operation_id, &plan)
        .await
        .expect("the resumed run finishes");
    assert_eq!(outcome.state, LocationOperationState::Completed);

    assert_eq!(fixture.root().await.path, fixture.destination().to_string_lossy());
    for title in [&first.id, &second.id] {
        assert_eq!(fixture.title(title).await.root_folder_id, fixture.root_id);
        for path in fixture.media_paths(title).await {
            assert!(path.starts_with(&*fixture.destination().to_string_lossy()));
        }
    }
    assert!(!fixture.source().exists());

    // Re-entering the tail is harmless: every step asks what is already true.
    let again = fixture
        .app
        .run_root_move(operation_id, &plan)
        .await
        .expect("a second run over a settled operation is a read");
    assert_eq!(again.state, LocationOperationState::Completed);
    assert_eq!(fixture.root().await.path, fixture.destination().to_string_lossy());
}

// ── FR-084 ───────────────────────────────────────────────────────────────────

/// FR-084: the operation owns the root and every title assigned to it — the
/// catalog-only ones too, which carry no files and would otherwise be left open
/// to a scan or an import while their root is being replaced underneath them.
#[tokio::test]
async fn a_root_change_owns_the_root_and_every_assigned_title_including_the_fileless_ones() {
    let fixture = RootChangeFixture::new().await;
    let with_files = fixture
        .seed_title("Owned One", 2010, "Owned One (2010)", &[("One.mkv", 64)])
        .await;
    let fileless = fixture.seed_fileless_title("Owned Two", 2009).await;

    let preview = fixture.preview().await;
    let entities = crate::location::executor::owned_entities(
        &crate::location::test_support::queued_operation(
            "operation-ownership",
            LocationOperationType::RootChange,
            LocationExecutionMode::MoveWithScryer,
            preview.plan.verification.depth,
        ),
        &preview.execution.to_work_plan(),
    );
    for title_id in [&with_files.id, &fileless.id] {
        assert!(
            entities.contains(&OwnedEntity::Title(title_id.clone())),
            "title {title_id} is not owned for the operation's duration"
        );
    }
}

// ── Verification depth ───────────────────────────────────────────────────────

/// The verification-depth preference is an import preference. A root change
/// moves the user's only copy and recycles the source once the destination
/// verifies, so it plans full depth whatever the setting says.
#[tokio::test]
async fn a_root_change_verifies_at_full_depth_whatever_the_import_preference_says() {
    let fixture = RootChangeFixture::new().await;
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
        .seed_title("Depth", 2008, "Depth (2008)", &[("Depth.mkv", 64)])
        .await;

    let preview = fixture.preview().await;
    assert_eq!(
        preview.plan.verification.depth,
        LOCATION_OPERATION_VERIFICATION_DEPTH
    );
    assert_eq!(preview.plan.verification.depth, VerificationDepth::Full);

    let operation = fixture.start_and_settle().await;
    assert_eq!(operation.verification_depth, VerificationDepth::Full);
    let records = fixture.operations.verifications();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].depth.applied, VerificationDepth::Full);
    assert!(!records[0].depth.fell_back);
    let _ = title;
}
