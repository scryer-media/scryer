//! US6 — "move titles to another library" — at the story level, for the half of
//! it that has no destination match: FR-055's detection outcomes and FR-056's
//! transfer.
//!
//! Everything here drives the use-case API against real directories and real
//! files, so the assertions are about what ends up on disk and in the catalog.
//!
//! # The transfer repoints the title it already has
//!
//! FR-056 requires that a transfer preserve "valid title-specific settings,
//! tags, history, requests, monitored state, and media associations". The title
//! keeps its id, so it keeps all of that by construction: nothing keyed on a
//! title id is rewritten, because nothing needs to be. What changes is the
//! title's `library_id`, its `root_folder_id`, its folder, and the paths of its
//! media files — and "the source title is removed" is satisfied by the source
//! folder and its contents going through the same verified cleanup every move
//! uses. The alternative reading, creating a fresh destination row, would orphan
//! every one of the tables FR-056 names in order to satisfy a sentence about
//! preserving them.
//!
//! The tests below assert that identity directly: the transferred title is the
//! same row, carrying the same tags and monitored state, in a different library.

use super::*;

use crate::location::classify::{DestinationRequest, TitleLocationClass, reason_codes};
use crate::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationState, LocationOperationType,
};
use crate::location::operations::{RootMovePreviewRequest, StartRootMoveRequest};
use crate::location::preview::{PlanConfirmationRequest, PlanItemKind};
use crate::location::root_move::plan_reasons;
use crate::location::merge::MergedMediaRole;
use crate::location::merge::engine::MergeCatalogSnapshot;
use crate::location::merge::map::EpisodeIdentityFacts;
use crate::location::merge::roles::FileEpisodeRoleRow;
use crate::location::model::TitleCheckpointState;
use crate::location::test_support::{
    InMemoryLocationOperationStore, InMemoryTitleMergeStore, queued_operation,
    title_boundary_cancel_check,
};

/// Two movie libraries, each with its own real root directory, and the operation
/// store the runner checkpoints through.
struct TransferFixture {
    app: AppUseCase,
    user: User,
    operations: Arc<InMemoryLocationOperationStore>,
    /// The US7 merge engine, over the same in-memory catalog the app holds.
    merges: Arc<InMemoryTitleMergeStore>,
    temp: tempfile::TempDir,
    source_library_id: String,
    source_root_id: String,
    source_facet: MediaFacet,
    source_root_path: PathBuf,
    destination_library_id: String,
    destination_root_id: String,
    destination_facet: MediaFacet,
    destination_root_path: PathBuf,
}

impl TransferFixture {
    /// Two libraries with the given facets, each on its own real root. A movie
    /// source reuses the bootstrapped default library the way US6.1 does; an
    /// episodic source gets a library of its own, because a title's facet and
    /// its library's facet are one invariant and the default movie library
    /// cannot hold a series.
    async fn with_facets(source_facet: MediaFacet, destination_facet: MediaFacet) -> Self {
        let mut fixture = Self::new().await;
        if source_facet == MediaFacet::Movie && destination_facet == MediaFacet::Movie {
            return fixture;
        }

        if source_facet != MediaFacet::Movie {
            // Its own directory: a root path belongs to exactly one library, and
            // the bootstrapped movie library already claimed `library-a`.
            let root = fixture
                .temp
                .path()
                .join(format!("library-a-{}", source_facet.as_str()));
            std::fs::create_dir_all(&root).expect("create the episodic source root");
            let source_library = fixture
                .app
                .create_library(
                    &fixture.user,
                    source_facet.clone(),
                    format!("Source {}", source_facet.as_str()),
                    vec![LibraryRootDraft {
                        path: root.to_string_lossy().to_string(),
                        is_default: true,
                    }],
                    None,
                )
                .await
                .expect("create the episodic source library");
            fixture.source_root_path = root;
            fixture.source_root_id = source_library.roots[0].id.clone();
            fixture.source_library_id = source_library.id.clone();
        }
        fixture.source_facet = source_facet;

        if destination_facet != MediaFacet::Movie {
            let root = fixture
                .temp
                .path()
                .join(format!("library-b-{}", destination_facet.as_str()));
            std::fs::create_dir_all(&root).expect("create the episodic destination root");
            let destination_library = fixture
                .app
                .create_library(
                    &fixture.user,
                    destination_facet.clone(),
                    format!("Destination {}", destination_facet.as_str()),
                    vec![LibraryRootDraft {
                        path: root.to_string_lossy().to_string(),
                        is_default: true,
                    }],
                    None,
                )
                .await
                .expect("create the episodic destination library");
            fixture.destination_root_path = root;
            fixture.destination_root_id = destination_library.roots[0].id.clone();
            fixture.destination_library_id = destination_library.id.clone();
        }
        fixture.destination_facet = destination_facet;

        fixture
    }

    /// Give one facet its own title-folder template, so "the destination
    /// library's naming policy calculated the folder" is observable as a
    /// different folder name rather than only as an assertion about which
    /// function was called (FR-058).
    async fn set_folder_template(&self, facet: &MediaFacet, template: &str) {
        self.app
            .services
            .config
            .settings
            .upsert_setting_json(
                crate::SETTINGS_SCOPE_SYSTEM,
                crate::FOLDER_TEMPLATE_KEY,
                Some(facet.as_str().to_string()),
                serde_json::to_string(template).expect("serialize template"),
                "test",
                None,
            )
            .await
            .expect("set the folder template for the facet");
    }

    async fn new() -> Self {
        let temp = tempfile::tempdir().expect("transfer tempdir");
        let source_root = temp.path().join("library-a");
        let destination_root = temp.path().join("library-b");
        std::fs::create_dir_all(&source_root).expect("create source root");
        std::fs::create_dir_all(&destination_root).expect("create destination root");

        let (app, user, _) = bootstrap_movie_scan_app(
            &source_root,
            Vec::new(),
            Arc::new(EmptySearchMetadataGateway),
        )
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
        // The merge store reads the catalog the app was built with, so it is
        // bound once the app exists rather than constructed with it.
        merges.bind(
            app.services.catalog.titles.clone(),
            app.services.library.media_files.clone(),
        );

        let source_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
        let source_library = app
            .services
            .catalog
            .libraries
            .get_by_id(&source_library_id)
            .await
            .expect("read the default movie library")
            .expect("the default movie library exists");

        let destination_library = app
            .create_library(
                &user,
                MediaFacet::Movie,
                "Archive Movies".to_string(),
                vec![LibraryRootDraft {
                    path: destination_root.to_string_lossy().to_string(),
                    is_default: true,
                }],
                None,
            )
            .await
            .expect("create the destination movie library");

        Self {
            source_root_id: source_library.roots[0].id.clone(),
            source_library_id,
            source_facet: MediaFacet::Movie,
            source_root_path: source_root,
            destination_root_id: destination_library.roots[0].id.clone(),
            destination_library_id: destination_library.id.clone(),
            destination_facet: MediaFacet::Movie,
            destination_root_path: destination_root,
            app,
            user,
            operations,
            merges,
            temp,
        }
    }

    fn source_root(&self) -> PathBuf {
        self.source_root_path.clone()
    }

    fn destination_root(&self) -> PathBuf {
        self.destination_root_path.clone()
    }

    /// A monitored movie in the source library owning `folder_name` under the
    /// source root, with one tracked media file inside it.
    #[allow(clippy::too_many_arguments)]
    async fn seed_source_title(
        &self,
        name: &str,
        year: i32,
        folder_name: &str,
        file_name: &str,
        size: usize,
        tags: Vec<String>,
        external_ids: Vec<ExternalId>,
    ) -> Title {
        let title = self
            .create_title(
                name,
                year,
                tags,
                external_ids,
                &self.source_library_id,
                &self.source_root_id,
            )
            .await;

        let folder = self.source_root().join(folder_name);
        std::fs::create_dir_all(&folder).expect("create title folder");
        self.app
            .services
            .catalog
            .titles
            .set_folder_path(&title.id, folder.to_string_lossy().as_ref())
            .await
            .expect("set title folder");

        let path = folder.join(file_name);
        std::fs::write(&path, vec![b'x'; size]).expect("write fixture file");
        self.app
            .services
            .library
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: path.to_string_lossy().to_string(),
                size_bytes: size as i64,
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .expect("seed media file row");

        self.title(&title.id).await
    }

    /// A monitored source title owning `folder_name`, with several hashed files
    /// inside it. The hashes are what let the collision engine prove a
    /// duplicate against the destination without reading anything (D4).
    async fn seed_source_title_with_files(
        &self,
        name: &str,
        year: i32,
        folder_name: &str,
        external_ids: Vec<ExternalId>,
        files: &[(&str, &[u8])],
    ) -> Title {
        let title = self
            .create_title(
                name,
                year,
                Vec::new(),
                external_ids,
                &self.source_library_id,
                &self.source_root_id,
            )
            .await;

        let folder = self.source_root().join(folder_name);
        std::fs::create_dir_all(&folder).expect("create source title folder");
        self.app
            .services
            .catalog
            .titles
            .set_folder_path(&title.id, folder.to_string_lossy().as_ref())
            .await
            .expect("set source title folder");

        for (file_name, content) in files {
            let path = folder.join(file_name);
            std::fs::write(&path, content).expect("write source fixture file");
            self.seed_media_file(&title.id, &path, content).await;
        }

        self.title(&title.id).await
    }

    /// A title in the destination library that already owns a folder with
    /// content in it, so a merge has two sides and the collision engine has
    /// destination entries to plan against.
    ///
    /// Every seeded file carries a persisted full BLAKE3 derived from its
    /// content, because the FR-073 dedup gate refuses to call two files
    /// identical without one on both sides (D4).
    async fn seed_destination_title_with_files(
        &self,
        name: &str,
        year: i32,
        folder_name: &str,
        external_ids: Vec<ExternalId>,
        files: &[(&str, &[u8])],
    ) -> Title {
        let title = self
            .create_title(
                name,
                year,
                Vec::new(),
                external_ids,
                &self.destination_library_id,
                &self.destination_root_id,
            )
            .await;

        let folder = self.destination_root().join(folder_name);
        std::fs::create_dir_all(&folder).expect("create destination title folder");
        self.app
            .services
            .catalog
            .titles
            .set_folder_path(&title.id, folder.to_string_lossy().as_ref())
            .await
            .expect("set destination title folder");

        for (file_name, content) in files {
            let path = folder.join(file_name);
            std::fs::write(&path, content).expect("write destination fixture file");
            self.seed_media_file(&title.id, &path, content).await;
        }

        self.title(&title.id).await
    }

    /// One tracked media file with its full-BLAKE3 already persisted.
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

    /// A fileless title already living in the destination library, used to stage
    /// the FR-055 detection outcomes.
    async fn seed_destination_title(
        &self,
        name: &str,
        year: i32,
        external_ids: Vec<ExternalId>,
    ) -> Title {
        self.create_title(
            name,
            year,
            Vec::new(),
            external_ids,
            &self.destination_library_id,
            &self.destination_root_id,
        )
        .await
    }

    async fn create_title(
        &self,
        name: &str,
        year: i32,
        tags: Vec<String>,
        external_ids: Vec<ExternalId>,
        library_id: &str,
        root_id: &str,
    ) -> Title {
        // The facet follows the library the title is created in: the catalog
        // refuses a title whose facet does not match its library's.
        let facet = if library_id == self.destination_library_id {
            self.destination_facet.clone()
        } else {
            self.source_facet.clone()
        };
        let title = self
            .app
            .add_title_with_outcome_in_library(
                &self.user,
                NewTitle {
                    name: name.to_string(),
                    facet,
                    monitored: true,
                    year: Some(year),
                    tags,
                    external_ids,
                    root_folder_id: Some(root_id.to_string()),
                    ..Default::default()
                },
                library_id.to_string(),
            )
            .await
            .expect("create movie title")
            .title;
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

    fn destination(&self) -> DestinationRequest {
        DestinationRequest::to_library_root(
            self.destination_library_id.clone(),
            self.destination_root_id.clone(),
        )
    }

    async fn preview(&self, title_ids: &[&str]) -> crate::location::operations::RootMovePreview {
        self.app
            .preview_root_move(
                &self.user,
                RootMovePreviewRequest {
                    title_ids: title_ids.iter().map(|id| (*id).to_string()).collect(),
                    destination: self.destination(),
                },
            )
            .await
            .expect("preview cross-library transfer")
    }

    async fn start_and_settle(&self, title_ids: &[&str]) -> LocationOperation {
        let preview = self.preview(title_ids).await;
        let accepted = self
            .app
            .start_root_move(
                &self.user,
                StartRootMoveRequest {
                    title_ids: title_ids.iter().map(|id| (*id).to_string()).collect(),
                    destination: self.destination(),
                    confirmation: PlanConfirmationRequest {
                        fingerprint: preview.plan.fingerprint.clone(),
                        typed_confirmation: None,
                    },
                },
            )
            .await
            .expect("start cross-library transfer");
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
}

fn external_id(source: &str, value: &str) -> ExternalId {
    ExternalId {
        source: source.to_string(),
        value: value.to_string(),
    }
}

/// Every classified title in a preview, whatever group it landed in.
fn classified_titles(
    preview: &crate::location::operations::RootMovePreview,
) -> Vec<&crate::location::classify::TitleClassification> {
    preview.classification.titles.iter().collect()
}

fn items_of(
    preview: &crate::location::operations::RootMovePreview,
    kind: PlanItemKind,
) -> Vec<&crate::location::preview::PlanItem> {
    preview
        .plan
        .section(kind)
        .map(|section| section.items.items.iter().collect())
        .unwrap_or_default()
}

// ── US6.1 ────────────────────────────────────────────────────────────────────

/// US6.1: with no matching title in the destination, the transfer runs and the
/// title arrives whole — its own settings, tags, and monitored state intact, its
/// media rows following it, its folder named by the destination policy, and the
/// source removed only once the destination was verified.
#[tokio::test]
async fn a_transfer_without_a_destination_match_carries_the_title_into_the_destination_library() {
    let fixture = TransferFixture::new().await;
    let title = fixture
        .seed_source_title(
            "Transferred Movie",
            2019,
            // Deliberately stale: the destination library's naming policy is
            // what calculates the folder the title lands in (FR-056).
            "transferred.movie.2019",
            "Transferred.Movie.2019.1080p.mkv",
            4096,
            vec![
                "scryer:quality-profile:1080p".to_string(),
                "favourites".to_string(),
            ],
            Vec::new(),
        )
        .await;
    assert_eq!(title.library_id, fixture.source_library_id);

    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(preview.classification.counts.cross_library_transfer, 1);
    assert!(!preview.classification.blocks_start());
    // FR-055: nothing in the destination relates to this title.
    let classified = classified_titles(&preview);
    assert_eq!(classified[0].class, TitleLocationClass::CrossLibraryTransfer);
    assert_eq!(classified[0].merge_target_title_id(), None);
    assert_eq!(classified[0].same_named_destination_title_id(), None);
    // FR-056, stated before confirmation: the library changes, and inherited
    // behaviour is replaced by the destination library's.
    let transfer_item = items_of(&preview, PlanItemKind::CatalogChange)
        .into_iter()
        .find(|item| item.reason_code.as_deref() == Some(plan_reasons::LIBRARY_TRANSFER))
        .expect("the preview states the library change");
    assert_eq!(transfer_item.title_id.as_deref(), Some(title.id.as_str()));
    let detail = transfer_item.detail.clone().expect("transfer detail");
    assert!(
        detail.contains(&fixture.destination_library_id),
        "the detail names the destination library: {detail}"
    );

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert_eq!(operation.state, LocationOperationState::Completed);
    assert_eq!(
        operation.operation_type,
        LocationOperationType::CrossLibraryTransfer,
        "Activity shows a transfer, not a root move (FR-091)"
    );
    assert_eq!(operation.counters.titles_processed, 1);
    assert_eq!(operation.counters.files_processed, 1);

    // The catalog: the same row, in the destination library, on the destination
    // root, with everything the title held itself still on it (FR-056).
    let transferred = fixture.title(&title.id).await;
    assert_eq!(transferred.id, title.id, "the title keeps its identity");
    assert_eq!(transferred.library_id, fixture.destination_library_id);
    assert_eq!(transferred.root_folder_id, fixture.destination_root_id);
    assert!(transferred.monitored, "monitored state is preserved");
    assert_eq!(
        transferred.tags, title.tags,
        "title-specific settings and tags are preserved verbatim"
    );
    assert!(
        transferred
            .tags
            .iter()
            .filter(|tag| tag.starts_with("scryer:"))
            .count()
            == 1,
        "nothing the title only inherited from the source library was frozen onto it: {:?}",
        transferred.tags
    );

    // The filesystem: the destination naming policy calculated the folder, and
    // the source is gone only because its destination was verified (FR-044).
    let destination_folder = fixture.destination_root().join("Transferred Movie (2019)");
    assert!(
        destination_folder
            .join("Transferred.Movie.2019.1080p.mkv")
            .exists()
    );
    assert!(!fixture.source_root().join("transferred.movie.2019").exists());
    assert_eq!(
        transferred.folder_path.as_deref(),
        Some(destination_folder.to_string_lossy().as_ref())
    );
    assert_eq!(
        fixture.media_paths(&title.id).await,
        vec![
            destination_folder
                .join("Transferred.Movie.2019.1080p.mkv")
                .to_string_lossy()
                .to_string()
        ],
        "media associations follow the title rather than being rebuilt"
    );

    let records = fixture.operations.verifications();
    assert_eq!(records.len(), 1);
    assert!(records[0].outcome.permits_source_removal());
}

/// FR-055: a destination title with the same name and no shared identity is
/// never merged into. The transfer proceeds, both titles exist afterwards, and
/// the preview said so before it started.
#[tokio::test]
async fn a_same_named_destination_title_is_warned_about_and_never_merged_into() {
    let fixture = TransferFixture::new().await;
    let impostor = fixture
        .seed_destination_title("The Gift", 2015, vec![external_id("tmdb", "300000")])
        .await;
    let title = fixture
        .seed_source_title(
            "The Gift",
            2000,
            "The Gift (2000)",
            "The.Gift.2000.mkv",
            2048,
            Vec::new(),
            vec![external_id("tmdb", "10000")],
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(
        preview.classification.counts.cross_library_transfer, 1,
        "a same name is not a merge, so the title still transfers"
    );
    assert!(!preview.classification.blocks_start());
    let classified = classified_titles(&preview);
    assert_eq!(classified[0].merge_target_title_id(), None);
    assert_eq!(
        classified[0].same_named_destination_title_id(),
        Some(impostor.id.as_str()),
        "the preview names the same-named title it refused to merge into"
    );
    let warning = items_of(&preview, PlanItemKind::Warning)
        .into_iter()
        .find(|item| {
            item.reason_code.as_deref() == Some(plan_reasons::SAME_NAMED_DESTINATION_TITLE)
        })
        .expect("the preview warns about the same-named destination title");
    assert!(
        warning
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains(&impostor.id)),
        "the warning names the other title"
    );

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert_eq!(
        operation.state,
        LocationOperationState::CompletedWithWarnings,
        "the transfer succeeds, and the warning the preview showed is repeated in the outcome"
    );

    let transferred = fixture.title(&title.id).await;
    assert_eq!(transferred.library_id, fixture.destination_library_id);
    let survivor = fixture.title(&impostor.id).await;
    assert_eq!(
        survivor.library_id, fixture.destination_library_id,
        "the same-named destination title is untouched; both now exist"
    );
    assert_ne!(transferred.id, survivor.id);
}

// ── US7 ──────────────────────────────────────────────────────────────────────

fn merge_episode(id: &str, season: &str, number: &str) -> EpisodeIdentityFacts {
    EpisodeIdentityFacts {
        id: id.to_string(),
        episode_type: scryer_domain::EpisodeType::Standard,
        season_number: Some(season.to_string()),
        episode_number: Some(number.to_string()),
        absolute_number: None,
        collection_id: None,
    }
}

/// A Group 0 snapshot where both sides claim primary for the same episode
/// (FR-068/FR-070).
fn contested_slot_snapshot(
    source_title_id: &str,
    destination_title_id: &str,
) -> MergeCatalogSnapshot {
    let row = |file_id: &str, episode_id: &str| FileEpisodeRoleRow {
        file_id: file_id.to_string(),
        episode_id: episode_id.to_string(),
        role: MergedMediaRole::Primary,
        is_filler: false,
    };
    MergeCatalogSnapshot {
        source_title_id: source_title_id.to_string(),
        destination_title_id: destination_title_id.to_string(),
        source_episodes: vec![merge_episode("s-e1", "1", "1")],
        destination_episodes: vec![merge_episode("d-e1", "1", "1")],
        source_file_episode_rows: vec![row("incoming-file", "s-e1")],
        destination_file_episode_rows: vec![row("destination-file", "d-e1")],
        ..MergeCatalogSnapshot::default()
    }
}

/// The FR-066 refusal, staged as a read-phase snapshot: the source has an
/// episode no destination episode can be mapped onto, and a source media file
/// sits on it.
fn unmappable_snapshot(source_title_id: &str, destination_title_id: &str) -> MergeCatalogSnapshot {
    let episode = merge_episode;
    MergeCatalogSnapshot {
        source_title_id: source_title_id.to_string(),
        destination_title_id: destination_title_id.to_string(),
        source_episodes: vec![episode("s-e1", "1", "1"), episode("s-e2", "1", "2")],
        destination_episodes: vec![episode("d-e1", "1", "1")],
        source_file_episode_rows: vec![FileEpisodeRoleRow {
            file_id: "stranded-file".to_string(),
            episode_id: "s-e2".to_string(),
            role: MergedMediaRole::Primary,
            is_filler: false,
        }],
        ..MergeCatalogSnapshot::default()
    }
}

/// US7.1–US7.5 end to end: a unique canonical identity match is a merge, and a
/// merge is a startable cross-library transfer.
///
/// Both sides carry files; one of the incoming files is byte-identical to a
/// file the destination already has, so FR-073 deduplicates it rather than
/// landing a second copy; the rest move into the *destination title's* folder
/// (FR-063) rather than into a folder named after the title that is about to
/// stop existing. When it settles the source title is gone (FR-067), the
/// destination title owns the media, the checkpoint records where the title
/// went, and Activity counts one merge (FR-091).
#[tokio::test]
async fn a_unique_identity_match_merges_into_the_destination_title() {
    let fixture = TransferFixture::new().await;
    let destination = fixture
        .seed_destination_title_with_files(
            "Same Film",
            2018,
            "Same Film (2018)",
            vec![external_id("tmdb", "4242")],
            &[("Same.Film.2018.Bonus.mkv", b"identical-bonus-content")],
        )
        .await;
    let title = fixture
        .seed_source_title_with_files(
            "Same Film",
            2018,
            "Same Film 2018",
            vec![external_id("tmdb", "4242")],
            &[
                ("Same.Film.2018.mkv", b"the incoming feature"),
                ("Same.Film.2018.Bonus.mkv", b"identical-bonus-content"),
            ],
        )
        .await;

    // ── Preview ─────────────────────────────────────────────────────────────
    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(
        preview.classification.counts.cross_library_transfer, 1,
        "a merge rides the cross-library machinery rather than blocking"
    );
    assert!(!preview.classification.blocks_start());
    let classified = classified_titles(&preview);
    assert_eq!(
        classified[0].merge_target_title_id(),
        Some(destination.id.as_str())
    );

    // FR-071: the merge summary is on the plan, and it is what the fingerprint
    // was taken over.
    assert_eq!(preview.plan.merges.len(), 1);
    assert_eq!(preview.plan.merges[0].destination_title_id, destination.id);
    assert!(!preview.plan.merges[0].is_blocked());
    let merge_item = items_of(&preview, PlanItemKind::Merge)
        .into_iter()
        .find(|item| item.reason_code.as_deref() == Some(plan_reasons::TITLE_MERGE))
        .expect("the preview states the merge");
    assert!(
        merge_item
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains(&destination.id))
    );

    // FR-063 + FR-073: the files are planned into the destination title's own
    // folder, and the byte-identical bonus feature deduplicates against it.
    let planned = preview
        .execution
        .title(&title.id)
        .expect("the merge title is executable work");
    assert_eq!(
        planned.merge_target_title_id.as_deref(),
        Some(destination.id.as_str())
    );
    assert_eq!(
        planned.destination_folder_path.as_deref(),
        Some(
            fixture
                .destination_root()
                .join("Same Film (2018)")
                .to_string_lossy()
                .as_ref()
        ),
        "the merged content lands in the surviving title's folder"
    );
    assert_eq!(
        planned.deduplicated_sources.len(),
        1,
        "the byte-identical incoming copy is recycled, not written twice"
    );
    assert_eq!(planned.files.len(), 1, "only the new feature is copied");

    // OQ7: the preview has no operation of its own to exclude.
    assert_eq!(fixture.merges.excluded_operations(), vec![None]);

    // SC-004: what the preview counts is what Activity will count. The plan's
    // merge and dedup facts are the runner's counters once the title settles.
    let previewed_merges = preview
        .execution
        .titles
        .iter()
        .filter(|title| title.merges())
        .count();
    let previewed_dedups: usize = preview
        .execution
        .titles
        .iter()
        .map(|title| title.deduplicated_sources.len())
        .sum();
    assert_eq!(previewed_merges, 1);
    assert_eq!(previewed_dedups, 1);

    // ── Execution ───────────────────────────────────────────────────────────
    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert!(
        matches!(
            operation.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "unexpected terminal state: {:?} ({:?})",
        operation.state,
        operation.detail
    );

    // FR-067: the source title is removed, and only after the transaction.
    assert!(
        fixture
            .app
            .services
            .catalog
            .titles
            .get_by_id(&title.id)
            .await
            .expect("read the source title")
            .is_none(),
        "the merged-away title is gone"
    );
    let survivor = fixture.title(&destination.id).await;
    assert_eq!(survivor.library_id, fixture.destination_library_id);

    // The destination title owns the merged media, at its own folder.
    let paths = fixture.media_paths(&destination.id).await;
    assert!(
        paths.iter().any(|path| path.ends_with("Same.Film.2018.mkv")),
        "the incoming feature followed the merge: {paths:?}"
    );
    assert!(
        paths
            .iter()
            .all(|path| path.starts_with(&*fixture.destination_root().to_string_lossy())),
        "nothing is left pointing at the source root: {paths:?}"
    );

    // FR-091 / SC-004: the preview's merge count is the outcome's merge count.
    assert_eq!(operation.counters.merges, previewed_merges as i64);
    assert_eq!(operation.counters.dedups, previewed_dedups as i64);
    assert_eq!(operation.counters.merges, 1);

    // FR-073: the redundant source row does not survive as a second row on the
    // merged title pointing into the recycle bin.
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.ends_with("Same.Film.2018.Bonus.mkv"))
            .count(),
        1,
        "the deduplicated copy leaves exactly one catalog row: {paths:?}"
    );
    let checkpoint = fixture
        .operations
        .checkpoint(&operation.id, &title.id)
        .expect("the merged title has a checkpoint");
    assert_eq!(
        checkpoint.placement.merged_into_title_id.as_deref(),
        Some(destination.id.as_str()),
        "D8: the checkpoint records the title this one merged into"
    );

    // The engine ran once, with the plan the preview described, and Group 0 was
    // told to exclude the operation performing the merge (OQ7).
    let executed = fixture.merges.executed();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].destination_title_id, destination.id);
    assert!(
        fixture
            .merges
            .excluded_operations()
            .iter()
            .any(|excluded| excluded.as_deref() == Some(operation.id.as_str())),
        "the merging operation excludes itself from the OQ7 resumable-operation check"
    );
}

/// FR-068–FR-070 / US7.3: both sides hold a primary for the same logical slot,
/// so the destination's stays primary and the incoming one becomes additional.
/// The demotion is in the preview before anything runs, and it is never silent.
#[tokio::test]
async fn an_incoming_primary_is_demoted_and_the_preview_says_so_first() {
    let fixture = TransferFixture::new().await;
    let destination = fixture
        .seed_destination_title("Two Primaries", 2021, vec![external_id("tmdb", "7007")])
        .await;
    let title = fixture
        .seed_source_title(
            "Two Primaries",
            2021,
            "Two Primaries (2021)",
            "Two.Primaries.2021.mkv",
            1024,
            Vec::new(),
            vec![external_id("tmdb", "7007")],
        )
        .await;
    fixture.merges.stage_snapshot(
        &title.id,
        &destination.id,
        contested_slot_snapshot(&title.id, &destination.id),
    );

    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(preview.classification.counts.cross_library_transfer, 1);
    assert!(!preview.classification.blocks_start());

    let summary = &preview.plan.merges[0];
    assert_eq!(summary.role_changes.len(), 1);
    let change = &summary.role_changes[0];
    assert_eq!(change.previous_role, MergedMediaRole::Primary);
    assert_eq!(change.new_role, MergedMediaRole::Additional);
    assert_eq!(change.destination_episode_id.as_deref(), Some("d-e1"));

    let stated = items_of(&preview, PlanItemKind::RoleChange)
        .into_iter()
        .filter(|item| item.reason_code.as_deref() == Some(plan_reasons::MERGE_ROLE_CHANGE))
        .filter_map(|item| item.detail.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(stated.len(), 1, "FR-070: the demotion has its own line");
    assert!(stated[0].contains("additional"), "{}", stated[0]);
}

/// FR-066 / US7.4: episode identities that cannot be mapped block the operation
/// at preview time, naming the records rather than attaching them to a guess.
#[tokio::test]
async fn an_unmappable_episode_blocks_the_merge_before_anything_starts() {
    let fixture = TransferFixture::new().await;
    let destination = fixture
        .seed_destination_title("Blocked Merge", 2019, vec![external_id("tmdb", "5150")])
        .await;
    let title = fixture
        .seed_source_title(
            "Blocked Merge",
            2019,
            "Blocked Merge (2019)",
            "Blocked.Merge.2019.mkv",
            1024,
            Vec::new(),
            vec![external_id("tmdb", "5150")],
        )
        .await;
    fixture
        .merges
        .stage_snapshot(&title.id, &destination.id, unmappable_snapshot(&title.id, &destination.id));

    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(preview.classification.counts.needs_resolution, 1);
    assert_eq!(preview.classification.counts.cross_library_transfer, 0);
    assert!(preview.classification.blocks_start());

    let classified = classified_titles(&preview);
    assert_eq!(
        classified[0].reason_code.as_deref(),
        Some(reason_codes::MERGE_RECORDS_UNMAPPED)
    );
    let reason = classified[0].reason.as_deref().expect("reason");
    assert!(reason.contains("episodes"), "{reason}");
    assert!(reason.contains("s-e2"), "{reason}");

    // The refusal is on the plan too, one item per blocked record: FR-066
    // blocks on the slot the merge is carrying a file onto, and names it.
    let blocked: Vec<&str> = items_of(&preview, PlanItemKind::Blocked)
        .into_iter()
        .filter(|item| item.reason_code.as_deref() == Some(plan_reasons::MERGE_RECORDS_UNMAPPED))
        .filter_map(|item| item.detail.as_deref())
        .collect();
    assert!(
        blocked.iter().any(|detail| detail.contains("episodes")),
        "the unmappable episode itself is named: {blocked:?}"
    );
    assert!(
        blocked.iter().any(|detail| detail.contains("s-e2")),
        "so is the slot that cannot be placed: {blocked:?}"
    );
    assert!(preview.plan.merges[0].is_blocked());

    let error = fixture
        .app
        .start_root_move(
            &fixture.user,
            StartRootMoveRequest {
                title_ids: vec![title.id.clone()],
                destination: fixture.destination(),
                confirmation: PlanConfirmationRequest {
                    fingerprint: preview.plan.fingerprint.clone(),
                    typed_confirmation: None,
                },
            },
        )
        .await
        .expect_err("a blocked merge cannot start");
    assert!(
        matches!(error, AppError::LocationPlanRefused { .. } | AppError::Validation(_)),
        "unexpected error: {error:?}"
    );

    // Nothing moved, nothing merged, and the engine was never asked to execute.
    assert_eq!(
        fixture.title(&title.id).await.library_id,
        fixture.source_library_id
    );
    assert!(fixture.merges.executed().is_empty());
}

/// FR-033 for a merge: `execute_title_merge` is one transaction, so a crash
/// leaves either no merge or a complete one. This is the second case — the
/// merge committed and the checkpoint that should have settled it did not.
///
/// The resumed run must recognize the merge as already applied and settle,
/// rather than re-planning against a source title that no longer exists. Two
/// things make that work: the admission check reads a missing source title with
/// a present destination as this operation's own footprint (FR-089), and the
/// reconciler asks the same question before touching the engine.
#[tokio::test]
async fn a_merge_that_committed_before_the_crash_settles_idempotently_on_resume() {
    let fixture = TransferFixture::new().await;
    let destination = fixture
        .seed_destination_title_with_files(
            "Interrupted Merge",
            2020,
            "Interrupted Merge (2020)",
            vec![external_id("tmdb", "9001")],
            &[("Interrupted.Merge.2020.Extra.mkv", b"destination extra")],
        )
        .await;
    let title = fixture
        .seed_source_title_with_files(
            "Interrupted Merge",
            2020,
            "Interrupted Merge 2020",
            vec![external_id("tmdb", "9001")],
            &[("Interrupted.Merge.2020.mkv", b"the incoming feature")],
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;
    let operation_id = "operation-merge-resume";
    let operation = queued_operation(
        operation_id,
        LocationOperationType::CrossLibraryTransfer,
        LocationExecutionMode::MoveWithScryer,
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

    // The store dies the moment the merged title's checkpoint would advance
    // past `reconciling` — after Groups 1–5 committed.
    fixture
        .operations
        .fail_checkpoint_writes_from(TitleCheckpointState::CleaningUp);
    let crashed = fixture
        .app
        .run_root_move(operation_id, &preview.execution)
        .await;
    assert!(crashed.is_err(), "the injected store failure aborts the run");

    // The merge is committed and the checkpoint is not settled: exactly the
    // window this test exists for.
    assert!(
        fixture
            .app
            .services
            .catalog
            .titles
            .get_by_id(&title.id)
            .await
            .expect("read the source title")
            .is_none(),
        "the transaction committed before the crash"
    );
    assert_eq!(fixture.merges.executed().len(), 1);
    let checkpoint = fixture
        .operations
        .checkpoint(operation_id, &title.id)
        .expect("the interrupted title has a checkpoint");
    assert!(!checkpoint.state.is_settled());
    // The file was proven before the crash. Held so the resumed run can be
    // asserted against the exact records, not just their count: re-verifying
    // would rewrite them in place (FR-092).
    let proven = fixture.operations.verifications();
    assert_eq!(proven.len(), 1);

    // ── Resume ──────────────────────────────────────────────────────────────
    fixture.operations.clear_checkpoint_faults();
    let resumed_plan = fixture
        .app
        .resume_location_operation(operation_id)
        .await
        .expect("resume")
        .plan()
        .expect("an interrupted merge resumes through the same runner");
    let outcome = fixture
        .app
        .run_root_move(operation_id, &resumed_plan)
        .await
        .expect("resumed run");

    assert!(
        matches!(
            outcome.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "unexpected terminal state: {:?} ({:?})",
        outcome.state,
        outcome.detail
    );
    assert_eq!(
        fixture.merges.executed().len(),
        1,
        "the merge is never executed a second time"
    );
    assert_eq!(
        fixture.operations.verifications(),
        proven,
        "the merged title's file is never copied or verified a second time (FR-092)"
    );
    assert_eq!(outcome.counters.merges, 1);
    let settled = fixture
        .operations
        .checkpoint(operation_id, &title.id)
        .expect("checkpoint");
    assert!(settled.state.is_settled());
    assert_eq!(
        settled.placement.merged_into_title_id.as_deref(),
        Some(destination.id.as_str())
    );
    assert_eq!(
        fixture.operations.open_claim_count(),
        0,
        "a finished operation owns nothing (FR-084)"
    );
}

/// FR-084 for the row a merge rewrites but no instruction set moves: the
/// **destination** title.
///
/// A merging transfer plans against a snapshot of the destination title — its
/// media rows, its episodes, its folder — and Groups 1–5 rewrite that row. Left
/// unclaimed it could be deleted, renamed, or moved out from under the merge
/// between confirmation and commit. The operation's own claim on it is not a
/// conflict: the store's Group 3 remap collapses the source claim onto the
/// destination one when the same operation holds both, the way OQ7 excludes the
/// current operation from the resumable-operation check — so the operation
/// still completes, which is the second half of this test.
#[tokio::test]
async fn a_merging_transfer_owns_the_destination_title_and_still_completes() {
    let fixture = TransferFixture::new().await;
    let destination = fixture
        .seed_destination_title_with_files(
            "Held Merge Target",
            2019,
            "Held Merge Target (2019)",
            vec![external_id("tmdb", "5150")],
            &[("Held.Merge.Target.2019.Extra.mkv", b"destination extra")],
        )
        .await;
    let bystander = fixture
        .seed_destination_title("Untouched Neighbour", 2003, vec![external_id("tmdb", "606")])
        .await;
    let title = fixture
        .seed_source_title_with_files(
            "Held Merge Target",
            2019,
            "Held Merge Target 2019",
            vec![external_id("tmdb", "5150")],
            &[("Held.Merge.Target.2019.mkv", b"the incoming feature")],
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;
    let operation_id = "operation-merge-owns-its-target";
    let operation = LocationOperation {
        source_root_id: Some(fixture.source_root_id.clone()),
        destination_root_id: Some(fixture.destination_root_id.clone()),
        ..queued_operation(
            operation_id,
            LocationOperationType::CrossLibraryTransfer,
            LocationExecutionMode::MoveWithScryer,
            preview.plan.verification.depth,
        )
    };

    let entities =
        crate::location::executor::owned_entities(&operation, &preview.execution.to_work_plan());
    assert!(
        entities.contains(&crate::location::ownership_guard::OwnedEntity::Title(
            destination.id.clone()
        )),
        "the merge target is not owned for the operation's duration (FR-084)"
    );
    assert!(
        !entities.contains(&crate::location::ownership_guard::OwnedEntity::Title(
            bystander.id.clone()
        )),
        "a destination title nobody merges into must stay open: there is no global lock"
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

    // The run claims its entities and dies at the first title boundary, before
    // the merge transaction: the window a second actor would find.
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

    // Deleting the merge target is refused while the merge is in flight.
    let deleted = fixture
        .app
        .delete_title(&fixture.user, &destination.id, false, None)
        .await;
    let Err(crate::AppError::Validation(message)) = deleted else {
        panic!("deleting a merge target under a running operation must be refused: {deleted:?}");
    };
    assert!(
        message.contains(operation_id) && message.contains(&destination.id),
        "the refusal names the operation and the title it holds: {message}"
    );

    // So is moving it: another location operation cannot take the row.
    let outcome = fixture
        .app
        .services
        .library
        .location_operations
        .claim_location_operation_ownership(
            "operation-second-actor",
            &[crate::location::ownership_guard::OwnedEntity::Title(
                destination.id.clone(),
            )],
        )
        .await
        .expect("ask for the merge target");
    let crate::ports::LocationOwnershipOutcome::Conflict(conflicts) = outcome else {
        panic!("the merge target must be refused to a second operation (FR-084)");
    };
    assert_eq!(conflicts[0].operation_id, operation_id);

    // The neighbouring destination title is untouched by any of it.
    fixture
        .app
        .delete_title(&fixture.user, &bystander.id, false, None)
        .await
        .expect("an unrelated destination title is not blocked");

    // ── And the operation still finishes, holding both sides ────────────────
    let resumed_plan = fixture
        .app
        .resume_location_operation(operation_id)
        .await
        .expect("resume")
        .plan()
        .expect("an interrupted merge resumes through the same runner");
    let resumed = fixture
        .app
        .run_root_move(operation_id, &resumed_plan)
        .await
        .expect("resumed run");
    assert!(
        matches!(
            resumed.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "unexpected terminal state: {:?} ({:?})",
        resumed.state,
        resumed.detail
    );
    assert_eq!(resumed.counters.merges, 1);
    assert_eq!(fixture.merges.executed().len(), 1);
    assert_eq!(
        fixture.operations.open_claim_count(),
        0,
        "a finished operation owns nothing, on either side of the merge (FR-084)"
    );
    // And the guard lets go with it.
    fixture
        .app
        .delete_title(&fixture.user, &destination.id, false, None)
        .await
        .expect("the merge target is free once the operation settles");
}

/// FR-055 / FR-016: several destination titles claim identities this title
/// holds, so the user decides which one it is before anything starts.
#[tokio::test]
async fn an_ambiguous_destination_identity_blocks_the_title_for_resolution() {
    let fixture = TransferFixture::new().await;
    let first = fixture
        .seed_destination_title("Split A", 2001, vec![external_id("tmdb", "111")])
        .await;
    let second = fixture
        .seed_destination_title("Split B", 2002, vec![external_id("imdb", "tt222")])
        .await;
    let title = fixture
        .seed_source_title(
            "Split Source",
            2003,
            "Split Source (2003)",
            "Split.Source.2003.mkv",
            1024,
            Vec::new(),
            vec![external_id("tmdb", "111"), external_id("imdb", "tt222")],
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(preview.classification.counts.needs_resolution, 1);
    assert!(preview.classification.blocks_start());

    let classified = classified_titles(&preview);
    assert_eq!(
        classified[0].reason_code.as_deref(),
        Some(reason_codes::AMBIGUOUS_DESTINATION_IDENTITY)
    );
    assert_eq!(
        classified[0].merge_target_title_id(),
        None,
        "an ambiguous outcome never carries a merge target"
    );
    let candidates = classified[0]
        .destination_identity
        .as_ref()
        .expect("detection ran")
        .ambiguous_title_ids();
    assert_eq!(candidates.len(), 2);
    assert!(candidates.contains(&first.id));
    assert!(candidates.contains(&second.id));

    assert_eq!(
        fixture.title(&title.id).await.library_id,
        fixture.source_library_id
    );
}

// ── US6.3 / FR-033 ───────────────────────────────────────────────────────────

/// US6.3 plus FR-033: a two-title transfer interrupted after the first title
/// settled resumes from its persisted plan, admits the title it already flipped
/// as its own footprint rather than as a stale input, and converges with both
/// titles in the destination library.
#[tokio::test]
async fn an_interrupted_transfer_resumes_and_converges_on_the_destination_library() {
    let fixture = TransferFixture::new().await;
    let first = fixture
        .seed_source_title(
            "Settled Transfer",
            2011,
            "Settled Transfer (2011)",
            "Settled.Transfer.2011.mkv",
            1500,
            Vec::new(),
            Vec::new(),
        )
        .await;
    let second = fixture
        .seed_source_title(
            "Interrupted Transfer",
            2012,
            "Interrupted Transfer (2012)",
            "Interrupted.Transfer.2012.mkv",
            2500,
            Vec::new(),
            Vec::new(),
        )
        .await;

    let preview = fixture.preview(&[&first.id, &second.id]).await;
    assert_eq!(preview.execution.titles.len(), 2);
    assert_eq!(
        preview.plan.header.operation_type,
        LocationOperationType::CrossLibraryTransfer
    );

    let operation_id = "operation-transfer-resume";
    let operation = LocationOperation {
        counters: crate::location::model::LocationOperationCounters {
            titles_total: 2,
            files_total: 2,
            bytes_total: 4000,
            ..Default::default()
        },
        ..queued_operation(
            operation_id,
            LocationOperationType::CrossLibraryTransfer,
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

    fixture
        .operations
        .crash_on_cancel_check(title_boundary_cancel_check(2, 1));
    let crashed = fixture
        .app
        .run_root_move(operation_id, &preview.execution)
        .await;
    assert!(crashed.is_err(), "the injected store failure aborts the run");

    // The first title's flip already landed; the second has not started.
    assert_eq!(
        fixture.title(&first.id).await.library_id,
        fixture.destination_library_id
    );
    assert_eq!(
        fixture.title(&second.id).await.library_id,
        fixture.source_library_id
    );

    let resumed_plan = fixture
        .app
        .resume_location_operation(operation_id)
        .await
        .expect("resume")
        .plan()
        .expect("an interrupted transfer resumes through the same runner");
    assert_eq!(resumed_plan, preview.execution);

    let outcome = fixture
        .app
        .run_root_move(operation_id, &resumed_plan)
        .await
        .expect("resumed run");
    assert_eq!(outcome.state, LocationOperationState::Completed);
    assert_eq!(outcome.counters.titles_processed, 2);
    assert_eq!(
        fixture.operations.verifications().len(),
        2,
        "one record per file, not one per attempt"
    );

    for title_id in [&first.id, &second.id] {
        let transferred = fixture.title(title_id).await;
        assert_eq!(transferred.library_id, fixture.destination_library_id);
        assert_eq!(transferred.root_folder_id, fixture.destination_root_id);
    }
    assert!(
        fixture
            .destination_root()
            .join("Interrupted Transfer (2012)")
            .join("Interrupted.Transfer.2012.mkv")
            .exists()
    );
    assert_eq!(
        fixture.operations.open_claim_count(),
        0,
        "a finished operation owns nothing (FR-084)"
    );

    // The checkpoints record where each title landed, destination library
    // included, so Activity and a later merge read the same placement.
    let checkpoint = fixture
        .operations
        .checkpoint(operation_id, &second.id)
        .expect("the resumed title has a checkpoint");
    assert_eq!(
        checkpoint.placement.destination_library_id.as_deref(),
        Some(fixture.destination_library_id.as_str())
    );
    assert_eq!(
        checkpoint.placement.source_library_id.as_deref(),
        Some(fixture.source_library_id.as_str())
    );
    assert_eq!(
        checkpoint.placement.merged_into_title_id, None,
        "a transfer without a destination match merges into nothing"
    );
}

/// FR-089/FR-033, the narrower transfer window: the catalog flip committed and
/// the *cleanup* never ran, so the source folder is still on disk with the
/// title already pointing at the destination library.
///
/// Admission has to read that flipped library as this operation's own footprint
/// rather than as a foreign change — otherwise the resume would stop the title
/// as stale and leave the source folder behind forever — and the resumed run
/// has to finish the cleanup without re-copying or re-verifying anything.
#[tokio::test]
async fn a_transfer_whose_flip_committed_before_the_crash_cleans_up_on_resume() {
    let fixture = TransferFixture::new().await;
    let title = fixture
        .seed_source_title(
            "Half Finished Transfer",
            2013,
            "Half Finished Transfer (2013)",
            "Half.Finished.Transfer.2013.mkv",
            1200,
            Vec::new(),
            Vec::new(),
        )
        .await;
    let source_folder = fixture.source_root().join("Half Finished Transfer (2013)");

    let preview = fixture.preview(&[&title.id]).await;
    let operation_id = "operation-transfer-cleanup-resume";
    let operation = queued_operation(
        operation_id,
        LocationOperationType::CrossLibraryTransfer,
        LocationExecutionMode::MoveWithScryer,
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

    // The store dies the moment the checkpoint would advance past
    // `reconciling` — after the catalog flip, before the cleanup.
    fixture
        .operations
        .fail_checkpoint_writes_from(TitleCheckpointState::CleaningUp);
    let crashed = fixture
        .app
        .run_root_move(operation_id, &preview.execution)
        .await;
    assert!(
        crashed.is_err(),
        "the injected store failure aborts the run"
    );

    let flipped = fixture.title(&title.id).await;
    assert_eq!(flipped.library_id, fixture.destination_library_id);
    assert_eq!(flipped.root_folder_id, fixture.destination_root_id);
    assert!(
        source_folder.exists(),
        "the cleanup never ran, so the source folder is still there"
    );
    let proven = fixture.operations.verifications();
    assert_eq!(proven.len(), 1, "the file was proven before the crash");
    let checkpoint = fixture
        .operations
        .checkpoint(operation_id, &title.id)
        .expect("the interrupted title has a checkpoint");
    assert!(!checkpoint.state.is_settled());

    // ── Resume ──────────────────────────────────────────────────────────────
    fixture.operations.clear_checkpoint_faults();
    let resumed_plan = fixture
        .app
        .resume_location_operation(operation_id)
        .await
        .expect("resume")
        .plan()
        .expect("a transfer whose flip committed is still resumable");
    let outcome = fixture
        .app
        .run_root_move(operation_id, &resumed_plan)
        .await
        .expect("resumed run");

    assert!(
        matches!(
            outcome.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "unexpected terminal state: {:?} ({:?})",
        outcome.state,
        outcome.detail
    );
    assert_eq!(
        fixture.operations.verifications(),
        proven,
        "the file proven before the crash is never verified a second time (FR-092)"
    );
    assert!(
        !source_folder.exists(),
        "the resumed run finished the cleanup the crash interrupted"
    );
    assert!(
        fixture
            .destination_root()
            .join("Half Finished Transfer (2013)")
            .join("Half.Finished.Transfer.2013.mkv")
            .exists()
    );
    let settled = fixture
        .operations
        .checkpoint(operation_id, &title.id)
        .expect("checkpoint");
    assert!(settled.state.is_settled());
    assert_eq!(
        fixture.operations.open_claim_count(),
        0,
        "a finished operation owns nothing (FR-084)"
    );
}

/// FR-084 is why FR-089's staleness rule never has to answer "what if the
/// source library was deleted, or the destination root was taken out of its
/// library, while the operation was interrupted?": it cannot happen. An
/// interrupted operation still holds its claims across a restart, and both
/// entry points that would move that ground refuse while it does.
#[tokio::test]
async fn an_interrupted_transfer_refuses_library_and_root_reconfiguration_underneath_it() {
    let fixture = TransferFixture::new().await;
    let title = fixture
        .seed_source_title(
            "Ground Under Foot",
            2014,
            "Ground Under Foot (2014)",
            "Ground.Under.Foot.2014.mkv",
            900,
            Vec::new(),
            Vec::new(),
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;
    let operation_id = "operation-transfer-guarded-ground";
    // The row `start_root_move` writes: the roots on both sides are named, and
    // they are what the runner claims (FR-084).
    let operation = LocationOperation {
        source_library_id: Some(fixture.source_library_id.clone()),
        destination_library_id: Some(fixture.destination_library_id.clone()),
        source_root_id: Some(fixture.source_root_id.clone()),
        destination_root_id: Some(fixture.destination_root_id.clone()),
        ..queued_operation(
            operation_id,
            LocationOperationType::CrossLibraryTransfer,
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

    // The run claims its titles and roots and then dies at the very first title
    // boundary, before touching anything: the interrupted row a restart finds,
    // with its claims still open.
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
    assert!(fixture.operations.open_claim_count() > 0);

    // Deleting the destination library would retire the root this operation is
    // moving into.
    let deleted = fixture
        .app
        .delete_library(&fixture.user, &fixture.destination_library_id)
        .await;
    let Err(crate::AppError::Validation(message)) = deleted else {
        panic!("deleting a library under an interrupted operation must be refused: {deleted:?}");
    };
    assert!(
        message.contains(operation_id),
        "the refusal names the operation holding the ground: {message}"
    );

    // Retiring the source library's roots would do the same on the other side.
    let repointed = fixture
        .app
        .update_library(
            &fixture.user,
            &fixture.source_library_id,
            None,
            Some(Vec::new()),
            None,
        )
        .await;
    let Err(crate::AppError::Validation(message)) = repointed else {
        panic!("retiring a root under an interrupted operation must be refused: {repointed:?}");
    };
    assert!(
        message.contains(operation_id),
        "the refusal names the operation holding the ground: {message}"
    );

    // A name-only edit is harmless and still goes through, so the guard is not
    // simply freezing the library.
    fixture
        .app
        .update_library(
            &fixture.user,
            &fixture.source_library_id,
            Some("Renamed While Interrupted".to_string()),
            None,
            None,
        )
        .await
        .expect("a name-only edit touches no root");
}

// ── US6.2 / FR-057 / FR-058 / FR-060 / FR-062 ────────────────────────────────

/// Seed a season with two episodes on `title`, so a transfer has collection
/// structure to preserve and a season folder to reason about.
async fn seed_season(fixture: &TransferFixture, title: &Title, season: i32) -> Collection {
    let collection = fixture
        .app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: scryer_domain::Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: season.to_string(),
            label: Some(format!("Season {season}")),
            ordered_path: None,
            narrative_order: Some(season.to_string()),
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("seed season collection");

    for episode_number in 1..=2 {
        fixture
            .app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: scryer_domain::Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(collection.id.clone()),
                episode_type: EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some(season.to_string()),
                episode_label: None,
                title: Some(format!("S{season}E{episode_number}")),
                air_date: None,
                duration_seconds: None,
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("seed episode");
    }

    collection
}

/// Every catalog-change detail in a preview carrying `reason_code`.
fn details_for(
    preview: &crate::location::operations::RootMovePreview,
    reason_code: &str,
) -> Vec<String> {
    items_of(preview, PlanItemKind::CatalogChange)
        .into_iter()
        .filter(|item| item.reason_code.as_deref() == Some(reason_code))
        .filter_map(|item| item.detail.clone())
        .collect()
}

/// US6.2 end to end: a series moving into an anime library converts its facet,
/// the preview names every setting the conversion invalidates or resets and says
/// files keep their names, the destination naming policy calculates the folder
/// under the *post-conversion* facet, and the anime-derived tags are gone while
/// the user's own settings survive (FR-057, FR-058).
#[tokio::test]
async fn a_series_into_an_anime_library_converts_its_facet_and_enumerates_the_consequences() {
    let fixture = TransferFixture::with_facets(MediaFacet::Series, MediaFacet::Anime).await;
    // The two facets get different templates, so the folder the title lands in
    // proves which facet's policy was asked (FR-058).
    fixture
        .set_folder_template(&MediaFacet::Series, "{title} ({year})")
        .await;
    fixture
        .set_folder_template(&MediaFacet::Anime, "[anime] {title} ({year})")
        .await;

    let title = fixture
        .seed_source_title(
            "Converted Show",
            2015,
            "Converted Show (2015)",
            "Converted.Show.S01E01.mkv",
            2048,
            vec![
                // A user-set anime policy that was inert on a series title.
                "scryer:filler-policy:skip_filler".to_string(),
                // A quality profile, which no facet gates.
                "scryer:quality-profile:1080p".to_string(),
                // Metadata-derived under a facet the title is leaving behind.
                "scryer:mal-score:8.4".to_string(),
                "favourites".to_string(),
            ],
            Vec::new(),
        )
        .await;
    assert_eq!(title.facet, MediaFacet::Series);
    let season = seed_season(&fixture, &title, 1).await;

    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(preview.classification.counts.cross_library_transfer, 1);
    assert!(!preview.classification.blocks_start());

    // The conversion is stated, folder-only scope included.
    let headline = details_for(&preview, plan_reasons::FACET_CONVERSION);
    assert_eq!(headline.len(), 1, "stated once: {headline:?}");
    assert!(headline[0].contains("series"), "{}", headline[0]);
    assert!(headline[0].contains("anime"), "{}", headline[0]);
    assert!(
        headline[0].contains("files keep their names"),
        "FR-058's statement is required: {}",
        headline[0]
    );

    // Every affected setting is its own named line.
    let meaning_changes = details_for(&preview, plan_reasons::FACET_SETTING_MEANING_CHANGE);
    assert!(
        meaning_changes
            .iter()
            .any(|detail| detail.contains("filler handling")),
        "the inert filler policy starts taking effect: {meaning_changes:?}"
    );
    assert!(
        meaning_changes
            .iter()
            .any(|detail| detail.contains("season-folder layout")),
        "the season-folder default is resolved per facet: {meaning_changes:?}"
    );
    let resets = details_for(&preview, plan_reasons::FACET_SETTING_RESET);
    assert!(
        resets
            .iter()
            .any(|detail| detail.contains("MyAnimeList score")),
        "the derived score resets: {resets:?}"
    );

    // FR-062: the title has seasons, and the conversion changes how they are
    // treated, so the preview says what happens to them.
    let collections = details_for(&preview, plan_reasons::COLLECTION_PRESERVATION);
    assert_eq!(collections.len(), 1, "{collections:?}");
    assert!(collections[0].contains("1 of its seasons"));
    assert!(collections[0].contains("2 of its episodes"));

    // Typed, for the client (FR-057 on the GraphQL surface).
    let classified = classified_titles(&preview);
    let conversion = classified[0]
        .facet_conversion
        .as_ref()
        .expect("the classification carries the conversion");
    assert_eq!(conversion.from, MediaFacet::Series);
    assert_eq!(conversion.to, MediaFacet::Anime);
    assert!(
        conversion
            .settings
            .iter()
            .any(|setting| setting.setting == "filler_policy")
    );

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert_eq!(operation.state, LocationOperationState::Completed);

    let transferred = fixture.title(&title.id).await;
    assert_eq!(transferred.id, title.id, "the title keeps its identity");
    assert_eq!(transferred.library_id, fixture.destination_library_id);
    assert_eq!(
        transferred.facet,
        MediaFacet::Anime,
        "FR-057: the facet converted with the library, not after it"
    );
    assert!(
        transferred
            .tags
            .contains(&"scryer:quality-profile:1080p".to_string()),
        "settings no facet gates are untouched: {:?}",
        transferred.tags
    );
    assert!(
        transferred
            .tags
            .contains(&"scryer:filler-policy:skip_filler".to_string()),
        "a setting that merely becomes effective is not rewritten: {:?}",
        transferred.tags
    );
    assert!(
        transferred.tags.contains(&"favourites".to_string()),
        "user tags survive: {:?}",
        transferred.tags
    );
    assert!(
        !transferred
            .tags
            .iter()
            .any(|tag| tag.starts_with("scryer:mal-score:")),
        "metadata-derived anime values do not survive the conversion: {:?}",
        transferred.tags
    );

    // FR-058: the folder came from the anime policy — the post-conversion facet
    // — and the file inside it kept its name.
    let destination_folder = fixture
        .destination_root()
        .join("[anime] Converted Show (2015)");
    assert_eq!(
        transferred.folder_path.as_deref(),
        Some(destination_folder.to_string_lossy().as_ref()),
        "the destination folder is calculated from the facet the title converts to"
    );
    assert!(
        destination_folder
            .join("Converted.Show.S01E01.mkv")
            .exists(),
        "files keep their names"
    );

    // FR-062: collection structure rode the title row untouched.
    let episodes = fixture
        .app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
        .expect("list episodes");
    assert_eq!(episodes.len(), 2);
    assert!(
        episodes
            .iter()
            .all(|episode| episode.collection_id.as_deref() == Some(season.id.as_str())),
        "every episode still belongs to the season it belonged to"
    );
    let seasons = fixture
        .app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    assert_eq!(seasons.len(), 1);
    assert_eq!(seasons[0].id, season.id, "the season row is the same row");
}

/// The reverse crossing: anime → series. The same four anime-gated settings that
/// start applying in one direction stop applying in the other, and the preview
/// says "invalid", not "resets" — the values stay on the title so a transfer
/// back restores the behaviour (FR-057).
#[tokio::test]
async fn an_anime_into_a_series_library_invalidates_the_settings_only_anime_reads() {
    let fixture = TransferFixture::with_facets(MediaFacet::Anime, MediaFacet::Series).await;
    let title = fixture
        .seed_source_title(
            "Reverse Show",
            2018,
            "Reverse Show (2018)",
            "Reverse.Show.S01E01.mkv",
            1024,
            vec![
                "scryer:filler-policy:skip_filler".to_string(),
                "scryer:recap-policy:skip_recap".to_string(),
                "scryer:monitor-specials:true".to_string(),
                "scryer:inter-season-movies:true".to_string(),
                "scryer:anime-status:finished".to_string(),
            ],
            Vec::new(),
        )
        .await;
    assert_eq!(title.facet, MediaFacet::Anime);

    let preview = fixture.preview(&[&title.id]).await;
    let invalid = details_for(&preview, plan_reasons::FACET_SETTING_INVALID);
    for label in [
        "filler handling",
        "recap handling",
        "specials monitoring",
        "inter-season movie inclusion",
    ] {
        assert!(
            invalid.iter().any(|detail| detail.contains(label)),
            "{label} is named individually: {invalid:?}"
        );
    }

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert_eq!(operation.state, LocationOperationState::Completed);

    let transferred = fixture.title(&title.id).await;
    assert_eq!(transferred.facet, MediaFacet::Series);
    assert_eq!(transferred.library_id, fixture.destination_library_id);
    assert!(
        transferred
            .tags
            .contains(&"scryer:filler-policy:skip_filler".to_string()),
        "an invalidated setting is kept, not deleted: {:?}",
        transferred.tags
    );
    assert!(
        !transferred
            .tags
            .iter()
            .any(|tag| tag.starts_with("scryer:anime-status:")),
        "the derived anime status does not survive: {:?}",
        transferred.tags
    );
}

/// FR-060: a series with series-movie links transfers with an explicit
/// disposition in the preview, and the links themselves come through intact —
/// they are keyed on the title id, which the transfer does not reissue, and the
/// movie entity at the far end belongs to no library at all.
#[tokio::test]
async fn series_movie_links_travel_with_the_series_and_the_preview_says_so() {
    let fixture = TransferFixture::with_facets(MediaFacet::Anime, MediaFacet::Series).await;
    let title = fixture
        .seed_source_title(
            "Linked Show",
            2016,
            "Linked Show (2016)",
            "Linked.Show.S01E01.mkv",
            1024,
            vec!["scryer:monitor-type:allepisodes".to_string()],
            Vec::new(),
        )
        .await;
    let link = fixture
        .app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Linked Show: The Movie",
            Some(2018),
            Some("tt7777777"),
            None,
        ))
        .await
        .expect("seed a series-movie link");

    let preview = fixture.preview(&[&title.id]).await;
    let stated = details_for(&preview, plan_reasons::SERIES_MOVIE_LINKS);
    assert_eq!(stated.len(), 1, "one disposition, not a silent omission");
    assert!(stated[0].contains("1 series-movie link"), "{}", stated[0]);
    assert!(
        stated[0].contains(&fixture.destination_library_id),
        "the disposition names where the link ends up: {}",
        stated[0]
    );
    // FR-060 is also why the monitoring mode is a meaning change here: only an
    // anime title's monitor type decides link monitoring.
    let meaning_changes = details_for(&preview, plan_reasons::FACET_SETTING_MEANING_CHANGE);
    assert!(
        meaning_changes
            .iter()
            .any(|detail| detail.contains("monitoring mode")),
        "{meaning_changes:?}"
    );

    let operation = fixture.start_and_settle(&[&title.id]).await;
    assert_eq!(operation.state, LocationOperationState::Completed);

    let links = fixture
        .app
        .services
        .catalog
        .shows
        .list_series_movie_links_for_title(&title.id)
        .await
        .expect("list links after the transfer");
    assert_eq!(links.len(), 1, "the link is not orphaned");
    assert_eq!(links[0].id, link.id, "it is the same link row");
    assert_eq!(links[0].series_title_id, title.id);
    assert_eq!(
        links[0].movie.id, link.movie.id,
        "the shared movie entity is untouched: no library owns it"
    );
    assert_eq!(links[0].monitored, link.monitored);
}

/// FR-017 is unchanged by FR-060–FR-062: the carve-outs those rules describe are
/// title-level dispositions inside an episodic transfer, never a licence to move
/// a movie into a series library.
#[tokio::test]
async fn a_movie_into_a_series_library_is_still_refused() {
    let fixture = TransferFixture::with_facets(MediaFacet::Movie, MediaFacet::Series).await;
    let title = fixture
        .seed_source_title(
            "Just A Movie",
            2020,
            "Just A Movie (2020)",
            "Just.A.Movie.2020.mkv",
            1024,
            Vec::new(),
            Vec::new(),
        )
        .await;

    let preview = fixture.preview(&[&title.id]).await;
    assert_eq!(preview.classification.counts.incompatible, 1);
    assert_eq!(preview.classification.counts.cross_library_transfer, 0);
    assert!(preview.classification.blocks_start());

    let classified = classified_titles(&preview);
    assert_eq!(classified[0].class, TitleLocationClass::Incompatible);
    assert_eq!(
        classified[0].reason_code.as_deref(),
        Some(reason_codes::INCOMPATIBLE_FACET)
    );
    assert!(
        classified[0].facet_conversion.is_none(),
        "an incompatible pairing never carries a conversion"
    );
    assert!(
        details_for(&preview, plan_reasons::FACET_CONVERSION).is_empty(),
        "and never claims one in the preview"
    );

    assert_eq!(
        fixture.title(&title.id).await.library_id,
        fixture.source_library_id,
        "nothing moved"
    );
}
