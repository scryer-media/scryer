//! Folder-match correction (US1): the acceptance scenarios that live below the
//! API boundary.
//!
//! Every test here works against real directories and real files, because the
//! whole point of the workflow is that catalog ownership moves and the
//! filesystem does not (FR-002, SC-001).

use super::*;
use crate::location::folder_match::{
    FolderMatchOutcome, FolderMatchOwnership, FolderMatchResolution,
};

/// A scanner that answers per directory, so a title-scoped rescan sees the files
/// under the folder the title now owns and nothing else. The shared
/// `MutableLibraryScanner` returns one fixed list for every root, which cannot
/// tell "rebuilt from the new folder" apart from "never rebuilt".
#[derive(Default, Clone)]
struct FolderScopedLibraryScanner {
    files: Arc<Mutex<Vec<LibraryFile>>>,
}

impl FolderScopedLibraryScanner {
    async fn set_files(&self, paths: &[&Path]) {
        *self.files.lock().await = build_test_library_files(paths);
    }

    async fn files_under(&self, root: &str) -> Vec<LibraryFile> {
        let root = Path::new(root).to_path_buf();
        self.files
            .lock()
            .await
            .iter()
            .filter(|file| Path::new(&file.path).starts_with(&root))
            .cloned()
            .collect()
    }
}

#[async_trait]
impl LibraryScanner for FolderScopedLibraryScanner {
    async fn scan_library(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        Ok(self.files_under(root).await)
    }

    async fn scan_library_batched(
        &self,
        root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let files = self.files_under(root).await;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(Ok(files))
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        Ok(rx)
    }

    async fn scan_directory_batched(
        &self,
        root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let files = self.files_under(root).await;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(Ok(files))
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        Ok(rx)
    }
}

struct FolderMatchFixture {
    app: AppUseCase,
    user: User,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
    scanner: Arc<FolderScopedLibraryScanner>,
    root: tempfile::TempDir,
}

impl FolderMatchFixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().expect("library root tempdir");
        let (app, user, unmatched_items) =
            bootstrap_movie_scan_app(root.path(), Vec::new(), Arc::new(EmptySearchMetadataGateway))
                .await;
        let scanner = Arc::new(FolderScopedLibraryScanner::default());
        let app = app.with_test_overrides({
            let scanner = scanner.clone();
            move |services| services.with_library_scanner(scanner)
        });
        Self {
            app,
            user,
            unmatched_items,
            scanner,
            root,
        }
    }

    fn folder(&self, name: &str) -> std::path::PathBuf {
        let folder = self.root.path().join(name);
        std::fs::create_dir_all(&folder).expect("create title folder");
        folder
    }

    fn write_media(&self, folder: &Path, file_name: &str) -> std::path::PathBuf {
        let path = folder.join(file_name);
        std::fs::write(&path, vec![7_u8; 512]).expect("write media file");
        path
    }

    async fn seed_media_row(&self, title_id: &str, path: &Path) {
        self.app
            .services
            .library
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title_id.to_string(),
                file_path: path.to_string_lossy().to_string(),
                size_bytes: 512,
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .expect("seed media file row");
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

    async fn folder_path_of(&self, title_id: &str) -> Option<String> {
        self.app
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .expect("load title")
            .expect("title exists")
            .folder_path
    }
}

/// Directory snapshot used to prove the filesystem never changed (SC-001).
fn snapshot_tree(root: &Path) -> Vec<(String, Vec<u8>, std::time::SystemTime)> {
    fn walk(dir: &Path, out: &mut Vec<(String, Vec<u8>, std::time::SystemTime)>) {
        let mut entries = std::fs::read_dir(dir)
            .expect("read directory")
            .map(|entry| entry.expect("directory entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else {
                let metadata = std::fs::metadata(&path).expect("file metadata");
                out.push((
                    path.to_string_lossy().to_string(),
                    std::fs::read(&path).expect("read file"),
                    metadata.modified().expect("file mtime"),
                ));
            }
        }
    }

    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

/// US1.1 — an unowned folder is claimed, old-folder associations are detached,
/// the new folder is scanned, and no byte on disk changes (FR-003, SC-001).
#[tokio::test]
async fn correcting_a_match_to_an_unowned_folder_reassigns_and_rescans_without_touching_files() {
    let fixture = FolderMatchFixture::new().await;
    let wrong_folder = fixture.folder("Wrong Match (2019)");
    let right_folder = fixture.folder("Right Match (2024)");
    let wrong_file = fixture.write_media(&wrong_folder, "Wrong.Match.2019.1080p.mkv");
    let right_file = fixture.write_media(&right_folder, "Right.Match.2024.1080p.mkv");
    fixture
        .scanner
        .set_files(&[wrong_file.as_path(), right_file.as_path()])
        .await;

    let title = create_movie_title_with_folder(
        &fixture.app,
        &fixture.user,
        "Right Match",
        wrong_folder.as_path(),
    )
    .await;
    fixture.seed_media_row(&title.id, &wrong_file).await;

    let before = snapshot_tree(fixture.root.path());

    let preview = fixture
        .app
        .change_title_folder_preview(
            &fixture.user,
            &title.id,
            right_folder.to_string_lossy().as_ref(),
        )
        .await
        .expect("preview folder change");
    assert_eq!(preview.ownership, FolderMatchOwnership::Unowned);
    assert!(!preview.no_op);
    assert!(!preview.files_will_move);
    assert_eq!(preview.current_folder_tracked_media_count, 1);
    assert_eq!(preview.selected_folder_tracked_media_count, 0);
    assert_eq!(
        preview.available_resolutions,
        vec![FolderMatchResolution::Assign]
    );
    assert_eq!(preview.selected_root_path, fixture.root.path().to_string_lossy());

    let result = fixture
        .app
        .apply_title_folder_change(
            &fixture.user,
            &title.id,
            right_folder.to_string_lossy().as_ref(),
            FolderMatchResolution::Assign,
        )
        .await
        .expect("apply folder change");

    assert_eq!(result.outcome, FolderMatchOutcome::Assigned);
    assert_eq!(result.detached_media_file_count, 1);
    assert_eq!(
        result.previous_folder_path.as_deref(),
        Some(wrong_folder.to_string_lossy().as_ref())
    );
    assert_eq!(
        fixture.folder_path_of(&title.id).await.as_deref(),
        Some(right_folder.to_string_lossy().as_ref())
    );
    // Associations were rebuilt from the new folder: nothing left pointing at
    // the folder the title gave up.
    let paths = fixture.media_paths(&title.id).await;
    assert!(
        paths
            .iter()
            .all(|path| !path.starts_with(&*wrong_folder.to_string_lossy())),
        "old-folder associations should be detached, got {paths:?}"
    );

    // SC-001: byte-for-byte, mtime-for-mtime.
    assert_eq!(snapshot_tree(fixture.root.path()), before);
}

/// US1.2 — identity, monitoring, tags, and the catalog record survive the
/// correction untouched (FR-004).
#[tokio::test]
async fn correcting_a_match_leaves_identity_monitoring_and_tags_untouched() {
    let fixture = FolderMatchFixture::new().await;
    let wrong_folder = fixture.folder("Wrong Series (2019)");
    let right_folder = fixture.folder("Right Series (2024)");
    let right_file = fixture.write_media(&right_folder, "Right.Series.2024.1080p.mkv");
    fixture.scanner.set_files(&[right_file.as_path()]).await;

    let title = create_movie_title_with_folder(
        &fixture.app,
        &fixture.user,
        "Right Series",
        wrong_folder.as_path(),
    )
    .await;

    fixture
        .app
        .apply_title_folder_change(
            &fixture.user,
            &title.id,
            right_folder.to_string_lossy().as_ref(),
            FolderMatchResolution::Assign,
        )
        .await
        .expect("apply folder change");

    let after = fixture
        .app
        .services
        .catalog
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(after.name, title.name);
    assert_eq!(after.monitored, title.monitored);
    assert_eq!(after.tags, title.tags);
    assert_eq!(after.external_ids, title.external_ids);
    assert_eq!(after.library_id, title.library_id);
    assert_eq!(after.root_folder_id, title.root_folder_id);
    assert_eq!(after.year, title.year);
}

/// US1.3 — selecting the folder the title already owns explains itself and
/// submits nothing (FR-005).
#[tokio::test]
async fn selecting_the_currently_owned_folder_is_an_explicit_no_op() {
    let fixture = FolderMatchFixture::new().await;
    let folder = fixture.folder("Already Mine (2024)");

    let title =
        create_movie_title_with_folder(&fixture.app, &fixture.user, "Already Mine", folder.as_path())
            .await;

    let preview = fixture
        .app
        .change_title_folder_preview(&fixture.user, &title.id, folder.to_string_lossy().as_ref())
        .await
        .expect("preview folder change");
    assert_eq!(preview.ownership, FolderMatchOwnership::OwnedByThisTitle);
    assert!(preview.no_op);
    assert!(preview.available_resolutions.is_empty());

    let result = fixture
        .app
        .apply_title_folder_change(
            &fixture.user,
            &title.id,
            folder.to_string_lossy().as_ref(),
            FolderMatchResolution::Assign,
        )
        .await
        .expect("apply folder change");
    assert_eq!(result.outcome, FolderMatchOutcome::AlreadyOwned);
    assert!(result.scan.is_none());
    assert_eq!(result.detached_media_file_count, 0);
}

/// US1.4 — two titles trade folders and both are rescanned (FR-006).
#[tokio::test]
async fn swapping_folders_gives_each_title_the_other_folder() {
    let fixture = FolderMatchFixture::new().await;
    let first_folder = fixture.folder("First Title (2020)");
    let second_folder = fixture.folder("Second Title (2021)");
    let first_file = fixture.write_media(&first_folder, "First.Title.2020.1080p.mkv");
    let second_file = fixture.write_media(&second_folder, "Second.Title.2021.1080p.mkv");
    fixture
        .scanner
        .set_files(&[first_file.as_path(), second_file.as_path()])
        .await;

    let first = create_movie_title_with_folder(
        &fixture.app,
        &fixture.user,
        "First Title",
        first_folder.as_path(),
    )
    .await;
    let second = create_movie_title_with_folder(
        &fixture.app,
        &fixture.user,
        "Second Title",
        second_folder.as_path(),
    )
    .await;
    fixture.seed_media_row(&first.id, &first_file).await;
    fixture.seed_media_row(&second.id, &second_file).await;

    let before = snapshot_tree(fixture.root.path());

    let preview = fixture
        .app
        .change_title_folder_preview(
            &fixture.user,
            &first.id,
            second_folder.to_string_lossy().as_ref(),
        )
        .await
        .expect("preview folder change");
    assert_eq!(preview.ownership, FolderMatchOwnership::OwnedByAnotherTitle);
    assert_eq!(
        preview
            .current_owner
            .as_ref()
            .map(|owner| owner.title_id.as_str()),
        Some(second.id.as_str())
    );
    assert_eq!(
        preview.available_resolutions,
        vec![FolderMatchResolution::Swap, FolderMatchResolution::TakeOver]
    );

    let result = fixture
        .app
        .apply_title_folder_change(
            &fixture.user,
            &first.id,
            second_folder.to_string_lossy().as_ref(),
            FolderMatchResolution::Swap,
        )
        .await
        .expect("apply folder swap");

    assert_eq!(result.outcome, FolderMatchOutcome::Swapped);
    assert_eq!(
        fixture.folder_path_of(&first.id).await.as_deref(),
        Some(second_folder.to_string_lossy().as_ref())
    );
    assert_eq!(
        fixture.folder_path_of(&second.id).await.as_deref(),
        Some(first_folder.to_string_lossy().as_ref())
    );
    assert!(result.swapped_title_scan.is_some());
    assert_eq!(snapshot_tree(fixture.root.path()), before);
}

/// FR-006 — the default resolution never takes an owned folder; it names the
/// owner instead.
#[tokio::test]
async fn assigning_an_owned_folder_is_refused_and_names_the_owner() {
    let fixture = FolderMatchFixture::new().await;
    let first_folder = fixture.folder("Requester (2020)");
    let owned_folder = fixture.folder("Owner (2021)");

    let first = create_movie_title_with_folder(
        &fixture.app,
        &fixture.user,
        "Requester",
        first_folder.as_path(),
    )
    .await;
    let owner =
        create_movie_title_with_folder(&fixture.app, &fixture.user, "Owner", owned_folder.as_path())
            .await;

    let error = fixture
        .app
        .apply_title_folder_change(
            &fixture.user,
            &first.id,
            owned_folder.to_string_lossy().as_ref(),
            FolderMatchResolution::Assign,
        )
        .await
        .expect_err("assigning an owned folder should be refused");
    assert!(
        matches!(&error, AppError::Validation(message) if message.contains(&owner.name)),
        "expected a validation error naming the owner, got {error:?}"
    );
    // Nothing moved: both titles still own what they owned.
    assert_eq!(
        fixture.folder_path_of(&first.id).await.as_deref(),
        Some(first_folder.to_string_lossy().as_ref())
    );
    assert_eq!(
        fixture.folder_path_of(&owner.id).await.as_deref(),
        Some(owned_folder.to_string_lossy().as_ref())
    );
}

/// US1.5 — takeover leaves the former owner unmatched, discoverable in repair
/// with the documented reason (FR-007, SC-008).
#[tokio::test]
async fn taking_over_a_folder_surfaces_the_displaced_title_for_repair() {
    let fixture = FolderMatchFixture::new().await;
    let taker_folder = fixture.folder("Taker (2020)");
    let owned_folder = fixture.folder("Displaced (2021)");
    let owned_file = fixture.write_media(&owned_folder, "Displaced.2021.1080p.mkv");
    fixture.scanner.set_files(&[owned_file.as_path()]).await;

    let taker =
        create_movie_title_with_folder(&fixture.app, &fixture.user, "Taker", taker_folder.as_path())
            .await;
    let displaced = create_movie_title_with_folder(
        &fixture.app,
        &fixture.user,
        "Displaced",
        owned_folder.as_path(),
    )
    .await;
    fixture.seed_media_row(&displaced.id, &owned_file).await;

    let before = snapshot_tree(fixture.root.path());

    let result = fixture
        .app
        .apply_title_folder_change(
            &fixture.user,
            &taker.id,
            owned_folder.to_string_lossy().as_ref(),
            FolderMatchResolution::TakeOver,
        )
        .await
        .expect("apply folder takeover");

    assert_eq!(result.outcome, FolderMatchOutcome::TakenOver);
    assert_eq!(
        fixture.folder_path_of(&taker.id).await.as_deref(),
        Some(owned_folder.to_string_lossy().as_ref())
    );
    // The displaced title owns nothing and keeps no association to the folder it
    // lost.
    assert!(
        fixture
            .folder_path_of(&displaced.id)
            .await
            .is_none_or(|folder| folder.is_empty())
    );
    assert!(fixture.media_paths(&displaced.id).await.is_empty());

    let repair = result.displaced_title.expect("displaced title reported");
    assert_eq!(repair.title_id, displaced.id);
    assert_eq!(
        repair.repair_reason_code,
        crate::library_scan_unmatched::LIBRARY_SCAN_FOLDER_OWNERSHIP_CHANGED_BY_USER
    );

    let unmatched = fixture.unmatched_items.items().await;
    let item = unmatched
        .iter()
        .find(|item| item.title_id.as_deref() == Some(displaced.id.as_str()))
        .expect("displaced title surfaces in unmatched discovery");
    assert_eq!(
        item.reason_code,
        crate::library_scan_unmatched::LIBRARY_SCAN_FOLDER_OWNERSHIP_CHANGED_BY_USER
    );
    assert_eq!(item.item_path, owned_folder.to_string_lossy());
    assert_eq!(snapshot_tree(fixture.root.path()), before);
}

/// FR-001 — candidates outside the title's library roots are refused outright,
/// preview and apply alike.
#[tokio::test]
async fn folders_outside_the_titles_library_roots_are_rejected() {
    let fixture = FolderMatchFixture::new().await;
    let folder = fixture.folder("Inside Root (2024)");
    let outside = tempfile::tempdir().expect("outside tempdir");

    let title =
        create_movie_title_with_folder(&fixture.app, &fixture.user, "Inside Root", folder.as_path())
            .await;

    let error = fixture
        .app
        .change_title_folder_preview(
            &fixture.user,
            &title.id,
            outside.path().to_string_lossy().as_ref(),
        )
        .await
        .expect_err("a folder outside the library roots should be refused");
    assert!(
        matches!(&error, AppError::Validation(message) if message.contains("is not inside a root")),
        "expected a root-scope validation error, got {error:?}"
    );

    let error = fixture
        .app
        .apply_title_folder_change(
            &fixture.user,
            &title.id,
            outside.path().to_string_lossy().as_ref(),
            FolderMatchResolution::Assign,
        )
        .await
        .expect_err("a folder outside the library roots should be refused");
    assert!(matches!(error, AppError::Validation(_)));
}

/// FR-083 — the workflow needs management permission on the title's library.
#[tokio::test]
async fn changing_a_folder_match_requires_library_management_permission() {
    let fixture = FolderMatchFixture::new().await;
    let folder = fixture.folder("Guarded (2024)");
    let other_folder = fixture.folder("Guarded Target (2024)");

    let title =
        create_movie_title_with_folder(&fixture.app, &fixture.user, "Guarded", folder.as_path())
            .await;

    let viewer = test_user_with_app_permissions("viewer", AppPermissionMask::NONE);

    let error = fixture
        .app
        .change_title_folder_preview(&viewer, &title.id, other_folder.to_string_lossy().as_ref())
        .await
        .expect_err("preview should require management permission");
    assert!(matches!(error, AppError::Unauthorized(_)));

    let error = fixture
        .app
        .apply_title_folder_change(
            &viewer,
            &title.id,
            other_folder.to_string_lossy().as_ref(),
            FolderMatchResolution::Assign,
        )
        .await
        .expect_err("apply should require management permission");
    assert!(matches!(error, AppError::Unauthorized(_)));
    assert_eq!(
        fixture.folder_path_of(&title.id).await.as_deref(),
        Some(folder.to_string_lossy().as_ref())
    );
}
