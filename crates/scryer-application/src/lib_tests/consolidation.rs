//! US5 — "fold one root into another root of the same library" — at the story
//! level (T071).
//!
//! Everything here drives the use-case API (`preview_root_scope`,
//! `start_root_scope`, `resume_location_operation`, `run_root_move`)
//! against real directories and real files, so the assertions are about what
//! ends up on disk, in the catalog, and in the library's root configuration.
//!
//! # What is deliberately not staged here
//!
//! A genuinely **cross-filesystem** copy cannot be staged inside one temp
//! directory: the mover decides rename-vs-copy from the actual device ids, and
//! two paths under one `tempdir` always share a device. A consolidation adds no
//! code to that path — it reuses `RootMoveFileMover`, `VerifiedCopier`, and the
//! same reconciler as a root move — so the copy/verify/recycle sequence is owned
//! by `location::execution::tests` and `location::verify`.
//!
//! The merge engine's own SQL semantics are likewise owned by
//! `title_merge_store::tests`; what a consolidation adds is the *handoff*, and
//! that is staged directly (see the merge test below).

use super::*;

use crate::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationState, LocationOperationType,
    VerificationDepth,
};
use crate::location::operations::LOCATION_OPERATION_VERIFICATION_DEPTH;
use crate::location::preview::{
    LOCATION_TYPED_CONFIRMATION_PHRASE, PlanConfirmationRequest, PlanItemKind,
};
use crate::location::root_scope::retirement_blockers;
use crate::location::root_scope::{PlannedRootScope, plan_reasons, refusal_codes};
use crate::location::root_scope_execution::{
    RootScopeCall, RootScopeCallDestination, StartRootScopeRequest,
};
use crate::location::test_support::{
    InMemoryLocationOperationStore, InMemoryTitleMergeStore, queued_operation,
    title_boundary_cancel_check,
};

/// One movie library with two configured roots: `old-disk` is folded into
/// `keep-disk`, which already holds content.
struct ConsolidationFixture {
    app: AppUseCase,
    user: User,
    operations: Arc<InMemoryLocationOperationStore>,
    merges: Arc<InMemoryTitleMergeStore>,
    temp: tempfile::TempDir,
    library_id: String,
    source_root_id: String,
    destination_root_id: String,
}

impl ConsolidationFixture {
    /// `source_is_default` stages FR-022's two branches.
    async fn new(source_is_default: bool) -> Self {
        let temp = tempfile::tempdir().expect("consolidation tempdir");
        let source = temp.path().join("old-disk");
        let destination = temp.path().join("keep-disk");
        std::fs::create_dir_all(&source).expect("create source root");
        std::fs::create_dir_all(&destination).expect("create destination root");

        let (app, user, _) =
            bootstrap_movie_scan_app(&source, Vec::new(), Arc::new(EmptySearchMetadataGateway))
                .await;
        let operations = Arc::new(InMemoryLocationOperationStore::new());
        let merges = Arc::new(InMemoryTitleMergeStore::new());
        let app = app.with_test_overrides({
            let operations = operations.clone();
            let merges = merges.clone();
            move |services| {
                services
                    .with_location_operation_repository(operations)
                    .with_title_merge_repository(merges)
            }
        });
        merges.bind(
            app.services.catalog.titles.clone(),
            app.services.library.media_files.clone(),
        );

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
                        is_default: source_is_default,
                    },
                    LibraryRootDraft {
                        path: destination.to_string_lossy().to_string(),
                        is_default: !source_is_default,
                    },
                ],
            )
            .await
            .expect("configure two roots");

        Self {
            source_root_id: library.roots[0].id.clone(),
            destination_root_id: library.roots[1].id.clone(),
            library_id,
            app,
            user,
            operations,
            merges,
            temp,
        }
    }

    fn source(&self) -> PathBuf {
        self.temp.path().join("old-disk")
    }

    fn destination(&self) -> PathBuf {
        self.temp.path().join("keep-disk")
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_title_on(
        &self,
        root_id: &str,
        root_path: &Path,
        name: &str,
        year: i32,
        folder_name: Option<&str>,
        files: &[(&str, &[u8])],
        external_ids: Vec<ExternalId>,
    ) -> Title {
        let title = self
            .app
            .add_title(
                &self.user,
                NewTitle {
                    name: name.to_string(),
                    facet: MediaFacet::Movie,
                    monitored: true,
                    year: Some(year),
                    external_ids,
                    root_folder_id: Some(root_id.to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("create movie title");
        self.app
            .services
            .catalog
            .titles
            .update_metadata(&title.id, None, None, None, Some(root_id.to_string()))
            .await
            .expect("set title root");

        if let Some(folder_name) = folder_name {
            let folder = root_path.join(folder_name);
            std::fs::create_dir_all(&folder).expect("create title folder");
            self.app
                .services
                .catalog
                .titles
                .set_folder_path(&title.id, folder.to_string_lossy().as_ref())
                .await
                .expect("set title folder");

            for (file_name, content) in files {
                let path = folder.join(file_name);
                std::fs::write(&path, content).expect("write fixture file");
                self.seed_media_file(&title.id, &path, content).await;
            }
        }
        self.title(&title.id).await
    }

    /// A title on the root being folded away.
    async fn seed_source_title(
        &self,
        name: &str,
        year: i32,
        folder_name: &str,
        files: &[(&str, &[u8])],
        external_ids: Vec<ExternalId>,
    ) -> Title {
        self.seed_title_on(
            &self.source_root_id.clone(),
            &self.source(),
            name,
            year,
            Some(folder_name),
            files,
            external_ids,
        )
        .await
    }

    /// A monitored title on the source root that owns no folder: FR-076's
    /// catalog-only case, which FR-023 still insists on accounting for.
    async fn seed_fileless_source_title(&self, name: &str, year: i32) -> Title {
        self.seed_title_on(
            &self.source_root_id.clone(),
            &self.source(),
            name,
            year,
            None,
            &[],
            Vec::new(),
        )
        .await
    }

    /// A title that already lives on the destination root.
    async fn seed_destination_title(
        &self,
        name: &str,
        year: i32,
        folder_name: &str,
        files: &[(&str, &[u8])],
        external_ids: Vec<ExternalId>,
    ) -> Title {
        self.seed_title_on(
            &self.destination_root_id.clone(),
            &self.destination(),
            name,
            year,
            Some(folder_name),
            files,
            external_ids,
        )
        .await
    }

    /// One tracked media file with its full-BLAKE3 already persisted, which is
    /// what lets the collision engine prove a duplicate without reading a byte
    /// (D4, FR-073).
    async fn seed_media_file(&self, title_id: &str, path: &Path, content: &[u8]) -> String {
        let file_id = self
            .app
            .services
            .library
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title_id.to_string(),
                file_path: path.to_string_lossy().to_string(),
                size_bytes: content.len() as i64,
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .expect("seed media file row");
        self.app
            .services
            .library
            .media_files
            .update_media_file_content_hashes(
                &file_id,
                &crate::location::model::PersistedContentHashes {
                    full_blake3: blake3::hash(content).to_hex().to_string(),
                    move_crc: None,
                    crc_algorithm: None,
                    hash_computed_at: Some(Utc::now()),
                },
            )
            .await
            .expect("persist the fixture file's full hash");
        file_id
    }

    /// Give `title_id` the canonical identity a title already on the
    /// destination root carries, so the pair becomes an FR-024 (2) merge.
    ///
    /// It has to be done *after* creation: `create_or_get_existing` refuses to
    /// make two titles in one library share an external id — it returns the
    /// existing row instead — so a same-library merge pair cannot be seeded in
    /// one step. That is also how the pair arises in the wild: two rows created
    /// separately, and ids attached later by matching.
    async fn share_identity(&self, title_id: &str, external_ids: Vec<ExternalId>) -> Title {
        self.app
            .services
            .catalog
            .titles
            .replace_match_state(title_id, external_ids, Vec::new())
            .await
            .expect("share the destination title's identity");
        self.title(title_id).await
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

    async fn title_if_present(&self, title_id: &str) -> Option<Title> {
        self.app
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .expect("load title")
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

    fn request(&self) -> RootScopeCall {
        RootScopeCall {
            library_id: self.library_id.clone(),
            root_id: self.source_root_id.clone(),
            destination: RootScopeCallDestination::Root(self.destination_root_id.clone()),
            mode: LocationExecutionMode::MoveWithScryer,
        }
    }

    async fn preview(&self) -> PlannedRootScope {
        self.app
            .preview_root_scope(&self.user, &self.request())
            .await
            .expect("preview consolidation")
    }

    async fn start_and_settle(&self) -> LocationOperation {
        let preview = self.preview().await;
        let accepted = self
            .app
            .start_root_scope(
                &self.user,
                StartRootScopeRequest {
                    call: self.request(),
                    confirmation: PlanConfirmationRequest {
                        fingerprint: preview.plan.fingerprint.clone(),
                        typed_confirmation: Some(LOCATION_TYPED_CONFIRMATION_PHRASE.to_string()),
                    },
                },
            )
            .await
            .expect("start consolidation");
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

    async fn roots(&self) -> Vec<scryer_domain::LibraryRoot> {
        self.app
            .services
            .catalog
            .libraries
            .get_by_id(&self.library_id)
            .await
            .expect("load library")
            .expect("library exists")
            .roots
    }

    async fn root(&self, root_id: &str) -> Option<scryer_domain::LibraryRoot> {
        self.roots()
            .await
            .into_iter()
            .find(|root| root.id == root_id)
    }

    /// Recycle a file that lives under the source root, the way a cross-device
    /// copy would once its destination verified (FR-073).
    async fn recycle_under_source(&self, title_id: &str, relative: &str) -> PathBuf {
        let path = self.source().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create recycle fixture parent");
        }
        std::fs::write(&path, b"recycled-before-the-retirement").expect("write recycled fixture");

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
                size_bytes: 30,
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

fn external_id(source: &str, value: &str) -> ExternalId {
    ExternalId {
        source: source.to_string(),
        value: value.to_string(),
    }
}

fn plan_items(
    preview: &PlannedRootScope,
    kind: PlanItemKind,
) -> Vec<&crate::location::preview::PlanItem> {
    preview
        .plan
        .section(kind)
        .map(|section| section.items.items.iter().collect())
        .unwrap_or_default()
}

// ── US5.1 ────────────────────────────────────────────────────────────────────

/// US5.1, FR-023/FR-024: the preview classifies every one of the seven kinds of
/// thing a consolidation can meet, and the every-title ledger still closes.
#[tokio::test]
async fn the_preview_classifies_every_title_and_every_collision_kind() {
    let fixture = ConsolidationFixture::new(false).await;

    // (2) A destination title sharing a canonical identity: a merge — whose
    //     colliding file is (4) a media collision.
    fixture
        .seed_destination_title(
            "Overlap",
            2021,
            "Overlap (2021)",
            &[("Overlap.mkv", b"destination-copy")],
            vec![external_id("tmdb", "4242")],
        )
        .await;
    // (3) An unrelated destination title owning the folder name an incoming
    //     title calculates.
    fixture
        .seed_destination_title(
            "Resident Clash",
            2019,
            "Clash (2019)",
            &[("Clash.mkv", b"unrelated")],
            vec![external_id("tmdb", "999")],
        )
        .await;

    // (1) A title moving into an unused destination folder, carrying (5) a file
    //     identical to nothing and (6) a sidecar that collides with nothing.
    let unused = fixture
        .seed_source_title(
            "Alone",
            2020,
            "Alone (2020)",
            &[("Alone.mkv", b"alone"), ("movie.nfo", b"<nfo/>")],
            Vec::new(),
        )
        .await;
    let merging = fixture
        .seed_source_title(
            "Overlap",
            2021,
            "Overlap (2021)",
            &[("Overlap.mkv", b"incoming-copy")],
            Vec::new(),
        )
        .await;
    let merging = fixture
        .share_identity(&merging.id, vec![external_id("tmdb", "4242")])
        .await;
    let clashing = fixture
        .seed_source_title(
            "Clash",
            2019,
            "Clash (2019)",
            &[("Clash.mkv", b"mine")],
            Vec::new(),
        )
        .await;
    let fileless = fixture.seed_fileless_source_title("Nothing", 2018).await;
    // (7) Untracked content at the source root.
    let stray = fixture.source().join("someone-elses-notes.txt");
    std::fs::write(&stray, b"not Scryer's").expect("write stray file");

    let preview = fixture.preview().await;

    assert_eq!(preview.accounting.assigned_total, 4);
    assert!(
        preview.accounting.accounts_for_every_title(),
        "the FR-023 ledger has to close"
    );
    assert!(
        preview
            .classification
            .accounts_for(preview.accounting.assigned_total),
        "every title lands in exactly one FR-024 title-scoped bucket"
    );
    assert_eq!(preview.classification.moving_into_unused_folders, 1);
    assert_eq!(preview.classification.merging_with_destination_titles, 1);
    assert_eq!(preview.classification.folder_name_collisions, 1);
    assert_eq!(preview.classification.catalog_only, 1);
    assert_eq!(preview.classification.blocked, 0);
    assert_eq!(preview.classification.media_collisions, 1);
    assert_eq!(preview.classification.untracked_source_entries, 1);

    // Every assigned title reaches the instruction set, the fileless one
    // included: its stored root reference still has to change.
    let planned: Vec<&str> = preview
        .execution
        .titles
        .iter()
        .map(|title| title.title_id.as_str())
        .collect();
    for title_id in [&unused.id, &merging.id, &clashing.id, &fileless.id] {
        assert!(
            planned.contains(&title_id.as_str()),
            "title {title_id} is missing from the confirmed plan"
        );
    }

    // Nothing has happened yet: a preview is a read.
    assert!(fixture.source().join("Alone (2020)").exists());
    assert!(stray.exists());
    assert!(fixture.root(&fixture.source_root_id).await.is_some());
}

// ── US5.2 ────────────────────────────────────────────────────────────────────

/// US5.2, FR-025: two unrelated titles calculating the same destination folder
/// never merge over the name — the incoming folder gets a unique previewed name,
/// and both titles exist afterwards.
#[tokio::test]
async fn an_unrelated_title_with_the_same_folder_name_is_uniqued_rather_than_merged() {
    let fixture = ConsolidationFixture::new(false).await;
    let resident = fixture
        .seed_destination_title(
            "Blade Runner",
            1982,
            "Blade Runner (1982)",
            &[("Blade.Runner.1982.mkv", b"the resident cut")],
            vec![external_id("tmdb", "78")],
        )
        .await;
    let incoming = fixture
        .seed_source_title(
            "Blade Runner",
            1982,
            "Blade Runner (1982)",
            &[("Blade.Runner.1982.mkv", b"a different cut entirely")],
            // No shared identity: the same name is not evidence of the same
            // title (FR-025, FR-055).
            vec![external_id("tmdb", "78-remaster")],
        )
        .await;

    let preview = fixture.preview().await;
    assert_eq!(preview.classification.folder_name_collisions, 1);
    assert_eq!(preview.classification.merging_with_destination_titles, 0);
    let renamed = plan_items(&preview, PlanItemKind::Rename);
    let folder_rename = renamed
        .iter()
        .find(|item| item.reason_code.as_deref() == Some(plan_reasons::FOLDER_NAME_UNIQUED))
        .expect("US5.2: the changed folder name is shown before confirmation");
    let previewed_destination = folder_rename
        .destination_path
        .clone()
        .expect("the uniqued folder is named");
    assert!(
        previewed_destination.contains("(from old-disk)"),
        "the unique name reuses the collision engine's own suffix scheme: {previewed_destination}"
    );

    let operation = fixture.start_and_settle().await;
    // FR-055's same-name statement is a warning by design: two same-named
    // titles are about to sit side by side and the user is told so.
    assert_eq!(
        operation.state,
        LocationOperationState::CompletedWithWarnings,
        "detail: {:?}",
        operation.detail
    );
    assert!(
        operation
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("shares no metadata identity")),
        "detail was {:?}",
        operation.detail
    );

    // SC-004: the executed outcome is the previewed one, to the character.
    assert!(
        Path::new(&previewed_destination).exists(),
        "the incoming folder did not land where the preview promised"
    );
    assert!(
        fixture
            .destination()
            .join("Blade Runner (1982)")
            .join("Blade.Runner.1982.mkv")
            .exists(),
        "the resident title's content was overwritten"
    );
    assert_eq!(
        std::fs::read(
            fixture
                .destination()
                .join("Blade Runner (1982)")
                .join("Blade.Runner.1982.mkv")
        )
        .expect("read the resident file"),
        b"the resident cut".to_vec(),
        "FR-072: destination content always wins the pathname"
    );

    // Both titles still exist, on the same root.
    let incoming = fixture.title(&incoming.id).await;
    assert_eq!(incoming.root_folder_id, fixture.destination_root_id);
    assert_eq!(
        incoming.folder_path.as_deref(),
        Some(previewed_destination.as_str())
    );
    assert_eq!(
        fixture.title(&resident.id).await.root_folder_id,
        fixture.destination_root_id
    );
}

// ── US5.1 (2): the merge handoff ─────────────────────────────────────────────

/// US5.1 second classification, FR-055/FR-063: a source title that shares a
/// canonical identity with a title already on the destination root folds into
/// it — through the real merge engine, whose transaction removes the source
/// title row and leaves the destination owning the content.
#[tokio::test]
async fn an_overlapping_title_merges_into_the_destination_title_it_shares_an_identity_with() {
    let fixture = ConsolidationFixture::new(false).await;
    let destination = fixture
        .seed_destination_title(
            "Arrival",
            2016,
            "Arrival (2016)",
            &[("Arrival.2016.1080p.mkv", b"the good copy")],
            vec![external_id("tmdb", "329865")],
        )
        .await;
    let source = fixture
        .seed_source_title(
            "Arrival",
            2016,
            "Arrival (2016)",
            &[("Arrival.2016.720p.mkv", b"the older copy")],
            Vec::new(),
        )
        .await;
    let source = fixture
        .share_identity(&source.id, vec![external_id("tmdb", "329865")])
        .await;

    let preview = fixture.preview().await;
    assert_eq!(preview.classification.merging_with_destination_titles, 1);
    let merges = plan_items(&preview, PlanItemKind::Merge);
    assert!(
        merges.iter().any(|item| {
            item.reason_code.as_deref() == Some(plan_reasons::MERGES_WITH_DESTINATION_TITLE)
        }),
        "the merge is stated in the preview before anything is confirmed"
    );
    assert_eq!(
        preview
            .execution
            .title(&source.id)
            .expect("the merging title has instructions")
            .merge_target_title_id
            .as_deref(),
        Some(destination.id.as_str())
    );

    let operation = fixture.start_and_settle().await;
    assert_eq!(
        operation.state,
        LocationOperationState::Completed,
        "detail: {:?}",
        operation.detail
    );
    assert_eq!(operation.counters.merges, 1, "FR-091 counts the merge");
    assert_eq!(
        fixture.merges.executed().len(),
        1,
        "the real merge engine ran, not a stub"
    );

    // FR-063: the destination title keeps its folder, and the incoming file
    // landed inside it.
    let landed = fixture
        .destination()
        .join("Arrival (2016)")
        .join("Arrival.2016.720p.mkv");
    assert!(
        landed.exists(),
        "the incoming copy did not reach the merge target"
    );
    assert!(!fixture.source().join("Arrival (2016)").exists());
    assert!(
        fixture.title_if_present(&source.id).await.is_none(),
        "FR-067: the source title row is gone once its records transferred"
    );
    let survivor = fixture.title(&destination.id).await;
    assert_eq!(survivor.root_folder_id, fixture.destination_root_id);
    let paths = fixture.media_paths(&destination.id).await;
    assert!(
        paths.iter().any(|path| path == &landed.to_string_lossy()),
        "the merged media did not follow the surviving title: {paths:?}"
    );
}

// ── US5.3 ────────────────────────────────────────────────────────────────────

/// US5.3, FR-022: consolidating a default source root makes the destination the
/// default.
#[tokio::test]
async fn consolidating_the_default_root_hands_the_default_to_the_destination() {
    let fixture = ConsolidationFixture::new(true).await;
    fixture
        .seed_source_title(
            "Moved",
            2020,
            "Moved (2020)",
            &[("Moved.mkv", b"bytes")],
            Vec::new(),
        )
        .await;

    let preview = fixture.preview().await;
    assert!(preview.default_transfer.source_was_default);
    assert!(!preview.default_transfer.destination_was_default);
    assert!(preview.default_transfer.transfers_the_default());
    assert!(
        plan_items(&preview, PlanItemKind::CatalogChange)
            .iter()
            .any(|item| item.reason_code.as_deref()
                == Some(plan_reasons::DEFAULT_ROOT_TRANSFERRED)),
        "the default transfer is stated before the user confirms"
    );

    let operation = fixture.start_and_settle().await;
    assert_eq!(
        operation.state,
        LocationOperationState::Completed,
        "detail: {:?}",
        operation.detail
    );

    let roots = fixture.roots().await;
    assert_eq!(roots.len(), 1, "the source root was retired");
    assert_eq!(roots[0].id, fixture.destination_root_id);
    assert!(
        roots[0].is_default,
        "FR-022: the destination became default"
    );

    // The legacy per-facet mirror follows the surviving roots, or scanning and
    // import would keep pointing at a root the library no longer has.
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
    assert!(mirrored.contains("keep-disk"), "mirror was {mirrored}");
    assert!(!mirrored.contains("old-disk"), "mirror was {mirrored}");
}

/// US5.3 second half, FR-022: consolidating a non-default root leaves the
/// default exactly where it was.
#[tokio::test]
async fn consolidating_a_non_default_root_leaves_the_library_default_alone() {
    let fixture = ConsolidationFixture::new(false).await;
    fixture
        .seed_source_title(
            "Moved",
            2020,
            "Moved (2020)",
            &[("Moved.mkv", b"bytes")],
            Vec::new(),
        )
        .await;

    let preview = fixture.preview().await;
    assert!(!preview.default_transfer.transfers_the_default());
    assert!(
        preview.default_transfer.destination_becomes_default(),
        "the destination was already the default and stays it"
    );

    let operation = fixture.start_and_settle().await;
    assert_eq!(operation.state, LocationOperationState::Completed);

    let roots = fixture.roots().await;
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, fixture.destination_root_id);
    assert!(roots[0].is_default);
}

// ── US5.4 ────────────────────────────────────────────────────────────────────

/// US5.4, FR-026/FR-028/FR-087: the source root's relative folder layout is
/// preserved where nothing collides, only empty source directories are removed,
/// and the source root's *configuration* is retired last.
#[tokio::test]
async fn the_layout_is_preserved_and_the_source_root_configuration_is_retired_last() {
    let fixture = ConsolidationFixture::new(false).await;
    let nested = fixture
        .seed_source_title(
            "Nested",
            2017,
            "shelf/Nested (2017)",
            &[("Nested.2017.mkv", b"nested bytes")],
            Vec::new(),
        )
        .await;
    let fileless = fixture.seed_fileless_source_title("Fileless", 2016).await;
    let empty_shelf = fixture.source().join("empty-shelf");
    std::fs::create_dir_all(&empty_shelf).expect("create empty root directory");

    let preview = fixture.preview().await;
    assert!(preview.retirement.empty_directories_only);
    assert!(
        preview
            .retirement
            .requires_verification_before_source_removal
    );
    assert!(preview.retirement.permits_source_removal());
    assert!(
        preview
            .retirement
            .removable_directories
            .contains(&empty_shelf.to_string_lossy().to_string())
    );

    let operation = fixture.start_and_settle().await;
    assert_eq!(
        operation.state,
        LocationOperationState::Completed,
        "detail: {:?}",
        operation.detail
    );

    // FR-026: the whole relative position survived, nesting included.
    let landed = fixture
        .destination()
        .join("shelf")
        .join("Nested (2017)")
        .join("Nested.2017.mkv");
    assert!(landed.exists(), "the nested layout was not preserved");
    assert_eq!(
        fixture.title(&nested.id).await.folder_path.as_deref(),
        Some(
            fixture
                .destination()
                .join("shelf")
                .join("Nested (2017)")
                .to_string_lossy()
                .as_ref()
        )
    );

    // Every file was proven before its source was touched (FR-031/FR-044).
    let records = fixture.operations.verifications();
    assert_eq!(records.len(), 1);
    assert!(records[0].outcome.permits_source_removal());

    assert!(!empty_shelf.exists(), "an empty directory is removable");
    assert!(
        !fixture.source().exists(),
        "with nothing unexplained left, the old location goes too"
    );

    // Both titles now belong to the destination root, and the source root has
    // left the configuration entirely (FR-020, FR-087).
    for title_id in [&nested.id, &fileless.id] {
        assert_eq!(
            fixture.title(title_id).await.root_folder_id,
            fixture.destination_root_id
        );
    }
    assert!(
        fixture.root(&fixture.source_root_id).await.is_none(),
        "the source root's configuration was not retired"
    );
    assert!(
        fixture.root(&fixture.destination_root_id).await.is_some(),
        "FR-078: the destination root keeps the synthetic id every title now names"
    );
}

// ── FR-072/073/075: collisions and dedup ─────────────────────────────────────

/// FR-024 (5) + FR-073 + SC-003: a file proven identical by full BLAKE3 is
/// deduplicated — the destination copy survives, the redundant source copy is
/// recycled, and nothing is deleted.
#[tokio::test]
async fn an_identical_file_deduplicates_through_the_recycle_bin() {
    let fixture = ConsolidationFixture::new(false).await;
    let destination = fixture
        .seed_destination_title(
            "Twin",
            2015,
            "Twin (2015)",
            &[("Twin.mkv", b"identical content")],
            vec![external_id("tmdb", "1515")],
        )
        .await;
    let source = fixture
        .seed_source_title(
            "Twin",
            2015,
            "Twin (2015)",
            &[("Twin.mkv", b"identical content")],
            Vec::new(),
        )
        .await;
    let source = fixture
        .share_identity(&source.id, vec![external_id("tmdb", "1515")])
        .await;
    let source_file = fixture.source().join("Twin (2015)").join("Twin.mkv");

    let preview = fixture.preview().await;
    assert_eq!(preview.classification.dedup_eligible_files, 1);
    assert_eq!(preview.classification.media_collisions, 0);
    assert_eq!(preview.plan.counts.for_kind(PlanItemKind::Dedup), 1);

    let operation = fixture.start_and_settle().await;
    // Recycling during the run creates the bin under the source root, and the
    // bin never moves — so the source directory is left standing and named.
    assert_eq!(
        operation.state,
        LocationOperationState::CompletedWithWarnings,
        "detail: {:?}",
        operation.detail
    );
    assert_eq!(operation.counters.dedups, 1);

    assert!(
        !source_file.exists(),
        "the redundant source copy stayed put"
    );
    assert_eq!(
        std::fs::read(fixture.destination().join("Twin (2015)").join("Twin.mkv"))
            .expect("read the surviving copy"),
        b"identical content".to_vec()
    );
    // SC-003: recycled, never deleted. The bin stays under the source root,
    // which the fold retires from the configuration — so housekeeping no longer
    // enumerates it and the proof is the bin on disk.
    let bin = fixture.source().join(".scryer-recycle");
    let entries = std::fs::read_dir(&bin)
        .expect("the source root's bin")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .count();
    assert_eq!(entries, 1, "the duplicate was not recycled");
    let _ = (source, destination);
}

/// SC-003 + FR-073's recycle-unavailable path: with the bin switched off the
/// incoming copy is preserved under a new name, with a visible warning, and
/// permanent deletion is never the fallback.
#[tokio::test]
async fn an_identical_file_is_preserved_and_renamed_when_the_recycle_bin_is_off() {
    let fixture = ConsolidationFixture::new(false).await;
    fixture
        .app
        .services
        .config
        .settings
        .upsert_setting_json(
            crate::SETTINGS_SCOPE_MEDIA,
            crate::RECYCLE_BIN_ENABLED_KEY,
            None,
            "false".to_string(),
            crate::SETTINGS_SOURCE_TYPED_GRAPHQL,
            None,
        )
        .await
        .expect("switch the recycle bin off");

    fixture
        .seed_destination_title(
            "Twin",
            2015,
            "Twin (2015)",
            &[("Twin.mkv", b"identical content")],
            vec![external_id("tmdb", "1515")],
        )
        .await;
    let source = fixture
        .seed_source_title(
            "Twin",
            2015,
            "Twin (2015)",
            &[("Twin.mkv", b"identical content")],
            Vec::new(),
        )
        .await;
    fixture
        .share_identity(&source.id, vec![external_id("tmdb", "1515")])
        .await;

    let preview = fixture.preview().await;
    assert_eq!(
        preview.classification.dedup_eligible_files, 0,
        "a duplicate the bin cannot take is not deduplicated"
    );
    assert!(
        preview
            .warnings
            .iter()
            .any(|warning| warning.contains("preserved")),
        "the user is told before confirming: {:?}",
        preview.warnings
    );

    let operation = fixture.start_and_settle().await;
    assert!(
        matches!(
            operation.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "detail: {:?}",
        operation.detail
    );

    let folder = fixture.destination().join("Twin (2015)");
    assert_eq!(
        std::fs::read(folder.join("Twin.mkv")).expect("the destination copy survived"),
        b"identical content".to_vec(),
        "FR-072: destination content always wins the pathname"
    );
    let preserved: Vec<String> = std::fs::read_dir(&folder)
        .expect("read the merged folder")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name != "Twin.mkv")
        .collect();
    assert_eq!(
        preserved.len(),
        1,
        "the incoming copy was not preserved beside the destination's: {preserved:?}"
    );
}

// ── FR-027/FR-028 ────────────────────────────────────────────────────────────

/// FR-027/FR-028: unexplained content at the source is listed separately, never
/// removed, and keeps the source root *configured* — without stopping the titles
/// moving or the default transferring.
#[tokio::test]
async fn unexplained_source_content_keeps_the_source_root_configured() {
    let fixture = ConsolidationFixture::new(true).await;
    let title = fixture
        .seed_source_title(
            "Tidy",
            2018,
            "Tidy (2018)",
            &[("Tidy.mkv", b"tidy")],
            Vec::new(),
        )
        .await;
    let stray = fixture.source().join("someone-elses-notes.txt");
    std::fs::write(&stray, b"not Scryer's").expect("write stray file");

    let preview = fixture.preview().await;
    assert_eq!(preview.content.unknown.len(), 1);
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
        "unknown content blocks the root's removal, not the move"
    );

    let operation = fixture.start_and_settle().await;
    assert_eq!(
        operation.state,
        LocationOperationState::CompletedWithWarnings,
        "the user is told why the source root survived: {:?}",
        operation.detail
    );

    // The title moved…
    assert!(
        fixture
            .destination()
            .join("Tidy (2018)")
            .join("Tidy.mkv")
            .exists()
    );
    assert_eq!(
        fixture.title(&title.id).await.root_folder_id,
        fixture.destination_root_id
    );
    // …the stray file is exactly where it was, on a root that was not taken
    // away underneath it…
    assert!(stray.exists(), "unknown content was never deleted");
    assert!(
        fixture.root(&fixture.source_root_id).await.is_some(),
        "FR-028: a root is not removed while unexplained content remains"
    );
    // …and the default still moved, because which root new content lands on is
    // a different question from whether an old root can be deleted (FR-022).
    let source_root = fixture
        .root(&fixture.source_root_id)
        .await
        .expect("the source root survived");
    let destination_root = fixture
        .root(&fixture.destination_root_id)
        .await
        .expect("the destination root exists");
    assert!(!source_root.is_default);
    assert!(destination_root.is_default);
}

// ── The recycle bin never moves ──────────────────────────────────────────────

/// The operator decision, on the fold branch: the bin under the source root
/// stays where it is. It is still excluded from unexplained content, and the
/// source directory it keeps standing is named in the warnings rather than
/// failing the operation.
#[tokio::test]
async fn a_recycle_bin_under_the_source_root_is_left_where_it_is() {
    let fixture = ConsolidationFixture::new(false).await;
    let title = fixture
        .seed_source_title(
            "Binned",
            2014,
            "Binned (2014)",
            &[("Binned.mkv", b"kept")],
            Vec::new(),
        )
        .await;
    fixture
        .recycle_under_source(&title.id, "Binned (2014)/superseded.mkv")
        .await;

    let source_bin = fixture.source().join(".scryer-recycle");
    assert!(source_bin.exists(), "the fixture put an entry in the bin");

    // The bin is Scryer's own storage, not content the catalog failed to
    // explain, so it must not show up as unknown and must not block the plan.
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
        LocationOperationState::CompletedWithWarnings,
        "detail: {:?}",
        operation.detail
    );
    let detail = operation.detail.clone().unwrap_or_default();
    assert!(
        detail.contains(&*fixture.source().to_string_lossy()),
        "the warning has to name the source directory left standing: {detail}"
    );
    // D6: the bin is the only thing left, so the warning says so rather than
    // sending the user looking for content that is not there.
    assert!(
        detail.contains("was kept because it holds Scryer's recycle bin"),
        "the warning has to name the bin as the reason the root was kept: {detail}"
    );

    assert!(
        source_bin.exists(),
        "the bin was moved rather than left alone"
    );
    assert!(
        !fixture.destination().join(".scryer-recycle").exists(),
        "nothing put a bin under the destination root"
    );
}

// ── Resume ───────────────────────────────────────────────────────────────────

/// FR-033/FR-087: a restart picks the consolidation back up, finishes the
/// remaining titles, and only then retires the source root's configuration —
/// once, and idempotently, however many times the tail is re-entered.
#[tokio::test]
async fn a_restart_resumes_a_consolidation_and_retires_the_root_exactly_once() {
    let fixture = ConsolidationFixture::new(false).await;
    let first = fixture
        .seed_source_title(
            "Resume One",
            2012,
            "Resume One (2012)",
            &[("One.mkv", b"one")],
            Vec::new(),
        )
        .await;
    let second = fixture
        .seed_source_title(
            "Resume Two",
            2011,
            "Resume Two (2011)",
            &[("Two.mkv", b"two")],
            Vec::new(),
        )
        .await;

    let preview = fixture.preview().await;
    let operation_id = "operation-consolidation-resume";
    let plan_json = serde_json::to_string(&preview.execution).expect("serialize plan");
    fixture
        .app
        .services
        .library
        .location_operations
        .create_location_operation(
            &queued_operation(
                operation_id,
                LocationOperationType::RootConsolidation,
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
    assert!(
        fixture.root(&fixture.source_root_id).await.is_some(),
        "the configuration must not be retired while titles are still on the source root"
    );

    // The resume reads the persisted plan — tail included — and carries on.
    let resumed = fixture
        .app
        .resume_location_operation(operation_id)
        .await
        .expect("resume decision");
    let plan = resumed
        .plan()
        .expect("a consolidation resumes through the shared runner");
    let tail = plan
        .root_change
        .as_ref()
        .expect("the root-scoped tail has to survive the round trip through the plan JSON");
    assert!(
        tail.consolidation.is_some(),
        "and it has to still say which branch of FR-020 this is"
    );

    let outcome = fixture
        .app
        .run_root_move(operation_id, &plan)
        .await
        .expect("the resumed run finishes");
    assert_eq!(outcome.state, LocationOperationState::Completed);

    assert!(fixture.root(&fixture.source_root_id).await.is_none());
    for title_id in [&first.id, &second.id] {
        assert_eq!(
            fixture.title(title_id).await.root_folder_id,
            fixture.destination_root_id
        );
        for path in fixture.media_paths(title_id).await {
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
    assert_eq!(fixture.roots().await.len(), 1);
}

// ── FR-020, FR-023, FR-029, verification depth ───────────────────────────────

/// FR-020: a destination root id that names no root of this library describes
/// no plan at all. There is no other query to route the user to any more, so it
/// is the same "that does not exist" the source root gets.
#[tokio::test]
async fn a_destination_root_that_is_not_configured_here_is_not_found() {
    let fixture = ConsolidationFixture::new(false).await;
    let error = fixture
        .app
        .preview_root_scope(
            &fixture.user,
            &RootScopeCall {
                destination: RootScopeCallDestination::Root(
                    "not-a-root-of-this-library".to_string(),
                ),
                ..fixture.request()
            },
        )
        .await
        .expect_err("a destination that is not a root of this library does not exist");
    assert!(
        matches!(&error, AppError::NotFound(message)
            if message.contains("not-a-root-of-this-library")),
        "got {error:?}"
    );
}

/// US5's execution modes: **files are already there** is US3's adoption of a
/// destination folder, not a way to fold two configured roots together.
#[tokio::test]
async fn files_already_there_is_refused_as_a_consolidation_mode() {
    let fixture = ConsolidationFixture::new(false).await;
    let error = fixture
        .app
        .preview_root_scope(
            &fixture.user,
            &RootScopeCall {
                mode: LocationExecutionMode::FilesAlreadyThere,
                ..fixture.request()
            },
        )
        .await
        .expect_err("adoption is not a consolidation mode");
    assert!(
        matches!(
            &error,
            AppError::LocationRootRefused { code, .. }
                if *code == refusal_codes::FOLD.mode_not_supported
        ),
        "got {error:?}"
    );
}

/// FR-023/FR-086: a blocked title is named in the preview and stops the
/// consolidation, because a consolidation cannot drop it either.
#[tokio::test]
async fn a_blocked_title_is_named_and_stops_the_consolidation_until_it_is_repaired() {
    let fixture = ConsolidationFixture::new(false).await;
    let moving = fixture
        .seed_source_title(
            "Free",
            2020,
            "Free (2020)",
            &[("Free.mkv", b"free")],
            Vec::new(),
        )
        .await;
    let blocked = fixture
        .seed_source_title(
            "Held",
            2019,
            "Held (2019)",
            &[("Held.mkv", b"held")],
            Vec::new(),
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
            &[crate::location::ownership_guard::OwnedEntity::Title(
                blocked.id.clone(),
            )],
        )
        .await
        .expect("claim the title for another operation");

    let preview = fixture.preview().await;
    assert_eq!(preview.accounting.blocked, 1);
    let named = preview
        .accounting
        .blocked_titles
        .first()
        .expect("the blocked title is named, not counted");
    assert_eq!(named.title_id, blocked.id);
    assert!(named.reason.contains("some-other-operation"));
    assert!(preview.plan.blocks_start());

    let error = fixture
        .app
        .start_root_scope(
            &fixture.user,
            StartRootScopeRequest {
                call: fixture.request(),
                confirmation: PlanConfirmationRequest {
                    fingerprint: preview.plan.fingerprint.clone(),
                    typed_confirmation: Some(LOCATION_TYPED_CONFIRMATION_PHRASE.to_string()),
                },
            },
        )
        .await
        .expect_err("a consolidation holding a blocked title cannot start");
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
    assert!(
        preview
            .retirement
            .blocker(retirement_blockers::BLOCKED_TITLES)
            .is_some()
    );

    // The free title is still fully planned: FR-023 accounts for it, it just
    // cannot run yet.
    assert!(
        preview
            .execution
            .titles
            .iter()
            .any(|title| title.title_id == moving.id)
    );
    assert!(fixture.source().join("Free (2020)").exists());
}

/// FR-084 for the operation that has content on **both** sides: a
/// consolidation owns the source root, the destination root, every title
/// assigned to the source root (including the fileless ones FR-076 never gives
/// instructions to), and every destination title it merges into.
///
/// The destination root's other titles are deliberately *not* claimed. A
/// consolidation reads their folder names to avoid collisions (FR-025); it
/// never writes to them, and freezing a whole destination root's catalog for
/// the duration would be the global lock FR-084 exists to avoid.
#[tokio::test]
async fn a_consolidation_owns_both_roots_and_every_title_the_plan_touches() {
    let fixture = ConsolidationFixture::new(false).await;
    let merge_target = fixture
        .seed_destination_title(
            "Shared Identity",
            2016,
            "Shared Identity (2016)",
            &[("Shared.Identity.2016.mkv", b"the destination copy")],
            vec![external_id("tmdb", "808080")],
        )
        .await;
    let bystander = fixture
        .seed_destination_title(
            "Destination Neighbour",
            2004,
            "Destination Neighbour (2004)",
            &[("Destination.Neighbour.mkv", b"nobody touches this")],
            Vec::new(),
        )
        .await;
    let merging = fixture
        .seed_source_title(
            "Shared Identity",
            2016,
            "Shared Identity (2016)",
            &[("Shared.Identity.2016.720p.mkv", b"the incoming copy")],
            Vec::new(),
        )
        .await;
    let merging = fixture
        .share_identity(&merging.id, vec![external_id("tmdb", "808080")])
        .await;
    let plain = fixture
        .seed_source_title(
            "Plain Mover",
            2013,
            "Plain Mover (2013)",
            &[("Plain.Mover.mkv", b"just moves")],
            Vec::new(),
        )
        .await;
    let fileless = fixture
        .seed_fileless_source_title("No Files Here", 2007)
        .await;

    let preview = fixture.preview().await;
    let operation_id = "operation-consolidation-owns-both-sides";
    let operation = LocationOperation {
        source_root_id: preview.plan.header.source_root_id.clone(),
        destination_root_id: preview.plan.header.destination_root_id.clone(),
        ..queued_operation(
            operation_id,
            LocationOperationType::RootConsolidation,
            LocationExecutionMode::MoveWithScryer,
            preview.plan.verification.depth,
        )
    };
    let entities =
        crate::location::executor::owned_entities(&operation, &preview.execution.to_work_plan());

    for root_id in [&fixture.source_root_id, &fixture.destination_root_id] {
        assert!(
            entities.contains(&crate::location::ownership_guard::OwnedEntity::Root(
                root_id.clone()
            )),
            "root {root_id} is not owned for the operation's duration (FR-084)"
        );
    }
    for title_id in [&merging.id, &plain.id, &fileless.id, &merge_target.id] {
        assert!(
            entities.contains(&crate::location::ownership_guard::OwnedEntity::Title(
                title_id.clone()
            )),
            "title {title_id} is not owned for the operation's duration (FR-084)"
        );
    }
    assert!(
        !entities.contains(&crate::location::ownership_guard::OwnedEntity::Title(
            bystander.id.clone()
        )),
        "a destination title the plan only reads a folder name from must stay open"
    );

    // Drive the real claim, then stop at the first title boundary.
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
        .crash_on_cancel_check(title_boundary_cancel_check(1, 1));
    assert!(
        fixture
            .app
            .run_root_move(operation_id, &preview.execution)
            .await
            .is_err(),
        "the injected store failure aborts the run"
    );

    // Work on either root, or on any owned title, is refused.
    for entity in [
        crate::location::ownership_guard::OwnedEntity::Root(fixture.source_root_id.clone()),
        crate::location::ownership_guard::OwnedEntity::Root(fixture.destination_root_id.clone()),
        crate::location::ownership_guard::OwnedEntity::Title(merge_target.id.clone()),
        crate::location::ownership_guard::OwnedEntity::Title(fileless.id.clone()),
    ] {
        let outcome = fixture
            .app
            .services
            .library
            .location_operations
            .claim_location_operation_ownership(
                "operation-second-actor",
                std::slice::from_ref(&entity),
            )
            .await
            .expect("ask for the entity");
        let crate::ports::LocationOwnershipOutcome::Conflict(conflicts) = outcome else {
            panic!("{entity:?} must be refused to a second operation (FR-084)");
        };
        assert_eq!(conflicts[0].operation_id, operation_id);
    }

    // Root reconfiguration underneath the operation is refused too.
    let repointed = fixture
        .app
        .update_library(
            &fixture.user,
            &fixture.library_id,
            None,
            Some(Vec::new()),
            None,
        )
        .await;
    let Err(AppError::Validation(message)) = repointed else {
        panic!("retiring a held root must be refused: {repointed:?}");
    };
    assert!(message.contains(operation_id), "got {message}");

    // The bystander is untouched by any of it.
    fixture
        .app
        .delete_title(&fixture.user, &bystander.id, false, None)
        .await
        .expect("a destination title the plan never writes to is not blocked");
}

/// FR-029: a root-wide operation requires the stronger typed confirmation, and
/// the phrase is the shared one.
#[tokio::test]
async fn a_consolidation_requires_the_stronger_typed_confirmation() {
    let fixture = ConsolidationFixture::new(false).await;
    fixture
        .seed_source_title(
            "Typed",
            2016,
            "Typed (2016)",
            &[("Typed.mkv", b"typed")],
            Vec::new(),
        )
        .await;

    let preview = fixture.preview().await;
    assert_eq!(
        preview.plan.header.operation_type,
        LocationOperationType::RootConsolidation
    );
    assert!(preview.plan.confirmation.requires_typed_confirmation());
    assert_eq!(
        preview.plan.confirmation.typed_phrase.as_deref(),
        Some(LOCATION_TYPED_CONFIRMATION_PHRASE)
    );

    let start = async |typed: Option<&str>| {
        fixture
            .app
            .start_root_scope(
                &fixture.user,
                StartRootScopeRequest {
                    call: fixture.request(),
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

/// The verification-depth preference is an import preference. A consolidation
/// moves the user's only copy and recycles the source once the destination
/// verifies, so it plans full depth whatever the setting says.
#[tokio::test]
async fn a_consolidation_verifies_at_full_depth_whatever_the_import_preference_says() {
    let fixture = ConsolidationFixture::new(false).await;
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
    fixture
        .seed_source_title(
            "Depth",
            2008,
            "Depth (2008)",
            &[("Depth.mkv", b"depth")],
            Vec::new(),
        )
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
}
