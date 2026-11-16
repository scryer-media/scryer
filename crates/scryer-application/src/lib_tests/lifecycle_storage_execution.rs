//! Storage-pressure execution acceptance tests.
//!
//! The scripted probe is intentionally used only here: it proves that an
//! already-armed deletion refreshes capacity before every unlink without
//! depending on a workstation's real free space.

use super::*;

use crate::lib_tests::lifecycle_title_completion::{TitleFixture, title_fixture};
use crate::lifecycle_rules::storage::{
    TestCapacityProbe, install_lifecycle_storage_capacity_probe_for_test,
};
use crate::lifecycle_rules::{LifecycleCandidateFilter, LifecycleRuleDraft};
use crate::ports::SubtitleDownloadRepository;
use scryer_domain::{
    ExternalSubtitleSourceKind, LifecycleActionRunStatus, LifecycleActionStepState,
    LifecycleCandidateState, LifecycleEffectArming, LifecycleRuleSubjectKind,
    SubtitleBlocklistEntry, SubtitleDownload,
};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;

#[derive(Default)]
struct TrackedSubtitleRepo {
    downloads: Mutex<Vec<SubtitleDownload>>,
}

#[async_trait]
impl SubtitleDownloadRepository for TrackedSubtitleRepo {
    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        Ok(self
            .downloads
            .lock()
            .await
            .iter()
            .filter(|download| download.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn get(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        Ok(self
            .downloads
            .lock()
            .await
            .iter()
            .find(|download| download.id == id)
            .cloned())
    }

    async fn list_for_media_file(&self, media_file_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        Ok(self
            .downloads
            .lock()
            .await
            .iter()
            .filter(|download| download.media_file_id == media_file_id)
            .cloned()
            .collect())
    }

    async fn list_probe_cache_for_media_file(
        &self,
        _: &str,
    ) -> AppResult<Vec<crate::subtitles::ExternalSubtitleProbeCacheEntry>> {
        Ok(Vec::new())
    }

    async fn list_blocklist_for_media_file(
        &self,
        _: &str,
    ) -> AppResult<Vec<SubtitleBlocklistEntry>> {
        Ok(Vec::new())
    }

    async fn insert(&self, download: &SubtitleDownload) -> AppResult<()> {
        self.downloads.lock().await.push(download.clone());
        Ok(())
    }

    async fn upsert_probe_cache_entry(
        &self,
        _: &crate::subtitles::ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        if let Some(download) = self
            .downloads
            .lock()
            .await
            .iter_mut()
            .find(|download| download.id == id)
        {
            download.synced = synced;
        }
        Ok(())
    }

    async fn delete(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        let mut downloads = self.downloads.lock().await;
        let Some(index) = downloads.iter().position(|download| download.id == id) else {
            return Ok(None);
        };
        Ok(Some(downloads.remove(index)))
    }

    async fn delete_probe_cache_entry(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn is_blocklisted(&self, _: &str, _: &str, _: &str) -> AppResult<bool> {
        Ok(false)
    }

    async fn blocklist(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn list_search_backoffs_for_media_files(
        &self,
        _media_file_ids: &[String],
    ) -> AppResult<Vec<crate::subtitles::SubtitleSearchBackoff>> {
        Ok(Vec::new())
    }
    async fn apply_search_backoffs(
        &self,
        _media_file_id: &str,
        _upserts: &[crate::subtitles::SubtitleSearchBackoff],
        _cleared: &[crate::subtitles::SubtitleSearchBackoffKey],
    ) -> AppResult<()> {
        Ok(())
    }
}

fn capacity(available_bytes: u64) -> crate::helpers::FilesystemSpace {
    crate::helpers::FilesystemSpace {
        total_bytes: 1_000,
        available_bytes,
    }
}

fn scripted_capacity_probe(
    readings: impl IntoIterator<Item = Option<crate::helpers::FilesystemSpace>>,
) -> TestCapacityProbe {
    let readings = Arc::new(StdMutex::new(VecDeque::from_iter(readings)));
    Arc::new(move |_| {
        readings
            .lock()
            .expect("capacity sequence lock")
            .pop_front()
            .expect("scripted capacity reading")
    })
}

/// Low readings taken before the first unlink of an armed title deletion: the
/// evaluation pass, then for each of the unmonitor and delete steps the safety
/// pass before the step is claimed and the one under the title guard.
fn low_readings_before_first_unlink() -> Vec<Option<crate::helpers::FilesystemSpace>> {
    vec![Some(capacity(100)); 5]
}

/// Low readings for a retry that resumes a held delete step: the scheduled
/// evaluation pass, then the delete step's two safety passes. The completed
/// unmonitor step is not run again.
fn low_readings_before_resumed_unlink() -> Vec<Option<crate::helpers::FilesystemSpace>> {
    vec![Some(capacity(100)); 3]
}

fn scripted(
    phases: impl IntoIterator<Item = Vec<Option<crate::helpers::FilesystemSpace>>>,
) -> TestCapacityProbe {
    scripted_capacity_probe(phases.into_iter().flatten())
}

async fn add_movie_file(fixture: &TitleFixture, relative_path: &str) -> (PathBuf, String) {
    let path = fixture.root.join(relative_path);
    let file_id = add_movie_file_at(fixture, &path).await;
    (path, file_id)
}

async fn add_movie_file_at(fixture: &TitleFixture, path: &Path) -> String {
    add_title_file_at(fixture, &fixture.title.id, path).await
}

async fn add_title_file_at(fixture: &TitleFixture, title_id: &str, path: &Path) -> String {
    std::fs::create_dir_all(path.parent().expect("media parent")).expect("create media parent");
    std::fs::write(path, b"video").expect("write media file");
    super::lifecycle_title_completion::record_fixture_title_folder(
        &fixture.execution.app,
        title_id,
        path.parent().unwrap(),
    )
    .await;
    fixture
        .execution
        .app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.to_string(),
            file_path: path.to_string_lossy().to_string(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert movie file")
}

/// A matcher for the fixture title that holds while any root its files are
/// on has under 200 bytes free.
fn low_space_matcher(fixture: &TitleFixture, extra: &str) -> String {
    format!(
        "match if {{ input.subject.title_id == {:?};{extra} some root in input.facts.storage_roots; root.available_bytes < 200 }}",
        fixture.title.id
    )
}

async fn arm_storage_title_deletion(fixture: &TitleFixture) -> String {
    arm_storage_title_deletion_with_matcher(fixture, low_space_matcher(fixture, "")).await
}

async fn arm_storage_title_deletion_with_matcher(
    fixture: &TitleFixture,
    matcher: String,
) -> String {
    let created = fixture
        .execution
        .app
        .create_lifecycle_rule_set(
            &fixture.execution.user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                name: "Storage pressure".into(),
                description: String::new(),
                rego_source: crate::lib_tests::legacy_source(
                    matcher,
                    crate::lifecycle_rules::LifecycleActionKind::UnmonitorTitleDeleteAllFiles,
                ),
                grace_days: 0,
                library_ids: vec![],
            },
        )
        .await
        .expect("create storage rule");
    super::lifecycle_rules::set_rule_state(
        &fixture.execution.app,
        &fixture.execution.user,
        &created.rule_set.id,
        scryer_domain::LifecycleRuleState::DryRun,
        None,
    )
    .await;
    super::lifecycle_rules::open_lifecycle_gates(&fixture.execution.app, false, true).await;
    fixture
        .execution
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("evaluate storage rule");
    let active_candidates = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .filter(|candidate| {
            candidate.rule_set_id == created.rule_set.id && !candidate.state.is_terminal()
        })
        .count();
    assert_eq!(active_candidates, 1, "one selected movie candidate");
    super::lifecycle_rules::arm_rule(
        &fixture.execution.app,
        &fixture.execution.user,
        &created.rule_set.id,
        LifecycleEffectArming::Destructive,
        Some(active_candidates as i64),
    )
    .await;
    created.rule_set.id
}

#[tokio::test]
async fn candidate_view_reports_every_owned_file_across_roots() {
    let fixture = title_fixture("Candidate Root Summary", MediaFacet::Movie).await;
    let root_b = second_movie_root(&fixture).await;
    let (probe, _reads) = two_root_probe(&root_b, Some(150));
    let _probe = install_lifecycle_storage_capacity_probe_for_test(probe).await;
    add_movie_file(&fixture, "Selected/movie.mkv").await;
    add_movie_file_at(&fixture, &root_b.join("Selected/movie.mkv")).await;

    let rule_set_id = arm_storage_title_deletion(&fixture).await;
    let candidate = fixture
        .execution
        .app
        .list_lifecycle_candidates(
            &fixture.execution.user,
            LifecycleCandidateFilter {
                rule_set_id: Some(rule_set_id),
                ..Default::default()
            },
        )
        .await
        .expect("list storage candidate")
        .into_iter()
        .next()
        .expect("storage candidate");

    assert_eq!(candidate.file_count, Some(2));
    assert_eq!(candidate.total_size_bytes, Some(10));
}

#[tokio::test]
async fn candidate_listing_subjects_match_evaluation_subjects_and_evaluation_still_stats() {
    use crate::lib_tests::lifecycle_title_completion::{add_episode_file, season_and_episode};
    use crate::lifecycle_rules::subjects::LifecycleListingInventories;

    let fixture = title_fixture("Cinderwake", MediaFacet::Series).await;
    let (_, present) = season_and_episode(&fixture, 1, 1, "2024-01-01").await;
    let (_, missing) = season_and_episode(&fixture, 2, 1, "2024-02-01").await;
    add_episode_file(&fixture, &present, "Cinderwake/S01E01.mkv").await;
    add_episode_file(&fixture, &missing, "Cinderwake/S02E01.mkv").await;
    std::fs::remove_file(fixture.root.join("Cinderwake/S02E01.mkv")).expect("remove one file");

    let app = &fixture.execution.app;
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(&fixture.title.id)
        .await
        .expect("read title")
        .expect("title exists");
    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("read title files");

    // Evaluation's loader still stats: only the episode whose file is on
    // disk counts as downloaded.
    let inventory = app
        .lifecycle_load_title_episode_inventory(&title)
        .await
        .expect("load evaluation inventory");
    let downloaded: HashMap<String, i64> = app
        .lifecycle_subjects_from_episode_inventory(
            LifecycleRuleSubjectKind::Episode,
            &title,
            &files,
            &inventory,
        )
        .expect("episode subjects")
        .into_iter()
        .map(|subject| (subject.id, subject.episode_file_count))
        .collect();
    assert_eq!(downloaded.get(&present.id), Some(&1));
    assert_eq!(downloaded.get(&missing.id), Some(&0));

    let mut inventories = LifecycleListingInventories::default();
    for kind in [
        LifecycleRuleSubjectKind::Title,
        LifecycleRuleSubjectKind::Season,
        LifecycleRuleSubjectKind::Episode,
    ] {
        let fresh: Vec<_> = app
            .lifecycle_subjects_for_title(kind, &title, &files)
            .await
            .expect("fresh subjects")
            .into_iter()
            .map(|subject| {
                (
                    subject.id,
                    subject.label,
                    subject
                        .files
                        .into_iter()
                        .map(|file| file.id)
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let listed: Vec<_> = app
            .lifecycle_listing_subjects(kind, &title, &files, &mut inventories)
            .await
            .expect("listing subjects")
            .into_iter()
            .map(|subject| {
                (
                    subject.id,
                    subject.label,
                    subject
                        .files
                        .into_iter()
                        .map(|file| file.id)
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert!(!fresh.is_empty(), "{kind:?} has subjects");
        assert_eq!(listed, fresh, "{kind:?} listing subjects");
    }
}

#[tokio::test]
async fn evaluation_pass_probes_each_file_once_and_sees_what_a_fresh_load_sees() {
    use crate::lib_tests::lifecycle_title_completion::{add_episode_file, season_and_episode};
    use crate::lifecycle_rules::subjects::LifecyclePassFilePresence;

    let fixture = title_fixture("Quillmarsh", MediaFacet::Series).await;
    let (_, first) = season_and_episode(&fixture, 1, 1, "2024-01-01").await;
    let (_, second) = season_and_episode(&fixture, 2, 1, "2024-02-01").await;
    let (_, gone) = season_and_episode(&fixture, 3, 1, "2024-03-01").await;
    add_episode_file(&fixture, &first, "Quillmarsh/S01E01.mkv").await;
    add_episode_file(&fixture, &second, "Quillmarsh/S02E01.mkv").await;
    add_episode_file(&fixture, &gone, "Quillmarsh/S03E01.mkv").await;
    std::fs::remove_file(fixture.root.join("Quillmarsh/S03E01.mkv")).expect("remove one file");

    let app = &fixture.execution.app;
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(&fixture.title.id)
        .await
        .expect("read title")
        .expect("title exists");
    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("read title files");
    let downloaded = |inventory: std::sync::Arc<
        crate::lifecycle_rules::subjects::LifecycleTitleEpisodeInventory,
    >|
     -> Vec<(String, i64)> {
        let mut counts: Vec<_> = app
            .lifecycle_subjects_from_episode_inventory(
                LifecycleRuleSubjectKind::Episode,
                &title,
                &files,
                &inventory,
            )
            .expect("episode subjects")
            .into_iter()
            .map(|subject| (subject.id, subject.episode_file_count))
            .collect();
        counts.sort();
        counts
    };

    // The first rule of a pass loads exactly what a fresh load sees.
    let mut presence = LifecyclePassFilePresence::default();
    let fresh = downloaded(
        app.lifecycle_load_title_episode_inventory(&title)
            .await
            .expect("fresh inventory"),
    );
    let first_rule = downloaded(
        app.lifecycle_load_title_episode_inventory_for_pass(&title, &mut presence)
            .await
            .expect("first rule inventory"),
    );
    assert_eq!(first_rule, fresh);
    assert_eq!(presence.probed_paths(), 3, "one probe per file");

    // A later rule of the same pass reuses those probes rather than asking
    // the share again: removing a file in between is not seen until the next
    // pass, while a fresh load (preview, execution) sees it at once.
    std::fs::remove_file(fixture.root.join("Quillmarsh/S02E01.mkv")).expect("remove second");
    let later_rule = downloaded(
        app.lifecycle_load_title_episode_inventory_for_pass(&title, &mut presence)
            .await
            .expect("later rule inventory"),
    );
    assert_eq!(later_rule, first_rule);
    assert_eq!(presence.probed_paths(), 3, "no further probes");
    let fresh_after = downloaded(
        app.lifecycle_load_title_episode_inventory(&title)
            .await
            .expect("fresh inventory after removal"),
    );
    assert_ne!(fresh_after, later_rule);
}

/// The batched page read builds the same inventory and probes the same files
/// as loading each title on its own, over
/// several shows, a movie and a show with no episodes.
#[tokio::test]
async fn a_page_of_inventories_read_together_matches_loading_each_title() {
    use crate::lib_tests::lifecycle_title_completion::{add_episode_file, season_and_episode};
    use crate::lifecycle_rules::subjects::LifecyclePassFilePresence;

    let fixture = title_fixture("Quillmarsh", MediaFacet::Series).await;
    let (_, first) = season_and_episode(&fixture, 1, 1, "2024-01-01").await;
    let (_, second) = season_and_episode(&fixture, 2, 1, "2024-02-01").await;
    let (_, gone) = season_and_episode(&fixture, 3, 1, "2024-03-01").await;
    let (_, _unfiled) = season_and_episode(&fixture, 3, 2, "2024-03-08").await;
    add_episode_file(&fixture, &first, "Quillmarsh/S01E01.mkv").await;
    add_episode_file(&fixture, &second, "Quillmarsh/S02E01.mkv").await;
    add_episode_file(&fixture, &gone, "Quillmarsh/S03E01.mkv").await;
    std::fs::remove_file(fixture.root.join("Quillmarsh/S03E01.mkv")).expect("remove one file");

    let app = &fixture.execution.app;
    let user = &fixture.execution.user;
    let add = |name: &str, facet: MediaFacet| {
        let name = name.to_string();
        async move {
            app.add_title(
                user,
                NewTitle {
                    name,
                    facet,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    ..Default::default()
                },
            )
            .await
            .expect("create title")
        }
    };
    let second_show = add("Brackenfold", MediaFacet::Series).await;
    let empty_show = add("Thistledown", MediaFacet::Series).await;
    let movie_root = fixture
        .root
        .parent()
        .expect("fixture parent")
        .join("movies");
    std::fs::create_dir_all(&movie_root).expect("movie root");
    update_default_library_roots(
        app,
        user,
        MediaFacet::Movie,
        vec![build_root_folder_entry(&movie_root, true)],
    )
    .await
    .expect("save movie root");
    let movie = add("Lanternwick", MediaFacet::Movie).await;
    let movie_path = movie_root.join("Lanternwick/Lanternwick.mkv");
    std::fs::create_dir_all(movie_path.parent().unwrap()).expect("movie folder");
    std::fs::write(&movie_path, b"video").expect("movie file");
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: movie.id.clone(),
            file_path: movie_path.to_string_lossy().to_string(),
            size_bytes: 7,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert movie file");

    let season = app
        .create_collection(
            user,
            second_show.id.clone(),
            "season".into(),
            "1".into(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create season");
    let mut other_episodes = Vec::new();
    for (number, air_date) in [(1, "2023-05-01"), (2, "2023-05-08")] {
        other_episodes.push(
            app.create_episode(
                user,
                second_show.id.clone(),
                Some(season.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".to_string()),
                None,
                Some(format!("Episode 1-{number}")),
                Some(air_date.to_string()),
                None,
                false,
                false,
            )
            .await
            .expect("create episode"),
        );
    }
    let other_path = fixture.root.join("Brackenfold/S01E01.mkv");
    std::fs::create_dir_all(other_path.parent().unwrap()).expect("show folder");
    std::fs::write(&other_path, b"video").expect("show file");
    let other_file = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: second_show.id.clone(),
            file_path: other_path.to_string_lossy().to_string(),
            size_bytes: 9,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert show file");
    app.services
        .library
        .media_files
        .link_file_to_episode(&other_file, &other_episodes[0].id)
        .await
        .expect("link show file");

    let mut titles = Vec::new();
    for id in [
        &fixture.title.id,
        &movie.id,
        &second_show.id,
        &empty_show.id,
    ] {
        titles.push(
            app.services
                .catalog
                .titles
                .get_by_id(id)
                .await
                .expect("read title")
                .expect("title exists"),
        );
    }

    // Preview's form: no pass memo.
    let batched = app
        .lifecycle_load_title_episode_inventories(&titles, None)
        .await;
    let mut shows = Vec::new();
    for title in &titles {
        if title.facet == MediaFacet::Movie {
            assert!(!batched.contains_key(&title.id), "a movie has no inventory");
            continue;
        }
        let alone = app
            .lifecycle_load_title_episode_inventory(title)
            .await
            .expect("per-title inventory");
        assert_eq!(
            batched[&title.id].comparable(),
            alone.comparable(),
            "{} inventory",
            title.name
        );
        shows.push(title.id.clone());
    }
    let mut keys: Vec<_> = batched.keys().cloned().collect();
    keys.sort();
    shows.sort();
    assert_eq!(keys, shows);

    // The evaluation pass's form: one probe per file, as the per-title loads
    // would make, and the same inventories.
    let mut batched_presence = LifecyclePassFilePresence::default();
    let with_memo = app
        .lifecycle_load_title_episode_inventories(&titles, Some(&mut batched_presence))
        .await;
    let mut per_title_presence = LifecyclePassFilePresence::default();
    for title in titles
        .iter()
        .filter(|title| title.facet != MediaFacet::Movie)
    {
        let alone = app
            .lifecycle_load_title_episode_inventory_for_pass(title, &mut per_title_presence)
            .await
            .expect("per-title pass inventory");
        assert_eq!(with_memo[&title.id].comparable(), alone.comparable());
    }
    assert_eq!(batched_presence.probed_paths(), 4);
    assert_eq!(
        batched_presence.probed_paths(),
        per_title_presence.probed_paths()
    );
}

#[tokio::test]
async fn storage_recovery_stops_before_the_next_file() {
    // Low through the first file, then a recovered capacity before the second
    // file. The recovery must cancel the remaining manifest.
    let _probe = install_lifecycle_storage_capacity_probe_for_test(scripted([
        low_readings_before_first_unlink(),
        vec![Some(capacity(100)), Some(capacity(700))],
    ]))
    .await;
    let fixture = title_fixture("Pressure Movie", MediaFacet::Movie).await;
    let (first, _) = add_movie_file(&fixture, "Pressure/first.mkv").await;
    let (second, _) = add_movie_file(&fixture, "Pressure/second.mkv").await;

    arm_storage_title_deletion(&fixture).await;
    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("run storage deletion");
    assert_eq!(
        report.canceled, 1,
        "recovered capacity cancels remaining work"
    );
    assert_eq!(report.executed, 0);
    assert_ne!(
        first.exists(),
        second.exists(),
        "exactly one file was deleted"
    );
    let action_runs =
        crate::lib_tests::deletion_runs(fixture.execution.evaluation.all_action_runs().await);
    assert_eq!(action_runs.len(), 1);
    assert_eq!(action_runs[0].status, LifecycleActionRunStatus::Held);
}

#[tokio::test]
async fn unknown_storage_capacity_holds_without_consuming_an_attempt_then_resumes() {
    // Everything through the first file is low. The second per-file refresh
    // is unavailable, then scheduled evaluation and a retry can reuse the
    // incomplete manifest without repeating file one.
    let _probe = install_lifecycle_storage_capacity_probe_for_test(scripted([
        low_readings_before_first_unlink(),
        vec![Some(capacity(100)), None],
        low_readings_before_resumed_unlink(),
        vec![Some(capacity(100))],
    ]))
    .await;
    let fixture = title_fixture("Recoverable Unknown", MediaFacet::Movie).await;
    let (first_path, first_id) = add_movie_file(&fixture, "Recoverable/first.mkv").await;
    let (second_path, second_id) = add_movie_file(&fixture, "Recoverable/second.mkv").await;
    let rule_set_id = arm_storage_title_deletion(&fixture).await;

    let held = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("hold while capacity is unavailable");
    assert_eq!(held.held, 1);
    let action_runs =
        crate::lib_tests::deletion_runs(fixture.execution.evaluation.all_action_runs().await);
    assert_eq!(action_runs.len(), 1);
    assert_eq!(action_runs[0].status, LifecycleActionRunStatus::Held);
    let detail: serde_json::Value =
        serde_json::from_str(&action_runs[0].detail).expect("parse partial deletion journal");
    let completed_ids = detail["files"]
        .as_array()
        .expect("journal files")
        .iter()
        .filter(|file| file["completed"] == true)
        .filter_map(|file| file["file_id"].as_str())
        .collect::<HashSet<_>>();
    assert_eq!(completed_ids.len(), 1, "one file completed before the hold");
    let (completed_path, remaining_path) = if completed_ids.contains(first_id.as_str()) {
        (&first_path, &second_path)
    } else {
        assert!(completed_ids.contains(second_id.as_str()));
        (&second_path, &first_path)
    };
    assert!(
        !completed_path.exists(),
        "journaled completed file stays deleted"
    );
    assert!(
        remaining_path.exists(),
        "unknown capacity preserves the next file"
    );
    let held_candidate = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .find(|candidate| candidate.rule_set_id == rule_set_id)
        .expect("candidate remains for recovery");
    let held_steps = fixture
        .execution
        .evaluation
        .action_steps_with_id("delete_files")
        .await;
    assert_eq!(held_steps.len(), 1);
    assert_eq!(held_steps[0].state, LifecycleActionStepState::Held);
    assert_eq!(
        held_steps[0].attempt, 1,
        "unknown capacity does not consume an attempt"
    );
    let match_generation = held_candidate.match_generation;
    let due_at = held_candidate.due_at;

    fixture
        .execution
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("re-evaluate recoverable candidate");
    let after_evaluation = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .find(|candidate| candidate.rule_set_id == rule_set_id)
        .expect("partial candidate stays active after evaluation");
    assert_eq!(after_evaluation.match_generation, match_generation);
    assert_eq!(after_evaluation.due_at, due_at);

    let resumed = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("resume after fresh capacity returns");
    assert_eq!(
        resumed.executed, 1,
        "recovered low capacity resumes deletion"
    );
    assert!(
        !completed_path.exists(),
        "retry resumes at the journaled remaining file"
    );
    assert!(!remaining_path.exists());
    let resumed_steps = fixture
        .execution
        .evaluation
        .action_steps_with_id("delete_files")
        .await;
    assert_eq!(resumed_steps.len(), 1);
    assert_eq!(resumed_steps[0].state, LifecycleActionStepState::Succeeded);
    assert_eq!(
        resumed_steps[0].attempt, 1,
        "the resumed delete reuses the attempt the unknown hold refunded"
    );
    let action_runs =
        crate::lib_tests::deletion_runs(fixture.execution.evaluation.all_action_runs().await);
    // The resumed delete step continues the held journal row rather than
    // opening a second one.
    assert_eq!(action_runs.len(), 1);
    assert_eq!(action_runs[0].status, LifecycleActionRunStatus::Succeeded);
}

#[tokio::test]
async fn storage_retry_reconciles_a_journaled_path_missing_before_catalog_cleanup() {
    // The delete step creates and persists the manifest, then capacity becomes
    // unavailable before unlink. Removing the path now models a crash after an
    // authorized unlink but before the catalog row and journal were reconciled.
    let _probe = install_lifecycle_storage_capacity_probe_for_test(scripted([
        low_readings_before_first_unlink(),
        vec![None],
        low_readings_before_resumed_unlink(),
        vec![Some(capacity(100))],
    ]))
    .await;
    let fixture = title_fixture("Interrupted Cleanup", MediaFacet::Movie).await;
    let (media_path, media_file_id) = add_movie_file(&fixture, "Interrupted/movie.mkv").await;
    arm_storage_title_deletion(&fixture).await;

    let held = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("persist manifest before the interrupted unlink");
    assert_eq!(held.held, 1);
    let held_runs =
        crate::lib_tests::deletion_runs(fixture.execution.evaluation.all_action_runs().await);
    assert_eq!(held_runs.len(), 1);
    let held_detail: serde_json::Value =
        serde_json::from_str(&held_runs[0].detail).expect("parse held journal");
    assert_eq!(
        held_detail["files"]
            .as_array()
            .and_then(|files| files.first())
            .and_then(|file| file["file_id"].as_str()),
        Some(media_file_id.as_str()),
        "the retry may clean up only this persisted journal file"
    );
    assert!(media_path.exists());

    std::fs::remove_file(&media_path).expect("simulate unlink before catalog cleanup");
    fixture
        .execution
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("re-evaluate exact missing journal path");
    let resumed = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("reconcile missing journal path");
    assert_eq!(resumed.executed, 1, "root summary does not cancel recovery");
    assert_eq!(resumed.canceled, 0);
    assert!(!media_path.exists());
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&media_file_id)
            .await
            .expect("read cleaned catalog row")
            .is_none(),
        "the exact missing path's catalog row is reconciled"
    );
    let action_runs =
        crate::lib_tests::deletion_runs(fixture.execution.evaluation.all_action_runs().await);
    // The resumed delete step continues the held journal row rather than
    // opening a second one.
    assert_eq!(action_runs.len(), 1);
    assert_eq!(action_runs[0].status, LifecycleActionRunStatus::Succeeded);
    let completed_detail: serde_json::Value =
        serde_json::from_str(&action_runs[0].detail).expect("parse completed journal");
    assert_eq!(
        completed_detail["files"]
            .as_array()
            .and_then(|files| files.first())
            .and_then(|file| file["file_id"].as_str()),
        Some(media_file_id.as_str())
    );
    assert_eq!(
        completed_detail["files"]
            .as_array()
            .and_then(|files| files.first())
            .and_then(|file| file["completed"].as_bool()),
        Some(true)
    );
}

#[tokio::test]
async fn a_file_replaced_after_its_deletion_was_planned_is_preserved_on_retry() {
    // The delete step persists its plan, then capacity becomes unavailable
    // before unlink. A replacement written to the planned path afterwards no
    // longer matches the plan's proof, so the retry must keep it.
    let _probe = install_lifecycle_storage_capacity_probe_for_test(scripted([
        low_readings_before_first_unlink(),
        vec![None],
        low_readings_before_resumed_unlink(),
        vec![Some(capacity(100)); 2],
    ]))
    .await;
    let fixture = title_fixture("Replaced Plan", MediaFacet::Movie).await;
    let (media_path, media_file_id) = add_movie_file(&fixture, "Replaced/movie.mkv").await;
    arm_storage_title_deletion(&fixture).await;

    let held = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("persist the plan before the interrupted unlink");
    assert_eq!(held.held, 1);
    assert!(media_path.exists());

    std::fs::write(&media_path, b"new replacement version").expect("replace planned file");
    fixture
        .execution
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("re-evaluate the planned deletion");
    let retried = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("retry the planned deletion");
    assert_eq!(retried.executed, 0);
    assert_eq!(
        std::fs::read(&media_path).expect("read replacement"),
        b"new replacement version"
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&media_file_id)
            .await
            .expect("read retained media row")
            .is_some()
    );
    let runs =
        crate::lib_tests::deletion_runs(fixture.execution.evaluation.all_action_runs().await);
    let recorded_reasons: Vec<String> = runs
        .iter()
        .filter_map(|run| serde_json::from_str::<serde_json::Value>(&run.detail).ok())
        .flat_map(|detail| {
            detail["files"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|file| file["error_reason"].as_str().map(str::to_string))
        })
        .collect();
    assert!(
        recorded_reasons
            .iter()
            .any(|reason| reason == crate::error_reasons::library::LIFECYCLE_DELETION_FILE_CHANGED),
        "the journal records why the file was kept: {recorded_reasons:?}"
    );
}

/// Configure the fixture's movie library with its own root and a second one
/// beside it, returning the second.
async fn second_movie_root(fixture: &TitleFixture) -> PathBuf {
    let root_b = fixture
        .root
        .parent()
        .expect("fixture root parent")
        .join("root-b");
    configure_second_movie_root(fixture, root_b).await
}

/// Configure `root_b` as the movie library's second root, beside the
/// fixture's own.
async fn configure_second_movie_root(fixture: &TitleFixture, root_b: PathBuf) -> PathBuf {
    std::fs::create_dir_all(&root_b).expect("create second configured root");
    update_default_library_roots(
        &fixture.execution.app,
        &fixture.execution.user,
        MediaFacet::Movie,
        vec![
            build_root_folder_entry(&fixture.root, true),
            build_root_folder_entry(&root_b, false),
        ],
    )
    .await
    .expect("configure both movie roots");
    root_b
}

/// A probe that answers 100 bytes free for the fixture root and 250 for the
/// second root, or nothing for it, and counts the reads of each path.
fn two_root_probe(
    root_b: &Path,
    root_b_figure: Option<u64>,
) -> (TestCapacityProbe, Arc<StdMutex<HashMap<String, usize>>>) {
    let reads = Arc::new(StdMutex::new(HashMap::<String, usize>::new()));
    let counted = reads.clone();
    let root_b = root_b.to_string_lossy().into_owned();
    (
        Arc::new(move |path: &str| {
            *counted
                .lock()
                .expect("probe reads lock")
                .entry(path.to_string())
                .or_default() += 1;
            if path == root_b {
                root_b_figure.map(capacity)
            } else {
                Some(capacity(100))
            }
        }),
        reads,
    )
}

/// Evaluate a dry-run rule over the fixture title and return its active
/// candidate count.
async fn evaluate_dry_run(fixture: &TitleFixture, matcher: String) -> usize {
    let app = &fixture.execution.app;
    let created = app
        .create_lifecycle_rule_set(
            &fixture.execution.user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                name: "Two root facts".into(),
                description: String::new(),
                rego_source: crate::lib_tests::legacy_source(
                    matcher,
                    crate::lifecycle_rules::LifecycleActionKind::UnmonitorTitleDeleteAllFiles,
                ),
                grace_days: 0,
                library_ids: vec![],
            },
        )
        .await
        .expect("create storage rule");
    super::lifecycle_rules::set_rule_state(
        app,
        &fixture.execution.user,
        &created.rule_set.id,
        scryer_domain::LifecycleRuleState::DryRun,
        None,
    )
    .await;
    super::lifecycle_rules::open_lifecycle_gates(app, false, true).await;
    app.run_lifecycle_rule_evaluation_job()
        .await
        .expect("evaluate storage rule");
    fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .filter(|candidate| {
            candidate.rule_set_id == created.rule_set.id && !candidate.state.is_terminal()
        })
        .count()
}

/// The storage facts list each configured root the subject's files are on,
/// once, with that root's own figures; each root is read once in the pass
/// however many of the subject's files are on it.
#[tokio::test]
async fn storage_facts_list_each_root_the_files_are_on_with_its_own_figures() {
    let fixture = title_fixture("Two Root Facts", MediaFacet::Movie).await;
    let root_b = second_movie_root(&fixture).await;
    let (probe, reads) = two_root_probe(&root_b, Some(250));
    let _probe = install_lifecycle_storage_capacity_probe_for_test(probe).await;
    add_movie_file(&fixture, "Facts/first.mkv").await;
    add_movie_file(&fixture, "Facts/second.mkv").await;
    add_movie_file_at(&fixture, &root_b.join("Facts/third.mkv")).await;

    let matcher = format!(
        "match if {{ input.subject.title_id == {title:?}; count(input.facts.storage_roots) == 2; \
         some a in input.facts.storage_roots; a.path == {root_a:?}; a.available_bytes == 100; \
         a.total_bytes == 1000; a.available_percent == 10; \
         some b in input.facts.storage_roots; b.path == {root_b:?}; b.available_bytes == 250; \
         a.root_id != b.root_id }}",
        title = fixture.title.id,
        root_a = fixture.root.to_string_lossy(),
        root_b = root_b.to_string_lossy(),
    );
    assert_eq!(evaluate_dry_run(&fixture, matcher).await, 1);
    let reads = reads.lock().expect("probe reads lock").clone();
    let root_a_key = fixture.root.to_string_lossy().into_owned();
    let root_b_key = root_b.to_string_lossy().into_owned();
    assert_eq!(reads.get(&root_a_key), Some(&1), "{reads:?}");
    assert_eq!(reads.get(&root_b_key), Some(&1), "{reads:?}");
}

/// A root with no usable figure makes the whole storage fact unknown, so a
/// rule that reads it holds instead of matching on the other root alone.
#[tokio::test]
async fn an_unavailable_root_makes_the_storage_facts_unknown_and_holds() {
    let fixture = title_fixture("Two Root Unknown", MediaFacet::Movie).await;
    let root_b = second_movie_root(&fixture).await;
    let (probe, _reads) = two_root_probe(&root_b, None);
    let _probe = install_lifecycle_storage_capacity_probe_for_test(probe).await;
    add_movie_file(&fixture, "Unknown/first.mkv").await;
    add_movie_file_at(&fixture, &root_b.join("Unknown/second.mkv")).await;

    assert_eq!(
        evaluate_dry_run(&fixture, low_space_matcher(&fixture, "")).await,
        0,
        "an unknown root never matches"
    );
}

/// A file under no configured root adds no root and leaves the fact known:
/// the rule decides on the roots the subject's other files are on.
#[tokio::test]
async fn a_file_under_no_configured_root_is_left_out_and_the_rule_still_decides() {
    let fixture = title_fixture("Unrooted File", MediaFacet::Movie).await;
    let (probe, reads) = two_root_probe(Path::new("/no-second-root"), None);
    let _probe = install_lifecycle_storage_capacity_probe_for_test(probe).await;
    add_movie_file(&fixture, "Unrooted/first.mkv").await;
    let outside_root = fixture
        .root
        .parent()
        .expect("fixture root parent")
        .join("unconfigured/Unrooted/second.mkv");
    add_movie_file_at(&fixture, &outside_root).await;

    let matcher = format!(
        "match if {{ input.subject.title_id == {title:?}; count(input.facts.storage_roots) == 1; \
         some root in input.facts.storage_roots; root.path == {root_a:?}; root.available_bytes < 200 }}",
        title = fixture.title.id,
        root_a = fixture.root.to_string_lossy(),
    );
    assert_eq!(evaluate_dry_run(&fixture, matcher).await, 1);
    let reads = reads.lock().expect("probe reads lock").clone();
    assert_eq!(
        reads.get(&fixture.root.to_string_lossy().into_owned()),
        Some(&1),
        "{reads:?}"
    );
    assert_eq!(
        reads.len(),
        1,
        "only the configured root is read: {reads:?}"
    );
}

/// Deletion does not narrow to a root: a storage rule's delete takes in the
/// subject's files on every configured root they are on. The title owns one
/// folder on the first root; its file on the second root lies outside that
/// folder, so its deletion cannot be planned and fails. The owned file is
/// still deleted, and every file beside them the title does not own is kept.
#[tokio::test]
async fn a_storage_delete_reaches_both_roots_and_every_ownership_check_still_holds() {
    let fixture = title_fixture("Two Root Delete", MediaFacet::Movie).await;
    let root_b = second_movie_root(&fixture).await;
    let (probe, reads) = two_root_probe(&root_b, Some(150));
    let _probe = install_lifecycle_storage_capacity_probe_for_test(probe).await;
    let (root_a_path, root_a_file_id) = add_movie_file(&fixture, "Two Roots/root-a.mkv").await;
    let root_b_path = root_b.join("Two Roots/root-b.mkv");
    let root_b_file_id = add_movie_file_at(&fixture, &root_b_path).await;
    let strays = [
        fixture.root.join("Two Roots/unowned.mkv"),
        root_b.join("Two Roots/unowned.mkv"),
        fixture.root.join("Neighbour/neighbour.mkv"),
    ];
    for stray in &strays {
        std::fs::create_dir_all(stray.parent().expect("stray parent")).expect("stray parent");
        std::fs::write(stray, b"not the title's").expect("write unowned file");
    }
    arm_storage_title_deletion(&fixture).await;

    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("run the two-root deletion");
    assert_eq!(report.executed, 0, "{report:?}");
    assert_eq!(report.failed, 1, "{report:?}");
    assert!(
        reads
            .lock()
            .expect("probe reads lock")
            .contains_key::<str>(&root_b.to_string_lossy()),
        "the second root's own figure was read"
    );
    let runs =
        crate::lib_tests::deletion_runs(fixture.execution.evaluation.all_action_runs().await);
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].status, LifecycleActionRunStatus::Failed, "{runs:?}");
    let detail: serde_json::Value =
        serde_json::from_str(&runs[0].detail).expect("parse deletion journal");
    let files = detail["files"].as_array().expect("journal files");
    let saved = |file_id: &str| {
        files
            .iter()
            .find(|file| file["file_id"].as_str() == Some(file_id))
            .cloned()
            .unwrap_or_else(|| panic!("{file_id} is journaled: {detail}"))
    };
    // The second root's file was taken into the manifest, not left out.
    assert_eq!(files.len(), 2, "{detail}");
    assert_eq!(saved(&root_a_file_id)["completed"], true, "{detail}");
    assert_eq!(saved(&root_b_file_id)["completed"], false, "{detail}");
    assert!(saved(&root_b_file_id)["error"].is_string(), "{detail}");

    let media_files = &fixture.execution.app.services.library.media_files;
    assert!(!root_a_path.exists(), "the owned file is deleted");
    assert!(
        media_files
            .get_media_file_by_id(&root_a_file_id)
            .await
            .expect("read deleted row")
            .is_none()
    );
    assert!(
        root_b_path.exists(),
        "the file outside the title folder is kept"
    );
    assert!(
        media_files
            .get_media_file_by_id(&root_b_file_id)
            .await
            .expect("read kept row")
            .is_some()
    );
    for stray in &strays {
        assert!(stray.exists(), "{} is kept", stray.display());
    }
}

/// A title folder that holds a configured root is a library root, not a
/// title folder, so neither file under it can be planned: both are journaled
/// with their error, the run fails, and neither file nor row is removed.
#[tokio::test]
async fn a_title_folder_holding_a_second_root_deletes_nothing() {
    let fixture = title_fixture("Nested Root Delete", MediaFacet::Movie).await;
    let (root_a_path, root_a_file_id) = add_movie_file(&fixture, "Nested Roots/root-a.mkv").await;
    let root_b =
        configure_second_movie_root(&fixture, fixture.root.join("Nested Roots/Extras")).await;
    let (probe, _reads) = two_root_probe(&root_b, Some(150));
    let _probe = install_lifecycle_storage_capacity_probe_for_test(probe).await;
    let root_b_path = root_b.join("root-b.mkv");
    let root_b_file_id = add_movie_file_at(&fixture, &root_b_path).await;
    arm_storage_title_deletion(&fixture).await;

    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("run the nested-root deletion");
    assert_eq!(report.executed, 0, "{report:?}");
    assert_eq!(report.held, 0, "{report:?}");
    assert_eq!(report.failed, 1, "{report:?}");
    let runs =
        crate::lib_tests::deletion_runs(fixture.execution.evaluation.all_action_runs().await);
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].status, LifecycleActionRunStatus::Failed, "{runs:?}");
    let detail: serde_json::Value =
        serde_json::from_str(&runs[0].detail).expect("parse deletion journal");
    let files = detail["files"].as_array().expect("journal files");
    assert_eq!(files.len(), 2, "{detail}");
    for file_id in [&root_a_file_id, &root_b_file_id] {
        let saved = files
            .iter()
            .find(|file| file["file_id"].as_str() == Some(file_id.as_str()))
            .unwrap_or_else(|| panic!("{file_id} is journaled: {detail}"));
        assert_eq!(saved["completed"], false, "{detail}");
        assert!(saved["plan"].is_null(), "{detail}");
        assert!(
            saved["error"]
                .as_str()
                .is_some_and(|error| error.contains("is a library root, not a title folder")),
            "{detail}"
        );
    }
    assert!(root_a_path.exists());
    assert!(root_b_path.exists());
    let media_files = &fixture.execution.app.services.library.media_files;
    for file_id in [&root_a_file_id, &root_b_file_id] {
        assert!(
            media_files
                .get_media_file_by_id(file_id)
                .await
                .expect("read kept row")
                .is_some()
        );
    }
}

/// A rule that does not read storage keeps the established contract: a file
/// whose deletion cannot be planned is journaled with its error and the
/// independently authorized siblings are still deleted.
#[tokio::test]
async fn a_rule_without_storage_facts_still_deletes_the_files_it_can_plan() {
    let fixture = title_fixture("Plain Two Root Delete", MediaFacet::Movie).await;
    let root_b = second_movie_root(&fixture).await;
    let (root_a_path, root_a_file_id) = add_movie_file(&fixture, "Plain/root-a.mkv").await;
    let root_b_path = root_b.join("Plain/root-b.mkv");
    let root_b_file_id = add_movie_file_at(&fixture, &root_b_path).await;
    arm_storage_title_deletion_with_matcher(
        &fixture,
        format!(
            "match if {{ input.subject.title_id == {:?} }}",
            fixture.title.id
        ),
    )
    .await;

    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("run the plain deletion");
    assert_eq!(report.executed, 0, "{report:?}");
    assert_eq!(report.failed, 1, "{report:?}");
    let runs =
        crate::lib_tests::deletion_runs(fixture.execution.evaluation.all_action_runs().await);
    assert_eq!(runs.len(), 1);
    let detail: serde_json::Value =
        serde_json::from_str(&runs[0].detail).expect("parse deletion journal");
    let files = detail["files"].as_array().expect("journal files");
    let saved = |file_id: &str| {
        files
            .iter()
            .find(|file| file["file_id"].as_str() == Some(file_id))
            .cloned()
            .unwrap_or_else(|| panic!("{file_id} is journaled: {detail}"))
    };
    assert_eq!(saved(&root_a_file_id)["completed"], true, "{detail}");
    assert_eq!(saved(&root_b_file_id)["completed"], false, "{detail}");
    assert!(saved(&root_b_file_id)["error"].is_string(), "{detail}");
    assert!(!root_a_path.exists(), "the owned file is deleted");
    assert!(root_b_path.exists(), "the unplanned file is kept");
}

/// A whole-title delete on a rule that reads storage is serialized with every
/// other storage-driven deletion and rechecks free space before it runs. Two
/// titles match while the root is low; deleting either one frees enough, so
/// the other no longer matches at its recheck and is kept.
#[tokio::test]
async fn a_storage_delete_title_rechecks_space_and_keeps_the_title_no_longer_needed() {
    let fixture = title_fixture("Pressure Fixture First", MediaFacet::Movie).await;
    let second = fixture
        .execution
        .app
        .add_title(
            &fixture.execution.user,
            NewTitle {
                name: "Pressure Fixture Second".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                ..Default::default()
            },
        )
        .await
        .expect("create second title");
    let first_path = fixture.root.join("Pressure First/movie.mkv");
    add_movie_file_at(&fixture, &first_path).await;
    let second_path = fixture.root.join("Pressure Second/movie.mkv");
    add_title_file_at(&fixture, &second.id, &second_path).await;
    // The root is low while both titles' files are on it; removing either
    // one leaves it with enough room.
    let watched = [first_path.clone(), second_path.clone()];
    let _probe = install_lifecycle_storage_capacity_probe_for_test(Arc::new(move |_| {
        Some(capacity(if watched.iter().all(|path| path.exists()) {
            100
        } else {
            900
        }))
    }))
    .await;

    let app = &fixture.execution.app;
    let created = app
        .create_lifecycle_rule_set(
            &fixture.execution.user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                name: "Storage pressure titles".into(),
                description: String::new(),
                rego_source: crate::lib_tests::legacy_source(
                    "match if { some root in input.facts.storage_roots; root.available_bytes < 200 }",
                    crate::lifecycle_rules::LifecycleActionKind::DeleteTitleAndFiles,
                ),
                grace_days: 0,
                library_ids: vec![],
            },
        )
        .await
        .expect("create storage title rule");
    super::lifecycle_rules::set_rule_state(
        app,
        &fixture.execution.user,
        &created.rule_set.id,
        scryer_domain::LifecycleRuleState::DryRun,
        None,
    )
    .await;
    super::lifecycle_rules::open_lifecycle_gates(app, false, true).await;
    app.run_lifecycle_rule_evaluation_job()
        .await
        .expect("evaluate storage title rule");
    let open = || async {
        fixture
            .execution
            .evaluation
            .all_candidates()
            .await
            .into_iter()
            .filter(|candidate| candidate.rule_set_id == created.rule_set.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        open()
            .await
            .iter()
            .filter(|candidate| !candidate.state.is_terminal())
            .count(),
        2,
        "both titles match while the root is low"
    );
    super::lifecycle_rules::arm_rule(
        app,
        &fixture.execution.user,
        &created.rule_set.id,
        LifecycleEffectArming::Destructive,
        Some(2),
    )
    .await;

    // Another storage-driven deletion is running: neither title is touched.
    let guard = app
        .runtime
        .jobs
        .interactive_operation_guards
        .try_acquire("maintenance-storage-deletion")
        .await
        .expect("hold storage deletion guard");
    let held = app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("pass while storage deletion is busy");
    assert_eq!(held.executed, 0, "{held:?}");
    assert_eq!(held.held, 2, "{held:?}");
    assert!(first_path.exists() && second_path.exists());
    let held_steps = fixture.execution.evaluation.all_step_attempts().await;
    assert!(
        held_steps
            .iter()
            .filter(|step| step.state == LifecycleActionStepState::Held)
            .all(|step| step.hold_reason.as_deref()
                == Some(crate::lifecycle_rules::execution_reason::STORAGE_DELETION_IN_PROGRESS)),
        "{held_steps:?}"
    );
    assert!(
        open().await.iter().all(|candidate| candidate.state_reason
            == crate::lifecycle_rules::execution_reason::STORAGE_DELETION_IN_PROGRESS),
        "both candidates wait on the storage deletion"
    );
    drop(guard);
    // The next scheduled evaluation still finds both titles matching.
    app.run_lifecycle_rule_evaluation_job()
        .await
        .expect("re-evaluate after the storage deletion ends");

    let report = app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("pass with the storage deletion free");
    assert_eq!(report.executed, 1, "{report:?}");
    assert_eq!(report.canceled, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    assert!(
        first_path.exists() != second_path.exists(),
        "exactly one title's file is deleted"
    );
    let (kept_id, deleted_id) = if first_path.exists() {
        (fixture.title.id.as_str(), second.id.as_str())
    } else {
        (second.id.as_str(), fixture.title.id.as_str())
    };
    let titles = &app.services.catalog.titles;
    assert!(
        titles
            .get_by_id(kept_id)
            .await
            .expect("read kept title")
            .is_some()
    );
    assert!(
        titles
            .get_by_id(deleted_id)
            .await
            .expect("read deleted title")
            .is_none()
    );
    let candidates = open().await;
    let kept = candidates
        .iter()
        .find(|candidate| candidate.title_id == kept_id)
        .expect("kept title's candidate");
    assert_eq!(kept.state, LifecycleCandidateState::Canceled, "{kept:?}");
    assert_eq!(
        kept.state_reason,
        crate::lifecycle_rules::execution_reason::NO_MATCH_AT_EXECUTION,
        "{kept:?}"
    );
}

#[tokio::test]
async fn storage_guard_hold_before_mutation_rechecks_live_monitoring_on_retry() {
    // The storage guard holds the delete step after the unmonitor step has
    // already run. An operator then re-monitors the title. The retry must see
    // that live state: the unmonitor baseline may stand in for the monitored
    // fact only while Scryer's own unmonitor still holds, and the delete step
    // rechecks its unmonitored prerequisite before any unlink.
    let _probe = install_lifecycle_storage_capacity_probe_for_test(scripted([
        low_readings_before_first_unlink(),
        low_readings_before_resumed_unlink(),
        vec![Some(capacity(100)); 2],
    ]))
    .await;
    let fixture = title_fixture("Guarded Retry", MediaFacet::Movie).await;
    let (media_path, media_file_id) = add_movie_file(&fixture, "Guarded/movie.mkv").await;
    let matcher = low_space_matcher(&fixture, " input.facts.monitored;");
    arm_storage_title_deletion_with_matcher(&fixture, matcher).await;
    let guard = fixture
        .execution
        .app
        .runtime
        .jobs
        .interactive_operation_guards
        .try_acquire("maintenance-storage-deletion")
        .await
        .expect("hold storage deletion guard");

    let held = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("hold before mutation starts");
    assert_eq!(held.held, 1);
    assert!(media_path.exists());
    let held_runs =
        crate::lib_tests::deletion_runs(fixture.execution.evaluation.all_action_runs().await);
    let held_detail: serde_json::Value =
        serde_json::from_str(&held_runs[0].detail).expect("parse pre-mutation journal");
    assert_eq!(held_detail["mutation_started"], false);
    let held_steps = fixture
        .execution
        .evaluation
        .action_steps_with_id("delete_files")
        .await;
    assert_eq!(held_steps.len(), 1);
    assert_eq!(held_steps[0].state, LifecycleActionStepState::Held);
    assert_eq!(
        held_steps[0].attempt, 1,
        "guard hold does not consume an attempt"
    );

    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, true)
        .await
        .expect("re-monitor while mutation has not started");
    drop(guard);
    let retried = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("retry with live monitoring state");
    assert_eq!(retried.executed, 0);
    assert!(
        media_path.exists(),
        "an unstarted checkpoint never deletes over a live re-monitor"
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&media_file_id)
            .await
            .expect("read retained media row")
            .is_some()
    );
}

#[tokio::test]
async fn a_tracked_subtitle_outside_every_root_holds_the_manifest_before_video_deletion() {
    let _probe = install_lifecycle_storage_capacity_probe_for_test(scripted([
        low_readings_before_first_unlink(),
    ]))
    .await;
    let mut fixture = title_fixture("Bounded Subtitle", MediaFacet::Movie).await;
    let subtitles = Arc::new(TrackedSubtitleRepo::default());
    fixture.execution.app = fixture
        .execution
        .app
        .with_test_overrides(|services| services.with_subtitle_downloads(subtitles.clone()));
    let (video_path, media_file_id) = add_movie_file(&fixture, "Bounded/video.mkv").await;
    let subtitle_path = fixture
        .root
        .parent()
        .expect("fixture root parent")
        .join("outside/Bounded.en.srt");
    std::fs::create_dir_all(subtitle_path.parent().expect("subtitle parent"))
        .expect("create subtitle parent");
    std::fs::write(&subtitle_path, b"subtitle").expect("write tracked subtitle");
    subtitles
        .insert(&SubtitleDownload {
            id: Id::new().0,
            media_file_id,
            title_id: fixture.title.id.clone(),
            episode_id: None,
            source_kind: ExternalSubtitleSourceKind::Downloaded,
            language: "eng".to_string(),
            provider: Some("fixture".to_string()),
            provider_file_id: Some("fixture-subtitle".to_string()),
            file_path: subtitle_path.to_string_lossy().to_string(),
            score: Some(100),
            hearing_impaired: false,
            forced: false,
            ai_translated: false,
            machine_translated: false,
            uploader: None,
            release_info: None,
            synced: false,
            downloaded_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("track off-root subtitle");

    arm_storage_title_deletion(&fixture).await;
    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("hold cross-root manifest");
    assert_eq!(report.held, 1);
    assert_eq!(report.executed, 0);
    assert!(
        video_path.exists(),
        "cross-root manifest never unlinks video"
    );
    assert!(
        subtitle_path.exists(),
        "tracked subtitle remains on its own root"
    );
    let candidate = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .next()
        .expect("candidate remains held");
    assert!(!candidate.state.is_terminal(), "candidate remains held");
    let held_steps = fixture
        .execution
        .evaluation
        .action_steps_with_id("delete_files")
        .await;
    assert_eq!(held_steps.len(), 1);
    assert_eq!(held_steps[0].state, LifecycleActionStepState::Held);
    assert_eq!(held_steps[0].attempt, 1);
}

/// Scheduled evaluation over a storage rule, with no scripted figure: the
/// root's free space comes from the capacity owner's cache. Returns the
/// disk calls the pass made and the rule's active candidate count.
async fn scheduled_storage_pass_from_sampler(
    fixture: &TitleFixture,
    sampled_at: Option<chrono::DateTime<chrono::Utc>>,
) -> (u64, usize) {
    add_movie_file(fixture, "Sampled/movie.mkv").await;
    let app = &fixture.execution.app;
    let capacity = &app.runtime.health.storage_capacity;
    if let Some(sampled_at) = sampled_at {
        capacity.record_for_test(
            &fixture.root.to_string_lossy(),
            sampled_at,
            Some(capacity_figure(100)),
        );
    }
    let created = app
        .create_lifecycle_rule_set(
            &fixture.execution.user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                name: "Storage pressure".into(),
                description: String::new(),
                rego_source: crate::lib_tests::legacy_source(
                    low_space_matcher(fixture, ""),
                    crate::lifecycle_rules::LifecycleActionKind::UnmonitorTitleDeleteAllFiles,
                ),
                grace_days: 0,
                library_ids: vec![],
            },
        )
        .await
        .expect("create storage rule");
    super::lifecycle_rules::set_rule_state(
        app,
        &fixture.execution.user,
        &created.rule_set.id,
        scryer_domain::LifecycleRuleState::DryRun,
        None,
    )
    .await;
    super::lifecycle_rules::open_lifecycle_gates(app, false, true).await;

    let calls_before = capacity.disk_calls();
    app.run_lifecycle_rule_evaluation_job()
        .await
        .expect("evaluate storage rule");
    let calls = capacity.disk_calls() - calls_before;
    let candidates = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .filter(|candidate| {
            candidate.rule_set_id == created.rule_set.id && !candidate.state.is_terminal()
        })
        .count();
    (calls, candidates)
}

fn capacity_figure(available_bytes: u64) -> crate::helpers::FilesystemSpace {
    crate::helpers::FilesystemSpace {
        total_bytes: 1_000,
        available_bytes,
    }
}

#[tokio::test]
async fn scheduled_evaluation_reads_free_space_from_the_sampler_and_never_the_disk() {
    let fixture = title_fixture("Sampled Root Movie", MediaFacet::Movie).await;
    let (calls, candidates) =
        scheduled_storage_pass_from_sampler(&fixture, Some(chrono::Utc::now())).await;
    assert_eq!(calls, 0, "scheduled evaluation made a disk call");
    assert_eq!(candidates, 1, "the sampled figure matched the rule");
}

#[tokio::test]
async fn scheduled_evaluation_holds_an_unsampled_or_stale_root_unknown_without_a_disk_call() {
    let unsampled = title_fixture("Unsampled Root Movie", MediaFacet::Movie).await;
    let (calls, candidates) = scheduled_storage_pass_from_sampler(&unsampled, None).await;
    assert_eq!(calls, 0, "an unsampled root was called");
    assert_eq!(candidates, 0, "an unsampled root is unknown, not a match");

    let stale = title_fixture("Stale Root Movie", MediaFacet::Movie).await;
    let (calls, candidates) = scheduled_storage_pass_from_sampler(
        &stale,
        Some(chrono::Utc::now() - chrono::Duration::hours(1)),
    )
    .await;
    assert_eq!(calls, 0, "a stale root was called");
    assert_eq!(candidates, 0, "a stale root is unknown, not a match");
}
