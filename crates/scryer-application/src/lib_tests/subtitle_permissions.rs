//! Guards and search behaviour for the library-scoped Manage Subtitles
//! permission.
//!
//! Every guard is exercised with the same four actors — a view-only user, a
//! Manage Subtitles holder, a Manage Titles holder (allowed only because Manage
//! Titles shadows Manage Subtitles), and a catalog administrator (allowed only
//! through the app-permission library override) — plus a cross-library case
//! where the grant is on one library and the file is in another.

use super::*;

use crate::subtitles::orchestration::SubtitleSearchStatus;
use crate::subtitles::provider::{SubtitleFile, SubtitleQuery};
use scryer_domain::{
    ExternalSubtitleSourceKind, LibraryPermissionMask, PluginHostBindingId, SubtitleBlocklistEntry,
    SubtitleDownload, SubtitleProviderConfig,
};

// ── Stub ports ────────────────────────────────────────────────────────────

/// In-memory subtitle download store: the null repository cannot hold the row
/// a delete or blocklist guard needs to resolve before it checks permission.
#[derive(Default)]
struct StubSubtitleDownloadRepo {
    rows: Mutex<Vec<SubtitleDownload>>,
    blocklist: Mutex<Vec<SubtitleBlocklistEntry>>,
}

impl StubSubtitleDownloadRepo {
    async fn blocklisted(&self) -> Vec<SubtitleBlocklistEntry> {
        self.blocklist.lock().await.clone()
    }
}

#[async_trait]
impl crate::SubtitleDownloadRepository for StubSubtitleDownloadRepo {
    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .filter(|row| row.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn get(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }

    async fn list_for_media_file(&self, media_file_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .filter(|row| row.media_file_id == media_file_id)
            .cloned()
            .collect())
    }

    async fn list_probe_cache_for_media_file(
        &self,
        _media_file_id: &str,
    ) -> AppResult<Vec<crate::subtitles::ExternalSubtitleProbeCacheEntry>> {
        Ok(Vec::new())
    }

    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<SubtitleBlocklistEntry>> {
        Ok(self
            .blocklist
            .lock()
            .await
            .iter()
            .filter(|row| row.media_file_id == media_file_id)
            .cloned()
            .collect())
    }

    async fn insert(&self, download: &SubtitleDownload) -> AppResult<()> {
        self.rows.lock().await.push(download.clone());
        Ok(())
    }

    async fn upsert_probe_cache_entry(
        &self,
        _entry: &crate::subtitles::ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn set_synced(&self, _id: &str, _synced: bool) -> AppResult<()> {
        Ok(())
    }

    async fn delete(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        let mut rows = self.rows.lock().await;
        let Some(index) = rows.iter().position(|row| row.id == id) else {
            return Ok(None);
        };
        Ok(Some(rows.remove(index)))
    }

    async fn delete_probe_cache_entry(
        &self,
        _media_file_id: &str,
        _file_path: &str,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        Ok(self.blocklist.lock().await.iter().any(|row| {
            row.media_file_id == media_file_id
                && row.provider == provider
                && row.provider_file_id == provider_file_id
        }))
    }

    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        self.blocklist.lock().await.push(SubtitleBlocklistEntry {
            id: Id::new().0,
            media_file_id: media_file_id.to_string(),
            provider: provider.to_string(),
            provider_file_id: provider_file_id.to_string(),
            language: language.to_string(),
            reason: reason.map(str::to_string),
            created_at: Utc::now().to_rfc3339(),
        });
        Ok(())
    }
}

#[derive(Default)]
struct StubSubtitleProviderConfigRepo {
    configs: Mutex<Vec<SubtitleProviderConfig>>,
}

impl StubSubtitleProviderConfigRepo {
    fn with_configs(configs: Vec<SubtitleProviderConfig>) -> Self {
        Self {
            configs: Mutex::new(configs),
        }
    }
}

#[async_trait]
impl crate::SubtitleProviderConfigRepository for StubSubtitleProviderConfigRepo {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<SubtitleProviderConfig>> {
        Ok(self
            .configs
            .lock()
            .await
            .iter()
            .filter(|config| {
                provider_type
                    .as_deref()
                    .is_none_or(|wanted| config.provider_type == wanted)
            })
            .cloned()
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<SubtitleProviderConfig>> {
        Ok(self
            .configs
            .lock()
            .await
            .iter()
            .find(|config| config.id == id)
            .cloned())
    }

    async fn create(&self, config: SubtitleProviderConfig) -> AppResult<SubtitleProviderConfig> {
        self.configs.lock().await.push(config.clone());
        Ok(config)
    }

    async fn update(
        &self,
        update: crate::SubtitleProviderConfigUpdate,
    ) -> AppResult<SubtitleProviderConfig> {
        let configs = self.configs.lock().await;
        configs
            .iter()
            .find(|config| config.id == update.id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("subtitle provider config {}", update.id)))
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.configs.lock().await.retain(|config| config.id != id);
        Ok(())
    }
}

struct StubSubtitleProviderClient;

#[async_trait]
impl crate::SubtitleProviderClient for StubSubtitleProviderClient {
    async fn search(
        &self,
        query: &SubtitleQuery,
    ) -> AppResult<Vec<crate::subtitles::SubtitleMatch>> {
        Ok(vec![crate::subtitles::SubtitleMatch {
            provider: STUB_PROVIDER_TYPE.to_string(),
            provider_file_id: "stub-file-1".to_string(),
            language: query
                .languages
                .first()
                .cloned()
                .unwrap_or_else(|| "eng".to_string()),
            release_info: Some("Synthetic.Release.Group".to_string()),
            score: 90,
            score_percent: 90,
            hearing_impaired: false,
            forced: false,
            ai_translated: false,
            machine_translated: false,
            uploader: None,
            download_count: None,
            hash_matched: false,
        }])
    }

    async fn download(&self, _provider_file_id: &str) -> AppResult<SubtitleFile> {
        Ok(SubtitleFile {
            content: b"1\n00:00:01,000 --> 00:00:02,000\nline\n".to_vec(),
            format: "srt".to_string(),
            filename: None,
            content_type: None,
        })
    }

    async fn validate_connection(&self) -> AppResult<crate::SubtitleProviderValidationResult> {
        Err(AppError::Repository(
            "validation is not used in these tests".to_string(),
        ))
    }

    fn name(&self) -> &str {
        STUB_PROVIDER_TYPE
    }
}

const STUB_PROVIDER_TYPE: &str = "stubtitles";

/// Plugin host stub. `instantiates` false reproduces the provider that is
/// configured and enabled but cannot be built (for example a missing API key).
struct StubSubtitlePluginProvider {
    instantiates: bool,
}

impl crate::SubtitlePluginProvider for StubSubtitlePluginProvider {
    fn client_for_config(
        &self,
        _config: &SubtitleProviderConfig,
        _host_bindings: &HashMap<PluginHostBindingId, String>,
    ) -> Option<Arc<dyn crate::SubtitleProviderClient>> {
        self.instantiates
            .then(|| Arc::new(StubSubtitleProviderClient) as Arc<dyn crate::SubtitleProviderClient>)
    }

    fn available_provider_types(&self) -> Vec<String> {
        vec![STUB_PROVIDER_TYPE.to_string()]
    }

    fn supports_catalog_search_for_provider(&self, provider_type: &str) -> bool {
        provider_type == STUB_PROVIDER_TYPE
    }

    fn recommended_facets_for_provider(&self, _provider_type: &str) -> Vec<String> {
        vec![
            "movie".to_string(),
            "series".to_string(),
            "anime".to_string(),
        ]
    }

    fn config_fields_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        Vec::new()
    }

    fn plugin_name_for_provider(&self, _provider_type: &str) -> Option<String> {
        Some("Stub subtitles".to_string())
    }
}

fn stub_provider_config() -> SubtitleProviderConfig {
    let now = Utc::now();
    SubtitleProviderConfig {
        id: "stub-config".to_string(),
        name: "Stub subtitles".to_string(),
        provider_type: STUB_PROVIDER_TYPE.to_string(),
        config_json: "{}".to_string(),
        enabled_facets: vec![
            "movie".to_string(),
            "series".to_string(),
            "anime".to_string(),
        ],
        is_enabled: true,
        last_health_status: None,
        last_error: None,
        last_error_at: None,
        disabled_until: None,
        created_at: now,
        updated_at: now,
    }
}

// ── Actors ────────────────────────────────────────────────────────────────

/// Build an actor whose authorization is already loaded, with grants stored the
/// way the datastore stores them (shadowed bits stripped).
fn library_actor(
    username: &str,
    app_permissions: AppPermissionMask,
    grants: &[(&str, LibraryPermissionMask)],
) -> User {
    let mut user = User {
        id: Id::new().0,
        username: username.to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    user.authorization.app = app_permissions;
    user.authorization.libraries = grants
        .iter()
        .map(|(library_id, mask)| (library_id.to_string(), mask.normalized_for_storage()))
        .collect();
    user.authorization.loaded = true;
    user
}

fn viewer_for(library_id: &str) -> User {
    library_actor(
        "viewer",
        AppPermissionMask::NONE,
        &[(library_id, LibraryPermissionMask::VIEW)],
    )
}

fn subtitle_manager_for(library_id: &str) -> User {
    let mut mask = LibraryPermissionMask::VIEW;
    mask.insert(LibraryPermissionMask::MANAGE_SUBTITLES);
    library_actor(
        "subtitle-manager",
        AppPermissionMask::NONE,
        &[(library_id, mask)],
    )
}

fn title_manager_for(library_id: &str) -> User {
    let mut mask = LibraryPermissionMask::VIEW;
    mask.insert(LibraryPermissionMask::MANAGE_TITLES);
    library_actor(
        "title-manager",
        AppPermissionMask::NONE,
        &[(library_id, mask)],
    )
}

fn catalog_admin() -> User {
    library_actor(
        "catalog-admin",
        AppPermissionMask::MANAGE_CATALOG_SETTINGS,
        &[],
    )
}

fn assert_unauthorized(result: AppResult<impl std::fmt::Debug>, what: &str) {
    match result {
        Err(AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }) => {}
        Err(error) => panic!("{what}: expected Unauthorized, got {error}"),
        Ok(value) => panic!("{what}: expected Unauthorized, got Ok({value:?})"),
    }
}

fn assert_not_unauthorized(result: &AppResult<impl std::fmt::Debug>, what: &str) {
    if let Err(AppError::Unauthorized(message)) = result {
        panic!("{what}: unexpectedly denied: {message}");
    }
}

// ── Fixture ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum StubProviders {
    /// No plugin host and no config repository at all.
    NoRuntime,
    /// Plugin host present, nothing configured.
    NoConfigs,
    /// Configured and enabled, but the host cannot build a client.
    InstantiationFails,
    /// Configured, enabled, and searchable.
    Ready,
}

struct SubtitleFixture {
    app: AppUseCase,
    admin: User,
    /// Movie library: the library a grant is issued on.
    movie_library_id: String,
    movie_media_file_id: String,
    movie_subtitle_id: String,
    /// Series library: the library the same grant must NOT reach.
    series_library_id: String,
    series_media_file_id: String,
    series_subtitle_id: String,
    subtitle_downloads: Arc<StubSubtitleDownloadRepo>,
    _tempdir: tempfile::TempDir,
}

async fn seed_fixture(subtitles_enabled: bool, providers: StubProviders) -> SubtitleFixture {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(&movie_root).expect("create movie root");
    std::fs::create_dir_all(&series_root).expect("create series root");

    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, admin, _titles) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files.clone(),
    );

    let subtitle_downloads = Arc::new(StubSubtitleDownloadRepo::default());
    let app = app.with_test_overrides(|services| {
        let services = services.with_subtitle_downloads(subtitle_downloads.clone());
        match providers {
            StubProviders::NoRuntime => services,
            StubProviders::NoConfigs => services
                .with_subtitle_provider_configs(Arc::new(StubSubtitleProviderConfigRepo::default()))
                .with_subtitle_plugin_provider(Arc::new(StubSubtitlePluginProvider {
                    instantiates: true,
                })),
            StubProviders::InstantiationFails => services
                .with_subtitle_provider_configs(Arc::new(
                    StubSubtitleProviderConfigRepo::with_configs(vec![stub_provider_config()]),
                ))
                .with_subtitle_plugin_provider(Arc::new(StubSubtitlePluginProvider {
                    instantiates: false,
                })),
            StubProviders::Ready => services
                .with_subtitle_provider_configs(Arc::new(
                    StubSubtitleProviderConfigRepo::with_configs(vec![stub_provider_config()]),
                ))
                .with_subtitle_plugin_provider(Arc::new(StubSubtitlePluginProvider {
                    instantiates: true,
                })),
        }
    });

    for (facet, root) in [
        (MediaFacet::Movie, &movie_root),
        (MediaFacet::Series, &series_root),
    ] {
        app.update_media_settings(
            &admin,
            facet,
            empty_update_media_settings_with_roots(vec![build_root_folder_entry(root, true)]),
        )
        .await
        .expect("save roots");
    }

    app.update_subtitle_settings(
        &admin,
        UpdateSubtitleSettings {
            enabled: subtitles_enabled,
            languages: vec![
                crate::subtitles::wanted::SubtitleLanguagePref {
                    code: "swe".to_string(),
                    hearing_impaired: false,
                    forced: false,
                },
                crate::subtitles::wanted::SubtitleLanguagePref {
                    code: "eng".to_string(),
                    hearing_impaired: false,
                    forced: false,
                },
            ],
            auto_download_on_import: false,
            minimum_score_series: 90,
            minimum_score_movie: 70,
            search_interval_hours: 6,
            include_ai_translated: false,
            include_machine_translated: false,
            sync_enabled: false,
            sync_threshold_series: 90,
            sync_threshold_movie: 70,
            sync_max_offset_seconds: 60,
        },
    )
    .await
    .expect("save subtitle settings");

    let mut seeded = Vec::new();
    for (facet, root, title_name) in [
        (MediaFacet::Movie, &movie_root, "Lumen Drift"),
        (MediaFacet::Series, &series_root, "Harbor Lantern"),
    ] {
        let title = app
            .add_title(
                &admin,
                NewTitle {
                    name: title_name.to_string(),
                    facet: facet.clone(),
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                    ..Default::default()
                },
            )
            .await
            .expect("create title");

        let title_folder = root.join(title_name);
        std::fs::create_dir(&title_folder).expect("create title folder");
        app.services
            .catalog
            .titles
            .set_folder_path(&title.id, title_folder.to_str().unwrap())
            .await
            .expect("record title folder ownership");
        let media_path = title_folder.join(format!("{title_name}.mkv"));
        std::fs::write(&media_path, b"video").expect("write media file");
        let subtitle_path = title_folder.join(format!("{title_name}.eng.srt"));
        std::fs::write(&subtitle_path, b"subtitle").expect("write subtitle file");

        let media_file_id = app
            .services
            .library
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: media_path.to_string_lossy().to_string(),
                size_bytes: 5,
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .expect("insert media file");

        let subtitle_id = Id::new().0;
        crate::SubtitleDownloadRepository::insert(
            subtitle_downloads.as_ref(),
            &SubtitleDownload {
                id: subtitle_id.clone(),
                media_file_id: media_file_id.clone(),
                title_id: title.id.clone(),
                episode_id: None,
                source_kind: ExternalSubtitleSourceKind::Downloaded,
                language: "eng".to_string(),
                provider: Some(STUB_PROVIDER_TYPE.to_string()),
                provider_file_id: Some("stub-file-1".to_string()),
                file_path: subtitle_path.to_string_lossy().to_string(),
                score: Some(90),
                hearing_impaired: false,
                forced: false,
                ai_translated: false,
                machine_translated: false,
                uploader: None,
                release_info: None,
                synced: false,
                downloaded_at: Utc::now().to_rfc3339(),
            },
        )
        .await
        .expect("seed subtitle download");

        seeded.push((title.library_id, media_file_id, subtitle_id));
    }

    let (series_library_id, series_media_file_id, series_subtitle_id) =
        seeded.pop().expect("series seed");
    let (movie_library_id, movie_media_file_id, movie_subtitle_id) =
        seeded.pop().expect("movie seed");

    SubtitleFixture {
        app,
        admin,
        movie_library_id,
        movie_media_file_id,
        movie_subtitle_id,
        series_library_id,
        series_media_file_id,
        series_subtitle_id,
        subtitle_downloads,
        _tempdir: tempdir,
    }
}

fn download_request(media_file_id: &str) -> DownloadSubtitleForMediaFileRequest {
    DownloadSubtitleForMediaFileRequest {
        media_file_id: media_file_id.to_string(),
        provider_name: STUB_PROVIDER_TYPE.to_string(),
        provider_file_id: "stub-file-1".to_string(),
        language: "eng".to_string(),
        forced: false,
        hearing_impaired: false,
        score: Some(90),
        release_info: None,
        uploader: None,
        ai_translated: false,
        machine_translated: false,
    }
}

// ── Search guard ──────────────────────────────────────────────────────────

#[tokio::test]
async fn subtitle_search_requires_manage_subtitles_on_the_files_library() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let library = fixture.movie_library_id.as_str();
    let media_file_id = fixture.movie_media_file_id.as_str();

    // View alone no longer reaches the administrator's providers.
    assert_unauthorized(
        fixture
            .app
            .search_subtitles_for_media_file(&viewer_for(library), media_file_id, None)
            .await,
        "view-only search",
    );

    for actor in [
        subtitle_manager_for(library),
        title_manager_for(library),
        catalog_admin(),
        fixture.admin.clone(),
    ] {
        let username = actor.username.clone();
        let outcome = fixture
            .app
            .search_subtitles_for_media_file(&actor, media_file_id, None)
            .await
            .unwrap_or_else(|error| panic!("{username} search should be allowed: {error}"));
        assert_eq!(
            outcome.status,
            SubtitleSearchStatus::Ready,
            "{username} search status"
        );
        assert_eq!(outcome.results.len(), 1, "{username} search results");
    }
}

#[tokio::test]
async fn subtitle_search_grant_does_not_cross_libraries() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let movie_manager = subtitle_manager_for(&fixture.movie_library_id);

    assert!(
        fixture
            .app
            .search_subtitles_for_media_file(&movie_manager, &fixture.movie_media_file_id, None)
            .await
            .is_ok(),
        "the grant's own library should be searchable"
    );
    assert_unauthorized(
        fixture
            .app
            .search_subtitles_for_media_file(&movie_manager, &fixture.series_media_file_id, None)
            .await,
        "search in a library without the grant",
    );
    assert_ne!(fixture.movie_library_id, fixture.series_library_id);
}

// ── Download guard ────────────────────────────────────────────────────────

#[tokio::test]
async fn subtitle_download_requires_manage_subtitles_on_the_files_library() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let library = fixture.movie_library_id.as_str();

    assert_unauthorized(
        fixture
            .app
            .download_subtitle_for_media_file(
                &viewer_for(library),
                download_request(&fixture.movie_media_file_id),
            )
            .await,
        "view-only download",
    );

    for actor in [
        subtitle_manager_for(library),
        title_manager_for(library),
        catalog_admin(),
    ] {
        let username = actor.username.clone();
        let result = fixture
            .app
            .download_subtitle_for_media_file(
                &actor,
                download_request(&fixture.movie_media_file_id),
            )
            .await;
        assert_not_unauthorized(&result, &format!("{username} download"));
        result.unwrap_or_else(|error| panic!("{username} download should succeed: {error}"));
    }
}

#[tokio::test]
async fn subtitle_download_grant_does_not_cross_libraries() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let movie_manager = subtitle_manager_for(&fixture.movie_library_id);

    assert_unauthorized(
        fixture
            .app
            .download_subtitle_for_media_file(
                &movie_manager,
                download_request(&fixture.series_media_file_id),
            )
            .await,
        "download in a library without the grant",
    );
}

// ── Delete / blocklist / preview guards ───────────────────────────────────

#[tokio::test]
async fn external_subtitle_preview_requires_manage_subtitles() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let library = fixture.movie_library_id.as_str();

    assert_unauthorized(
        fixture
            .app
            .preview_delete_external_subtitle_file(&viewer_for(library), &fixture.movie_subtitle_id)
            .await,
        "view-only subtitle delete preview",
    );

    for actor in [
        subtitle_manager_for(library),
        title_manager_for(library),
        catalog_admin(),
    ] {
        let username = actor.username.clone();
        let preview = fixture
            .app
            .preview_delete_external_subtitle_file(&actor, &fixture.movie_subtitle_id)
            .await
            .unwrap_or_else(|error| panic!("{username} preview should be allowed: {error}"));
        assert_eq!(
            preview.subtitle_count, 1,
            "{username} preview subtitle count"
        );
    }

    // Deleting the title itself still needs Manage Titles, so the narrower
    // grant must not open that door.
    assert_unauthorized(
        fixture
            .app
            .preview_delete_media_file(&subtitle_manager_for(library), &fixture.movie_media_file_id)
            .await,
        "subtitle manager previewing a media-file delete",
    );
}

#[tokio::test]
async fn external_subtitle_preview_grant_does_not_cross_libraries() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;

    assert_unauthorized(
        fixture
            .app
            .preview_delete_external_subtitle_file(
                &subtitle_manager_for(&fixture.movie_library_id),
                &fixture.series_subtitle_id,
            )
            .await,
        "subtitle preview in a library without the grant",
    );
}

#[tokio::test]
async fn external_subtitle_delete_requires_manage_subtitles() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let library = fixture.movie_library_id.as_str();

    let preview = fixture
        .app
        .preview_delete_external_subtitle_file(&fixture.admin, &fixture.movie_subtitle_id)
        .await
        .expect("admin preview");

    assert_unauthorized(
        fixture
            .app
            .delete_external_subtitle(
                &viewer_for(library),
                &fixture.movie_subtitle_id,
                &preview.fingerprint,
                None,
            )
            .await,
        "view-only subtitle delete",
    );
    assert_unauthorized(
        fixture
            .app
            .delete_external_subtitle(
                &subtitle_manager_for(library),
                &fixture.series_subtitle_id,
                &preview.fingerprint,
                None,
            )
            .await,
        "subtitle delete in a library without the grant",
    );

    // Each allowed actor deletes on its own fixture, because a delete consumes
    // the row. Manage Titles is allowed only because it shadows Manage
    // Subtitles; the catalog administrator only through the app-library
    // override.
    for build_actor in [
        subtitle_manager_for as fn(&str) -> User,
        title_manager_for,
        |_library: &str| catalog_admin(),
    ] {
        let fixture = seed_fixture(true, StubProviders::Ready).await;
        let actor = build_actor(&fixture.movie_library_id);
        let username = actor.username.clone();
        let preview = fixture
            .app
            .preview_delete_external_subtitle_file(&fixture.admin, &fixture.movie_subtitle_id)
            .await
            .expect("admin preview");

        let result = fixture
            .app
            .delete_external_subtitle(
                &actor,
                &fixture.movie_subtitle_id,
                &preview.fingerprint,
                None,
            )
            .await;
        assert_not_unauthorized(&result, &format!("{username} subtitle delete"));
        result.unwrap_or_else(|error| panic!("{username} should delete the subtitle: {error}"));
        assert!(
            crate::SubtitleDownloadRepository::get(
                fixture.subtitle_downloads.as_ref(),
                &fixture.movie_subtitle_id
            )
            .await
            .expect("read back subtitle")
            .is_none(),
            "{username}: the subtitle row should be gone"
        );
    }
}

#[tokio::test]
async fn external_subtitle_blocklist_requires_manage_subtitles() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let library = fixture.movie_library_id.as_str();

    let preview = fixture
        .app
        .preview_delete_external_subtitle_file(&fixture.admin, &fixture.movie_subtitle_id)
        .await
        .expect("admin preview");

    assert_unauthorized(
        fixture
            .app
            .blocklist_external_subtitle(
                &viewer_for(library),
                &fixture.movie_subtitle_id,
                Some("bad timing"),
                &preview.fingerprint,
                None,
            )
            .await,
        "view-only subtitle blocklist",
    );
    assert_unauthorized(
        fixture
            .app
            .blocklist_external_subtitle(
                &subtitle_manager_for(library),
                &fixture.series_subtitle_id,
                Some("bad timing"),
                &preview.fingerprint,
                None,
            )
            .await,
        "subtitle blocklist in a library without the grant",
    );

    // Each allowed actor blocklists on its own fixture, because blocklisting
    // consumes the row. Manage Titles is allowed only because it shadows
    // Manage Subtitles; the catalog administrator only through the
    // app-library override.
    for build_actor in [
        subtitle_manager_for as fn(&str) -> User,
        title_manager_for,
        |_library: &str| catalog_admin(),
    ] {
        let fixture = seed_fixture(true, StubProviders::Ready).await;
        let actor = build_actor(&fixture.movie_library_id);
        let username = actor.username.clone();
        let preview = fixture
            .app
            .preview_delete_external_subtitle_file(&fixture.admin, &fixture.movie_subtitle_id)
            .await
            .expect("admin preview");

        let result = fixture
            .app
            .blocklist_external_subtitle(
                &actor,
                &fixture.movie_subtitle_id,
                Some("bad timing"),
                &preview.fingerprint,
                None,
            )
            .await;
        assert_not_unauthorized(&result, &format!("{username} subtitle blocklist"));
        result.unwrap_or_else(|error| panic!("{username} should blocklist the subtitle: {error}"));

        let blocklisted = fixture.subtitle_downloads.blocklisted().await;
        assert_eq!(blocklisted.len(), 1, "{username}: one blocklisted row");
        assert_eq!(
            blocklisted[0].media_file_id, fixture.movie_media_file_id,
            "{username}: the blocklisted row should be the movie subtitle"
        );
    }
}

#[tokio::test]
async fn catalog_admin_can_blocklist_an_external_subtitle() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;

    let preview = fixture
        .app
        .preview_delete_external_subtitle_file(&fixture.admin, &fixture.movie_subtitle_id)
        .await
        .expect("admin preview");
    fixture
        .app
        .blocklist_external_subtitle(
            &catalog_admin(),
            &fixture.movie_subtitle_id,
            None,
            &preview.fingerprint,
            None,
        )
        .await
        .expect("catalog administrator keeps subtitle blocklisting");
}

// ── Listing stays on View ─────────────────────────────────────────────────

#[tokio::test]
async fn listing_external_subtitles_stays_on_view() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let viewer = viewer_for(&fixture.movie_library_id);

    let listings = fixture
        .app
        .list_external_subtitles_for_title(
            &viewer,
            &crate::SubtitleDownloadRepository::get(
                fixture.subtitle_downloads.as_ref(),
                &fixture.movie_subtitle_id,
            )
            .await
            .expect("read subtitle")
            .expect("subtitle row")
            .title_id,
        )
        .await
        .expect("a viewer can still list external subtitles");
    assert_eq!(listings.len(), 1);

    fixture
        .app
        .list_external_subtitle_blocklist_for_media_file(&viewer, &fixture.movie_media_file_id)
        .await
        .expect("a viewer can still list the subtitle blocklist");
}

// ── Search outcome statuses ───────────────────────────────────────────────

#[tokio::test]
async fn search_reports_disabled_without_touching_providers() {
    let fixture = seed_fixture(false, StubProviders::Ready).await;
    let outcome = fixture
        .app
        .search_subtitles_for_media_file(
            &subtitle_manager_for(&fixture.movie_library_id),
            &fixture.movie_media_file_id,
            None,
        )
        .await
        .expect("search should not error when subtitles are disabled");

    assert_eq!(outcome.status, SubtitleSearchStatus::Disabled);
    assert!(outcome.results.is_empty());
    assert_eq!(outcome.language, "swe");
    assert_eq!(outcome.available_languages, vec!["swe", "eng"]);
}

#[tokio::test]
async fn search_reports_no_providers_instead_of_an_error() {
    for providers in [StubProviders::NoRuntime, StubProviders::NoConfigs] {
        let fixture = seed_fixture(true, providers).await;
        let outcome = fixture
            .app
            .search_subtitles_for_media_file(
                &subtitle_manager_for(&fixture.movie_library_id),
                &fixture.movie_media_file_id,
                None,
            )
            .await
            .expect("an unconfigured provider set is a status, not an error");

        assert_eq!(outcome.status, SubtitleSearchStatus::NoProviders);
        assert!(outcome.results.is_empty());
        assert_eq!(outcome.available_languages, vec!["swe", "eng"]);
    }
}

#[tokio::test]
async fn search_reports_provider_unavailable_when_no_client_can_be_built() {
    let fixture = seed_fixture(true, StubProviders::InstantiationFails).await;
    let outcome = fixture
        .app
        .search_subtitles_for_media_file(
            &subtitle_manager_for(&fixture.movie_library_id),
            &fixture.movie_media_file_id,
            None,
        )
        .await
        .expect("a provider that cannot be instantiated is a status, not an error");

    assert_eq!(outcome.status, SubtitleSearchStatus::ProviderUnavailable);
    assert!(outcome.results.is_empty());
}

#[tokio::test]
async fn search_without_a_language_uses_the_configured_preferred_language() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let actor = subtitle_manager_for(&fixture.movie_library_id);

    let preferred = fixture
        .app
        .search_subtitles_for_media_file(&actor, &fixture.movie_media_file_id, None)
        .await
        .expect("search with no language");
    assert_eq!(preferred.status, SubtitleSearchStatus::Ready);
    assert_eq!(
        preferred.language, "swe",
        "first configured wanted language"
    );
    assert_eq!(preferred.available_languages, vec!["swe", "eng"]);
    assert_eq!(preferred.results[0].language, "swe");

    // A blank language is the same as none.
    let blank = fixture
        .app
        .search_subtitles_for_media_file(&actor, &fixture.movie_media_file_id, Some("  "))
        .await
        .expect("search with a blank language");
    assert_eq!(blank.language, "swe");

    // An explicit language wins.
    let explicit = fixture
        .app
        .search_subtitles_for_media_file(&actor, &fixture.movie_media_file_id, Some("eng"))
        .await
        .expect("search with an explicit language");
    assert_eq!(explicit.language, "eng");
    assert_eq!(explicit.results[0].language, "eng");
}

#[tokio::test]
async fn search_falls_back_to_english_when_no_language_is_configured() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    fixture
        .app
        .update_subtitle_settings(
            &fixture.admin,
            UpdateSubtitleSettings {
                enabled: true,
                languages: Vec::new(),
                auto_download_on_import: false,
                minimum_score_series: 90,
                minimum_score_movie: 70,
                search_interval_hours: 6,
                include_ai_translated: false,
                include_machine_translated: false,
                sync_enabled: false,
                sync_threshold_series: 90,
                sync_threshold_movie: 70,
                sync_max_offset_seconds: 60,
            },
        )
        .await
        .expect("clear configured languages");

    let outcome = fixture
        .app
        .search_subtitles_for_media_file(
            &subtitle_manager_for(&fixture.movie_library_id),
            &fixture.movie_media_file_id,
            None,
        )
        .await
        .expect("search with no configured language");
    assert_eq!(outcome.language, "eng");
    assert!(outcome.available_languages.is_empty());
}

#[tokio::test]
async fn search_filters_blocklisted_results() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    crate::SubtitleDownloadRepository::blocklist(
        fixture.subtitle_downloads.as_ref(),
        &fixture.movie_media_file_id,
        STUB_PROVIDER_TYPE,
        "stub-file-1",
        "swe",
        None,
    )
    .await
    .expect("blocklist the only result");

    let outcome = fixture
        .app
        .search_subtitles_for_media_file(
            &subtitle_manager_for(&fixture.movie_library_id),
            &fixture.movie_media_file_id,
            None,
        )
        .await
        .expect("search");
    assert_eq!(outcome.status, SubtitleSearchStatus::Ready);
    assert!(
        outcome.results.is_empty(),
        "blocklisted matches stay filtered server-side"
    );
}

// ── Stored grants round-trip ──────────────────────────────────────────────

#[tokio::test]
async fn granting_manage_subtitles_survives_storage_normalization() {
    let (app, admin) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &admin,
        "subtitle-grantee",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::SubtitleManagement,
        ],
    )
    .await
    .expect("create user");

    let authorization = app
        .load_user_authorization(&created)
        .await
        .expect("load authorization");
    let movie_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    assert!(authorization.has_library_permission(
        &movie_library,
        scryer_domain::LibraryPermission::ManageSubtitles
    ));
    assert!(!authorization.has_library_permission(
        &movie_library,
        scryer_domain::LibraryPermission::ManageTitles
    ));

    // A Manage Titles grant stores no Manage Subtitles bit but still resolves
    // to the permission, so existing grants keep the capability they had.
    let title_manager = create_user_with_permissions(
        &app,
        &admin,
        "title-grantee",
        "password123",
        vec![TestPermissionPreset::TitleManagement],
    )
    .await
    .expect("create title manager");
    let title_manager_authorization = app
        .load_user_authorization(&title_manager)
        .await
        .expect("load authorization");
    assert!(
        !title_manager_authorization
            .library_permissions(&movie_library)
            .contains(LibraryPermissionMask::MANAGE_SUBTITLES),
        "storage keeps only the broader grant"
    );
    assert!(title_manager_authorization.has_library_permission(
        &movie_library,
        scryer_domain::LibraryPermission::ManageSubtitles
    ));
}

#[tokio::test]
async fn manage_subtitles_appears_in_the_jwt_claim_vocabulary() {
    assert_eq!(
        AppUseCase::library_permission_claim_string(
            scryer_domain::LibraryPermission::ManageSubtitles
        ),
        "manageSubtitles"
    );
}
