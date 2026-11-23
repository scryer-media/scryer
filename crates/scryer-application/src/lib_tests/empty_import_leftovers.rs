//! The empty file a failed copy import left is removed only when it is an
//! empty regular file under a library root, and its catalog row goes with it
//! only when the catalog records an empty file too. Everything else is kept.

use super::*;
use crate::EmptyImportLeftover;

struct LeftoverFixture {
    temp: tempfile::TempDir,
    app: AppUseCase,
    media_files: Arc<MockMediaFileRepo>,
    title: Title,
}

impl LeftoverFixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let root = temp.path().join("library-a");
        std::fs::create_dir_all(&root).expect("create library root");
        let (app, user, _) =
            bootstrap_movie_scan_app(&root, Vec::new(), Arc::new(EmptySearchMetadataGateway)).await;
        let media_files = Arc::new(MockMediaFileRepo::default());
        let app = app.with_test_overrides(|services| {
            services
                .with_settings(Arc::new(StoredSettingsRepo::default()))
                .with_media_files(media_files.clone())
        });
        let folder = root.join("Lanternfall (2001)");
        std::fs::create_dir_all(&folder).expect("create title folder");
        let title = create_movie_title_with_folder(&app, &user, "Lanternfall", &folder).await;
        Self {
            temp,
            app,
            media_files,
            title,
        }
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self
            .temp
            .path()
            .join("library-a")
            .join("Lanternfall (2001)")
            .join(name);
        std::fs::write(&path, bytes).expect("write library file");
        path
    }

    async fn row(&self, id: &str, path: &Path, size_bytes: i64) {
        self.media_files.store.lock().await.push(TitleMediaFile {
            id: id.to_string(),
            title_id: self.title.id.clone(),
            file_path: path.to_string_lossy().to_string(),
            size_bytes,
            role: crate::MediaFileRole::Primary,
            ..Default::default()
        });
    }

    async fn row_ids(&self) -> Vec<String> {
        let rows = self.media_files.store.lock().await;
        rows.iter().map(|row| row.id.clone()).collect()
    }

    async fn sweep(&self, paths: &[PathBuf]) -> Vec<EmptyImportLeftover> {
        let roots = self
            .app
            .empty_import_leftover_roots()
            .await
            .expect("library roots");
        self.app
            .remove_empty_import_leftovers(&roots, paths)
            .await
            .expect("sweep")
    }
}

#[tokio::test]
async fn an_empty_file_under_a_library_root_is_removed_and_everything_else_kept() {
    let fixture = LeftoverFixture::new().await;
    let empty = fixture.file("empty.mkv", b"");
    let full = fixture.file("full.mkv", b"synthetic video bytes");
    let missing = fixture.file("missing.mkv", b"");
    std::fs::remove_file(&missing).expect("remove");
    let folder = fixture.temp.path().join("library-a").join("Empty Folder");
    std::fs::create_dir_all(&folder).expect("folder");
    let outside = fixture.temp.path().join("outside.mkv");
    std::fs::write(&outside, b"").expect("write outside");
    let root = fixture.temp.path().join("library-a");
    let dotted = root.join("Lanternfall (2001)").join("..").join("up.mkv");
    std::fs::write(root.join("up.mkv"), b"").expect("write dotted target");

    let outcomes = fixture
        .sweep(&[
            empty.clone(),
            full.clone(),
            missing,
            folder.clone(),
            outside.clone(),
            root.clone(),
            dotted,
            PathBuf::from("relative/empty.mkv"),
        ])
        .await;

    assert_eq!(
        outcomes,
        vec![
            EmptyImportLeftover::Removed { catalog_row: false },
            EmptyImportLeftover::NotAnEmptyFile,
            EmptyImportLeftover::NotAnEmptyFile,
            EmptyImportLeftover::NotAnEmptyFile,
            EmptyImportLeftover::OutsideLibraryRoots,
            EmptyImportLeftover::OutsideLibraryRoots,
            EmptyImportLeftover::OutsideLibraryRoots,
            EmptyImportLeftover::OutsideLibraryRoots,
        ]
    );
    assert!(!empty.exists());
    assert_eq!(
        std::fs::read(&full).expect("full"),
        b"synthetic video bytes"
    );
    assert!(folder.is_dir());
    assert!(outside.exists());
    assert!(root.join("up.mkv").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_to_an_empty_file_is_kept() {
    let fixture = LeftoverFixture::new().await;
    let target = fixture.file("target.mkv", b"");
    let link = target.with_file_name("link.mkv");
    std::os::unix::fs::symlink(&target, &link).expect("link");

    let outcomes = fixture.sweep(std::slice::from_ref(&link)).await;

    assert_eq!(outcomes, vec![EmptyImportLeftover::NotAnEmptyFile]);
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link")
            .file_type()
            .is_symlink()
    );
    assert!(target.exists());
}

#[tokio::test]
async fn a_cataloged_empty_file_goes_with_its_row_only_when_the_catalog_records_it_empty() {
    let fixture = LeftoverFixture::new().await;
    let scanned_empty = fixture.file("scanned-empty.mkv", b"");
    fixture.row("synthetic-empty-row", &scanned_empty, 0).await;
    let once_full = fixture.file("once-full.mkv", b"");
    fixture.row("synthetic-full-row", &once_full, 4096).await;

    let outcomes = fixture
        .sweep(&[scanned_empty.clone(), once_full.clone()])
        .await;

    assert_eq!(
        outcomes,
        vec![
            EmptyImportLeftover::Removed { catalog_row: true },
            EmptyImportLeftover::CatalogedWithContent,
        ]
    );
    assert!(!scanned_empty.exists());
    assert!(once_full.exists());
    assert_eq!(fixture.row_ids().await, vec!["synthetic-full-row"]);
}
