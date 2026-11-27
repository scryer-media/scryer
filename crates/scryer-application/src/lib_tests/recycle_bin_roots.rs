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

    async fn root_title_counts(&self) -> AppResult<std::collections::HashMap<String, i64>> {
        Err(unreadable())
    }
}

struct Fixture {
    temp: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        // Recovery refuses symlinked ancestors, so the fixture lives under
        // the temp folder's real path (macOS links `/var` to `/private/var`).
        let base = std::fs::canonicalize(std::env::temp_dir()).expect("temp dir");
        let temp = tempfile::tempdir_in(base).expect("fixture tempdir");
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
        episode_ids: None,
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

        crate::recycle_bin::recycle_file(
            config,
            &media,
            removal_manifest(&media),
            crate::location::model::VerificationDepth::Full,
        )
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

    let result = crate::recycle_bin::recycle_file(
        &config,
        &media,
        removal_manifest(&media),
        crate::location::model::VerificationDepth::Full,
    )
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
            episode_ids: Vec::new(),
        },
        true,
        crate::location::model::VerificationDepth::Full,
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

    async fn root_title_counts(&self) -> AppResult<std::collections::HashMap<String, i64>> {
        self.inner.root_title_counts().await
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
            let recycled = crate::recycle_bin::recycle_file(
                &config,
                &source,
                manifest,
                crate::location::model::VerificationDepth::Full,
            )
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

/// The deleted titles awaiting recycle cleanup by housekeeping.
async fn recorded_recycle_purges(app: &AppUseCase) -> Vec<String> {
    app.services
        .catalog
        .titles
        .list_recycle_purges(usize::MAX)
        .await
        .expect("read recorded recycle cleanup")
        .into_iter()
        .map(|purge| purge.title_id)
        .collect()
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

    app.purge_recycle_bins_for_housekeeping().await;
    assert_eq!(
        snapshot_tree(&bin),
        before,
        "housekeeping leaves a bin it cannot verify alone"
    );
    assert_eq!(
        recorded_recycle_purges(&app).await,
        vec![fixture.deleted.id.clone()],
        "the title stays recorded for a later pass"
    );
}

/// A deleted title's entries are purged by housekeeping from a custom bin
/// outside every root and from the default per-root bin alike.
#[tokio::test]
async fn housekeeping_purges_a_deleted_titles_entries_from_custom_and_default_bins() {
    for custom in [true, false] {
        let fixture = TitleDeleteFixture::new().await;
        let bin = if custom {
            fixture.fixture.temp.path().join("outside-bin")
        } else {
            fixture.fixture.root_a().join(".scryer-recycle")
        };
        let (deleted_entry, kept_entry) = fixture.seed_bin(&bin).await;
        let kept_before = snapshot_tree(&kept_entry);
        let app = fixture.with_bin_setting(custom.then_some(bin.as_path()));

        app.delete_title(&fixture.user, &fixture.deleted.id, false, None)
            .await
            .expect("delete title");
        assert!(
            deleted_entry.exists(),
            "deletion leaves the entry to housekeeping"
        );
        assert_eq!(
            recorded_recycle_purges(&app).await,
            vec![fixture.deleted.id.clone()]
        );

        assert_eq!(app.purge_recycle_bins_for_housekeeping().await, 1);
        assert!(
            !deleted_entry.exists(),
            "the deleted title's entry is purged"
        );
        assert_eq!(snapshot_tree(&kept_entry), kept_before);
        assert_untouched(&bin.join("unrelated.txt"), "unrelated bin content");
        assert!(recorded_recycle_purges(&app).await.is_empty());
    }
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

    app.purge_title_logical_dependents(
        &fixture.deleted,
        true,
        DomainEventActor::system(),
        &mut crate::recycle_bin::TitleRecycleEntryIndex::default(),
    )
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
        .purge_title_logical_dependents(
            &fixture.deleted,
            true,
            DomainEventActor::system(),
            &mut crate::recycle_bin::TitleRecycleEntryIndex::default(),
        )
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
async fn housekeeping_preserves_pending_and_quarantined_entries_of_a_deleted_title() {
    for status in [
        crate::recycle_bin::RECYCLE_STATUS_PENDING,
        crate::recycle_bin::RECYCLE_STATUS_QUARANTINED,
    ] {
        let fixture = TitleDeleteFixture::new().await;
        let bin = fixture.fixture.root_a().join(".scryer-recycle");
        let (deleted, _) = fixture.seed_bin(&bin).await;
        rewrite_entry(&deleted, "upgrade_replaced", status);
        let before = snapshot_tree(&bin);
        let app = fixture.with_bin_setting(None);
        app.delete_title(&fixture.user, &fixture.deleted.id, false, None)
            .await
            .unwrap();
        assert_eq!(app.purge_recycle_bins_for_housekeeping().await, 0);
        assert_eq!(snapshot_tree(&bin), before);
        assert_eq!(
            recorded_recycle_purges(&app).await,
            vec![fixture.deleted.id.clone()],
            "a withheld entry keeps the title recorded"
        );
    }
}

#[tokio::test]
async fn title_delete_bulk_leaves_a_busy_recycle_entry_recorded_for_housekeeping() {
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
    assert_eq!(run.status, JobRunStatus::Completed);
    let summary: serde_json::Value =
        serde_json::from_str(run.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["succeededTitleIds"][0], fixture.deleted.id);
    assert!(summary.get("recycleIncomplete").is_none());
    assert!(summary.get("recycleOutcomes").is_none());
    assert!(deleted.exists());

    assert_eq!(app.purge_recycle_bins_for_housekeeping().await, 0);
    assert!(deleted.exists(), "a busy entry survives the pass");
    assert_eq!(
        recorded_recycle_purges(&app).await,
        vec![fixture.deleted.id.clone()],
        "and keeps its title recorded"
    );
    drop(_guard);
    assert_eq!(app.purge_recycle_bins_for_housekeeping().await, 1);
    assert!(!deleted.exists(), "the next pass purges it");
    assert!(recorded_recycle_purges(&app).await.is_empty());
}

#[tokio::test]
async fn housekeeping_purges_recycle_entry_created_during_a_deletion_job() {
    crate::test_wait::within_deadline("late recycle entry is purged by housekeeping", async {
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
        let recycled = crate::recycle_bin::recycle_file(
            &config,
            &source,
            manifest,
            crate::location::model::VerificationDepth::Full,
        )
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
        assert!(summary.get("recycleOutcomes").is_none());
        assert!(
            recycled.recycled_path.exists(),
            "deletion leaves it to housekeeping"
        );
        assert_eq!(app.purge_recycle_bins_for_housekeeping().await, 1);
        assert!(!recycled.recycled_path.exists());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep unrelated bytes");
        assert!(recorded_recycle_purges(&app).await.is_empty());
    })
    .await;
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
        Some(with_stored_root_ids(
            &library,
            vec![root_draft(&disk), root_draft(&fixture.root_b())],
        )),
        None,
    )
    .await
    .expect("the unchanged root is not rechecked and the new one is clear");

    let error = app
        .update_library(
            &user,
            &library.id,
            None,
            Some(with_stored_root_ids(
                &library,
                vec![root_draft(&disk), root_draft(&bin.join("inside"))],
            )),
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
async fn a_default_library_root_that_conflicts_with_the_custom_bin_is_refused() {
    let fixture = Fixture::new();
    let (app, user, bin) = app_with_custom_bin(&fixture).await;
    let root = bin.join("inside");

    let error = update_default_library_roots(
        &app,
        &user,
        MediaFacet::Movie,
        vec![RootFolderEntry {
            path: root.to_string_lossy().to_string(),
            is_default: true,
        }],
    )
    .await
    .expect_err("a default-library root inside the bin is refused");
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
    crate::recycle_bin::recycle_file(
        &config,
        &media,
        removal_manifest(&media),
        crate::location::model::VerificationDepth::Full,
    )
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
    crate::recycle_bin::recycle_file(
        &config,
        &media,
        removal_manifest(&media),
        crate::location::model::VerificationDepth::Full,
    )
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
    let result = crate::recycle_bin::recycle_file(
        &config,
        &media,
        removal_manifest(&media),
        crate::location::model::VerificationDepth::Full,
    )
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

/// Three titles, each with an entry in the default bin, deleted as one job
/// whose middle item is refused because another deletion holds it. The job
/// reports the same outcomes, records the same titles, and after one
/// housekeeping pass leaves the bin in the same state as deleting the same
/// titles one at a time.
#[tokio::test]
async fn title_delete_bulk_matches_one_title_at_a_time() {
    struct Run {
        outcomes: Vec<Result<(), Option<&'static str>>>,
        recorded: Vec<String>,
        bin: Vec<(String, Option<Vec<u8>>)>,
        remaining: Vec<bool>,
    }
    async fn scenario(bulk: bool) -> Run {
        let fixture = TitleDeleteFixture::new().await;
        let root_a = fixture.fixture.root_a();
        let middle_folder = root_a.join("Synthetic Middle Feature (2003)");
        std::fs::create_dir_all(&middle_folder).unwrap();
        let middle = create_movie_title_with_folder(
            &fixture.app,
            &fixture.user,
            "Synthetic Middle Feature",
            &middle_folder,
        )
        .await;
        let bin = root_a.join(".scryer-recycle");
        fixture.seed_bin(&bin).await;
        let config = crate::recycle_bin::RecycleBinConfig {
            enabled: true,
            base_path: bin.clone(),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![root_a.clone()],
        };
        let source = root_a.join("synthetic-middle-feature.mkv");
        std::fs::write(&source, "synthetic middle content").unwrap();
        let mut manifest = removal_manifest(&source);
        manifest.title_id = Some(middle.id.clone());
        crate::recycle_bin::recycle_file(
            &config,
            &source,
            manifest,
            crate::location::model::VerificationDepth::Full,
        )
        .await
        .unwrap()
        .unwrap();

        let jobs = Arc::new(RecordingJobRunRepo::default());
        let app = fixture
            .with_bin_setting(None)
            .with_test_overrides(|services| services.with_job_runs(jobs.clone()));
        let titles = [
            fixture.deleted.id.clone(),
            middle.id.clone(),
            fixture.kept.id.clone(),
        ];
        let _held = app
            .runtime
            .jobs
            .interactive_operation_guards
            .try_acquire(&format!("title-delete:{}", middle.id))
            .await
            .unwrap();
        let mut outcomes = Vec::new();
        if bulk {
            let accepted = app
                .start_delete_titles_job(
                    &fixture.user,
                    DeleteTitlesJobRequest {
                        items: titles
                            .iter()
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
            let run = crate::test_wait::wait_for("the deletion job finishes", || async {
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
            let succeeded = summary["succeededTitleIds"].as_array().unwrap();
            for id in &titles {
                match succeeded.iter().any(|succeeded| succeeded == id.as_str()) {
                    true => outcomes.push(Ok(())),
                    false => {
                        let failure = summary["failures"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|failure| failure["titleId"] == id.as_str())
                            .unwrap();
                        assert_eq!(
                            failure["reason"],
                            crate::title_delete::IN_PROGRESS,
                            "the held title is refused with its reason"
                        );
                        outcomes.push(Err(Some(crate::title_delete::IN_PROGRESS)));
                    }
                }
            }
        } else {
            for id in &titles {
                outcomes.push(
                    app.delete_title(&fixture.user, id, false, None)
                        .await
                        .map_err(|error| error.validation_reason()),
                );
            }
        }
        let recorded = recorded_recycle_purges(&app).await;
        app.purge_recycle_bins_for_housekeeping().await;
        let mut remaining = Vec::new();
        for id in &titles {
            remaining.push(
                app.services
                    .catalog
                    .titles
                    .get_by_id(id)
                    .await
                    .unwrap()
                    .is_some(),
            );
        }
        // Entry directory names are generated, so compare the bin by what each
        // entry holds rather than by its name.
        let mut bin_state = snapshot_tree(&bin)
            .into_iter()
            .filter_map(|(path, bytes)| {
                let name = path.file_name()?.to_string_lossy().to_string();
                (name != "manifest.json").then_some((name, bytes))
            })
            .filter(|(name, bytes)| bytes.is_some() || !name.starts_with("20"))
            .collect::<Vec<_>>();
        bin_state.sort();
        Run {
            outcomes,
            recorded,
            bin: bin_state,
            remaining,
        }
    }

    let bulk = scenario(true).await;
    let single = scenario(false).await;
    assert_eq!(bulk.outcomes, single.outcomes);
    assert_eq!(bulk.recorded.len(), 2);
    assert_eq!(bulk.recorded.len(), single.recorded.len());
    assert_eq!(bulk.bin, single.bin);
    assert_eq!(bulk.remaining, single.remaining);
    assert_eq!(bulk.remaining, vec![false, true, false]);
    assert!(bulk.outcomes[0].is_ok());
    assert!(bulk.outcomes[2].is_ok());
    assert!(
        !bulk
            .bin
            .iter()
            .any(|(name, _)| name == "synthetic-feature.mkv"
                || name == "synthetic-other-feature.mkv"),
        "the deleted titles' entries are purged by the pass"
    );
    assert!(
        bulk.bin
            .iter()
            .any(|(name, _)| name == "synthetic-middle-feature.mkv"),
        "the refused title's entry stays"
    );
}

/// Entries that reach the bin while a deletion chunk deletes its titles are
/// purged by the next housekeeping pass, which lists the bin once for every
/// deleted title. An entry of a title outside the job, in the same bin, stays.
#[tokio::test]
async fn housekeeping_lists_each_bin_once_for_entries_of_deleted_titles() {
    crate::test_wait::within_deadline("late recycle entries are swept once per bin", async {
        let fixture = TitleDeleteFixture::new().await;
        let root_a = fixture.fixture.root_a();
        let third_folder = root_a.join("Synthetic Third Feature (2004)");
        let bystander_folder = root_a.join("Synthetic Bystander Feature (2005)");
        std::fs::create_dir_all(&third_folder).unwrap();
        std::fs::create_dir_all(&bystander_folder).unwrap();
        let third = create_movie_title_with_folder(
            &fixture.app,
            &fixture.user,
            "Synthetic Third Feature",
            &third_folder,
        )
        .await;
        let bystander = create_movie_title_with_folder(
            &fixture.app,
            &fixture.user,
            "Synthetic Bystander Feature",
            &bystander_folder,
        )
        .await;
        let titles = Arc::new(MockTitleRepo::default());
        for title in [&fixture.deleted, &fixture.kept, &third, &bystander] {
            titles.create(title.clone()).await.unwrap();
        }
        let operations = Arc::new(Mutex::new(Vec::new()));
        titles.set_delete_operation_log(operations.clone()).await;
        let barrier = operations.lock().await;
        let jobs = Arc::new(RecordingJobRunRepo::default());
        let mut app = fixture
            .with_bin_setting(None)
            .with_test_overrides(|services| services.with_job_runs(jobs.clone()));
        app.services.catalog.titles = titles.clone();
        let deleted_ids = [&fixture.deleted.id, &fixture.kept.id, &third.id];
        let accepted = app
            .start_delete_titles_job(
                &fixture.user,
                DeleteTitlesJobRequest {
                    items: deleted_ids
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
        crate::test_wait::wait_until("the chunk reaches catalog deletion", || async {
            titles
                .delete_started
                .lock()
                .await
                .contains(&fixture.deleted.id)
        })
        .await;
        let config = crate::recycle_bin::RecycleBinConfig {
            enabled: true,
            base_path: root_a.join(".scryer-recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![root_a.clone()],
        };
        let mut recycled = Vec::new();
        for (title, name) in [
            (&fixture.kept, "late-synthetic-other"),
            (&third, "late-synthetic-third"),
            (&bystander, "late-synthetic-bystander"),
        ] {
            let source = root_a.join(format!("{name}.mkv"));
            std::fs::write(&source, format!("{name} bytes")).unwrap();
            let mut manifest = removal_manifest(&source);
            manifest.title_id = Some(title.id.clone());
            recycled.push(
                crate::recycle_bin::recycle_file(
                    &config,
                    &source,
                    manifest,
                    crate::location::model::VerificationDepth::Full,
                )
                .await
                .unwrap()
                .unwrap()
                .recycled_path,
            );
        }
        drop(barrier);
        let run = crate::test_wait::wait_for("the deletion job finishes", || async {
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
        assert_eq!(summary["succeededTitleIds"], serde_json::json!(deleted_ids));
        assert!(summary.get("recycleOutcomes").is_none());
        assert!(recycled.iter().all(|path| path.exists()));
        let bin_calls_before = crate::recycle_bin::test_bin_calls::count(&config.base_path);
        assert_eq!(
            bin_calls_before, 0,
            "the deletion job never touched the bin"
        );
        assert_eq!(app.purge_recycle_bins_for_housekeeping().await, 2);
        assert_eq!(
            crate::recycle_bin::test_bin_calls::count_kind(
                &config.base_path,
                crate::recycle_bin::test_bin_calls::LIST
            ),
            1,
            "one listing of the bin for every deleted title"
        );
        assert!(!recycled[0].exists());
        assert!(!recycled[1].exists());
        assert_eq!(
            std::fs::read(&recycled[2]).unwrap(),
            b"late-synthetic-bystander bytes"
        );
    })
    .await;
}

// ── Title deletion leaves recycle cleanup to housekeeping ───────────────────

/// Recycle `name` into `bin` as an entry of `title_id`, from a file under
/// `root`, and return the entry directory.
async fn seed_entry(
    bin: &Path,
    root: &Path,
    title_id: &str,
    name: &str,
    adjust: impl FnOnce(&mut crate::recycle_bin::RecycleManifest),
) -> PathBuf {
    let config = crate::recycle_bin::RecycleBinConfig {
        enabled: true,
        base_path: bin.to_path_buf(),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![root.to_path_buf()],
    };
    std::fs::create_dir_all(root).unwrap();
    let source = root.join(format!("{name}.mkv"));
    std::fs::write(&source, format!("{name} bytes")).unwrap();
    let mut manifest = removal_manifest(&source);
    manifest.title_id = Some(title_id.to_string());
    adjust(&mut manifest);
    crate::recycle_bin::recycle_file(
        &config,
        &source,
        manifest,
        crate::location::model::VerificationDepth::Full,
    )
    .await
    .unwrap()
    .unwrap()
    .recycled_path
    .parent()
    .unwrap()
    .to_path_buf()
}

/// What each bin holds, by payload rather than by generated entry name: the
/// recycled file's name and bytes and the entry's status, plus loose files.
fn bin_contents(bin: &Path) -> Vec<(String, Option<Vec<u8>>, Option<String>)> {
    let mut contents = Vec::new();
    let Ok(read) = std::fs::read_dir(bin) else {
        return contents;
    };
    for entry in read {
        let path = entry.unwrap().path();
        if !path.is_dir() {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            contents.push((name, std::fs::read(&path).ok(), None));
            continue;
        }
        let manifest: crate::recycle_bin::RecycleManifest =
            serde_json::from_slice(&std::fs::read(path.join("manifest.json")).unwrap()).unwrap();
        let payload = snapshot_tree(&path)
            .into_iter()
            .filter(|(file, bytes)| bytes.is_some() && !file.ends_with("manifest.json"))
            .collect::<Vec<_>>();
        for (file, bytes) in payload {
            contents.push((
                file.file_name().unwrap().to_string_lossy().to_string(),
                bytes,
                manifest.status.clone(),
            ));
        }
    }
    contents.sort();
    contents
}

/// One housekeeping pass after a title delete purges exactly the deleted
/// title's free entries in its own library's bins. The bins hold, for the
/// deleted title, a plain removal, an upgrade whose replacement row is gone,
/// and an entry held busy by another operation; an entry of a surviving
/// title; an entry of a title that is missing from the catalog but was never
/// recorded; an entry of the deleted title in another library's bin; and one
/// in a directory no pass covers. Everything but the first two is left as it
/// was, and the busy entry keeps the title recorded until a later pass.
#[tokio::test]
async fn housekeeping_purges_only_a_deleted_titles_free_entries_in_its_librarys_bins() {
    let fixture = TitleDeleteFixture::new().await;
    let root_a = fixture.fixture.root_a();
    let root_b = fixture.fixture.root_b();
    let bin_a = root_a.join(".scryer-recycle");
    let bin_b = root_b.join(".scryer-recycle");
    let stray = fixture.fixture.temp.path().join("stray-bin");
    let deleted = fixture.deleted.id.clone();
    seed_entry(&bin_a, &root_a, &deleted, "emberfall-removed", |_| {}).await;
    seed_entry(
        &bin_a,
        &root_a,
        &deleted,
        "emberfall-upgraded",
        |manifest| {
            manifest.reason = "upgrade_replaced".to_string();
            manifest.original_file_id = Some("synthetic-emberfall-original".to_string());
            manifest.media_root = Some(root_a.to_string_lossy().to_string());
            manifest.replacement_file_id = Some("synthetic-emberfall-gone".to_string());
            manifest.replacement_path = Some(
                root_a
                    .join("emberfall-replacement.mkv")
                    .to_string_lossy()
                    .to_string(),
            );
        },
    )
    .await;
    let busy = seed_entry(&bin_a, &root_a, &deleted, "emberfall-busy", |_| {}).await;
    seed_entry(&bin_a, &root_a, &fixture.kept.id, "cinderwake-kept", |_| {}).await;
    seed_entry(
        &bin_a,
        &root_a,
        "synthetic-ashgrove-missing",
        "ashgrove-missing",
        |_| {},
    )
    .await;
    seed_entry(&bin_b, &root_b, &deleted, "emberfall-other-library", |_| {}).await;
    seed_entry(&stray, &root_a, &deleted, "emberfall-stray", |_| {}).await;
    let bins = [&bin_a, &bin_b, &stray];
    let before = bins.map(|bin| bin_contents(bin));

    let app = fixture.with_bin_setting(None);
    let busy_id = busy.file_name().unwrap().to_str().unwrap().to_string();
    let guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .try_acquire(&format!("recycle-entry:{busy_id}"))
        .await
        .unwrap();
    app.delete_title(&fixture.user, &deleted, false, None)
        .await
        .unwrap();
    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&deleted)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        bins.map(|bin| bin_contents(bin)),
        before,
        "deletion leaves every bin to housekeeping"
    );

    assert_eq!(app.purge_recycle_bins_for_housekeeping().await, 2);
    let mut expected = before.clone();
    expected[0]
        .retain(|(name, _, _)| name != "emberfall-removed.mkv" && name != "emberfall-upgraded.mkv");
    assert_eq!(bins.map(|bin| bin_contents(bin)), expected);
    assert_eq!(
        recorded_recycle_purges(&app).await,
        vec![deleted.clone()],
        "the busy entry keeps its title recorded"
    );

    drop(guard);
    assert_eq!(app.purge_recycle_bins_for_housekeeping().await, 1);
    assert!(
        !busy.exists(),
        "a later pass purges the entry once it is free"
    );
    assert!(recorded_recycle_purges(&app).await.is_empty());
}

/// Deleting titles, one at a time or as a job, makes no call on any recycle
/// bin: nothing is listed, looked up or purged, and the bins are unchanged.
#[tokio::test]
async fn title_delete_makes_no_recycle_bin_calls() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    let (deleted_entry, kept_entry) = fixture.seed_bin(&bin).await;
    let before = snapshot_tree(&bin);
    let jobs = Arc::new(RecordingJobRunRepo::default());
    let app = fixture
        .with_bin_setting(None)
        .with_test_overrides(|services| services.with_job_runs(jobs.clone()));

    app.delete_title(&fixture.user, &fixture.deleted.id, false, None)
        .await
        .unwrap();
    let accepted = app
        .start_delete_titles_job(
            &fixture.user,
            DeleteTitlesJobRequest {
                items: vec![DeleteTitlesJobItem {
                    title_id: fixture.kept.id.clone(),
                    preview_fingerprint: None,
                }],
                delete_files_on_disk: false,
                typed_confirmation: None,
            },
        )
        .await
        .unwrap();
    let run = crate::test_wait::wait_for("the deletion job finishes", || async {
        jobs.runs
            .lock()
            .await
            .iter()
            .find(|run| run.id == accepted.job_run.id && run.status.is_terminal())
            .cloned()
    })
    .await;
    assert_eq!(run.status, JobRunStatus::Completed);

    assert_eq!(crate::recycle_bin::test_bin_calls::count(&bin), 0);
    assert_eq!(snapshot_tree(&bin), before);
    assert!(deleted_entry.exists() && kept_entry.exists());
    let mut recorded = recorded_recycle_purges(&app).await;
    recorded.sort();
    let mut expected = vec![fixture.deleted.id.clone(), fixture.kept.id.clone()];
    expected.sort();
    assert_eq!(recorded, expected);
}

/// A pass with nothing recorded reads the records once and writes nothing; a
/// pass that finishes recorded titles clears them in one write.
#[tokio::test]
async fn housekeeping_with_no_recorded_deletions_writes_nothing() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    let (deleted_entry, _) = fixture.seed_bin(&bin).await;
    let before = snapshot_tree(&bin);
    let titles = Arc::new(MockTitleRepo::default());
    titles.create(fixture.deleted.clone()).await.unwrap();
    titles.create(fixture.kept.clone()).await.unwrap();
    let mut app = fixture.with_bin_setting(None);
    app.services.catalog.titles = titles.clone();

    // History kept forever: the records are read once.
    assert_eq!(app.purge_recycle_bins_with_record_retention(None).await, 0);
    assert_eq!(titles.recycle_purge_reads.load(Ordering::SeqCst), 1);
    assert_eq!(titles.recycle_purge_writes.load(Ordering::SeqCst), 0);
    // With a retention period, the expired-record probe adds one read.
    assert_eq!(
        app.purge_recycle_bins_with_record_retention(Some(30)).await,
        0
    );
    assert_eq!(titles.recycle_purge_reads.load(Ordering::SeqCst), 3);
    assert_eq!(titles.recycle_purge_writes.load(Ordering::SeqCst), 0);
    assert_eq!(snapshot_tree(&bin), before);

    app.delete_title(&fixture.user, &fixture.deleted.id, false, None)
        .await
        .unwrap();
    assert_eq!(
        app.purge_recycle_bins_with_record_retention(Some(30)).await,
        1
    );
    assert!(!deleted_entry.exists());
    // The young record is not dropped: the probe reads, the finish writes.
    assert_eq!(titles.recycle_purge_reads.load(Ordering::SeqCst), 5);
    assert_eq!(titles.recycle_purge_writes.load(Ordering::SeqCst), 1);
    assert!(titles.recycle_purges.lock().await.is_empty());
}

/// A deleted title whose quarantined entry keeps it recorded, deleted at a
/// fixed instant on the injected clock.
async fn quarantined_deleted_title(
    fixture: &TitleDeleteFixture,
) -> (AppUseCase, PathBuf, chrono::DateTime<chrono::Utc>) {
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    let (deleted, _) = fixture.seed_bin(&bin).await;
    rewrite_entry(
        &deleted,
        "upgrade_replaced",
        crate::recycle_bin::RECYCLE_STATUS_QUARANTINED,
    );
    let app = fixture.with_bin_setting(None);
    let deleted_at = chrono::DateTime::parse_from_rfc3339("2026-03-01T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(deleted_at));
    app.delete_title(&fixture.user, &fixture.deleted.id, false, None)
        .await
        .unwrap();
    (app, deleted, deleted_at)
}

/// A record older than the history retention period is dropped, whatever
/// entries remain; the drop deletes nothing, and the entries stay in the bin
/// for the bin's own retention. A younger record stays.
///
/// The title keeps a quarantined entry, which the recycle page never lists,
/// and a committed entry another operation holds, which it does list.
#[tokio::test]
async fn housekeeping_drops_a_recycle_cleanup_record_older_than_history_retention() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    let held = seed_entry(
        &bin,
        &fixture.fixture.root_a(),
        &fixture.deleted.id,
        "emberfall-held",
        |_| {},
    )
    .await;
    let (app, quarantined, deleted_at) = quarantined_deleted_title(&fixture).await;
    let entry_id = |entry: &Path| entry.file_name().unwrap().to_str().unwrap().to_string();
    let (quarantined_id, held_id) = (entry_id(&quarantined), entry_id(&held));
    let listed = |app: AppUseCase| {
        let user = fixture.user.clone();
        async move {
            let mut ids = app
                .list_recycled_items(&user, None)
                .await
                .unwrap()
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>();
            ids.sort();
            ids
        }
    };
    let listed_before = listed(app.clone()).await;
    assert!(listed_before.contains(&held_id));
    assert!(!listed_before.contains(&quarantined_id));
    let guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .try_acquire(&format!("recycle-entry:{held_id}"))
        .await
        .unwrap();
    let before = snapshot_tree(&bin);

    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(deleted_at + chrono::Duration::days(29)));
    assert_eq!(
        app.purge_recycle_bins_with_record_retention(Some(30)).await,
        0
    );
    assert_eq!(
        recorded_recycle_purges(&app).await,
        vec![fixture.deleted.id.clone()],
        "a record younger than the retention period stays"
    );

    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(deleted_at + chrono::Duration::days(31)));
    assert_eq!(
        app.purge_recycle_bins_with_record_retention(Some(30)).await,
        0
    );
    assert!(
        recorded_recycle_purges(&app).await.is_empty(),
        "an expired record is dropped"
    );
    assert_eq!(
        snapshot_tree(&bin),
        before,
        "the drop changes nothing on disk"
    );
    assert_eq!(
        listed(app.clone()).await,
        listed_before,
        "nor what is listed"
    );

    // Released, the held entry is left to the bin's own retention: it was
    // recycled moments ago, so the next pass keeps it.
    drop(guard);
    assert_eq!(
        app.purge_recycle_bins_with_record_retention(Some(30)).await,
        0
    );
    assert!(held.exists() && quarantined.exists());
    assert_eq!(listed(app.clone()).await, listed_before);
}

/// With history kept forever, no record is dropped for its age.
#[tokio::test]
async fn housekeeping_keeps_recycle_cleanup_records_when_history_is_kept_forever() {
    let fixture = TitleDeleteFixture::new().await;
    let (app, deleted, deleted_at) = quarantined_deleted_title(&fixture).await;
    let mut settings = app.general_settings().await.unwrap();
    settings.keep_history_forever = true;
    assert_eq!(
        crate::jobs::housekeeping::recycle_purge_record_retention_days(&settings),
        None
    );
    settings.keep_history_forever = false;
    settings.history_retention_days = 30;
    assert_eq!(
        crate::jobs::housekeeping::recycle_purge_record_retention_days(&settings),
        Some(30)
    );

    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(deleted_at + chrono::Duration::days(3650)));
    assert_eq!(app.purge_recycle_bins_with_record_retention(None).await, 0);
    assert_eq!(
        recorded_recycle_purges(&app).await,
        vec![fixture.deleted.id.clone()]
    );
    assert!(deleted.exists());
}

/// Until housekeeping runs, an entry of a deleted title stays on the recycle
/// bin page without a title name and can be restored. The pass that follows
/// finds nothing left and clears the record.
#[tokio::test]
async fn a_deleted_titles_entry_stays_listed_and_restorable_until_housekeeping() {
    let fixture = TitleDeleteFixture::new().await;
    let bin = fixture.fixture.root_a().join(".scryer-recycle");
    let (deleted_entry, _) = fixture.seed_bin(&bin).await;
    let app = fixture.with_bin_setting(None);
    let entry_id = deleted_entry
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    app.delete_title(&fixture.user, &fixture.deleted.id, false, None)
        .await
        .unwrap();

    let items = app.list_recycled_items(&fixture.user, None).await.unwrap();
    let item = items
        .iter()
        .find(|item| item.id == entry_id)
        .expect("the entry is still listed");
    assert_eq!(item.title_id.as_deref(), Some(fixture.deleted.id.as_str()));
    assert_eq!(item.title_name, None);
    assert_eq!(item.library_id, fixture.deleted.library_id);

    let original = PathBuf::from(&item.original_path);
    assert!(!original.exists());
    assert!(
        app.restore_recycled_item(&fixture.user, &entry_id)
            .await
            .unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(&original).unwrap(),
        "synthetic-feature content"
    );

    assert_eq!(app.purge_recycle_bins_for_housekeeping().await, 0);
    assert!(recorded_recycle_purges(&app).await.is_empty());
}

/// A recycled copy of a file marked bad for its title is not restored by
/// default. Confirming restores it and clears the title's bad-file entry.
#[tokio::test]
async fn restoring_a_file_marked_bad_needs_confirmation_and_clears_the_mark() {
    let fixture = TitleDeleteFixture::new().await;
    let root_a = fixture.fixture.root_a();
    let bin = root_a.join(".scryer-recycle");
    // The recycled file sits in the kept title's own folder, so the restore
    // passes the folder-ownership check and reaches the bad-file check.
    let original = root_a
        .join("Synthetic Other Feature (2002)")
        .join("synthetic-other-feature.mkv");
    std::fs::write(&original, "synthetic-other-feature content").unwrap();
    let mut manifest = removal_manifest(&original);
    manifest.title_id = Some(fixture.kept.id.clone());
    let recycled = crate::recycle_bin::recycle_file(
        &crate::recycle_bin::RecycleBinConfig {
            enabled: true,
            base_path: bin.clone(),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![root_a.clone()],
        },
        &original,
        manifest,
        crate::location::model::VerificationDepth::Full,
    )
    .await
    .unwrap()
    .expect("file recycled");
    let mut app = fixture.with_bin_setting(None);
    let blocklist = Arc::new(MockBlocklistRepo::default());
    app.services.workflow.blocklist_repo = blocklist.clone();
    let entry_id = recycled
        .recycled_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap()
        .to_string();

    let scratch = tempfile::tempdir().unwrap();
    let same_bytes = scratch.path().join("same-bytes.mkv");
    std::fs::write(&same_bytes, "synthetic-other-feature content").unwrap();
    let proof = crate::fs_integrity::import_content_proof_async(&same_bytes)
        .await
        .unwrap();
    blocklist.entries.lock().await.push(BlocklistEntry {
        id: "bad-file-entry".to_string(),
        title_id: fixture.kept.id.clone(),
        release_name: String::new(),
        normalized_release_name: String::new(),
        indexer_id: String::new(),
        info_hash: None,
        reason: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        content: Some(crate::media_issues::content_identity(&proof)),
    });

    let preview = app
        .preview_restore_recycled_items(&fixture.user, vec![entry_id.clone()])
        .await
        .unwrap();
    assert_eq!(
        preview.items[0].bad_file_entry_id.as_deref(),
        Some("bad-file-entry")
    );

    let refusal = app
        .restore_recycled_item(&fixture.user, &entry_id)
        .await
        .expect_err("a bad file is not restored without confirmation");
    let AppError::ValidationRefused { reason, facts, .. } = &refusal else {
        panic!("expected a coded refusal, got {refusal:?}");
    };
    assert_eq!(
        *reason,
        crate::error_reasons::media::RESTORE_MATCHES_BAD_FILE
    );
    assert_eq!(
        facts,
        &vec![crate::RefusalFact::new(
            crate::media_issues::BAD_FILE_ENTRY_ID_FACT,
            "bad-file-entry"
        )]
    );
    assert!(!original.exists());
    assert_eq!(blocklist.entries.lock().await.len(), 1);

    assert!(
        app.restore_recycled_item_clearing_bad_file(&fixture.user, &entry_id, true)
            .await
            .unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(&original).unwrap(),
        "synthetic-other-feature content"
    );
    assert!(blocklist.entries.lock().await.is_empty());
}

/// A reported file's library path, its copy left in the download folder it
/// was imported from, and unrelated files beside each.
struct DownloadCopyFixture {
    title: TitleDeleteFixture,
    library_file: PathBuf,
    library_neighbour: PathBuf,
    download_dir: PathBuf,
    copy: PathBuf,
    download_neighbours: Vec<PathBuf>,
    issue: scryer_domain::MediaIssue,
    artifacts: Arc<RecordingImportArtifactRepo>,
    imports: Arc<TrackingImportRepo>,
    blocklist: Arc<MockBlocklistRepo>,
}

impl DownloadCopyFixture {
    async fn new() -> Self {
        let title = TitleDeleteFixture::new().await;
        let folder = title
            .fixture
            .root_a()
            .join("Synthetic Other Feature (2002)");
        let library_file = folder.join("Synthetic Other Feature (2002).mkv");
        let library_neighbour = folder.join("Synthetic Other Feature (2002).nfo");
        std::fs::write(&library_file, "synthetic reported content").unwrap();
        std::fs::write(&library_neighbour, "synthetic library neighbour").unwrap();
        let download_dir = title
            .fixture
            .temp
            .path()
            .join("downloads")
            .join("Synthetic.Other.Feature.2002");
        std::fs::create_dir_all(download_dir.join("extras")).unwrap();
        let copy = download_dir.join("Synthetic.Other.Feature.2002.mkv");
        std::fs::write(&copy, "synthetic reported content").unwrap();
        let download_neighbours = vec![
            download_dir.join("Synthetic.Other.Feature.2002.nfo"),
            download_dir.join("extras").join("synthetic-sample.mkv"),
        ];
        std::fs::write(&download_neighbours[0], "synthetic download neighbour").unwrap();
        std::fs::write(&download_neighbours[1], "synthetic other content").unwrap();
        let proof = crate::fs_integrity::import_content_proof_async(&library_file)
            .await
            .unwrap();
        let (version, _) = crate::location::resolution::regular_file_snapshot(&library_file)
            .await
            .unwrap();
        let now = Utc::now();
        let issue = scryer_domain::MediaIssue {
            id: "synthetic-report".to_string(),
            library_id: title.kept.library_id.clone(),
            title_id: title.kept.id.clone(),
            media_file_id: "synthetic-media-file".to_string(),
            file_path_snapshot: library_file.to_string_lossy().to_string(),
            file_version: version,
            content: crate::media_issues::content_identity(&proof),
            reporter_user_id: title.user.id.clone(),
            reason: scryer_domain::MediaIssueReason::Corrupted,
            details: None,
            remediation: scryer_domain::MediaIssueRemediation::RecycleAndSearch,
            authorized_remediation: Some(scryer_domain::MediaIssueRemediation::RecycleAndSearch),
            status: scryer_domain::MediaIssueStatus::Approved,
            affected_episode_ids: Vec::new(),
            affected_series_movie_link_ids: Vec::new(),
            rule_decision_id: None,
            blocklist_id: None,
            reviewed_by_user_id: Some(title.user.id.clone()),
            reviewed_at: Some(now.to_rfc3339()),
            execution_status: scryer_domain::MediaIssueExecutionStatus::Recycling,
            execution_error_code: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
        };
        let artifacts = Arc::new(RecordingImportArtifactRepo::default());
        artifacts
            .artifacts
            .lock()
            .await
            .push(crate::ImportArtifact {
                id: "synthetic-artifact".to_string(),
                source_client_id: None,
                source_system: "synthetic-client".to_string(),
                source_ref: "synthetic-item".to_string(),
                import_id: Some("synthetic-import".to_string()),
                relative_path: Some("Synthetic.Other.Feature.2002.mkv".to_string()),
                workspace_relative_path: None,
                normalized_file_name: "synthetic.other.feature.2002.mkv".to_string(),
                media_kind: "movie".to_string(),
                title_id: Some(title.kept.id.clone()),
                episode_id: None,
                season_number: None,
                episode_number: None,
                result: "imported".to_string(),
                reason_code: None,
                imported_media_file_id: Some(issue.media_file_id.clone()),
                created_at: now,
            });
        let completed = scryer_domain::CompletedDownload {
            subtitle_paths: Vec::new(),
            client_type: "synthetic-client".to_string(),
            client_id: String::new(),
            download_client_item_id: "synthetic-item".to_string(),
            download_id: None,
            name: "Synthetic.Other.Feature.2002".to_string(),
            release_name: None,
            dest_dir: download_dir.to_string_lossy().to_string(),
            category: None,
            size_bytes: None,
            completed_at: None,
            parameters: vec![],
            posted_file_names: Vec::new(),
        };
        let imports = Arc::new(TrackingImportRepo::default());
        imports.records.lock().await.push(ImportRecord {
            id: "synthetic-import".to_string(),
            source_client_id: None,
            source_system: "synthetic-client".to_string(),
            source_ref: "synthetic-item".to_string(),
            import_type: scryer_domain::ImportType::MovieDownload,
            status: scryer_domain::ImportStatus::Completed,
            payload_json: serde_json::to_string(&completed).unwrap(),
            result_json: None,
            download_id: None,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            started_at: None,
            finished_at: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
        });
        Self {
            title,
            library_file,
            library_neighbour,
            download_dir,
            copy,
            download_neighbours,
            issue,
            artifacts,
            imports,
            blocklist: Arc::new(MockBlocklistRepo::default()),
        }
    }

    fn app(&self, bin: Option<&Path>) -> AppUseCase {
        let mut app = self.title.with_bin_setting(bin);
        app.services.workflow.import_artifacts = self.artifacts.clone();
        app.services.workflow.imports = self.imports.clone();
        app.services.workflow.blocklist_repo = self.blocklist.clone();
        app
    }

    fn assert_neighbours_kept(&self) {
        assert_eq!(
            std::fs::read_to_string(&self.library_neighbour).unwrap(),
            "synthetic library neighbour"
        );
        assert_eq!(
            std::fs::read_to_string(&self.download_neighbours[0]).unwrap(),
            "synthetic download neighbour"
        );
        assert_eq!(
            std::fs::read_to_string(&self.download_neighbours[1]).unwrap(),
            "synthetic other content"
        );
    }

    /// Make the download copy the library file's second name, and return
    /// the report as it would be filed against the linked file.
    #[cfg(unix)]
    async fn link_copy(&self) -> scryer_domain::MediaIssue {
        std::fs::remove_file(&self.copy).unwrap();
        std::fs::hard_link(&self.library_file, &self.copy).unwrap();
        let (version, _) = crate::location::resolution::regular_file_snapshot(&self.library_file)
            .await
            .unwrap();
        let mut issue = self.issue.clone();
        issue.file_version = version;
        issue
    }

    fn bin(&self) -> PathBuf {
        self.title.fixture.root_a().join(".scryer-recycle")
    }

    /// How many files in the bin hold the reported content.
    fn recycled_copies(&self) -> usize {
        snapshot_tree(&self.bin())
            .into_iter()
            .filter(|(path, bytes)| {
                bytes.as_deref() == Some(b"synthetic reported content".as_slice())
                    && path.extension().is_some_and(|ext| ext == "mkv")
            })
            .count()
    }
}

/// The receipt of a reported file's recycle when only a separate copy
/// follows it; the separate copy's step does not read it.
fn separate_copy_receipt(
    issue: &scryer_domain::MediaIssue,
) -> crate::recycle_bin::MediaIssueRecycleReceipt {
    crate::recycle_bin::MediaIssueRecycleReceipt {
        file: crate::recycle_bin::PreparedRecycleFile {
            entry_id: "synthetic-entry".to_string(),
            original_path: issue.file_path_snapshot.clone(),
            recycled_path: issue.file_path_snapshot.clone(),
            source_operation_id: issue.id.clone(),
        },
        linked_copy: None,
    }
}

/// Run the reported file's own step of a confirmed recycle: reserve its
/// entry, save the receipt and move it, allowing `linked_copy` as its other
/// name.
async fn recycle_reported_name(
    app: &AppUseCase,
    issue: &scryer_domain::MediaIssue,
    linked_copy: Option<&Path>,
) -> AppResult<crate::recycle_bin::MediaIssueRecycleReceipt> {
    let (config, root) = app.media_issue_bin_config(issue).await?;
    let path = PathBuf::from(&issue.file_path_snapshot);
    let mut manifest = crate::recycle_bin::RecycleManifest::pending_upgrade(
        issue.file_path_snapshot.clone(),
        issue.media_file_id.clone(),
        issue.content.size_bytes,
        issue.title_id.clone(),
        Some(root.to_string_lossy().to_string()),
    );
    manifest.source_operation_id = Some(issue.id.clone());
    let prepared = crate::recycle_bin::prepare_conflict_recycle(&config, &path, manifest).await?;
    let receipt = crate::recycle_bin::MediaIssueRecycleReceipt {
        file: prepared,
        linked_copy: None,
    };
    crate::recycle_bin::execute_new_conflict_recycle(
        &config,
        &path,
        &receipt.file,
        &issue.file_version,
        linked_copy,
    )
    .await?;
    Ok(receipt)
}

fn separate(version: &str) -> crate::media_issues::ConfirmedDownloadCopy<'_> {
    crate::media_issues::ConfirmedDownloadCopy {
        version,
        linked: false,
    }
}

fn linked(version: &str) -> crate::media_issues::ConfirmedDownloadCopy<'_> {
    crate::media_issues::ConfirmedDownloadCopy {
        version,
        linked: true,
    }
}

/// The download copy is found only at the path the import recorded, with the
/// reported content, and the confirmed recycle moves exactly that file into
/// the reported file's bin.
#[tokio::test]
async fn a_confirmed_recycle_moves_the_download_copy_and_keeps_its_neighbours() {
    let fixture = DownloadCopyFixture::new().await;
    let app = fixture.app(None);

    let copy = app
        .media_issue_download_copy(&fixture.issue, None)
        .await
        .unwrap()
        .expect("the copy in the download folder is found");
    assert_eq!(copy.path, fixture.copy);
    assert_eq!(copy.download_dir, fixture.download_dir);
    assert!(!copy.linked);

    app.recycle_media_issue_download_copy(
        &fixture.issue,
        separate(&copy.version),
        &separate_copy_receipt(&fixture.issue),
    )
    .await
    .unwrap();

    assert!(!fixture.copy.exists());
    assert_eq!(
        std::fs::read_to_string(&fixture.library_file).unwrap(),
        "synthetic reported content"
    );
    fixture.assert_neighbours_kept();
    assert_eq!(fixture.recycled_copies(), 1);
}

/// A copy that changed after the preview, or whose content is not the
/// reported content, is left where it is.
#[tokio::test]
async fn a_changed_or_different_download_copy_is_left_in_place() {
    let fixture = DownloadCopyFixture::new().await;
    let app = fixture.app(None);

    app.recycle_media_issue_download_copy(
        &fixture.issue,
        separate("synthetic-stale-version"),
        &separate_copy_receipt(&fixture.issue),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&fixture.copy).unwrap(),
        "synthetic reported content"
    );

    std::fs::write(&fixture.copy, "synthetic replaced content").unwrap();
    assert!(
        app.media_issue_download_copy(&fixture.issue, None)
            .await
            .unwrap()
            .is_none()
    );
    fixture.assert_neighbours_kept();
}

/// A recorded relative path that climbs out of the download folder names
/// nothing, even when a file with the reported content sits there.
#[tokio::test]
async fn a_recorded_path_outside_the_download_folder_names_no_copy() {
    let fixture = DownloadCopyFixture::new().await;
    let outside = fixture.download_dir.parent().unwrap().join("outside.mkv");
    std::fs::write(&outside, "synthetic reported content").unwrap();
    fixture.artifacts.artifacts.lock().await[0].relative_path = Some("../outside.mkv".to_string());
    let app = fixture.app(None);

    assert!(
        app.media_issue_download_copy(&fixture.issue, None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(outside.exists());
}

/// When the bin refuses the copy, the recycle fails and the copy stays.
#[tokio::test]
async fn a_refused_download_copy_recycle_fails_and_keeps_the_copy() {
    let fixture = DownloadCopyFixture::new().await;
    // A custom bin inside a library root is refused.
    let unsafe_bin = fixture.title.fixture.root_b().join("synthetic-bin");
    let app = fixture.app(Some(&unsafe_bin));
    let copy = app
        .media_issue_download_copy(&fixture.issue, None)
        .await
        .unwrap()
        .expect("copy found");

    app.recycle_media_issue_download_copy(
        &fixture.issue,
        separate(&copy.version),
        &separate_copy_receipt(&fixture.issue),
    )
    .await
    .expect_err("an unsafe bin refuses the copy");

    assert_eq!(
        std::fs::read_to_string(&fixture.copy).unwrap(),
        "synthetic reported content"
    );
    assert!(!unsafe_bin.exists());
    fixture.assert_neighbours_kept();
}

/// A download copy hard-linked to the reported file counts only as that
/// file's one other name, proven by its file number; with a third name it
/// counts as nothing.
#[cfg(unix)]
#[tokio::test]
async fn a_hard_linked_download_copy_counts_only_as_the_reported_files_other_name() {
    let fixture = DownloadCopyFixture::new().await;
    let issue = fixture.link_copy().await;
    let app = fixture.app(None);
    let (_, library_mark) =
        crate::location::resolution::regular_file_snapshot(&fixture.library_file)
            .await
            .unwrap();

    assert!(
        app.media_issue_download_copy(&issue, None)
            .await
            .unwrap()
            .is_none(),
        "a second name is never a separate copy"
    );
    let copy = app
        .media_issue_download_copy(&issue, Some(&library_mark))
        .await
        .unwrap()
        .expect("the linked copy is the reported file's other name");
    assert!(copy.linked);
    assert_eq!(copy.path, fixture.copy);

    // An unrelated file with the same content is not the other name.
    let unrelated = fixture.download_dir.join("synthetic-unrelated.mkv");
    std::fs::write(&unrelated, "synthetic reported content").unwrap();
    let (_, unrelated_mark) = crate::location::resolution::regular_file_snapshot(&unrelated)
        .await
        .unwrap();
    assert!(
        app.media_issue_download_copy(&issue, Some(&unrelated_mark))
            .await
            .unwrap()
            .is_none()
    );

    let third = fixture.download_dir.join("synthetic-third-name.mkv");
    std::fs::hard_link(&fixture.library_file, &third).unwrap();
    let (_, library_mark) =
        crate::location::resolution::regular_file_snapshot(&fixture.library_file)
            .await
            .unwrap();
    assert!(
        app.media_issue_download_copy(&issue, Some(&library_mark))
            .await
            .unwrap()
            .is_none()
    );
    assert!(fixture.copy.exists() && third.exists() && fixture.library_file.exists());
    fixture.assert_neighbours_kept();
}

/// With both names confirmed, the reported file and its linked download copy
/// each go to the bin, and nothing beside them moves.
#[cfg(unix)]
#[tokio::test]
async fn a_confirmed_linked_pair_recycles_both_names_and_keeps_neighbours() {
    let fixture = DownloadCopyFixture::new().await;
    let issue = fixture.link_copy().await;
    let app = fixture.app(None);
    let (_, library_mark) =
        crate::location::resolution::regular_file_snapshot(&fixture.library_file)
            .await
            .unwrap();
    let copy = app
        .media_issue_download_copy(&issue, Some(&library_mark))
        .await
        .unwrap()
        .expect("linked copy");

    let receipt = recycle_reported_name(&app, &issue, Some(&fixture.copy))
        .await
        .unwrap();
    assert!(!fixture.library_file.exists());
    assert!(
        fixture.copy.exists(),
        "the reported file's step moves one name"
    );

    app.recycle_media_issue_download_copy(&issue, linked(&copy.version), &receipt)
        .await
        .unwrap();

    assert!(!fixture.copy.exists());
    assert_eq!(fixture.recycled_copies(), 2);
    fixture.assert_neighbours_kept();
    let saved: crate::recycle_bin::MediaIssueRecycleReceipt = serde_json::from_str(
        &fixture
            .blocklist
            .get_media_issue_recycle_receipt(&issue.id)
            .await
            .unwrap()
            .expect("receipt saved"),
    )
    .unwrap();
    assert!(
        saved.linked_copy.is_some(),
        "the copy's entry was reserved first"
    );
}

/// Without a proven linked copy, a reported file with a second name is
/// moved back and both names stay: a third name, or no linked copy named.
#[cfg(unix)]
#[tokio::test]
async fn a_reported_file_with_an_unconfirmed_or_third_name_is_not_recycled() {
    let fixture = DownloadCopyFixture::new().await;
    let issue = fixture.link_copy().await;
    let app = fixture.app(None);

    recycle_reported_name(&app, &issue, None)
        .await
        .expect_err("a second name needs the confirmed linked copy");
    assert!(fixture.library_file.exists() && fixture.copy.exists());

    let third = fixture.download_dir.join("synthetic-third-name.mkv");
    std::fs::hard_link(&fixture.library_file, &third).unwrap();
    let (version, _) = crate::location::resolution::regular_file_snapshot(&fixture.library_file)
        .await
        .unwrap();
    let mut issue = issue;
    issue.file_version = version;
    recycle_reported_name(&app, &issue, Some(&fixture.copy))
        .await
        .expect_err("a third name is refused even with the linked copy named");
    assert!(fixture.library_file.exists() && fixture.copy.exists() && third.exists());
    assert_eq!(fixture.recycled_copies(), 0);
    fixture.assert_neighbours_kept();
}

/// A linked copy that changed after its preview, or gained another name, is
/// left in place.
#[cfg(unix)]
#[tokio::test]
async fn a_linked_copy_that_changed_after_its_preview_is_left_in_place() {
    let fixture = DownloadCopyFixture::new().await;
    let issue = fixture.link_copy().await;
    let app = fixture.app(None);
    let (_, library_mark) =
        crate::location::resolution::regular_file_snapshot(&fixture.library_file)
            .await
            .unwrap();
    let copy = app
        .media_issue_download_copy(&issue, Some(&library_mark))
        .await
        .unwrap()
        .expect("linked copy");
    let receipt = recycle_reported_name(&app, &issue, Some(&fixture.copy))
        .await
        .unwrap();

    app.recycle_media_issue_download_copy(&issue, linked("0:0:0:0:0:0:0"), &receipt)
        .await
        .unwrap();
    assert!(fixture.copy.exists(), "a stale version leaves the copy");

    let third = fixture.download_dir.join("synthetic-third-name.mkv");
    std::fs::hard_link(&fixture.copy, &third).unwrap();
    app.recycle_media_issue_download_copy(&issue, linked(&copy.version), &receipt)
        .await
        .unwrap();
    assert!(fixture.copy.exists() && third.exists());
    assert_eq!(fixture.recycled_copies(), 1);
    fixture.assert_neighbours_kept();
}

/// When the copy's step fails after the reported file was recycled, the
/// reported file waits in the bin under its receipt and the copy stays; the
/// retry recovers the reported file from the receipt and then moves the copy.
#[cfg(unix)]
#[tokio::test]
async fn a_failed_linked_copy_step_leaves_a_state_the_retry_finishes() {
    let fixture = DownloadCopyFixture::new().await;
    let issue = fixture.link_copy().await;
    let app = fixture.app(None);
    let (_, library_mark) =
        crate::location::resolution::regular_file_snapshot(&fixture.library_file)
            .await
            .unwrap();
    let copy = app
        .media_issue_download_copy(&issue, Some(&library_mark))
        .await
        .unwrap()
        .expect("linked copy");
    let receipt = recycle_reported_name(&app, &issue, Some(&fixture.copy))
        .await
        .unwrap();
    fixture
        .blocklist
        .save_media_issue_recycle_receipt(&issue.id, &serde_json::to_string(&receipt).unwrap())
        .await
        .unwrap();

    fixture
        .blocklist
        .fail_writes
        .store(true, std::sync::atomic::Ordering::SeqCst);
    app.recycle_media_issue_download_copy(&issue, linked(&copy.version), &receipt)
        .await
        .expect_err("the copy's receipt could not be saved, so it does not move");
    assert!(fixture.copy.exists());
    assert_eq!(fixture.recycled_copies(), 1);
    fixture
        .blocklist
        .fail_writes
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // The retry: the reported file is recovered from its receipt with the
    // linked copy as its proven other name, then the copy moves.
    let saved: crate::recycle_bin::MediaIssueRecycleReceipt = serde_json::from_str(
        &fixture
            .blocklist
            .get_media_issue_recycle_receipt(&issue.id)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(
        app.finish_recovered_media_issue_recycle(&issue, &saved, Some(linked(&copy.version)))
            .await
            .unwrap()
    );
    app.recycle_media_issue_download_copy(&issue, linked(&copy.version), &saved)
        .await
        .unwrap();
    assert!(!fixture.copy.exists() && !fixture.library_file.exists());
    assert_eq!(fixture.recycled_copies(), 2);
    fixture.assert_neighbours_kept();
}
