//! An entry recycled because an upgrade replaced it is purged only while the
//! library still holds a live file for what it covered, and never while the
//! catalog says its original path or row belongs to something else. Any other
//! entry is quarantined and its file kept.

use super::*;

const ORIGINAL_ID: &str = "synthetic-file-a";
const REPLACEMENT_ID: &str = "synthetic-file-b";

struct UpgradeFixture {
    temp: tempfile::TempDir,
    app: AppUseCase,
    user: User,
    media_files: Arc<MockMediaFileRepo>,
    title: Title,
}

impl UpgradeFixture {
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
            user,
            media_files,
            title,
        }
    }

    fn root(&self) -> PathBuf {
        self.temp.path().join("library-a")
    }

    fn library_path(&self, name: &str) -> PathBuf {
        self.root().join("Lanternfall (2001)").join(name)
    }

    /// A library file on disk at `name` with a catalog row of `title_id`.
    async fn live_file(&self, id: &str, title_id: &str, name: &str) -> PathBuf {
        let path = self.library_path(name);
        std::fs::write(&path, format!("{id} bytes")).expect("write library file");
        self.row(id, title_id, &path).await;
        path
    }

    async fn row(&self, id: &str, title_id: &str, path: &Path) {
        self.media_files.store.lock().await.push(TitleMediaFile {
            id: id.to_string(),
            title_id: title_id.to_string(),
            file_path: path.to_string_lossy().to_string(),
            role: crate::MediaFileRole::Primary,
            ..Default::default()
        });
    }

    /// Recycles the library file `name` as replaced by an upgrade from
    /// `original_id` to `replacement`, and returns the recycled file.
    async fn upgrade_entry(
        &self,
        name: &str,
        original_id: &str,
        replacement: Option<(&str, &Path)>,
    ) -> PathBuf {
        let root = self.root();
        let source = self.library_path(name);
        std::fs::write(&source, format!("{name} replaced bytes")).expect("write original");
        self.recycle(
            &source,
            crate::recycle_bin::RecycleManifest {
                schema: None,
                entry_id: None,
                source_operation_id: None,
                recycled_at: Utc::now().to_rfc3339(),
                original_path: source.to_string_lossy().to_string(),
                original_file_id: Some(original_id.to_string()),
                size_bytes: 0,
                title_id: Some(self.title.id.clone()),
                media_root: Some(root.to_string_lossy().to_string()),
                reason: "upgrade_replaced".to_string(),
                status: None,
                replacement_file_id: replacement.map(|(id, _)| id.to_string()),
                replacement_path: replacement.map(|(_, path)| path.to_string_lossy().to_string()),
                episode_ids: None,
                media_row: None,
            },
        )
        .await
    }

    async fn removal_entry(&self, name: &str) -> PathBuf {
        let source = self.library_path(name);
        std::fs::write(&source, format!("{name} removed bytes")).expect("write removed file");
        self.recycle(
            &source,
            crate::recycle_bin::RecycleManifest {
                schema: None,
                entry_id: None,
                source_operation_id: None,
                recycled_at: Utc::now().to_rfc3339(),
                original_path: source.to_string_lossy().to_string(),
                original_file_id: None,
                size_bytes: 0,
                title_id: Some(self.title.id.clone()),
                media_root: None,
                reason: "file_deleted".to_string(),
                status: None,
                replacement_file_id: None,
                replacement_path: None,
                episode_ids: None,
                media_row: None,
            },
        )
        .await
    }

    async fn recycle(
        &self,
        source: &Path,
        manifest: crate::recycle_bin::RecycleManifest,
    ) -> PathBuf {
        let root = self.root();
        let config = crate::recycle_bin::RecycleBinConfig {
            enabled: true,
            base_path: root.join(".scryer-recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![root],
        };
        crate::recycle_bin::recycle_file(
            &config,
            source,
            manifest,
            crate::location::model::VerificationDepth::Full,
        )
        .await
        .expect("seed recycle entry")
        .expect("file recycled")
        .recycled_path
    }

    /// Purges one entry the way the recycle page does: `true` when it was
    /// purged, `false` when validation quarantined it.
    async fn purge(&self, recycled: &Path) -> bool {
        let id = recycled
            .parent()
            .and_then(Path::file_name)
            .expect("entry directory")
            .to_string_lossy()
            .to_string();
        self.app
            .delete_recycled_item(&self.user, &id)
            .await
            .expect("purge runs")
    }
}

fn status(recycled: &Path) -> Option<String> {
    let manifest = recycled.parent().unwrap().join("manifest.json");
    std::fs::read(manifest).ok().map(|bytes| {
        serde_json::from_slice::<crate::recycle_bin::RecycleManifest>(&bytes)
            .expect("decode manifest")
            .status
            .unwrap_or_default()
    })
}

fn assert_quarantined_and_kept(recycled: &Path) {
    assert_eq!(
        status(recycled).as_deref(),
        Some(crate::recycle_bin::RECYCLE_STATUS_QUARANTINED)
    );
    assert!(recycled.exists(), "the recycled file is kept");
}

/// A → B → C: once B is replaced by C, B's row is gone. A's entry and B's
/// entry both purge while C is live on disk.
#[tokio::test]
async fn an_entry_of_an_upgrade_chain_purges_while_the_last_file_is_live() {
    let fixture = UpgradeFixture::new().await;
    let title_id = fixture.title.id.clone();
    let b_path = fixture.library_path("Lanternfall.720p.mkv");
    let a_entry = fixture
        .upgrade_entry(
            "Lanternfall.480p.mkv",
            ORIGINAL_ID,
            Some((REPLACEMENT_ID, &b_path)),
        )
        .await;
    let c_path = fixture
        .live_file("synthetic-file-c", &title_id, "Lanternfall.1080p.mkv")
        .await;
    let b_entry = fixture
        .upgrade_entry(
            "Lanternfall.720p.mkv",
            REPLACEMENT_ID,
            Some(("synthetic-file-c", &c_path)),
        )
        .await;

    assert!(fixture.purge(&a_entry).await);
    assert!(fixture.purge(&b_entry).await);
    assert!(!a_entry.exists() && !b_entry.exists());
    assert!(c_path.exists(), "the live file is untouched");
}

#[tokio::test]
async fn an_entry_whose_replacement_was_renamed_purges() {
    let fixture = UpgradeFixture::new().await;
    let title_id = fixture.title.id.clone();
    let recorded = fixture.library_path("Lanternfall.1080p.mkv");
    let entry = fixture
        .upgrade_entry(
            "Lanternfall.720p.mkv",
            ORIGINAL_ID,
            Some((REPLACEMENT_ID, &recorded)),
        )
        .await;
    let renamed = fixture
        .live_file(
            REPLACEMENT_ID,
            &title_id,
            "Lanternfall (2001) Bluray-1080p.mkv",
        )
        .await;

    assert!(fixture.purge(&entry).await);
    assert!(renamed.exists());
}

/// The replacement left the same path the original had, and a later file of
/// the same title now holds that path.
#[tokio::test]
async fn an_entry_whose_original_path_holds_a_later_file_of_the_title_purges() {
    let fixture = UpgradeFixture::new().await;
    let title_id = fixture.title.id.clone();
    let name = "Lanternfall.mkv";
    let entry = fixture
        .upgrade_entry(
            name,
            ORIGINAL_ID,
            Some((REPLACEMENT_ID, &fixture.library_path(name))),
        )
        .await;
    fixture.live_file("synthetic-file-c", &title_id, name).await;

    assert!(fixture.purge(&entry).await);
}

#[tokio::test]
async fn an_entry_with_no_live_file_for_its_movie_is_quarantined() {
    let fixture = UpgradeFixture::new().await;
    let gone = fixture.library_path("Lanternfall.1080p.mkv");
    let entry = fixture
        .upgrade_entry(
            "Lanternfall.720p.mkv",
            ORIGINAL_ID,
            Some((REPLACEMENT_ID, &gone)),
        )
        .await;
    // A catalog row whose file is not on disk does not count.
    fixture
        .row(
            "synthetic-file-c",
            &fixture.title.id,
            &fixture.library_path("Lanternfall.2160p.mkv"),
        )
        .await;

    assert!(!fixture.purge(&entry).await);
    assert_quarantined_and_kept(&entry);
}

/// Missing replacement fields are treated as a replacement that is gone.
#[tokio::test]
async fn an_entry_without_replacement_fields_follows_the_live_movie_file() {
    let fixture = UpgradeFixture::new().await;
    let title_id = fixture.title.id.clone();
    let entry = fixture
        .upgrade_entry("Lanternfall.720p.mkv", ORIGINAL_ID, None)
        .await;
    fixture
        .live_file("synthetic-file-c", &title_id, "Lanternfall.1080p.mkv")
        .await;

    assert!(fixture.purge(&entry).await);
}

#[tokio::test]
async fn an_entry_whose_original_row_is_still_active_is_quarantined() {
    let fixture = UpgradeFixture::new().await;
    let title_id = fixture.title.id.clone();
    let replacement = fixture
        .live_file(REPLACEMENT_ID, &title_id, "Lanternfall.1080p.mkv")
        .await;
    let entry = fixture
        .upgrade_entry(
            "Lanternfall.720p.mkv",
            ORIGINAL_ID,
            Some((REPLACEMENT_ID, &replacement)),
        )
        .await;
    fixture
        .row(
            ORIGINAL_ID,
            &title_id,
            &fixture.library_path("Lanternfall.720p.mkv"),
        )
        .await;

    assert!(!fixture.purge(&entry).await);
    assert_quarantined_and_kept(&entry);
}

#[tokio::test]
async fn an_entry_whose_original_path_belongs_to_another_title_is_quarantined() {
    let fixture = UpgradeFixture::new().await;
    let title_id = fixture.title.id.clone();
    let replacement = fixture
        .live_file(REPLACEMENT_ID, &title_id, "Lanternfall.1080p.mkv")
        .await;
    let name = "Lanternfall.720p.mkv";
    let entry = fixture
        .upgrade_entry(name, ORIGINAL_ID, Some((REPLACEMENT_ID, &replacement)))
        .await;
    fixture
        .live_file("synthetic-other-file", "synthetic-other-title", name)
        .await;

    assert!(!fixture.purge(&entry).await);
    assert_quarantined_and_kept(&entry);
}

/// An episode's entry does not record which episodes it covered, so once its
/// replacement row is gone nothing names its unit; another live file of the
/// series does not stand in for it.
#[tokio::test]
async fn a_series_entry_whose_replacement_row_is_gone_is_quarantined() {
    let fixture = UpgradeFixture::new().await;
    let series = Title {
        id: "synthetic-series".to_string(),
        name: "Lanternfall Series".to_string(),
        facet: MediaFacet::Series,
        folder_path: Some(
            fixture
                .root()
                .join("Lanternfall Series")
                .to_string_lossy()
                .to_string(),
        ),
        ..fixture.title.clone()
    };
    let titles = fixture.app.services.catalog.titles.clone();
    titles
        .create(series.clone())
        .await
        .expect("create series title");
    let gone = fixture.library_path("Lanternfall.S01E01.1080p.mkv");
    let source = fixture.library_path("Lanternfall.S01E01.720p.mkv");
    std::fs::write(&source, "episode replaced bytes").unwrap();
    let entry = fixture
        .recycle(
            &source,
            crate::recycle_bin::RecycleManifest {
                schema: None,
                entry_id: None,
                source_operation_id: None,
                recycled_at: Utc::now().to_rfc3339(),
                original_path: source.to_string_lossy().to_string(),
                original_file_id: Some(ORIGINAL_ID.to_string()),
                size_bytes: 0,
                title_id: Some(series.id.clone()),
                media_root: Some(fixture.root().to_string_lossy().to_string()),
                reason: "upgrade_replaced".to_string(),
                status: None,
                replacement_file_id: Some(REPLACEMENT_ID.to_string()),
                replacement_path: Some(gone.to_string_lossy().to_string()),
                episode_ids: None,
                media_row: None,
            },
        )
        .await;
    fixture
        .live_file(
            "synthetic-other-episode",
            &series.id,
            "Lanternfall.S01E02.1080p.mkv",
        )
        .await;

    assert!(!fixture.purge(&entry).await);
    assert_quarantined_and_kept(&entry);
}

/// Purging or quarantining an upgrade entry leaves every other entry as it
/// was, and an entry recycled for another reason purges as before.
#[tokio::test]
async fn other_entries_and_reasons_are_unchanged() {
    let fixture = UpgradeFixture::new().await;
    let gone = fixture.library_path("Lanternfall.1080p.mkv");
    let quarantined = fixture
        .upgrade_entry(
            "Lanternfall.720p.mkv",
            ORIGINAL_ID,
            Some((REPLACEMENT_ID, &gone)),
        )
        .await;
    let removed = fixture.removal_entry("Lanternfall.sample.mkv").await;
    let untouched = fixture
        .upgrade_entry(
            "Lanternfall.480p.mkv",
            "synthetic-file-z",
            Some((REPLACEMENT_ID, &gone)),
        )
        .await;

    assert!(!fixture.purge(&quarantined).await);
    assert_eq!(
        status(&removed).as_deref(),
        Some(crate::recycle_bin::RECYCLE_STATUS_COMMITTED)
    );
    assert_eq!(
        status(&untouched).as_deref(),
        Some(crate::recycle_bin::RECYCLE_STATUS_COMMITTED)
    );
    assert!(fixture.purge(&removed).await);
    assert!(!removed.exists());
    assert_eq!(
        status(&untouched).as_deref(),
        Some(crate::recycle_bin::RECYCLE_STATUS_COMMITTED)
    );
    assert!(untouched.exists());
}

const SERIES_ID: &str = "synthetic-series";

impl UpgradeFixture {
    /// A series in the same library, with its own folder.
    async fn series(&self) -> Title {
        let series = Title {
            id: SERIES_ID.to_string(),
            name: "Lanternfall Series".to_string(),
            facet: MediaFacet::Series,
            folder_path: Some(self.series_path("").to_string_lossy().to_string()),
            ..self.title.clone()
        };
        std::fs::create_dir_all(self.series_path("")).expect("create series folder");
        self.app
            .services
            .catalog
            .titles
            .create(series.clone())
            .await
            .expect("create series title");
        series
    }

    fn series_path(&self, name: &str) -> PathBuf {
        let folder = self.root().join("Lanternfall Series");
        if name.is_empty() {
            folder
        } else {
            folder.join(name)
        }
    }

    /// An episode file of the series on disk at `name`, with a primary row
    /// linked to `episode_id`.
    async fn live_episode_file(&self, id: &str, name: &str, episode_id: &str) -> PathBuf {
        let path = self.series_path(name);
        std::fs::write(&path, format!("{id} bytes")).expect("write episode file");
        self.media_files.store.lock().await.push(TitleMediaFile {
            id: id.to_string(),
            title_id: SERIES_ID.to_string(),
            episode_id: Some(episode_id.to_string()),
            file_path: path.to_string_lossy().to_string(),
            role: crate::MediaFileRole::Primary,
            ..Default::default()
        });
        path
    }

    /// Recycles the series file `name` as replaced by an upgrade from
    /// `original_id` to `replacement_id`, recording `episode_ids` when given.
    async fn episode_upgrade_entry(
        &self,
        name: &str,
        original_id: &str,
        replacement_id: &str,
        episode_ids: Option<&[&str]>,
    ) -> PathBuf {
        let source = self.series_path(name);
        std::fs::write(&source, format!("{name} replaced bytes")).expect("write original");
        self.recycle(
            &source,
            crate::recycle_bin::RecycleManifest {
                schema: None,
                entry_id: None,
                source_operation_id: None,
                recycled_at: Utc::now().to_rfc3339(),
                original_path: source.to_string_lossy().to_string(),
                original_file_id: Some(original_id.to_string()),
                size_bytes: 0,
                title_id: Some(SERIES_ID.to_string()),
                media_root: Some(self.root().to_string_lossy().to_string()),
                reason: "upgrade_replaced".to_string(),
                status: None,
                replacement_file_id: Some(replacement_id.to_string()),
                replacement_path: Some(
                    self.series_path(&format!("{replacement_id}.mkv"))
                        .to_string_lossy()
                        .to_string(),
                ),
                episode_ids: episode_ids
                    .map(|ids| ids.iter().map(|id| (*id).to_string()).collect()),
                media_row: None,
            },
        )
        .await
    }
}

/// A → B → C → D for one episode, recorded before entries named their
/// episodes: B's and C's rows are gone and D is live, so the bin's own
/// entries lead every one of A, B and C to D.
#[tokio::test]
async fn an_episode_chain_purges_every_entry_that_leads_to_the_live_file() {
    let fixture = UpgradeFixture::new().await;
    fixture.series().await;
    let a = fixture
        .episode_upgrade_entry("S01E01.480p.mkv", "synthetic-ep-a", "synthetic-ep-b", None)
        .await;
    let b = fixture
        .episode_upgrade_entry("S01E01.720p.mkv", "synthetic-ep-b", "synthetic-ep-c", None)
        .await;
    let c = fixture
        .episode_upgrade_entry("S01E01.1080p.mkv", "synthetic-ep-c", "synthetic-ep-d", None)
        .await;
    let d = fixture
        .live_episode_file("synthetic-ep-d", "S01E01.2160p.mkv", "synthetic-episode-1")
        .await;

    assert!(fixture.purge(&a).await);
    assert!(fixture.purge(&b).await);
    assert!(fixture.purge(&c).await);
    assert!(!a.exists() && !b.exists() && !c.exists());
    assert!(d.exists(), "the live file is untouched");
}

/// No entry records what replaced B, so nothing proves A's episode is still
/// in the library.
#[tokio::test]
async fn an_episode_chain_with_a_missing_link_is_quarantined() {
    let fixture = UpgradeFixture::new().await;
    fixture.series().await;
    let a = fixture
        .episode_upgrade_entry("S01E01.480p.mkv", "synthetic-ep-a", "synthetic-ep-b", None)
        .await;
    let c = fixture
        .episode_upgrade_entry("S01E01.1080p.mkv", "synthetic-ep-c", "synthetic-ep-d", None)
        .await;
    fixture
        .live_episode_file("synthetic-ep-d", "S01E01.2160p.mkv", "synthetic-episode-1")
        .await;

    assert!(!fixture.purge(&a).await);
    assert_quarantined_and_kept(&a);
    assert_eq!(
        status(&c).as_deref(),
        Some(crate::recycle_bin::RECYCLE_STATUS_COMMITTED)
    );
}

#[tokio::test]
async fn an_episode_chain_ending_at_another_titles_file_is_quarantined() {
    let fixture = UpgradeFixture::new().await;
    fixture.series().await;
    let a = fixture
        .episode_upgrade_entry("S01E01.480p.mkv", "synthetic-ep-a", "synthetic-ep-b", None)
        .await;
    let b = fixture
        .episode_upgrade_entry(
            "S01E01.720p.mkv",
            "synthetic-ep-b",
            "synthetic-other-file",
            None,
        )
        .await;
    fixture
        .live_file(
            "synthetic-other-file",
            "synthetic-other-title",
            "Lanternfall.other.mkv",
        )
        .await;

    assert!(!fixture.purge(&a).await);
    assert_quarantined_and_kept(&a);
    assert!(!fixture.purge(&b).await);
    assert_quarantined_and_kept(&b);
}

#[tokio::test]
async fn an_episode_chain_that_loops_is_quarantined() {
    let fixture = UpgradeFixture::new().await;
    fixture.series().await;
    let a = fixture
        .episode_upgrade_entry("S01E01.480p.mkv", "synthetic-ep-a", "synthetic-ep-b", None)
        .await;
    let b = fixture
        .episode_upgrade_entry("S01E01.720p.mkv", "synthetic-ep-b", "synthetic-ep-c", None)
        .await;
    let c = fixture
        .episode_upgrade_entry("S01E01.1080p.mkv", "synthetic-ep-c", "synthetic-ep-b", None)
        .await;
    fixture
        .live_episode_file("synthetic-ep-z", "S01E02.mkv", "synthetic-episode-2")
        .await;

    assert!(!fixture.purge(&a).await);
    assert_quarantined_and_kept(&a);
    assert!(!fixture.purge(&b).await);
    assert_quarantined_and_kept(&b);
    assert!(!fixture.purge(&c).await);
    assert_quarantined_and_kept(&c);
}

/// One pass over a bin holding one long chain walks the chain once: each
/// media file row is read once for the walk, and each entry then re-reads and
/// probes the live file once before its purge, so the cost stays linear.
#[tokio::test]
async fn a_long_episode_chain_is_resolved_with_linear_reads_and_probes() {
    const LENGTH: usize = 2_000;
    let fixture = UpgradeFixture::new().await;
    fixture.series().await;
    let id = |index: usize| format!("synthetic-ep-{index:05}");
    let mut entries = Vec::with_capacity(LENGTH);
    for index in 0..LENGTH {
        entries.push(
            fixture
                .episode_upgrade_entry(
                    &format!("S01E01.v{index:05}.mkv"),
                    &id(index),
                    &id(index + 1),
                    None,
                )
                .await,
        );
    }
    let live = fixture
        .live_episode_file(&id(LENGTH), "S01E01.final.mkv", "synthetic-episode-1")
        .await;
    let reads = fixture.media_files.file_id_reads.clone();
    let reads_before = reads.load(std::sync::atomic::Ordering::SeqCst);
    let probes_before =
        crate::jobs::housekeeping::test_upgrade_purge_probes::count_under(&fixture.root());

    let purged = fixture
        .app
        .empty_recycle_bin(&fixture.user, None)
        .await
        .expect("empty the bin");

    assert_eq!(purged as usize, LENGTH);
    assert!(entries.iter().all(|entry| !entry.exists()));
    assert!(live.exists(), "the live file is untouched");
    // Every entry's own original row and the live terminal row once for the
    // walk, plus one fresh read of the live row per entry before its purge.
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::SeqCst) - reads_before,
        2 * LENGTH + 1
    );
    // One probe of the live file per entry before its purge.
    assert_eq!(
        crate::jobs::housekeeping::test_upgrade_purge_probes::count_under(&fixture.root())
            - probes_before,
        LENGTH
    );
}

/// The live file leaves the disk and the catalog after the first entry of a
/// pass purged: the entries after it re-check the live file and are kept.
#[tokio::test]
async fn entries_after_the_live_file_disappears_mid_pass_are_quarantined() {
    let fixture = UpgradeFixture::new().await;
    let title_id = fixture.title.id.clone();
    let live = fixture
        .live_file(REPLACEMENT_ID, &title_id, "Lanternfall.2160p.mkv")
        .await;
    let mut entries = Vec::new();
    for index in 0..3 {
        entries.push(
            fixture
                .upgrade_entry(
                    &format!("Lanternfall.v{index}.mkv"),
                    &format!("synthetic-file-old-{index}"),
                    Some((REPLACEMENT_ID, &live)),
                )
                .await,
        );
    }
    let watched = entries.clone();
    let live_on_disk = live.clone();
    *fixture.media_files.before_file_id_read.lock().unwrap() = Some(Box::new(
        move |file_id: &str, store: &mut Vec<TitleMediaFile>| {
            if file_id == REPLACEMENT_ID && watched.iter().any(|entry| !entry.exists()) {
                store.retain(|file| file.id != REPLACEMENT_ID);
                let _ = std::fs::remove_file(&live_on_disk);
            }
        },
    ));

    let purged = fixture
        .app
        .empty_recycle_bin(&fixture.user, None)
        .await
        .expect("empty the bin");

    assert_eq!(purged, 1);
    let kept: Vec<_> = entries.iter().filter(|entry| entry.exists()).collect();
    assert_eq!(kept.len(), 2);
    for entry in kept {
        assert_quarantined_and_kept(entry);
    }
}

/// A batch purge of several entries that each follow the chain lists the bin
/// once for the whole batch.
#[tokio::test]
async fn a_batch_purge_of_chain_entries_lists_the_bin_once() {
    let fixture = UpgradeFixture::new().await;
    fixture.series().await;
    let a = fixture
        .episode_upgrade_entry("S01E01.480p.mkv", "synthetic-ep-a", "synthetic-ep-b", None)
        .await;
    let b = fixture
        .episode_upgrade_entry("S01E01.720p.mkv", "synthetic-ep-b", "synthetic-ep-c", None)
        .await;
    let c = fixture
        .episode_upgrade_entry("S01E01.1080p.mkv", "synthetic-ep-c", "synthetic-ep-d", None)
        .await;
    let d = fixture
        .live_episode_file("synthetic-ep-d", "S01E01.2160p.mkv", "synthetic-episode-1")
        .await;
    let entry_id = |recycled: &Path| {
        recycled
            .parent()
            .and_then(Path::file_name)
            .expect("entry directory")
            .to_string_lossy()
            .to_string()
    };
    let ids = vec![entry_id(&a), entry_id(&b), entry_id(&c)];
    let bin = fixture.root().join(".scryer-recycle");
    let lists_before = crate::recycle_bin::test_bin_calls::count_kind(
        &bin,
        crate::recycle_bin::test_bin_calls::LIST,
    );

    fixture
        .app
        .start_purge_recycled_items_job(&fixture.user, ids.clone())
        .await
        .expect("start the batch purge");
    // The batch holds every entry's guard until it finishes.
    for id in &ids {
        crate::test_wait::within_deadline(
            "the batch purge finishes",
            fixture
                .app
                .runtime
                .jobs
                .interactive_operation_guards
                .acquire(&format!("recycle-entry:{id}")),
        )
        .await;
    }

    assert!(!a.exists() && !b.exists() && !c.exists());
    assert!(d.exists(), "the live file is untouched");
    assert_eq!(
        crate::recycle_bin::test_bin_calls::count_kind(
            &bin,
            crate::recycle_bin::test_bin_calls::LIST
        ) - lists_before,
        1
    );
}

/// An entry that names its episodes is settled by the live file covering
/// them, with no chain to follow.
#[tokio::test]
async fn an_entry_naming_its_episodes_purges_while_a_live_file_covers_them() {
    let fixture = UpgradeFixture::new().await;
    fixture.series().await;
    let entry = fixture
        .episode_upgrade_entry(
            "S01E01.720p.mkv",
            "synthetic-ep-a",
            "synthetic-ep-b",
            Some(&["synthetic-episode-1"]),
        )
        .await;
    let live = fixture
        .live_episode_file("synthetic-ep-d", "S01E01.2160p.mkv", "synthetic-episode-1")
        .await;
    let scoped_before = fixture
        .media_files
        .scoped_file_reads
        .load(std::sync::atomic::Ordering::SeqCst);

    assert!(fixture.purge(&entry).await);
    assert!(!entry.exists());
    assert!(live.exists());
    assert_eq!(
        fixture
            .media_files
            .scoped_file_reads
            .load(std::sync::atomic::Ordering::SeqCst)
            - scoped_before,
        1
    );
}

#[tokio::test]
async fn an_entry_naming_episodes_no_live_file_covers_is_quarantined() {
    let fixture = UpgradeFixture::new().await;
    fixture.series().await;
    let entry = fixture
        .episode_upgrade_entry(
            "S01E01.720p.mkv",
            "synthetic-ep-a",
            "synthetic-ep-b",
            Some(&["synthetic-episode-1"]),
        )
        .await;
    fixture
        .live_episode_file("synthetic-ep-d", "S01E02.2160p.mkv", "synthetic-episode-2")
        .await;
    // A row covering the episode whose file is not on disk does not count.
    fixture.media_files.store.lock().await.push(TitleMediaFile {
        id: "synthetic-ep-e".to_string(),
        title_id: SERIES_ID.to_string(),
        episode_id: Some("synthetic-episode-1".to_string()),
        file_path: fixture
            .series_path("S01E01.missing.mkv")
            .to_string_lossy()
            .to_string(),
        role: crate::MediaFileRole::Primary,
        ..Default::default()
    });

    assert!(!fixture.purge(&entry).await);
    assert_quarantined_and_kept(&entry);
}

/// Rewrites an entry's manifest in place, as an earlier pass or version left
/// it.
fn rewrite_manifest(recycled: &Path, edit: impl FnOnce(&mut crate::recycle_bin::RecycleManifest)) {
    let path = recycled.parent().unwrap().join("manifest.json");
    let mut manifest: crate::recycle_bin::RecycleManifest =
        serde_json::from_slice(&std::fs::read(&path).expect("read manifest"))
            .expect("decode manifest");
    edit(&mut manifest);
    std::fs::write(
        &path,
        serde_json::to_vec(&manifest).expect("encode manifest"),
    )
    .expect("write manifest");
}

/// Dates the entry well past the bin's retention.
fn expire(recycled: &Path) {
    rewrite_manifest(recycled, |manifest| {
        manifest.recycled_at = (Utc::now() - chrono::Duration::days(60)).to_rfc3339();
    });
}

fn manifest_bytes(recycled: &Path) -> Vec<u8> {
    std::fs::read(recycled.parent().unwrap().join("manifest.json")).expect("read manifest")
}

/// An expired entry quarantined because nothing covered what it held is
/// left exactly as it is by housekeeping while that stays true, and purged
/// by the first pass after the library covers it.
#[tokio::test]
async fn housekeeping_purges_an_expired_quarantined_entry_once_the_library_covers_it() {
    let fixture = UpgradeFixture::new().await;
    let title_id = fixture.title.id.clone();
    let gone = fixture.library_path("Lanternfall.1080p.mkv");
    let entry = fixture
        .upgrade_entry(
            "Lanternfall.720p.mkv",
            ORIGINAL_ID,
            Some((REPLACEMENT_ID, &gone)),
        )
        .await;
    assert!(!fixture.purge(&entry).await);
    assert_quarantined_and_kept(&entry);
    expire(&entry);
    let before = manifest_bytes(&entry);

    assert_eq!(fixture.app.purge_recycle_bins_for_housekeeping().await, 0);
    assert_quarantined_and_kept(&entry);
    assert_eq!(
        manifest_bytes(&entry),
        before,
        "the manifest is not rewritten"
    );

    let live = fixture
        .live_file("synthetic-file-c", &title_id, "Lanternfall.2160p.mkv")
        .await;
    assert_eq!(fixture.app.purge_recycle_bins_for_housekeeping().await, 1);
    assert!(!entry.exists(), "the covered entry is purged");
    assert!(live.exists(), "the live file is untouched");
}

/// A quarantined entry the library covers stays for the bin's retention like
/// any other.
#[tokio::test]
async fn housekeeping_keeps_a_covered_quarantined_entry_that_has_not_expired() {
    let fixture = UpgradeFixture::new().await;
    let title_id = fixture.title.id.clone();
    let gone = fixture.library_path("Lanternfall.1080p.mkv");
    let entry = fixture
        .upgrade_entry(
            "Lanternfall.720p.mkv",
            ORIGINAL_ID,
            Some((REPLACEMENT_ID, &gone)),
        )
        .await;
    assert!(!fixture.purge(&entry).await);
    fixture
        .live_file("synthetic-file-c", &title_id, "Lanternfall.2160p.mkv")
        .await;

    assert_eq!(fixture.app.purge_recycle_bins_for_housekeeping().await, 0);
    assert_quarantined_and_kept(&entry);
}

/// Only an upgrade's entry, laid out as Scryer wrote it, is asked again. A
/// quarantined entry recycled for another reason, and one whose manifest no
/// longer names its directory, are kept however old, with a live file of the
/// title in the library.
#[tokio::test]
async fn housekeeping_keeps_quarantined_entries_that_are_not_trusted_upgrades() {
    let fixture = UpgradeFixture::new().await;
    let title_id = fixture.title.id.clone();
    let live = fixture
        .live_file(REPLACEMENT_ID, &title_id, "Lanternfall.2160p.mkv")
        .await;
    let removed = fixture.removal_entry("Lanternfall.sample.mkv").await;
    rewrite_manifest(&removed, |manifest| {
        manifest.status = Some(crate::recycle_bin::RECYCLE_STATUS_QUARANTINED.to_string());
        manifest.reason = format!("{}; quarantine: synthetic", manifest.reason);
    });
    expire(&removed);
    let renamed = fixture
        .upgrade_entry(
            "Lanternfall.720p.mkv",
            ORIGINAL_ID,
            Some((REPLACEMENT_ID, &live)),
        )
        .await;
    rewrite_manifest(&renamed, |manifest| {
        manifest.status = Some(crate::recycle_bin::RECYCLE_STATUS_QUARANTINED.to_string());
        manifest.reason = format!("{}; quarantine: synthetic", manifest.reason);
        manifest.entry_id = Some("20010101_000000000_abcdef".to_string());
    });
    expire(&renamed);

    assert_eq!(fixture.app.purge_recycle_bins_for_housekeeping().await, 0);
    assert_quarantined_and_kept(&removed);
    assert_quarantined_and_kept(&renamed);
    assert!(live.exists());
}

/// A → B → C for one episode, where an earlier version quarantined A when
/// B's row went: the pass that purges B's expired entry also purges A's,
/// through the upgrade B's entry records.
#[tokio::test]
async fn housekeeping_purges_an_entry_an_earlier_version_quarantined_mid_chain() {
    let fixture = UpgradeFixture::new().await;
    fixture.series().await;
    let a = fixture
        .episode_upgrade_entry("S01E01.480p.mkv", "synthetic-ep-a", "synthetic-ep-b", None)
        .await;
    rewrite_manifest(&a, |manifest| {
        manifest.status = Some(crate::recycle_bin::RECYCLE_STATUS_QUARANTINED.to_string());
        manifest.reason = format!("{}; quarantine: synthetic", manifest.reason);
    });
    let b = fixture
        .episode_upgrade_entry("S01E01.720p.mkv", "synthetic-ep-b", "synthetic-ep-c", None)
        .await;
    let c = fixture
        .live_episode_file("synthetic-ep-c", "S01E01.1080p.mkv", "synthetic-episode-1")
        .await;
    expire(&a);
    expire(&b);

    assert_eq!(fixture.app.purge_recycle_bins_for_housekeeping().await, 2);
    assert!(!a.exists() && !b.exists());
    assert!(c.exists(), "the live file is untouched");
}
