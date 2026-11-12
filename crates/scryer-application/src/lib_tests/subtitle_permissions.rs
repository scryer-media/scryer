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
    search_backoffs: Mutex<Vec<crate::subtitles::SubtitleSearchBackoff>>,
    /// One per `apply_search_backoffs` call: the backoff write transactions.
    search_backoff_writes: std::sync::atomic::AtomicUsize,
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

    async fn list_search_backoffs_for_media_files(
        &self,
        media_file_ids: &[String],
    ) -> AppResult<Vec<crate::subtitles::SubtitleSearchBackoff>> {
        Ok(self
            .search_backoffs
            .lock()
            .await
            .iter()
            .filter(|row| media_file_ids.contains(&row.media_file_id))
            .cloned()
            .collect())
    }

    async fn apply_search_backoffs(
        &self,
        media_file_id: &str,
        upserts: &[crate::subtitles::SubtitleSearchBackoff],
        cleared: &[crate::subtitles::SubtitleSearchBackoffKey],
    ) -> AppResult<()> {
        self.search_backoff_writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let same = |row: &crate::subtitles::SubtitleSearchBackoff,
                    language: &str,
                    hearing_impaired: bool,
                    forced: bool| {
            row.media_file_id == media_file_id
                && row.language == language
                && row.hearing_impaired == hearing_impaired
                && row.forced == forced
        };
        let mut rows = self.search_backoffs.lock().await;
        for key in cleared {
            rows.retain(|row| !same(row, &key.language, key.hearing_impaired, key.forced));
        }
        for upsert in upserts {
            rows.retain(|row| {
                !same(
                    row,
                    &upsert.language,
                    upsert.hearing_impaired,
                    upsert.forced,
                )
            });
            rows.push(upsert.clone());
        }
        Ok(())
    }
}

#[derive(Default)]
struct StubSubtitleProviderConfigRepo {
    configs: Mutex<Vec<SubtitleProviderConfig>>,
    list_calls: std::sync::atomic::AtomicUsize,
}

impl StubSubtitleProviderConfigRepo {
    fn with_configs(configs: Vec<SubtitleProviderConfig>) -> Self {
        Self {
            configs: Mutex::new(configs),
            list_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl crate::SubtitleProviderConfigRepository for StubSubtitleProviderConfigRepo {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<SubtitleProviderConfig>> {
        self.list_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

/// What the stub provider answers, and a record of what it was asked.
#[derive(Clone, Default)]
struct StubSearchControl {
    /// `(title, language)` of every search, in order.
    searches: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    /// When set, only these languages have a subtitle; the default finds every
    /// language.
    found_languages: Arc<std::sync::Mutex<Option<HashSet<String>>>>,
}

impl StubSearchControl {
    fn searches(&self) -> Vec<(String, String)> {
        self.searches.lock().unwrap().clone()
    }

    fn find_only(&self, languages: &[&str]) {
        *self.found_languages.lock().unwrap() = Some(
            languages
                .iter()
                .map(|language| language.to_string())
                .collect(),
        );
    }
}

struct StubSubtitleProviderClient {
    control: StubSearchControl,
}

#[async_trait]
impl crate::SubtitleProviderClient for StubSubtitleProviderClient {
    async fn search(
        &self,
        query: &SubtitleQuery,
    ) -> AppResult<Vec<crate::subtitles::SubtitleMatch>> {
        let language = query
            .languages
            .first()
            .cloned()
            .unwrap_or_else(|| "eng".to_string());
        self.control
            .searches
            .lock()
            .unwrap()
            .push((query.title.clone(), language.clone()));
        if self
            .control
            .found_languages
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|found| !found.contains(&language))
        {
            return Ok(Vec::new());
        }
        Ok(vec![crate::subtitles::SubtitleMatch {
            provider: STUB_PROVIDER_TYPE.to_string(),
            provider_file_id: "stub-file-1".to_string(),
            language,
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
    control: StubSearchControl,
}

impl crate::SubtitlePluginProvider for StubSubtitlePluginProvider {
    fn client_for_config(
        &self,
        _config: &SubtitleProviderConfig,
        _host_bindings: &HashMap<PluginHostBindingId, String>,
    ) -> Option<Arc<dyn crate::SubtitleProviderClient>> {
        self.instantiates.then(|| {
            Arc::new(StubSubtitleProviderClient {
                control: self.control.clone(),
            }) as Arc<dyn crate::SubtitleProviderClient>
        })
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
    /// What the ready stub provider finds, and what it was asked.
    search_control: StubSearchControl,
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
    let search_control = StubSearchControl::default();
    let app = app.with_test_overrides(|services| {
        let services = services.with_subtitle_downloads(subtitle_downloads.clone());
        match providers {
            StubProviders::NoRuntime => services,
            StubProviders::NoConfigs => services
                .with_subtitle_provider_configs(Arc::new(StubSubtitleProviderConfigRepo::default()))
                .with_subtitle_plugin_provider(Arc::new(StubSubtitlePluginProvider {
                    instantiates: true,
                    control: Default::default(),
                })),
            StubProviders::InstantiationFails => services
                .with_subtitle_provider_configs(Arc::new(
                    StubSubtitleProviderConfigRepo::with_configs(vec![stub_provider_config()]),
                ))
                .with_subtitle_plugin_provider(Arc::new(StubSubtitlePluginProvider {
                    instantiates: false,
                    control: Default::default(),
                })),
            StubProviders::Ready => services
                .with_subtitle_provider_configs(Arc::new(
                    StubSubtitleProviderConfigRepo::with_configs(vec![stub_provider_config()]),
                ))
                .with_subtitle_plugin_provider(Arc::new(StubSubtitlePluginProvider {
                    instantiates: true,
                    control: search_control.clone(),
                })),
        }
    });

    for (facet, root) in [
        (MediaFacet::Movie, &movie_root),
        (MediaFacet::Series, &series_root),
    ] {
        update_default_library_roots(
            &app,
            &admin,
            facet,
            vec![build_root_folder_entry(root, true)],
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
        search_control,
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
    // Where the sidecar lives on disk is privileged: a viewer gets its name only.
    assert_eq!(listings[0].download.file_path, "Lumen Drift.eng.srt");

    let manager_listings = fixture
        .app
        .list_external_subtitles_for_title(
            &title_manager_for(&fixture.movie_library_id),
            &listings[0].download.title_id,
        )
        .await
        .expect("a title manager can list external subtitles");
    assert!(
        manager_listings[0]
            .download
            .file_path
            .ends_with("/Lumen Drift/Lumen Drift.eng.srt"),
        "a title manager keeps the full sidecar path: {}",
        manager_listings[0].download.file_path
    );

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

// The explicit episode action uses the same subtitle policy, but only a bounded selection.
async fn seed_subtitle_batch() -> (SubtitleFixture, String, Vec<String>) {
    let mut fixture = seed_fixture(true, StubProviders::Ready).await;
    fixture.app = fixture.app.with_test_overrides(|services| {
        services.with_job_runs(Arc::new(RecordingJobRunRepo::default()))
    });
    fixture
        .app
        .services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(&fixture.admin.id, fixture.admin.authorization.app)
        .await
        .unwrap();
    let file = fixture
        .app
        .services
        .library
        .media_files
        .get_media_file_by_id(&fixture.series_media_file_id)
        .await
        .unwrap()
        .unwrap();
    let title_id = file.title_id;
    let mut episodes = Vec::new();
    for number in 1..=3 {
        let episode = fixture
            .app
            .create_episode(
                &fixture.admin,
                title_id.clone(),
                None,
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                None,
                None,
                None,
                false,
                false,
            )
            .await
            .unwrap();
        episodes.push(episode.id);
    }
    for episode in &episodes[..2] {
        fixture
            .app
            .services
            .library
            .media_files
            .link_file_to_episode(&fixture.series_media_file_id, episode)
            .await
            .unwrap();
    }
    fixture
        .app
        .services
        .catalog
        .titles
        .update_monitored(&title_id, false)
        .await
        .unwrap();
    // A raw score of 90 is deliberately enough here; threshold-specific tests keep the default.
    fixture
        .app
        .upsert_system_setting_json("subtitles.minimum_score_series", &0, None)
        .await
        .unwrap();
    (fixture, title_id, episodes)
}

async fn wait_subtitle_batch(
    receiver: &mut tokio::sync::broadcast::Receiver<crate::JobRun>,
    id: &str,
) -> crate::JobRun {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let run = receiver.recv().await.expect("job event");
            if run.id == id && run.completed_at.is_some() {
                return run;
            }
        }
    })
    .await
    .expect("subtitle batch should finish")
}

#[tokio::test]
async fn subtitle_batch_accepts_without_waiting_and_deduplicates_shared_files() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    let app = &fixture.app;
    let guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .acquire(&format!("subtitle-file:{}", fixture.series_media_file_id))
        .await;
    let next = Utc::now() + chrono::Duration::hours(6);
    app.set_job_next_run_at(crate::JobKey::SubtitleSearch, next)
        .await;
    let mut events = app.runtime.jobs.job_run_tracker.subscribe();
    let mut selected = episodes.clone();
    selected.push(episodes[0].clone());
    let accepted = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        app.start_episode_subtitle_search(&fixture.admin, &title_id, selected),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        accepted.actor_user_id.as_deref(),
        Some(fixture.admin.id.as_str())
    );
    assert_eq!(accepted.status, crate::JobRunStatus::Running);
    assert!(
        app.start_episode_subtitle_search(&fixture.admin, &title_id, episodes.clone())
            .await
            .is_err()
    );
    assert_eq!(
        app.runtime
            .jobs
            .job_run_tracker
            .next_run_at(crate::JobKey::SubtitleSearch)
            .await,
        Some(next)
    );
    drop(guard);
    let done = wait_subtitle_batch(&mut events, &accepted.id).await;
    assert_eq!(done.status, crate::JobRunStatus::Completed);
    let summary: serde_json::Value =
        serde_json::from_str(done.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["total"], 2); // one shared file, one episode without a file
    assert_eq!(summary["downloaded"], 1); // Swedish missing; English already on disk
    assert_eq!(summary["items"][0]["status"], "missing_media");
    let rows = fixture.subtitle_downloads.rows.lock().await;
    assert_eq!(
        rows.iter()
            .filter(|row| row.media_file_id == fixture.series_media_file_id)
            .count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.media_file_id == fixture.movie_media_file_id)
            .count(),
        1
    );
    drop(rows);
    let retry = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes[..2].to_vec())
        .await
        .unwrap();
    let retry = wait_subtitle_batch(&mut events, &retry.id).await;
    let summary: serde_json::Value =
        serde_json::from_str(retry.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["items"][0]["status"], "already_satisfied");
    assert_eq!(summary["downloaded"], 0);
}

#[tokio::test]
async fn subtitle_batch_validates_selection_and_permissions_before_acceptance() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    let app = &fixture.app;
    for ids in [
        vec![],
        vec!["missing-episode".into()],
        vec![episodes[0].clone(); 1001],
    ] {
        let error = app
            .start_episode_subtitle_search(&fixture.admin, &title_id, ids)
            .await
            .unwrap_err();
        assert_eq!(
            error.validation_reason(),
            Some(crate::error_reasons::subtitles::BATCH_SELECTION_INVALID)
        );
    }
    for actor in [
        viewer_for(&fixture.series_library_id),
        subtitle_manager_for(&fixture.movie_library_id),
    ] {
        assert_unauthorized(
            app.start_episode_subtitle_search(&actor, &title_id, episodes.clone())
                .await,
            "batch scope",
        );
    }
    app.upsert_system_setting_json("subtitles.enabled", &false, None)
        .await
        .unwrap();
    let error = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes.clone())
        .await
        .unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(crate::error_reasons::subtitles::BATCH_DISABLED)
    );
    app.upsert_system_setting_json("subtitles.enabled", &true, None)
        .await
        .unwrap();
    app.upsert_system_setting_json("subtitles.languages", &Vec::<String>::new(), None)
        .await
        .unwrap();
    assert_eq!(
        app.start_episode_subtitle_search(&fixture.admin, &title_id, episodes)
            .await
            .unwrap_err()
            .validation_reason(),
        Some(crate::error_reasons::subtitles::BATCH_LANGUAGES_REQUIRED)
    );
    assert!(
        app.active_job_runs(&fixture.admin)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn subtitle_batch_rechecks_revoked_permissions_after_waiting_for_a_file() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    let app = &fixture.app;
    let actor = subtitle_manager_for(&fixture.series_library_id);
    let grants = actor
        .authorization
        .libraries
        .iter()
        .map(|(library_id, permissions)| scryer_domain::LibraryGrant {
            user_id: actor.id.clone(),
            library_id: library_id.clone(),
            permissions: *permissions,
        })
        .collect();
    app.services
        .catalog
        .libraries
        .set_grants_for_user(&actor.id, grants)
        .await
        .unwrap();
    let guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .acquire(&format!("subtitle-file:{}", fixture.series_media_file_id))
        .await;
    let mut events = app.runtime.jobs.job_run_tracker.subscribe();
    let run = app
        .start_episode_subtitle_search(&actor, &title_id, episodes[..2].to_vec())
        .await
        .unwrap();
    // History and subscription reconciliation must work for a subtitle-only actor.
    assert_eq!(
        app.list_job_runs(&actor, crate::JobKey::SubtitleSearch, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    app.services
        .catalog
        .libraries
        .set_grants_for_user(&actor.id, vec![])
        .await
        .unwrap();
    drop(guard);
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    assert_eq!(done.status, crate::JobRunStatus::Failed);
    assert_eq!(fixture.subtitle_downloads.rows.lock().await.len(), 2);
}

#[tokio::test]
async fn subtitle_batch_reports_no_match_and_provider_failure_without_touching_other_files() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    let app = &fixture.app;
    let mut events = app.runtime.jobs.job_run_tracker.subscribe();
    app.upsert_system_setting_json("subtitles.minimum_score_series", &100, None)
        .await
        .unwrap();
    let run = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes.clone())
        .await
        .unwrap();
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    let summary: serde_json::Value =
        serde_json::from_str(done.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["items"][1]["status"], "no_match");
    let app = app.with_test_overrides(|services| {
        services.with_subtitle_plugin_provider(Arc::new(StubSubtitlePluginProvider {
            instantiates: false,
            control: Default::default(),
        }))
    });
    let run = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes)
        .await
        .unwrap();
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    assert_eq!(done.status, crate::JobRunStatus::Warning); // missing file plus provider failure
    assert_eq!(fixture.subtitle_downloads.rows.lock().await.len(), 2);
}

#[tokio::test]
async fn subtitle_batch_shutdown_interrupts_wait_and_preserves_retryability() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    let app = &fixture.app;
    let guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .acquire(&format!("subtitle-file:{}", fixture.series_media_file_id))
        .await;
    let mut events = app.runtime.jobs.job_run_tracker.subscribe();
    let run = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes)
        .await
        .unwrap();
    app.restore_coordinator().shutdown_workers.cancel();
    drop(guard);
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    assert_eq!(done.status, crate::JobRunStatus::Failed);
    assert_eq!(fixture.subtitle_downloads.rows.lock().await.len(), 2);
}

#[tokio::test]
async fn subtitle_batch_rechecks_current_settings_and_file_state_after_waiting() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    let app = &fixture.app;
    let mut events = app.runtime.jobs.job_run_tracker.subscribe();
    let guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .acquire(&format!("subtitle-file:{}", fixture.series_media_file_id))
        .await;
    let run = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes[..2].to_vec())
        .await
        .unwrap();
    app.upsert_system_setting_json(
        "subtitles.languages",
        &vec![crate::subtitles::wanted::SubtitleLanguagePref {
            code: "eng".into(),
            forced: false,
            hearing_impaired: false,
        }],
        None,
    )
    .await
    .unwrap();
    drop(guard);
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    let summary: serde_json::Value =
        serde_json::from_str(done.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["items"][0]["status"], "already_satisfied");
    assert_eq!(fixture.subtitle_downloads.rows.lock().await.len(), 2);

    let file = app
        .services
        .library
        .media_files
        .get_media_file_by_id(&fixture.series_media_file_id)
        .await
        .unwrap()
        .unwrap();
    // Only this fixture's synthetic file is removed; the worker must preserve every other file.
    std::fs::remove_file(crate::stored_paths::stored_path_to_path_buf(
        &file.file_path,
    ))
    .unwrap();
    let run = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes[..2].to_vec())
        .await
        .unwrap();
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    let summary: serde_json::Value =
        serde_json::from_str(done.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["items"][0]["status"], "missing_media");
}

#[tokio::test]
async fn subtitle_batch_applies_forced_preferences() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    let app = &fixture.app;
    let mut events = app.runtime.jobs.job_run_tracker.subscribe();
    app.upsert_system_setting_json(
        "subtitles.languages",
        &vec![crate::subtitles::wanted::SubtitleLanguagePref {
            code: "swe".into(),
            forced: true,
            hearing_impaired: false,
        }],
        None,
    )
    .await
    .unwrap();
    let run = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes[..2].to_vec())
        .await
        .unwrap();
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    let summary: serde_json::Value =
        serde_json::from_str(done.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["items"][0]["status"], "no_match");
    assert_eq!(fixture.subtitle_downloads.rows.lock().await.len(), 2);
}

#[tokio::test]
async fn subtitle_batch_honors_blocklisted_provider_files() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    fixture
        .app
        .services
        .workflow
        .subtitle_downloads
        .blocklist(
            &fixture.series_media_file_id,
            STUB_PROVIDER_TYPE,
            "stub-file-1",
            "swe",
            None,
        )
        .await
        .unwrap();
    let mut events = fixture.app.runtime.jobs.job_run_tracker.subscribe();
    let run = fixture
        .app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes[..2].to_vec())
        .await
        .unwrap();
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    let summary: serde_json::Value =
        serde_json::from_str(done.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["items"][0]["status"], "no_match");
    assert_eq!(fixture.subtitle_downloads.rows.lock().await.len(), 2);
}

#[cfg(feature = "runtime-media-analysis")]
#[tokio::test]
async fn subtitle_batch_reports_sync_warning_when_configured_engine_is_unavailable() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    fixture
        .app
        .upsert_system_setting_json("subtitles.sync_enabled", &true, None)
        .await
        .unwrap();
    fixture
        .app
        .upsert_system_setting_json("subtitles.sync_threshold_series", &100, None)
        .await
        .unwrap();
    let mut events = fixture.app.runtime.jobs.job_run_tracker.subscribe();
    let run = fixture
        .app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes[..2].to_vec())
        .await
        .unwrap();
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    assert_eq!(done.status, crate::JobRunStatus::Warning);
    let summary: serde_json::Value =
        serde_json::from_str(done.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["downloaded"], 1);
    assert_eq!(summary["items"][0]["sync_warnings"], 1);
    assert_eq!(
        summary["items"][0]["sync_reasons"][0],
        "subtitle_sync_plugin_required"
    );
}

#[tokio::test]
async fn subtitle_batch_and_periodic_cycle_share_file_deduplication() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    let app = &fixture.app;
    let movie = app
        .services
        .library
        .media_files
        .get_media_file_by_id(&fixture.movie_media_file_id)
        .await
        .unwrap()
        .unwrap();
    app.services
        .catalog
        .titles
        .update_monitored(&movie.title_id, false)
        .await
        .unwrap();
    app.services
        .catalog
        .titles
        .update_monitored(&title_id, true)
        .await
        .unwrap();
    let guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .acquire(&format!("subtitle-file:{}", fixture.series_media_file_id))
        .await;
    let mut events = app.runtime.jobs.job_run_tracker.subscribe();
    let run = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes[..2].to_vec())
        .await
        .unwrap();
    let periodic_app = app.clone();
    let periodic = tokio::spawn(async move { periodic_app.run_subtitle_search_job().await });
    drop(guard);
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    assert_eq!(done.status, crate::JobRunStatus::Completed);
    tokio::time::timeout(std::time::Duration::from_secs(60), periodic)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let rows = fixture.subtitle_downloads.rows.lock().await;
    assert_eq!(rows.iter().filter(|row| row.media_file_id == fixture.series_media_file_id && row.language == "swe").count(), 1);
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn subtitle_batch_interrupted_record_uses_standard_recovery() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    let guard = fixture
        .app
        .runtime
        .jobs
        .interactive_operation_guards
        .acquire(&format!("subtitle-file:{}", fixture.series_media_file_id))
        .await;
    let mut events = fixture.app.runtime.jobs.job_run_tracker.subscribe();
    let run = fixture
        .app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes[..2].to_vec())
        .await
        .unwrap();
    let snapshot = fixture
        .app
        .services
        .events
        .job_runs
        .list_job_runs(Some(crate::JobKey::SubtitleSearch), 10)
        .await
        .unwrap()
        .into_iter()
        .find(|record| record.id == run.id)
        .unwrap();
    assert_eq!(snapshot.status, crate::JobRunStatus::Running);
    fixture.app.restore_coordinator().shutdown_workers.cancel();
    drop(guard);
    wait_subtitle_batch(&mut events, &run.id).await;

    let (restarted, _, _) = seed_subtitle_batch().await;
    let jobs = Arc::new(RecordingJobRunRepo::default());
    jobs.seed(snapshot).await;
    let app = restarted
        .app
        .with_test_overrides(|services| services.with_job_runs(jobs));
    assert_eq!(app.reconcile_interrupted_job_runs(&[]).await.unwrap(), 1);
    let recovered = app
        .services
        .events
        .job_runs
        .list_job_runs(Some(crate::JobKey::SubtitleSearch), 10)
        .await
        .unwrap();
    assert_eq!(recovered[0].id, run.id);
    assert_eq!(recovered[0].status, crate::JobRunStatus::Failed);
    assert!(recovered[0].completed_at.is_some());
}

#[tokio::test]
async fn subtitle_batch_periodic_unavailable_provider_stops_before_library_walk() {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let configs = Arc::new(StubSubtitleProviderConfigRepo::default());
    let app = fixture
        .app
        .with_test_overrides(|services| services.with_subtitle_provider_configs(configs.clone()));
    app.run_subtitle_search_job().await.unwrap();
    assert_eq!(
        configs
            .list_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[tokio::test]
async fn subtitle_batch_reloads_provider_facets_between_files() {
    let (mut fixture, title_id, episodes) = seed_subtitle_batch().await;
    let configs = Arc::new(StubSubtitleProviderConfigRepo::with_configs(vec![
        stub_provider_config(),
    ]));
    fixture.app = fixture
        .app
        .with_test_overrides(|services| services.with_subtitle_provider_configs(configs.clone()));
    let path = fixture._tempdir.path().join("second.mkv");
    std::fs::write(&path, b"video").unwrap();
    let app = &fixture.app;
    let second = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.clone(),
            file_path: path.to_string_lossy().into_owned(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .unwrap();
    app.services
        .library
        .media_files
        .link_file_to_episode(&second, &episodes[2])
        .await
        .unwrap();
    let mut files = [fixture.series_media_file_id.clone(), second];
    files.sort();
    let guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .acquire(&format!("subtitle-file:{}", files[1]))
        .await;
    let mut events = app.runtime.jobs.job_run_tracker.subscribe();
    let run = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let progress = events.recv().await.unwrap();
            if progress.id == run.id
                && progress.progress_json.as_ref().is_some_and(|json| {
                    serde_json::from_str::<serde_json::Value>(json).unwrap()["completed"] == 1
                })
            {
                break;
            }
        }
    })
    .await
    .expect("first file should finish");
    configs.configs.lock().await[0].enabled_facets = vec!["movie".into()];
    drop(guard);
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    let summary: serde_json::Value =
        serde_json::from_str(done.summary_json.as_ref().unwrap()).unwrap();
    assert!(summary["items"][0]["downloaded"].as_u64().unwrap() > 0);
    assert_eq!(summary["items"][1]["downloaded"], 0);
    assert_eq!(summary["items"][1]["status"], "no_match");
}

// ── "Nothing found" backoff ───────────────────────────────────────────────

fn backoff_epoch() -> chrono::DateTime<Utc> {
    "2026-01-05T12:00:00Z".parse().unwrap()
}

fn wanted(code: &str) -> crate::subtitles::wanted::SubtitleLanguagePref {
    crate::subtitles::wanted::SubtitleLanguagePref {
        code: code.to_string(),
        hearing_impaired: false,
        forced: false,
    }
}

/// The ready fixture with only the movie walked by the periodic cycle; its file
/// has English and is missing Swedish.
async fn seed_backoff_fixture() -> SubtitleFixture {
    let fixture = seed_fixture(true, StubProviders::Ready).await;
    let series = fixture
        .app
        .services
        .library
        .media_files
        .get_media_file_by_id(&fixture.series_media_file_id)
        .await
        .unwrap()
        .unwrap();
    fixture
        .app
        .services
        .catalog
        .titles
        .update_monitored(&series.title_id, false)
        .await
        .unwrap();
    fixture
}

impl SubtitleFixture {
    fn at(&self, offset: chrono::Duration) {
        self.app
            .runtime
            .environment
            .set_fixed_now_for_tests(Some(backoff_epoch() + offset));
    }

    async fn backoffs_for(
        &self,
        media_file_id: &str,
    ) -> Vec<crate::subtitles::SubtitleSearchBackoff> {
        let mut rows = self
            .subtitle_downloads
            .search_backoffs
            .lock()
            .await
            .iter()
            .filter(|row| row.media_file_id == media_file_id)
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.language.cmp(&right.language));
        rows
    }

    fn backoff_writes(&self) -> usize {
        self.subtitle_downloads
            .search_backoff_writes
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn set_languages(&self, codes: &[&str]) {
        self.app
            .upsert_system_setting_json(
                "subtitles.languages",
                &codes.iter().map(|code| wanted(code)).collect::<Vec<_>>(),
                None,
            )
            .await
            .unwrap();
    }
}

fn retry_after(row: &crate::subtitles::SubtitleSearchBackoff) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(&row.retry_after)
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn automatic_subtitle_search_waits_longer_after_each_empty_search_up_to_thirty_days() {
    let fixture = seed_backoff_fixture().await;
    let app = &fixture.app;
    let file = fixture.movie_media_file_id.clone();
    fixture.search_control.find_only(&[]);

    fixture.at(chrono::Duration::zero());
    app.run_subtitle_search_job().await.unwrap();
    assert_eq!(
        fixture.search_control.searches(),
        vec![("Lumen Drift".to_string(), "swe".to_string())]
    );

    let mut searched_at = chrono::Duration::zero();
    let mut searches = 1;
    for (empty_searches, delay_days) in [(1, 1), (2, 3), (3, 7), (4, 14), (5, 30), (6, 30), (7, 30)]
    {
        let rows = fixture.backoffs_for(&file).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].language, "swe");
        assert_eq!(rows[0].empty_searches, empty_searches);
        let delay = chrono::Duration::days(delay_days);
        assert_eq!(retry_after(&rows[0]), backoff_epoch() + searched_at + delay);

        // A tick before the delay passes searches nothing and writes nothing.
        let writes = fixture.backoff_writes();
        fixture.at(searched_at + delay - chrono::Duration::minutes(1));
        app.run_subtitle_search_job().await.unwrap();
        assert_eq!(fixture.search_control.searches().len(), searches);
        assert_eq!(fixture.backoff_writes(), writes);
        assert_eq!(fixture.backoffs_for(&file).await, rows);

        // Once it has passed, the next tick searches again and records one write.
        searched_at = searched_at + delay;
        fixture.at(searched_at);
        app.run_subtitle_search_job().await.unwrap();
        searches += 1;
        assert_eq!(fixture.search_control.searches().len(), searches);
        assert_eq!(fixture.backoff_writes(), writes + 1);
    }
    assert!(
        fixture
            .subtitle_downloads
            .rows
            .lock()
            .await
            .iter()
            .all(|row| row.language == "eng")
    );
}

#[tokio::test]
async fn subtitle_backoff_is_per_file_and_per_language_and_resets_when_found() {
    let fixture = seed_backoff_fixture().await;
    fixture.set_languages(&["swe", "fin", "eng"]).await;
    fixture.search_control.find_only(&[]);
    let first = fixture.movie_media_file_id.clone();

    fixture.at(chrono::Duration::zero());
    fixture.app.run_subtitle_search_job().await.unwrap();
    let rows = fixture.backoffs_for(&first).await;
    assert_eq!(
        rows.iter()
            .map(|row| (row.language.as_str(), row.empty_searches))
            .collect::<Vec<_>>(),
        vec![("fin", 1), ("swe", 1)]
    );
    assert_eq!(
        fixture.backoff_writes(),
        1,
        "one write for the file's languages"
    );

    // A new file of the same title is searched while the first one waits.
    let movie = fixture
        .app
        .services
        .library
        .media_files
        .get_media_file_by_id(&first)
        .await
        .unwrap()
        .unwrap();
    let second_path =
        std::path::Path::new(&movie.file_path).with_file_name("Lumen Drift Part Two.mkv");
    std::fs::write(&second_path, b"video").unwrap();
    let second = fixture
        .app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: movie.title_id.clone(),
            file_path: second_path.to_string_lossy().into_owned(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .unwrap();
    fixture.at(chrono::Duration::hours(12));
    fixture.app.run_subtitle_search_job().await.unwrap();
    let mut asked = fixture.search_control.searches()[2..]
        .iter()
        .map(|(_, language)| language.clone())
        .collect::<Vec<_>>();
    asked.sort();
    assert_eq!(
        asked,
        vec!["eng", "fin", "swe"],
        "only the new file is searched, for every language it lacks"
    );
    assert_eq!(fixture.backoffs_for(&first).await, rows);
    assert_eq!(fixture.backoffs_for(&second).await.len(), 3);
    // Keep the rest of the test on the first file.
    fixture
        .subtitle_downloads
        .search_backoffs
        .lock()
        .await
        .iter_mut()
        .filter(|row| row.media_file_id == second)
        .for_each(|row| {
            row.retry_after = (backoff_epoch() + chrono::Duration::days(365)).to_rfc3339()
        });

    // A day later Finnish turns up: it downloads and its row goes; Swedish waits longer.
    fixture.search_control.find_only(&["fin"]);
    fixture.at(chrono::Duration::days(1));
    fixture.app.run_subtitle_search_job().await.unwrap();
    let rows = fixture.backoffs_for(&first).await;
    assert_eq!(
        rows.iter()
            .map(|row| (row.language.as_str(), row.empty_searches))
            .collect::<Vec<_>>(),
        vec![("swe", 2)]
    );
    assert_eq!(
        retry_after(&rows[0]),
        backoff_epoch() + chrono::Duration::days(1 + 3)
    );
    assert!(
        fixture
            .subtitle_downloads
            .rows
            .lock()
            .await
            .iter()
            .any(|row| row.media_file_id == first && row.language == "fin")
    );

    // Finnish goes missing again: the next empty search starts the count over.
    fixture
        .subtitle_downloads
        .rows
        .lock()
        .await
        .retain(|row| !(row.media_file_id == first && row.language == "fin"));
    fixture.search_control.find_only(&[]);
    fixture.at(chrono::Duration::days(2));
    fixture.app.run_subtitle_search_job().await.unwrap();
    let rows = fixture.backoffs_for(&first).await;
    assert_eq!(
        rows.iter()
            .map(|row| (row.language.as_str(), row.empty_searches))
            .collect::<Vec<_>>(),
        vec![("fin", 1), ("swe", 2)]
    );
    assert_eq!(
        retry_after(&rows[0]),
        backoff_epoch() + chrono::Duration::days(2 + 1)
    );
}

#[tokio::test]
async fn subtitle_backoff_stops_applying_when_the_file_or_wanted_languages_change() {
    let fixture = seed_backoff_fixture().await;
    let app = &fixture.app;
    let file = fixture.movie_media_file_id.clone();
    fixture.search_control.find_only(&[]);

    fixture.at(chrono::Duration::zero());
    app.run_subtitle_search_job().await.unwrap();
    fixture.at(chrono::Duration::days(1));
    app.run_subtitle_search_job().await.unwrap();
    assert_eq!(fixture.backoffs_for(&file).await[0].empty_searches, 2);

    // The wanted-language list changes: the old row no longer holds Swedish back.
    fixture.set_languages(&["eng", "swe", "fin"]).await;
    fixture.at(chrono::Duration::days(1) + chrono::Duration::hours(1));
    app.run_subtitle_search_job().await.unwrap();
    let rows = fixture.backoffs_for(&file).await;
    assert_eq!(
        rows.iter()
            .map(|row| (row.language.as_str(), row.empty_searches))
            .collect::<Vec<_>>(),
        vec![("fin", 1), ("swe", 1)]
    );
    assert_eq!(fixture.search_control.searches().len(), 4);

    // The file is replaced at another path: both languages are searched again.
    let movie = app
        .services
        .library
        .media_files
        .get_media_file_by_id(&file)
        .await
        .unwrap()
        .unwrap();
    let replaced = std::path::Path::new(&movie.file_path).with_file_name("Lumen Drift Remux.mkv");
    app.services
        .library
        .media_files
        .update_media_file_path(&file, &replaced.to_string_lossy())
        .await
        .unwrap();
    fixture.at(chrono::Duration::days(1) + chrono::Duration::hours(2));
    app.run_subtitle_search_job().await.unwrap();
    assert_eq!(fixture.search_control.searches().len(), 6);
    let rows = fixture.backoffs_for(&file).await;
    assert!(
        rows.iter()
            .all(|row| row.empty_searches == 1 && row.file_path == replaced.to_string_lossy())
    );
}

#[tokio::test]
async fn operator_subtitle_search_ignores_the_backoff_and_clears_it_on_download() {
    let (fixture, title_id, episodes) = seed_subtitle_batch().await;
    let app = &fixture.app;
    let movie = app
        .services
        .library
        .media_files
        .get_media_file_by_id(&fixture.movie_media_file_id)
        .await
        .unwrap()
        .unwrap();
    app.services
        .catalog
        .titles
        .update_monitored(&movie.title_id, false)
        .await
        .unwrap();
    app.services
        .catalog
        .titles
        .update_monitored(&title_id, true)
        .await
        .unwrap();
    let series_file = app
        .services
        .library
        .media_files
        .get_media_file_by_id(&fixture.series_media_file_id)
        .await
        .unwrap()
        .unwrap();
    fixture.at(chrono::Duration::zero());
    let key =
        crate::subtitles::search_backoff::wanted_languages_key(&[wanted("swe"), wanted("eng")]);
    let mut held = crate::subtitles::SubtitleSearchBackoff::after_empty_search(
        None,
        &series_file,
        &wanted("swe"),
        &key,
        &crate::subtitles::search_backoff::enabled_providers_key(["stub-config"]),
        backoff_epoch(),
    );
    held.empty_searches = 4;
    held.retry_after = (backoff_epoch() + chrono::Duration::days(14)).to_rfc3339();
    fixture
        .subtitle_downloads
        .search_backoffs
        .lock()
        .await
        .push(held);

    // The periodic cycle leaves the file alone.
    app.run_subtitle_search_job().await.unwrap();
    assert!(fixture.search_control.searches().is_empty());

    let mut events = app.runtime.jobs.job_run_tracker.subscribe();
    let run = app
        .start_episode_subtitle_search(&fixture.admin, &title_id, episodes[..2].to_vec())
        .await
        .unwrap();
    let done = wait_subtitle_batch(&mut events, &run.id).await;
    assert_eq!(done.status, crate::JobRunStatus::Completed);
    assert_eq!(
        fixture
            .search_control
            .searches()
            .iter()
            .filter(|(_, language)| language == "swe")
            .count(),
        1
    );
    assert!(
        fixture
            .backoffs_for(&fixture.series_media_file_id)
            .await
            .is_empty()
    );
    assert!(
        fixture
            .subtitle_downloads
            .rows
            .lock()
            .await
            .iter()
            .any(|row| row.media_file_id == fixture.series_media_file_id && row.language == "swe")
    );
}

#[tokio::test]
async fn subtitle_backoff_resets_when_the_providers_asked_change_but_not_the_minimum_score() {
    let mut fixture = seed_backoff_fixture().await;
    let configs = Arc::new(StubSubtitleProviderConfigRepo::with_configs(vec![
        stub_provider_config(),
    ]));
    fixture.app = fixture
        .app
        .with_test_overrides(|services| services.with_subtitle_provider_configs(configs.clone()));
    let app = &fixture.app;
    let file = fixture.movie_media_file_id.clone();
    fixture.search_control.find_only(&[]);

    fixture.at(chrono::Duration::zero());
    app.run_subtitle_search_job().await.unwrap();
    assert_eq!(fixture.search_control.searches().len(), 1);
    let held = fixture.backoffs_for(&file).await;
    assert_eq!(held[0].empty_searches, 1);

    // Only the minimum score changes: the file stays backed off, and the tick
    // searches nothing and writes nothing.
    app.upsert_system_setting_json("subtitles.minimum_score_movie", &10, None)
        .await
        .unwrap();
    let writes = fixture.backoff_writes();
    fixture.at(chrono::Duration::hours(1));
    app.run_subtitle_search_job().await.unwrap();
    assert_eq!(fixture.search_control.searches().len(), 1);
    assert_eq!(fixture.backoff_writes(), writes);
    assert_eq!(fixture.backoffs_for(&file).await, held);

    // A provider is added: the next tick asks both providers again.
    let mut ashgrove = stub_provider_config();
    ashgrove.id = "ashgrove-config".to_string();
    ashgrove.name = "Ashgrove subtitles".to_string();
    configs.configs.lock().await.push(ashgrove);
    fixture.at(chrono::Duration::hours(2));
    app.run_subtitle_search_job().await.unwrap();
    assert_eq!(fixture.search_control.searches().len(), 3);
    let rows = fixture.backoffs_for(&file).await;
    assert_eq!(rows[0].empty_searches, 1);
    assert_eq!(rows[0].enabled_providers, "ashgrove-config,stub-config");

    // Held again under the new set: an idle tick does nothing.
    let writes = fixture.backoff_writes();
    fixture.at(chrono::Duration::hours(3));
    app.run_subtitle_search_job().await.unwrap();
    assert_eq!(fixture.search_control.searches().len(), 3);
    assert_eq!(fixture.backoff_writes(), writes);

    // A provider is removed: the next tick searches again.
    configs
        .configs
        .lock()
        .await
        .retain(|config| config.id != "ashgrove-config");
    fixture.at(chrono::Duration::hours(4));
    app.run_subtitle_search_job().await.unwrap();
    assert_eq!(fixture.search_control.searches().len(), 4);
    let rows = fixture.backoffs_for(&file).await;
    assert_eq!(rows[0].empty_searches, 1);
    assert_eq!(rows[0].enabled_providers, "stub-config");
}
