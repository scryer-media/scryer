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
    /// `(folder, remaining_failures)`: make scans of one folder fail a bounded
    /// number of times. Bounded because the compensating transaction rescans the
    /// folders the titles are restored to, and a permanently broken scanner would
    /// only prove the restore's rescan is best-effort, not that the restore
    /// itself rebuilds anything (FR-008).
    scan_failure: Arc<Mutex<Option<(String, usize)>>>,
}

impl FolderScopedLibraryScanner {
    async fn set_files(&self, paths: &[&Path]) {
        *self.files.lock().await = build_test_library_files(paths);
    }

    /// Fail the next `times` scans that walk `folder`.
    async fn fail_scans_of(&self, folder: &Path, times: usize) {
        *self.scan_failure.lock().await = Some((folder.to_string_lossy().to_string(), times));
    }

    /// How many injected scan failures are still owed; `0` once every one the
    /// test armed has actually fired.
    async fn remaining_scan_failures(&self) -> usize {
        self.scan_failure
            .lock()
            .await
            .as_ref()
            .map(|(_, remaining)| *remaining)
            .unwrap_or_default()
    }

    async fn fail_if_armed(&self, root: &str) -> AppResult<()> {
        let mut failure = self.scan_failure.lock().await;
        let Some((folder, remaining)) = failure.as_mut() else {
            return Ok(());
        };
        if *remaining == 0 || !Path::new(root).starts_with(Path::new(folder)) {
            return Ok(());
        }
        *remaining -= 1;
        Err(AppError::Repository(format!(
            "injected scan failure for {root}"
        )))
    }

    async fn files_under(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        self.fail_if_armed(root).await?;
        let root = Path::new(root).to_path_buf();
        Ok(self
            .files
            .lock()
            .await
            .iter()
            .filter(|file| Path::new(&file.path).starts_with(&root))
            .cloned()
            .collect())
    }
}

#[async_trait]
impl LibraryScanner for FolderScopedLibraryScanner {
    async fn scan_library(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        self.files_under(root).await
    }

    async fn scan_library_batched(
        &self,
        root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let files = self.files_under(root).await?;
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
        let files = self.files_under(root).await?;
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
    facet: MediaFacet,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
    titles: Arc<MockTitleRepo>,
    scanner: Arc<FolderScopedLibraryScanner>,
    root: tempfile::TempDir,
}

impl FolderMatchFixture {
    async fn new() -> Self {
        Self::for_facet(MediaFacet::Movie).await
    }

    async fn new_series() -> Self {
        Self::for_facet(MediaFacet::Series).await
    }

    /// Assembled here rather than through `bootstrap_movie_scan_app` because the
    /// series scenario needs the series root and the atomicity scenarios need a
    /// handle on the title repository to fail one ownership commit.
    async fn for_facet(facet: MediaFacet) -> Self {
        let root = tempfile::tempdir().expect("library root tempdir");
        let settings = Arc::new(StoredSettingsRepo::default());
        let path_key = match facet {
            MediaFacet::Series => "series.path",
            _ => "movies.path",
        };
        settings
            .set_value(
                SETTINGS_SCOPE_MEDIA,
                path_key,
                root.path().to_string_lossy().as_ref(),
            )
            .await;
        let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
        let (app, user, titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
            settings,
            Arc::new(MutableLibraryScanner::default()),
            unmatched_items.clone(),
            Arc::new(EmptySearchMetadataGateway),
        );
        app.reconcile_default_library_roots()
            .await
            .expect("reconcile library root");
        let scanner = Arc::new(FolderScopedLibraryScanner::default());
        let app = app.with_test_overrides({
            let scanner = scanner.clone();
            move |services| services.with_library_scanner(scanner)
        });
        Self {
            app,
            user,
            facet,
            unmatched_items,
            titles,
            scanner,
            root,
        }
    }

    async fn create_title_with_folder(&self, name: &str, folder_path: &Path) -> Title {
        self.create_tagged_title_with_folder(name, folder_path, &[])
            .await
    }

    async fn create_tagged_title_with_folder(
        &self,
        name: &str,
        folder_path: &Path,
        tags: &[&str],
    ) -> Title {
        // Creation is registry-gated, so whatever user labels a case asks for
        // have to be defined before the title can be born carrying them.
        // Reserved `scryer:` entries are settings and are not gated.
        for tag in tags {
            if crate::is_reserved_title_tag(tag) {
                continue;
            }
            let _ = self
                .app
                .create_title_tag_definition(&self.user, tag, None)
                .await;
        }
        let title = self
            .app
            .add_title(
                &self.user,
                NewTitle {
                    name: name.into(),
                    facet: self.facet.clone(),
                    monitored: true,
                    tags: tags.iter().map(|tag| tag.to_string()).collect(),
                    ..Default::default()
                },
            )
            .await
            .expect("create title");
        self.app
            .services
            .catalog
            .titles
            .set_folder_path(&title.id, folder_path.to_string_lossy().as_ref())
            .await
            .expect("set title folder path");
        self.app
            .services
            .catalog
            .titles
            .get_by_id(&title.id)
            .await
            .expect("load title")
            .expect("title exists")
    }

    /// Seed one season and `episode_numbers` episodes so a series rescan has
    /// episode records to associate the files it finds with.
    async fn seed_season_with_episodes(&self, title_id: &str, episode_numbers: &[u32]) {
        let season = self
            .app
            .services
            .catalog
            .shows
            .create_collection(Collection {
                id: Id::new().0,
                title_id: title_id.to_string(),
                collection_type: CollectionType::Season,
                collection_index: "1".to_string(),
                label: Some("Season 1".to_string()),
                ordered_path: None,
                narrative_order: Some("1".to_string()),
                first_episode_number: episode_numbers.first().map(|number| number.to_string()),
                last_episode_number: episode_numbers.last().map(|number| number.to_string()),
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create season");
        for number in episode_numbers {
            self.app
                .services
                .catalog
                .shows
                .create_episode(Episode {
                    id: Id::new().0,
                    title_id: title_id.to_string(),
                    collection_id: Some(season.id.clone()),
                    episode_type: EpisodeType::Standard,
                    episode_number: Some(number.to_string()),
                    season_number: Some("1".to_string()),
                    episode_label: Some(format!("S01E{number:02}")),
                    title: Some(format!("Episode {number}")),
                    air_date: Some("2024-01-01".to_string()),
                    duration_seconds: Some(1_800),
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
                .expect("create episode");
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

    /// `(file_path, episode_id)` for every media row of the title, sorted by
    /// path — the shape the series scenario cares about (US1.2).
    async fn media_episode_links(&self, title_id: &str) -> Vec<(String, Option<String>)> {
        let mut rows = self
            .app
            .services
            .library
            .media_files
            .list_media_files_for_title(title_id)
            .await
            .expect("list media files")
            .into_iter()
            .map(|file| (file.file_path, file.episode_id))
            .collect::<Vec<_>>();
        rows.sort();
        rows
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
    assert_eq!(
        preview.selected_root_path,
        fixture.root.path().to_string_lossy()
    );

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

/// US1.1's tail — the folder the title gave up comes back in front of unmatched
/// discovery.
///
/// The assignment itself writes no repair row for it: `apply_folder_assignment`
/// only *clears* the unmatched item for the folder it claimed, and leaves the
/// abandoned folder unowned so discovery picks it up. The return trip is
/// therefore a property of the next scan, and that is what is asserted here —
/// both halves, so a future change that started emitting a repair row at apply
/// time would be visible rather than silently redundant.
#[tokio::test]
async fn the_folder_a_corrected_title_gave_up_returns_to_unmatched_discovery() {
    let fixture = FolderMatchFixture::new().await;
    let wrong_folder = fixture.folder("Wrong Match (2019)");
    let right_folder = fixture.folder("Right Match (2024)");
    let wrong_file = fixture.write_media(&wrong_folder, "Wrong.Match.2019.1080p.mkv");
    let right_file = fixture.write_media(&right_folder, "Right.Match.2024.1080p.mkv");
    fixture
        .scanner
        .set_files(&[wrong_file.as_path(), right_file.as_path()])
        .await;

    let title = fixture
        .create_title_with_folder("Right Match", wrong_folder.as_path())
        .await;
    fixture.seed_media_row(&title.id, &wrong_file).await;

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

    // Nothing is queued for repair by the correction itself; the folder is
    // simply unowned again.
    assert!(
        fixture
            .unmatched_items
            .items()
            .await
            .iter()
            .all(|item| !Path::new(&item.item_path).starts_with(&wrong_folder)),
        "assignment should not write a repair row for the abandoned folder"
    );

    fixture
        .app
        .scan_library(&fixture.user, MediaFacet::Movie)
        .await
        .expect("rescan the library");

    let unmatched = fixture.unmatched_items.items().await;
    assert!(
        unmatched
            .iter()
            .any(|item| Path::new(&item.item_path).starts_with(&wrong_folder)),
        "the abandoned folder should be back in unmatched discovery, got {:?}",
        unmatched
            .iter()
            .map(|item| item.item_path.as_str())
            .collect::<Vec<_>>()
    );
    // The folder the title now owns is matched, so it is not offered for repair.
    assert!(
        unmatched
            .iter()
            .all(|item| !Path::new(&item.item_path).starts_with(&right_folder)),
        "the newly owned folder should not be unmatched"
    );
}

/// US1.2 — a series' episode associations are rebuilt from the new folder while
/// identity, monitoring, quality settings, and tags survive (FR-004).
#[tokio::test]
async fn correcting_a_series_match_rebuilds_episode_associations_from_the_new_folder() {
    let fixture = FolderMatchFixture::new_series().await;
    let wrong_folder = fixture.folder("Wrong Show (2019)");
    let right_folder = fixture.folder("Right Show (2024)");
    let wrong_episode = fixture.write_media(&wrong_folder, "Wrong Show - S01E01.mkv");
    let right_episode_one = fixture.write_media(&right_folder, "Right Show - S01E01.mkv");
    let right_episode_two = fixture.write_media(&right_folder, "Right Show - S01E02.mkv");
    fixture
        .scanner
        .set_files(&[
            wrong_episode.as_path(),
            right_episode_one.as_path(),
            right_episode_two.as_path(),
        ])
        .await;

    let title = fixture
        .create_tagged_title_with_folder("Right Show", wrong_folder.as_path(), &["favourites"])
        .await;
    fixture.seed_season_with_episodes(&title.id, &[1, 2]).await;
    fixture.seed_media_row(&title.id, &wrong_episode).await;

    let before = snapshot_tree(fixture.root.path());

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

    // Rebuilt from the new folder: both episode files there are associated, and
    // the file under the old folder is not.
    let rows = fixture.media_episode_links(&title.id).await;
    let paths = rows
        .iter()
        .map(|(path, _)| path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            &*right_episode_one.to_string_lossy(),
            &*right_episode_two.to_string_lossy(),
        ],
        "episode associations should be rebuilt from the new folder"
    );

    // The associations are episode-bound, not just title-bound: each seeded
    // episode picked up its file from the new folder.
    let episodes = fixture
        .app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
        .expect("list episodes");
    assert_eq!(episodes.len(), 2);
    for episode in &episodes {
        assert!(
            rows.iter()
                .any(|(_, episode_id)| episode_id.as_deref() == Some(episode.id.as_str())),
            "episode {:?} should be associated with a file in the new folder, got {rows:?}",
            episode.episode_label
        );
    }

    // FR-004: nothing but folder ownership and the derived associations moved.
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
    assert_eq!(after.facet, MediaFacet::Series);
    assert_eq!(after.monitored, title.monitored);
    // Every tag the user set survives. The series scan additionally records the
    // folder layout it observed as a `scryer:`-namespaced structured tag when
    // that layout disagrees with the resolved season-folder setting
    // (`merge_title_scan_option_tags`). That is derived state written by every
    // series scan — not an edit to the title's tags — so it is filtered out
    // here rather than asserted away.
    let user_tags = |tags: &[String]| {
        tags.iter()
            .filter(|tag| !tag.starts_with("scryer:"))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(user_tags(&after.tags), user_tags(&title.tags));
    assert!(
        after.tags.iter().any(|tag| tag == "favourites"),
        "the user's tag should survive the correction, got {:?}",
        after.tags
    );
    assert_eq!(after.external_ids, title.external_ids);
    assert_eq!(after.canonical_tags, title.canonical_tags);
    assert_eq!(after.min_availability, title.min_availability);
    assert_eq!(after.metadata_language, title.metadata_language);
    assert_eq!(after.library_id, title.library_id);
    assert_eq!(after.root_folder_id, title.root_folder_id);
    // The season and its episodes are untouched records; only their file
    // associations were rebuilt.
    assert!(
        episodes.iter().all(|episode| episode.monitored),
        "episode monitoring should survive the correction"
    );

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

    let title = create_movie_title_with_folder(
        &fixture.app,
        &fixture.user,
        "Already Mine",
        folder.as_path(),
    )
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
    let owner = create_movie_title_with_folder(
        &fixture.app,
        &fixture.user,
        "Owner",
        owned_folder.as_path(),
    )
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

    let taker = create_movie_title_with_folder(
        &fixture.app,
        &fixture.user,
        "Taker",
        taker_folder.as_path(),
    )
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

    let title = create_movie_title_with_folder(
        &fixture.app,
        &fixture.user,
        "Inside Root",
        folder.as_path(),
    )
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

/// A pair of titles each owning a folder with one media file in it — the shape
/// every atomicity scenario below starts from.
struct SwapPair {
    first: Title,
    first_folder: std::path::PathBuf,
    first_file: std::path::PathBuf,
    second: Title,
    second_folder: std::path::PathBuf,
    second_file: std::path::PathBuf,
}

impl SwapPair {
    async fn seed(fixture: &FolderMatchFixture) -> Self {
        let first_folder = fixture.folder("First Title (2020)");
        let second_folder = fixture.folder("Second Title (2021)");
        let first_file = fixture.write_media(&first_folder, "First.Title.2020.1080p.mkv");
        let second_file = fixture.write_media(&second_folder, "Second.Title.2021.1080p.mkv");
        fixture
            .scanner
            .set_files(&[first_file.as_path(), second_file.as_path()])
            .await;

        let first = fixture
            .create_title_with_folder("First Title", first_folder.as_path())
            .await;
        let second = fixture
            .create_title_with_folder("Second Title", second_folder.as_path())
            .await;
        fixture.seed_media_row(&first.id, &first_file).await;
        fixture.seed_media_row(&second.id, &second_file).await;

        Self {
            first,
            first_folder,
            first_file,
            second,
            second_folder,
            second_file,
        }
    }

    /// Both titles own what they started with and still point at the files in
    /// those folders.
    async fn assert_unchanged(&self, fixture: &FolderMatchFixture) {
        assert_eq!(
            fixture.folder_path_of(&self.first.id).await.as_deref(),
            Some(self.first_folder.to_string_lossy().as_ref()),
            "the edited title should still own its original folder"
        );
        assert_eq!(
            fixture.folder_path_of(&self.second.id).await.as_deref(),
            Some(self.second_folder.to_string_lossy().as_ref()),
            "the other title should still own its original folder"
        );
        assert_eq!(
            fixture.media_paths(&self.first.id).await,
            vec![self.first_file.to_string_lossy().to_string()],
            "the edited title's associations should point back at its own folder"
        );
        assert_eq!(
            fixture.media_paths(&self.second.id).await,
            vec![self.second_file.to_string_lossy().to_string()],
            "the other title's associations should point back at its own folder"
        );
    }
}

/// US1.6 — a swap whose *second* ownership commit fails leaves neither title
/// holding the other's folder (FR-008).
///
/// The first commit already landed when the second one fails, so this is the
/// case the compensating transaction exists for: without it the edited title
/// would be squatting on a folder the other title still claims.
#[tokio::test]
async fn a_swap_whose_second_ownership_commit_fails_leaves_both_titles_on_their_original_folders() {
    let fixture = FolderMatchFixture::new().await;
    let pair = SwapPair::seed(&fixture).await;

    fixture
        .titles
        .fail_folder_path_writes_for(&pair.second.id, "injected ownership commit failure")
        .await;

    let error = fixture
        .app
        .apply_title_folder_change(
            &fixture.user,
            &pair.first.id,
            pair.second_folder.to_string_lossy().as_ref(),
            FolderMatchResolution::Swap,
        )
        .await
        .expect_err("the swap should fail when the second ownership commit fails");
    assert!(
        matches!(&error, AppError::Repository(message)
            if message.contains("injected ownership commit failure")),
        "the original cause should reach the caller, got {error:?}"
    );

    pair.assert_unchanged(&fixture).await;
}

/// US1.6 — a swap that fails while rebuilding, after both ownership commits
/// landed, restores both titles and the associations the rebuild detached
/// (FR-008).
#[tokio::test]
async fn a_swap_that_fails_while_rescanning_restores_both_titles_and_their_associations() {
    let fixture = FolderMatchFixture::new().await;
    let pair = SwapPair::seed(&fixture).await;

    // The edited title's rescan is the first one the rebuild runs, and it walks
    // the folder it just received. Failing it once puts the workflow in the
    // worst state it can reach: both commits applied, both titles' old
    // associations already detached.
    fixture.scanner.fail_scans_of(&pair.second_folder, 1).await;

    let error = fixture
        .app
        .apply_title_folder_change(
            &fixture.user,
            &pair.first.id,
            pair.second_folder.to_string_lossy().as_ref(),
            FolderMatchResolution::Swap,
        )
        .await
        .expect_err("the swap should fail when the new folder cannot be scanned");
    assert!(
        matches!(&error, AppError::Repository(message) if message.contains("injected scan failure")),
        "the scan failure should reach the caller, got {error:?}"
    );
    assert_eq!(
        fixture.scanner.remaining_scan_failures().await,
        0,
        "the injected scan failure should actually have fired"
    );

    pair.assert_unchanged(&fixture).await;
}

/// US1.6 — a takeover that fails while rescanning restores the taker *and* the
/// title it displaced, and queues nothing for repair (FR-007, FR-008).
#[tokio::test]
async fn a_takeover_that_fails_while_rescanning_restores_both_titles_and_queues_no_repair() {
    let fixture = FolderMatchFixture::new().await;
    let taker_folder = fixture.folder("Taker (2020)");
    let owned_folder = fixture.folder("Displaced (2021)");
    let taker_file = fixture.write_media(&taker_folder, "Taker.2020.1080p.mkv");
    let owned_file = fixture.write_media(&owned_folder, "Displaced.2021.1080p.mkv");
    fixture
        .scanner
        .set_files(&[taker_file.as_path(), owned_file.as_path()])
        .await;

    let taker = fixture
        .create_title_with_folder("Taker", taker_folder.as_path())
        .await;
    let displaced = fixture
        .create_title_with_folder("Displaced", owned_folder.as_path())
        .await;
    fixture.seed_media_row(&taker.id, &taker_file).await;
    fixture.seed_media_row(&displaced.id, &owned_file).await;

    // Both ownership commits land; the taker's rescan of the folder it took is
    // what fails.
    fixture.scanner.fail_scans_of(&owned_folder, 1).await;

    let error = fixture
        .app
        .apply_title_folder_change(
            &fixture.user,
            &taker.id,
            owned_folder.to_string_lossy().as_ref(),
            FolderMatchResolution::TakeOver,
        )
        .await
        .expect_err("the takeover should fail when the taken folder cannot be scanned");
    assert!(
        matches!(&error, AppError::Repository(message) if message.contains("injected scan failure")),
        "the scan failure should reach the caller, got {error:?}"
    );
    assert_eq!(
        fixture.scanner.remaining_scan_failures().await,
        0,
        "the injected scan failure should actually have fired"
    );

    // Ownership is back where it started: the taker never kept the folder, and
    // the displaced title was not left folderless.
    assert_eq!(
        fixture.folder_path_of(&taker.id).await.as_deref(),
        Some(taker_folder.to_string_lossy().as_ref())
    );
    assert_eq!(
        fixture.folder_path_of(&displaced.id).await.as_deref(),
        Some(owned_folder.to_string_lossy().as_ref())
    );
    assert_eq!(
        fixture.media_paths(&taker.id).await,
        vec![taker_file.to_string_lossy().to_string()]
    );
    assert_eq!(
        fixture.media_paths(&displaced.id).await,
        vec![owned_file.to_string_lossy().to_string()]
    );

    // Nothing was displaced, so nothing should be asking the user to repair it.
    let unmatched = fixture.unmatched_items.items().await;
    assert!(
        unmatched.iter().all(|item| item.reason_code
            != crate::library_scan_unmatched::LIBRARY_SCAN_FOLDER_OWNERSHIP_CHANGED_BY_USER),
        "a failed takeover should queue no repair item, got {unmatched:?}"
    );
}
