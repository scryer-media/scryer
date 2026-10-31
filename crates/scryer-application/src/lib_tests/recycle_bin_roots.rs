//! A custom recycle bin must stay outside every library root, whichever root
//! the file being recycled comes from. A recycle that is refused leaves the
//! file exactly where it was.

use super::*;

/// A library store whose every read fails, standing in for a catalog that
/// cannot be read at the moment a file is recycled.
struct UnreadableLibraryRepo;

fn unreadable() -> AppError {
    AppError::Repository("library store unavailable".into())
}

#[async_trait]
impl LibraryRepository for UnreadableLibraryRepo {
    async fn list(&self, _facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        Err(unreadable())
    }
    async fn get_by_id(&self, _id: &str) -> AppResult<Option<Library>> {
        Err(unreadable())
    }
    async fn default_for_facet(&self, _facet: MediaFacet) -> AppResult<Option<Library>> {
        Err(unreadable())
    }
    async fn create(&self, _library: Library, _roots: Vec<LibraryRootDraft>) -> AppResult<Library> {
        Err(unreadable())
    }
    async fn update(
        &self,
        _library_id: &str,
        _name: String,
        _slug: String,
        _roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        Err(unreadable())
    }
    async fn set_root_path(&self, _root_id: &str, _path: &str) -> AppResult<Library> {
        Err(unreadable())
    }
    async fn delete_library(&self, _library_id: &str) -> AppResult<bool> {
        Err(unreadable())
    }
    async fn app_permission_mask_for_user(&self, _user_id: &str) -> AppResult<AppPermissionMask> {
        Err(unreadable())
    }
    async fn set_app_permission_mask_for_user(
        &self,
        _user_id: &str,
        _permissions: AppPermissionMask,
    ) -> AppResult<()> {
        Err(unreadable())
    }
    async fn permission_masks_for_user(&self, _user_id: &str) -> AppResult<Vec<LibraryGrant>> {
        Err(unreadable())
    }
    async fn set_grants_for_user(
        &self,
        _user_id: &str,
        _grants: Vec<LibraryGrant>,
    ) -> AppResult<()> {
        Err(unreadable())
    }
    async fn title_library_id(&self, _title_id: &str) -> AppResult<Option<String>> {
        Err(unreadable())
    }
}

struct Fixture {
    temp: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        for dir in ["library-a", "library-b"] {
            std::fs::create_dir_all(temp.path().join(dir)).expect("create library root");
        }
        Self { temp }
    }

    fn root_a(&self) -> PathBuf {
        self.temp.path().join("library-a")
    }

    fn root_b(&self) -> PathBuf {
        self.temp.path().join("library-b")
    }

    /// A media file plus an unrelated neighbour in the same title folder.
    fn seed_media(&self, root: &Path, name: &str) -> (PathBuf, PathBuf) {
        let folder = root.join(format!("{name} (2001)"));
        std::fs::create_dir_all(&folder).expect("create title folder");
        let media = folder.join(format!("{name}.mkv"));
        let neighbour = folder.join(format!("{name}.nfo"));
        std::fs::write(&media, format!("{name} media")).expect("write media file");
        std::fs::write(&neighbour, format!("{name} neighbour")).expect("write neighbour");
        (media, neighbour)
    }
}

fn settings_with_bin(bin: Option<&Path>) -> Arc<StoredSettingsRepo> {
    let settings = StoredSettingsRepo::default();
    if let Some(bin) = bin {
        let value = serde_json::to_string(&bin.to_string_lossy()).expect("encode bin path");
        settings
            .values
            .try_lock()
            .expect("fresh settings store")
            .insert(
                (
                    SETTINGS_SCOPE_MEDIA.to_string(),
                    RECYCLE_BIN_PATH_KEY.to_string(),
                    None,
                ),
                value,
            );
    }
    Arc::new(settings)
}

fn removal_manifest(source: &Path) -> crate::recycle_bin::RecycleManifest {
    crate::recycle_bin::RecycleManifest {
        schema: None,
        entry_id: None,
        source_operation_id: None,
        recycled_at: Utc::now().to_rfc3339(),
        original_path: source.to_string_lossy().to_string(),
        original_file_id: None,
        size_bytes: 0,
        title_id: None,
        media_root: None,
        reason: "file_deleted".to_string(),
        status: None,
        replacement_file_id: None,
        replacement_path: None,
        media_row: None,
    }
}

fn assert_untouched(path: &Path, content: &str) {
    assert_eq!(
        std::fs::read_to_string(path).expect("file is still in place"),
        content
    );
}

async fn movie_app(root: &Path) -> (AppUseCase, User) {
    let (app, user, _) =
        bootstrap_movie_scan_app(root, Vec::new(), Arc::new(EmptySearchMetadataGateway)).await;
    (app, user)
}

#[tokio::test]
async fn unreadable_library_roots_refuse_a_custom_bin_and_keep_the_file() {
    let fixture = Fixture::new();
    let bin = fixture.temp.path().join("outside-bin");
    let (app, _) = movie_app(&fixture.root_a()).await;
    let app = app.with_test_overrides(|services| {
        services
            .with_settings(settings_with_bin(Some(&bin)))
            .with_libraries(Arc::new(UnreadableLibraryRepo))
    });
    let root_a = fixture.root_a();
    let (media, neighbour) = fixture.seed_media(&root_a, "Synthetic Feature");

    let by_str = app
        .recycle_bin_config_for_media_root(Some(root_a.to_string_lossy().as_ref()))
        .await;
    let by_path = app
        .recycle_bin_config_for_media_root_path(Some(&root_a))
        .await;
    let for_recycling = app
        .recycle_bin_configs_for_recycling(vec![root_a.to_string_lossy().to_string()])
        .await;
    assert_eq!(for_recycling.len(), 1);
    for config in [&by_str, &by_path, &for_recycling[0].1] {
        let error = config
            .validation_error
            .as_deref()
            .expect("a bin that cannot be checked is refused");
        assert!(
            error.contains("could not be checked against the library roots"),
            "unexpected refusal: {error}"
        );
        assert!(!config.cleanup_enabled);
        assert_eq!(config.source_roots, vec![root_a.clone()]);

        crate::recycle_bin::recycle_file(config, &media, removal_manifest(&media))
            .await
            .expect_err("the recycle is refused");
        assert_untouched(&media, "Synthetic Feature media");
        assert_untouched(&neighbour, "Synthetic Feature neighbour");
    }
    assert!(!bin.exists(), "a refused recycle creates no bin");
}

#[tokio::test]
async fn unreadable_library_roots_leave_the_default_bin_working() {
    let fixture = Fixture::new();
    let (app, _) = movie_app(&fixture.root_a()).await;
    let app = app.with_test_overrides(|services| {
        services
            .with_settings(settings_with_bin(None))
            .with_libraries(Arc::new(UnreadableLibraryRepo))
    });
    let root_a = fixture.root_a();
    let (media, neighbour) = fixture.seed_media(&root_a, "Synthetic Feature");

    let config = app
        .recycle_bin_config_for_media_root(Some(root_a.to_string_lossy().as_ref()))
        .await;
    assert_eq!(config.validation_error, None);
    assert_eq!(config.base_path, root_a.join(".scryer-recycle"));

    let result = crate::recycle_bin::recycle_file(&config, &media, removal_manifest(&media))
        .await
        .expect("the default bin needs no library roots")
        .expect("file recycled");
    assert!(!media.exists());
    assert_untouched(&result.recycled_path, "Synthetic Feature media");
    assert_untouched(&neighbour, "Synthetic Feature neighbour");
}

#[tokio::test]
async fn upgrade_root_check_failure_preserves_existing_payload() {
    let fixture = Fixture::new();
    let root = fixture.root_a();
    let (app, user) = movie_app(&root).await;
    let (media, neighbour) = fixture.seed_media(&root, "Synthetic Feature");
    let title =
        create_movie_title_with_folder(&app, &user, "Synthetic Feature", media.parent().unwrap())
            .await;
    let old_file = TitleMediaFile {
        id: "synthetic-old-file".into(),
        title_id: title.id.clone(),
        file_path: media.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let moved_root = root.with_file_name("unavailable-root-fixture");
    std::fs::rename(&root, &moved_root).unwrap();
    let result = crate::upgrade::resolve_old_file_recycle_context(&app, &title, &old_file).await;
    assert!(matches!(
        result,
        Err(AppError::Validation(_)
            | AppError::ValidationRefused { .. }
            | AppError::ValidationRefusedWithCount { .. })
    ));
    assert_untouched(
        &moved_root.join(media.strip_prefix(&root).unwrap()),
        "Synthetic Feature media",
    );
    assert_untouched(
        &moved_root.join(neighbour.strip_prefix(&root).unwrap()),
        "Synthetic Feature neighbour",
    );
}

#[tokio::test]
async fn upgrade_refuses_a_custom_bin_inside_another_library_root_and_keeps_the_old_file() {
    let fixture = Fixture::new();
    let root_a = fixture.root_a();
    let root_b = fixture.root_b();
    let (app, user) = movie_app(&root_a).await;
    let now = Utc::now();
    app.services
        .catalog
        .libraries
        .create(
            Library {
                id: "synthetic-library-b".to_string(),
                facet: MediaFacet::Movie,
                name: "Synthetic Library B".to_string(),
                slug: "synthetic-library-b".to_string(),
                is_default: false,
                roots: Vec::new(),
                created_at: now,
                updated_at: now,
            },
            vec![LibraryRootDraft {
                id: None,
                path: root_b.to_string_lossy().to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("create second library");
    let (media, neighbour) = fixture.seed_media(&root_a, "Synthetic Feature");
    let title = create_movie_title_with_folder(
        &app,
        &user,
        "Synthetic Feature",
        media.parent().expect("title folder"),
    )
    .await;
    let bin = root_b.join("shared-bin");
    let app =
        app.with_test_overrides(|services| services.with_settings(settings_with_bin(Some(&bin))));
    let old_file = TitleMediaFile {
        id: "synthetic-old-file".to_string(),
        title_id: title.id.clone(),
        file_path: media.to_string_lossy().to_string(),
        ..Default::default()
    };

    let context = crate::upgrade::resolve_old_file_recycle_context(&app, &title, &old_file)
        .await
        .expect("the old file's root resolves");
    let error = context
        .recycle_config
        .validation_error
        .as_deref()
        .expect("a bin inside another library's root is refused");
    assert!(
        error.contains(&*root_b.to_string_lossy()),
        "the refusal names the root the bin sits in: {error}"
    );
    assert_eq!(context.recycle_config.source_roots, vec![root_a.clone()]);

    let refused = crate::recycle_bin::recycle_replaced_media_file(
        &context.recycle_config,
        &media,
        crate::recycle_bin::ReplacedMediaRecycleMetadata {
            original_path: &old_file.file_path,
            original_file_id: &old_file.id,
            size_bytes: 0,
            title_id: &title.id,
            media_root: Some(context.media_root.as_str()),
            media_row: None,
        },
        true,
    )
    .await;
    assert!(refused.is_err(), "the recycle is refused");
    assert_untouched(&media, "Synthetic Feature media");
    assert_untouched(&neighbour, "Synthetic Feature neighbour");
    assert!(!bin.exists(), "a refused recycle creates no bin");
}

/// Reads the library store once, as a title deletion does for the deleted
/// title's own roots, then fails every later read.
struct LibraryReadFailsAfterFirstList {
    inner: Arc<dyn LibraryRepository>,
    list_calls: AtomicUsize,
}

#[async_trait]
impl LibraryRepository for LibraryReadFailsAfterFirstList {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        if self.list_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.inner.list(facet).await
        } else {
            Err(unreadable())
        }
    }
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>> {
        self.inner.get_by_id(id).await
    }
    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>> {
        self.inner.default_for_facet(facet).await
    }
    async fn create(&self, library: Library, roots: Vec<LibraryRootDraft>) -> AppResult<Library> {
        self.inner.create(library, roots).await
    }
    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        self.inner.update(library_id, name, slug, roots).await
    }
    async fn set_root_path(&self, root_id: &str, path: &str) -> AppResult<Library> {
        self.inner.set_root_path(root_id, path).await
    }
    async fn delete_library(&self, library_id: &str) -> AppResult<bool> {
        self.inner.delete_library(library_id).await
    }
    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask> {
        self.inner.app_permission_mask_for_user(user_id).await
    }
    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()> {
        self.inner
            .set_app_permission_mask_for_user(user_id, permissions)
            .await
    }
    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>> {
        self.inner.permission_masks_for_user(user_id).await
    }
    async fn set_grants_for_user(&self, user_id: &str, grants: Vec<LibraryGrant>) -> AppResult<()> {
        self.inner.set_grants_for_user(user_id, grants).await
    }
    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>> {
        self.inner.title_library_id(title_id).await
    }
}

/// Two titles of library A, a second library B, and entries for both titles
/// already sitting in a bin, beside a file the bin does not own.
struct TitleDeleteFixture {
    fixture: Fixture,
    app: AppUseCase,
    user: User,
    deleted: Title,
    kept: Title,
}

impl TitleDeleteFixture {
    async fn new() -> Self {
        let fixture = Fixture::new();
        let root_a = fixture.root_a();
        let (app, user) = movie_app(&root_a).await;
        let now = Utc::now();
        app.services
            .catalog
            .libraries
            .create(
                Library {
                    id: "synthetic-library-b".to_string(),
                    facet: MediaFacet::Movie,
                    name: "Synthetic Library B".to_string(),
                    slug: "synthetic-library-b".to_string(),
                    is_default: false,
                    roots: Vec::new(),
                    created_at: now,
                    updated_at: now,
                },
                vec![LibraryRootDraft {
                    id: None,
                    path: fixture.root_b().to_string_lossy().to_string(),
                    is_default: true,
                }],
            )
            .await
            .expect("create second library");
        let deleted_folder = root_a.join("Synthetic Feature (2001)");
        let kept_folder = root_a.join("Synthetic Other Feature (2002)");
        std::fs::create_dir_all(&deleted_folder).expect("create title folder");
        std::fs::create_dir_all(&kept_folder).expect("create title folder");
        let deleted =
            create_movie_title_with_folder(&app, &user, "Synthetic Feature", &deleted_folder).await;
        let kept =
            create_movie_title_with_folder(&app, &user, "Synthetic Other Feature", &kept_folder)
                .await;
        Self {
            fixture,
            app,
            user,
            deleted,
            kept,
        }
    }

    /// Recycle one file of each title into `bin` and drop an unrelated file
    /// beside the entries. Returns the entry directory of each title.
    async fn seed_bin(&self, bin: &Path) -> (PathBuf, PathBuf) {
        let root_a = self.fixture.root_a();
        let config = crate::recycle_bin::RecycleBinConfig {
            enabled: true,
            base_path: bin.to_path_buf(),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![root_a.clone()],
        };
        let mut entries = Vec::new();
        for (title, name) in [
            (&self.deleted, "synthetic-feature"),
            (&self.kept, "synthetic-other-feature"),
        ] {
            let source = root_a.join(format!("{name}.mkv"));
            std::fs::write(&source, format!("{name} content")).expect("write source file");
            let mut manifest = removal_manifest(&source);
            manifest.title_id = Some(title.id.clone());
            let recycled = crate::recycle_bin::recycle_file(&config, &source, manifest)
                .await
                .expect("seed recycle entry")
                .expect("file recycled");
            entries.push(
                recycled
                    .recycled_path
                    .parent()
                    .expect("entry directory")
                    .to_path_buf(),
            );
        }
        std::fs::write(bin.join("unrelated.txt"), "unrelated bin content")
            .expect("write unrelated file");
        let kept_entry = entries.pop().expect("kept entry");
        let deleted_entry = entries.pop().expect("deleted entry");
        (deleted_entry, kept_entry)
    }

    fn with_bin_setting(&self, bin: Option<&Path>) -> AppUseCase {
        self.app
            .with_test_overrides(|services| services.with_settings(settings_with_bin(bin)))
    }
}

/// Every path under `dir` with its bytes; directories map to `None`.
fn snapshot_tree(dir: &Path) -> std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut tree = std::collections::BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(&current).expect("read snapshot directory") {
            let path = entry.expect("snapshot entry").path();
            if path.is_dir() {
                tree.insert(path.clone(), None);
                pending.push(path);
            } else {
                tree.insert(path.clone(), Some(std::fs::read(&path).expect("read file")));
            }
        }
    }
    tree
}

#[tokio::test]
async fn title_delete_leaves_a_custom_bin_inside_another_library_root_untouched() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_b().join("shared-bin");
    let (deleted_entry, kept_entry) = fixture.seed_bin(&bin).await;
    let before = snapshot_tree(&bin);
    let app = fixture.with_bin_setting(Some(&bin));

    app.delete_title(&fixture.user, &fixture.deleted.id, false, None)
        .await
        .expect("the title is deleted even though its bin entries stay");

    assert_eq!(snapshot_tree(&bin), before, "nothing in the bin changed");
    assert!(deleted_entry.exists());
    assert!(kept_entry.exists());
    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&fixture.deleted.id)
            .await
            .expect("read title")
            .is_none(),
        "the title itself is gone"
    );
}

#[tokio::test]
async fn title_delete_purges_its_entries_from_a_custom_bin_outside_every_root() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.temp.path().join("outside-bin");
    let (deleted_entry, kept_entry) = fixture.seed_bin(&bin).await;
    let kept_before = snapshot_tree(&kept_entry);
    let app = fixture.with_bin_setting(Some(&bin));

    app.delete_title(&fixture.user, &fixture.deleted.id, false, None)
        .await
        .expect("delete title");

    assert!(
        !deleted_entry.exists(),
        "the deleted title's entry is purged"
    );
    assert_eq!(snapshot_tree(&kept_entry), kept_before);
    assert_untouched(&bin.join("unrelated.txt"), "unrelated bin content");
}

#[tokio::test]
async fn title_delete_purges_its_entries_from_the_default_per_root_bin() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    let (deleted_entry, kept_entry) = fixture.seed_bin(&bin).await;
    let kept_before = snapshot_tree(&kept_entry);
    let app = fixture.with_bin_setting(None);

    app.delete_title(&fixture.user, &fixture.deleted.id, false, None)
        .await
        .expect("delete title");

    assert!(
        !deleted_entry.exists(),
        "the deleted title's entry is purged"
    );
    assert_eq!(snapshot_tree(&kept_entry), kept_before);
    assert_untouched(&bin.join("unrelated.txt"), "unrelated bin content");
}

#[tokio::test]
async fn title_delete_leaves_a_custom_bin_untouched_when_library_roots_are_unreadable() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.temp.path().join("outside-bin");
    let (deleted_entry, kept_entry) = fixture.seed_bin(&bin).await;
    let before = snapshot_tree(&bin);
    let libraries = fixture.app.services.catalog.libraries.clone();
    let app = fixture.app.with_test_overrides(|services| {
        services
            .with_settings(settings_with_bin(Some(&bin)))
            .with_libraries(Arc::new(LibraryReadFailsAfterFirstList {
                inner: libraries,
                list_calls: AtomicUsize::new(0),
            }))
    });

    app.purge_title_logical_dependents(&fixture.deleted, true, DomainEventActor::system())
        .await
        .expect("the purge completes without the bin");

    assert_eq!(snapshot_tree(&bin), before, "nothing in the bin changed");
    assert!(deleted_entry.exists());
    assert!(kept_entry.exists());
}

fn rewrite_entry(entry: &Path, reason: &str, status: &str) {
    let path = entry.join("manifest.json");
    let mut manifest: crate::recycle_bin::RecycleManifest =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest.reason = reason.into();
    manifest.status = Some(status.into());
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

#[tokio::test]
async fn title_delete_policy_cleanup_keeps_replacement_validation() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    let (deleted, kept) = fixture.seed_bin(&bin).await;
    rewrite_entry(
        &deleted,
        "upgrade_replaced",
        crate::recycle_bin::RECYCLE_STATUS_COMMITTED,
    );
    let kept_before = snapshot_tree(&kept);
    fixture
        .with_bin_setting(None)
        .purge_title_logical_dependents(&fixture.deleted, true, DomainEventActor::system())
        .await
        .unwrap();
    let manifest: crate::recycle_bin::RecycleManifest =
        serde_json::from_slice(&std::fs::read(deleted.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(
        manifest.status.as_deref(),
        Some(crate::recycle_bin::RECYCLE_STATUS_QUARANTINED)
    );
    assert_eq!(snapshot_tree(&kept), kept_before);
    assert!(deleted.exists());
}

#[tokio::test]
async fn title_delete_purges_upgrade_without_replacement_but_preserves_other_title() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    let (deleted, kept) = fixture.seed_bin(&bin).await;
    rewrite_entry(
        &deleted,
        "upgrade_replaced",
        crate::recycle_bin::RECYCLE_STATUS_COMMITTED,
    );
    let before = snapshot_tree(&kept);
    let outcome = fixture
        .with_bin_setting(None)
        .delete_title_with_outcome(&fixture.user, &fixture.deleted.id, false, None)
        .await
        .unwrap();
    assert_eq!(outcome.purged, 1);
    assert!(!outcome.incomplete());
    assert!(!deleted.exists());
    assert_eq!(snapshot_tree(&kept), before);
}

#[tokio::test]
async fn title_delete_preserves_pending_and_quarantined_entries() {
    for status in [
        crate::recycle_bin::RECYCLE_STATUS_PENDING,
        crate::recycle_bin::RECYCLE_STATUS_QUARANTINED,
    ] {
        let fixture = TitleDeleteFixture::new().await;
        let bin = fixture.fixture.root_a().join(".scryer-recycle");
        let (deleted, _) = fixture.seed_bin(&bin).await;
        rewrite_entry(&deleted, "upgrade_replaced", status);
        let before = snapshot_tree(&bin);
        let outcome = fixture
            .with_bin_setting(None)
            .delete_title_with_outcome(&fixture.user, &fixture.deleted.id, false, None)
            .await
            .unwrap();
        assert_eq!(outcome.purged, 0);
        assert_eq!(snapshot_tree(&bin), before);
    }
}

#[tokio::test]
async fn title_delete_bulk_reports_recycle_warning_without_losing_successful_id() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    let (deleted, _) = fixture.seed_bin(&bin).await;
    let jobs = Arc::new(RecordingJobRunRepo::default());
    let app = fixture
        .with_bin_setting(None)
        .with_test_overrides(|services| services.with_job_runs(jobs.clone()));
    let entry_id = deleted.file_name().unwrap().to_str().unwrap();
    let _guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .try_acquire(&format!("recycle-entry:{entry_id}"))
        .await
        .unwrap();
    let accepted = app
        .start_delete_titles_job(
            &fixture.user,
            DeleteTitlesJobRequest {
                items: vec![DeleteTitlesJobItem {
                    title_id: fixture.deleted.id.clone(),
                    preview_fingerprint: None,
                }],
                delete_files_on_disk: false,
                typed_confirmation: None,
            },
        )
        .await
        .unwrap();
    let run = crate::test_wait::wait_for("the specific deletion job to finish", || async {
        jobs.runs
            .lock()
            .await
            .iter()
            .find(|run| run.id == accepted.job_run.id && run.status.is_terminal())
            .cloned()
    })
    .await;
    assert_eq!(run.status, JobRunStatus::Warning);
    let summary: serde_json::Value =
        serde_json::from_str(run.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["succeededTitleIds"][0], fixture.deleted.id);
    assert_eq!(
        summary["recycleIncomplete"][0]["reason"],
        crate::title_delete::RECYCLE_ENTRY_BUSY
    );
    assert_eq!(summary["recycleIncomplete"][0]["preservedCount"], 1);
    assert!(deleted.exists());
}

#[tokio::test]
async fn title_delete_bulk_purges_recycle_entry_created_during_earlier_item() {
    crate::test_wait::within_deadline("late recycle entry is purged by its batch item", async {
        let fixture = TitleDeleteFixture::new().await;
        let titles = Arc::new(MockTitleRepo::default());
        titles.create(fixture.deleted.clone()).await.unwrap();
        titles.create(fixture.kept.clone()).await.unwrap();
        let operations = Arc::new(Mutex::new(Vec::new()));
        titles.set_delete_operation_log(operations.clone()).await;
        let barrier = operations.lock().await;
        let jobs = Arc::new(RecordingJobRunRepo::default());
        let mut app = fixture
            .with_bin_setting(None)
            .with_test_overrides(|services| services.with_job_runs(jobs.clone()));
        app.services.catalog.titles = titles.clone();
        let accepted = app
            .start_delete_titles_job(
                &fixture.user,
                DeleteTitlesJobRequest {
                    items: [&fixture.deleted.id, &fixture.kept.id]
                        .into_iter()
                        .map(|id| DeleteTitlesJobItem {
                            title_id: id.clone(),
                            preview_fingerprint: None,
                        })
                        .collect(),
                    delete_files_on_disk: false,
                    typed_confirmation: None,
                },
            )
            .await
            .unwrap();
        crate::test_wait::wait_until(
            "first recycle batch item reaches catalog deletion",
            || async {
                titles
                    .delete_started
                    .lock()
                    .await
                    .contains(&fixture.deleted.id)
            },
        )
        .await;
        let root = fixture.fixture.root_a();
        let config = crate::recycle_bin::RecycleBinConfig {
            enabled: true,
            base_path: root.join(".scryer-recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![root.clone()],
        };
        let source = root.join("late-recycled-feature.mkv");
        std::fs::write(&source, b"late recycle bytes").unwrap();
        let mut manifest = removal_manifest(&source);
        manifest.title_id = Some(fixture.kept.id.clone());
        let recycled = crate::recycle_bin::recycle_file(&config, &source, manifest)
            .await
            .unwrap()
            .unwrap();
        let unrelated = config.base_path.join("unrelated.txt");
        std::fs::write(&unrelated, b"keep unrelated bytes").unwrap();
        drop(barrier);
        let run = crate::test_wait::wait_for("specific late-recycle batch completes", || async {
            jobs.runs
                .lock()
                .await
                .iter()
                .find(|run| run.id == accepted.job_run.id && run.status.is_terminal())
                .cloned()
        })
        .await;
        let summary: serde_json::Value =
            serde_json::from_str(run.summary_json.as_deref().unwrap()).unwrap();
        assert_eq!(run.status, JobRunStatus::Completed);
        assert_eq!(
            summary["succeededTitleIds"],
            serde_json::json!([fixture.deleted.id, fixture.kept.id])
        );
        assert_eq!(summary["recycleOutcomes"][&fixture.kept.id]["purged"], 1);
        assert!(!recycled.recycled_path.exists());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep unrelated bytes");
    })
    .await;
}

#[tokio::test]
async fn title_delete_reports_busy_recycle_entry_and_still_deletes_catalog_title() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    let (deleted, _) = fixture.seed_bin(&bin).await;
    let app = fixture.with_bin_setting(None);
    let entry_id = deleted.file_name().unwrap().to_str().unwrap();
    let _guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .try_acquire(&format!("recycle-entry:{entry_id}"))
        .await
        .unwrap();
    let before = snapshot_tree(&bin);
    let outcome = app
        .delete_title_with_outcome(&fixture.user, &fixture.deleted.id, false, None)
        .await
        .unwrap();
    assert_eq!(outcome.busy, 1);
    assert!(outcome.incomplete());
    assert_eq!(snapshot_tree(&bin), before);
    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&fixture.deleted.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn title_delete_exclusive_guard_preserves_title_and_recycle_files() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    fixture.seed_bin(&bin).await;
    let app = fixture.with_bin_setting(None);
    let _guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .try_acquire(&format!("title-delete:{}", fixture.deleted.id))
        .await
        .unwrap();
    let before = snapshot_tree(&bin);
    let error = app
        .delete_title(&fixture.user, &fixture.deleted.id, false, None)
        .await
        .unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(crate::title_delete::IN_PROGRESS)
    );
    assert_eq!(snapshot_tree(&bin), before);
    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&fixture.deleted.id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn title_delete_rechecks_revoked_permissions_before_purging_files() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    fixture.seed_bin(&bin).await;
    let app = fixture.with_bin_setting(None);
    // The request actor still carries its old administrator projection.
    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(&fixture.user.id, AppPermissionMask::NONE)
        .await
        .unwrap();
    let before = snapshot_tree(&bin);
    assert!(matches!(
        app.delete_title(&fixture.user, &fixture.deleted.id, false, None)
            .await,
        Err(AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. })
    ));
    assert_eq!(snapshot_tree(&bin), before);
    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&fixture.deleted.id)
            .await
            .unwrap()
            .is_some()
    );
}

// ── A new or changed library root must stay clear of the custom bin ─────────

fn root_draft(path: &Path) -> LibraryRootDraft {
    LibraryRootDraft {
        id: None,
        path: path.to_string_lossy().to_string(),
        is_default: true,
    }
}

/// A custom bin at `<temp>/disk/bin`, with `disk` and the bin created.
async fn app_with_custom_bin(fixture: &Fixture) -> (AppUseCase, User, PathBuf) {
    let bin = fixture.temp.path().join("disk").join("bin");
    std::fs::create_dir_all(&bin).expect("create custom bin");
    let (app, user) = movie_app(&fixture.root_a()).await;
    let app =
        app.with_test_overrides(|services| services.with_settings(settings_with_bin(Some(&bin))));
    (app, user, bin)
}

async fn library_names(app: &AppUseCase) -> Vec<String> {
    app.services
        .catalog
        .libraries
        .list(None)
        .await
        .expect("list libraries")
        .into_iter()
        .map(|library| library.name)
        .collect()
}

fn assert_bin_conflict(error: AppError, root: &Path) {
    match error {
        AppError::Validation(message)
        | AppError::ValidationRefused { message, .. }
        | AppError::ValidationRefusedWithCount { message, .. } => {
            assert!(
                message.contains(&*root.to_string_lossy())
                    && message.contains("custom recycle bin path"),
                "unexpected refusal: {message}"
            );
        }
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn creating_a_library_whose_root_conflicts_with_the_custom_bin_is_refused() {
    let fixture = Fixture::new();
    let (app, user, bin) = app_with_custom_bin(&fixture).await;
    let libraries_before = library_names(&app).await;

    for root in [
        fixture.temp.path().join("disk"),
        bin.clone(),
        bin.join("inside"),
    ] {
        let error = app
            .create_library(
                &user,
                MediaFacet::Movie,
                "Synthetic Conflicting Library".to_string(),
                vec![root_draft(&root)],
                None,
            )
            .await
            .expect_err("a root holding, equal to, or inside the bin is refused");
        assert_bin_conflict(error, &root);
    }
    assert_eq!(library_names(&app).await, libraries_before);
    assert!(!bin.join("inside").exists(), "the check creates nothing");
}

#[tokio::test]
async fn creating_a_library_with_an_unrelated_root_is_accepted() {
    let fixture = Fixture::new();
    let (app, user, _) = app_with_custom_bin(&fixture).await;

    let library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Synthetic Library B".to_string(),
            vec![root_draft(&fixture.root_b())],
            None,
        )
        .await
        .expect("a root clear of the bin is accepted");
    assert_eq!(library.roots.len(), 1);
}

#[tokio::test]
async fn without_a_custom_bin_any_root_is_accepted() {
    let fixture = Fixture::new();
    let (app, user) = movie_app(&fixture.root_a()).await;
    let app = app.with_test_overrides(|services| services.with_settings(settings_with_bin(None)));
    let disk = fixture.temp.path().join("disk");
    std::fs::create_dir_all(disk.join("bin")).expect("create folders");

    for (name, root) in [
        ("Synthetic Library One", disk.clone()),
        ("Synthetic Library Two", disk.join("bin").join("inside")),
    ] {
        app.create_library(
            &user,
            MediaFacet::Movie,
            name.to_string(),
            vec![root_draft(&root)],
            None,
        )
        .await
        .expect("nothing to conflict with");
    }
}

#[tokio::test]
async fn with_the_recycle_bin_turned_off_a_root_inside_the_custom_bin_is_accepted() {
    let fixture = Fixture::new();
    let (app, user) = movie_app(&fixture.root_a()).await;
    let bin = fixture.temp.path().join("disk").join("bin");
    std::fs::create_dir_all(&bin).expect("create custom bin");
    let settings = settings_with_bin(Some(&bin));
    settings
        .values
        .try_lock()
        .expect("fresh settings store")
        .insert(
            (
                SETTINGS_SCOPE_MEDIA.to_string(),
                RECYCLE_BIN_ENABLED_KEY.to_string(),
                None,
            ),
            "false".to_string(),
        );
    let app = app.with_test_overrides(|services| services.with_settings(settings));

    app.create_library(
        &user,
        MediaFacet::Movie,
        "Synthetic Library B".to_string(),
        vec![root_draft(&bin.join("inside"))],
        None,
    )
    .await
    .expect("a disabled bin refuses no root");
}

#[tokio::test]
async fn updating_a_library_refuses_only_new_roots_that_conflict_with_the_custom_bin() {
    let fixture = Fixture::new();
    let (app, user, bin) = app_with_custom_bin(&fixture).await;
    let disk = fixture.temp.path().join("disk");
    let now = Utc::now();
    // Stored directly: a root that already holds the bin, the shape a bin
    // saved before this check leaves behind.
    let library = app
        .services
        .catalog
        .libraries
        .create(
            Library {
                id: "synthetic-library-b".to_string(),
                facet: MediaFacet::Movie,
                name: "Synthetic Library B".to_string(),
                slug: "synthetic-library-b".to_string(),
                is_default: false,
                roots: Vec::new(),
                created_at: now,
                updated_at: now,
            },
            vec![root_draft(&disk)],
        )
        .await
        .expect("create library with a conflicting root");

    let renamed = app
        .update_library(
            &user,
            &library.id,
            Some("Synthetic Library Renamed".to_string()),
            None,
            None,
        )
        .await
        .expect("a rename never checks the roots it keeps");
    assert_eq!(renamed.name, "Synthetic Library Renamed");

    app.update_library(
        &user,
        &library.id,
        None,
        Some(vec![root_draft(&disk), root_draft(&fixture.root_b())]),
        None,
    )
    .await
    .expect("the unchanged root is not rechecked and the new one is clear");

    let error = app
        .update_library(
            &user,
            &library.id,
            None,
            Some(vec![root_draft(&disk), root_draft(&bin.join("inside"))]),
            None,
        )
        .await
        .expect_err("an added root inside the bin is refused");
    assert_bin_conflict(error, &bin.join("inside"));
    let roots = app
        .services
        .catalog
        .libraries
        .get_by_id(&library.id)
        .await
        .expect("read library")
        .expect("library exists")
        .roots
        .into_iter()
        .map(|root| root.path)
        .collect::<Vec<_>>();
    assert_eq!(
        roots,
        vec![
            disk.to_string_lossy().to_string(),
            fixture.root_b().to_string_lossy().to_string()
        ],
        "the refused update wrote nothing"
    );
}

#[tokio::test]
async fn media_settings_refuse_a_default_library_root_that_conflicts_with_the_custom_bin() {
    let fixture = Fixture::new();
    let (app, user, bin) = app_with_custom_bin(&fixture).await;
    let root = bin.join("inside");

    let error = app
        .update_media_settings(
            &user,
            MediaFacet::Movie,
            empty_update_media_settings_with_roots(vec![RootFolderEntry {
                path: root.to_string_lossy().to_string(),
                is_default: true,
            }]),
        )
        .await
        .expect_err("a default-library root inside the bin is refused");
    assert_bin_conflict(error, &root);

    let error = app
        .update_library_paths(
            &user,
            UpdateLibraryPaths {
                movie_path: root.to_string_lossy().to_string(),
                series_path: String::new(),
                anime_path: None,
            },
        )
        .await
        .expect_err("the legacy library path save is refused the same way");
    assert_bin_conflict(error, &root);
}

#[tokio::test]
async fn a_root_change_into_the_custom_bin_is_refused_before_anything_moves() {
    use crate::location::model::LocationExecutionMode;
    use crate::location::root_scope::refusal_codes;
    use crate::location::root_scope_execution::{RootScopeCall, RootScopeCallDestination};

    let fixture = Fixture::new();
    let (app, user, bin) = app_with_custom_bin(&fixture).await;
    let (media, neighbour) = fixture.seed_media(&fixture.root_a(), "Synthetic Feature");
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let library = app
        .services
        .catalog
        .libraries
        .get_by_id(&library_id)
        .await
        .expect("read library")
        .expect("default library");
    let call = |destination: &Path| RootScopeCall {
        library_id: library_id.clone(),
        root_id: library.roots[0].id.clone(),
        destination: RootScopeCallDestination::Path(destination.to_string_lossy().to_string()),
        mode: LocationExecutionMode::MoveWithScryer,
    };

    let error = app
        .preview_root_scope(&user, &call(&bin.join("new-root")))
        .await
        .expect_err("a new root inside the bin is refused");
    assert!(
        matches!(
            &error,
            AppError::LocationRootRefused { code, .. }
                if *code == refusal_codes::CHANGE_DESTINATION_CONFLICTS_WITH_RECYCLE_BIN
        ),
        "got {error:?}"
    );
    assert_untouched(&media, "Synthetic Feature media");
    assert_untouched(&neighbour, "Synthetic Feature neighbour");
    assert!(!bin.join("new-root").exists());

    app.preview_root_scope(&user, &call(&fixture.temp.path().join("new-disk")))
        .await
        .expect("a new root clear of the bin plans as before");
}

#[cfg(unix)]
fn link(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("create fixture link");
}

#[cfg(unix)]
#[tokio::test]
async fn a_custom_bin_that_links_into_a_library_root_is_refused_and_keeps_the_file() {
    let fixture = Fixture::new();
    let root_a = fixture.root_a();
    let inside = root_a.join("linked-bin");
    std::fs::create_dir_all(&inside).expect("create folder inside the root");
    let bin = fixture.temp.path().join("bin-link");
    link(&inside, &bin);
    let (app, user) = movie_app(&root_a).await;
    let app =
        app.with_test_overrides(|services| services.with_settings(settings_with_bin(Some(&bin))));
    let (media, neighbour) = fixture.seed_media(&root_a, "Synthetic Feature");

    let config = app
        .recycle_bin_config_for_media_root(Some(root_a.to_string_lossy().as_ref()))
        .await;
    let error = config
        .validation_error
        .as_deref()
        .expect("a bin reaching into a root through a link is refused");
    assert!(
        error.contains("once links are resolved"),
        "unexpected refusal: {error}"
    );
    assert!(!config.cleanup_enabled);
    crate::recycle_bin::recycle_file(&config, &media, removal_manifest(&media))
        .await
        .expect_err("the recycle is refused");
    assert_untouched(&media, "Synthetic Feature media");
    assert_untouched(&neighbour, "Synthetic Feature neighbour");
    assert_eq!(
        std::fs::read_dir(&inside)
            .expect("read linked folder")
            .count(),
        0,
        "nothing was written through the link"
    );

    // Saving such a location is refused too, and the stored one stays.
    let app = app.with_test_overrides(|services| services.with_settings(settings_with_bin(None)));
    let error = app
        .update_recycle_bin_settings(
            &user,
            UpdateRecycleBinSettings {
                enabled: None,
                path: Some(Some(bin.to_string_lossy().to_string())),
                retention_days: None,
            },
        )
        .await
        .expect_err("the linked location is refused");
    assert!(
        matches!(
            error,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{error:?}"
    );
    let settings = app
        .get_recycle_bin_settings(&user)
        .await
        .expect("read settings");
    assert_eq!(settings.path, None);
}

#[cfg(unix)]
#[tokio::test]
async fn a_custom_bin_under_a_link_that_leads_nowhere_is_refused_and_creates_nothing() {
    let fixture = Fixture::new();
    let root_a = fixture.root_a();
    let missing = fixture.temp.path().join("missing-target");
    let dangling = fixture.temp.path().join("dangling");
    link(&missing, &dangling);
    let bin = dangling.join("bin");
    let (app, _) = movie_app(&root_a).await;
    let app =
        app.with_test_overrides(|services| services.with_settings(settings_with_bin(Some(&bin))));
    let (media, neighbour) = fixture.seed_media(&root_a, "Synthetic Feature");

    let config = app
        .recycle_bin_config_for_media_root(Some(root_a.to_string_lossy().as_ref()))
        .await;
    let error = config
        .validation_error
        .as_deref()
        .expect("a bin that cannot be resolved is refused");
    assert!(
        error.contains("could not be resolved"),
        "unexpected refusal: {error}"
    );
    crate::recycle_bin::recycle_file(&config, &media, removal_manifest(&media))
        .await
        .expect_err("the recycle is refused");
    assert_untouched(&media, "Synthetic Feature media");
    assert_untouched(&neighbour, "Synthetic Feature neighbour");
    assert!(!missing.exists(), "nothing was created behind the link");
}

#[cfg(unix)]
#[tokio::test]
async fn a_custom_bin_that_links_outside_every_root_still_recycles() {
    let fixture = Fixture::new();
    let root_a = fixture.root_a();
    let outside = fixture.temp.path().join("outside-bin");
    std::fs::create_dir_all(&outside).expect("create outside folder");
    let bin = fixture.temp.path().join("bin-link");
    link(&outside, &bin);
    let (app, _) = movie_app(&root_a).await;
    let app =
        app.with_test_overrides(|services| services.with_settings(settings_with_bin(Some(&bin))));
    let (media, neighbour) = fixture.seed_media(&root_a, "Synthetic Feature");

    let config = app
        .recycle_bin_config_for_media_root(Some(root_a.to_string_lossy().as_ref()))
        .await;
    assert_eq!(config.validation_error, None);
    let result = crate::recycle_bin::recycle_file(&config, &media, removal_manifest(&media))
        .await
        .expect("the bin is usable")
        .expect("file recycled");
    assert_untouched(&result.recycled_path, "Synthetic Feature media");
    assert_untouched(&neighbour, "Synthetic Feature neighbour");
}

#[cfg(unix)]
#[tokio::test]
async fn a_library_root_that_links_to_the_folder_holding_the_custom_bin_is_refused() {
    let fixture = Fixture::new();
    let (app, user, _) = app_with_custom_bin(&fixture).await;
    let libraries_before = library_names(&app).await;
    let root = fixture.temp.path().join("disk-link");
    link(&fixture.temp.path().join("disk"), &root);

    let error = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Synthetic Linked Library".to_string(),
            vec![root_draft(&root)],
            None,
        )
        .await
        .expect_err("a root reaching the bin through a link is refused");
    assert_bin_conflict(error, &root);
    assert_eq!(library_names(&app).await, libraries_before);
}
