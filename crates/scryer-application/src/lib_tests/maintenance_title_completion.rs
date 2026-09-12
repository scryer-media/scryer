//! Acceptance coverage for title-preserving maintenance deletion and the
//! show-level facts that authorize it.
//!
//! These tests deliberately use the lifecycle evaluator and the scoped
//! deletion journal with real temporary files. They keep the policy journey
//! together: subject builder, preview, grace candidate, execution, and the
//! retained catalog/history evidence.

use super::*;

use crate::lib_tests::maintenance_execution::{ExecutionFixture, execution_app};
use crate::lib_tests::media_server_signals::{
    CONNECTION_ID, InMemorySignalRepo, StubExternalAccounts, StubSignalConnections,
    jellyfin_connection,
};
use crate::maintenance_rules::{
    MaintenanceActionKind, MaintenanceActionSpec, MaintenanceGatesUpdate,
    MaintenancePreviewMatcher, MaintenancePreviewRequest, MaintenancePreviewSelection,
    MaintenanceRuleDraft,
};
use crate::ports::MediaServerSignalRepository;
use scryer_domain::{
    ExternalAccountProvider, ExternalAccountStatus, LifecycleActionRunStatus,
    MaintenanceCandidateState, MaintenanceEffectArming, MaintenanceEvaluationMode,
    MaintenanceRuleSubjectKind, MediaServerProvider, MediaServerSignalKind,
    MediaServerSignalSyncState, NewUserMediaSignal, UserExternalAccount,
};
use scryer_rules::maintenance::MaintenanceOutcome;
use std::path::{Path, PathBuf};

const RETAIN_NEWEST_EPISODE: &str = "package retention\n\
    import rego.v1\n\n\
    match if {\n\
    \tinput.facts.episode_position_by_air_date > 1\n\
    }\n";

const COMPLETE_SHOW_WATCH: &str = "package watched\n\
    import rego.v1\n\n\
    match if {\n\
    \tcount(input.facts.watched_by_user_ids) > 0\n\
    }\n";

pub(super) struct TitleFixture {
    pub(super) execution: ExecutionFixture,
    pub(super) title: Title,
    pub(super) root: PathBuf,
    _tempdir: tempfile::TempDir,
}

pub(super) async fn title_fixture(name: &str, facet: MediaFacet) -> TitleFixture {
    let execution = execution_app(None);
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("series");
    std::fs::create_dir_all(&root).expect("create media root");
    execution
        .app
        .update_media_settings(
            &execution.user,
            facet.clone(),
            empty_update_media_settings_with_roots(vec![build_root_folder_entry(&root, true)]),
        )
        .await
        .expect("save series root");
    let title = execution
        .app
        .add_title(
            &execution.user,
            NewTitle {
                name: name.to_string(),
                facet,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                ..Default::default()
            },
        )
        .await
        .expect("create series");
    TitleFixture {
        execution,
        title,
        root,
        _tempdir: tempdir,
    }
}

pub(super) async fn season_and_episode(
    fixture: &TitleFixture,
    season_number: i32,
    episode_number: i32,
    air_date: &str,
) -> (Collection, Episode) {
    let collection = fixture
        .execution
        .app
        .create_collection(
            &fixture.execution.user,
            fixture.title.id.clone(),
            if season_number == 0 {
                "specials"
            } else {
                "season"
            }
            .into(),
            season_number.to_string(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create season");
    let episode = fixture
        .execution
        .app
        .create_episode(
            &fixture.execution.user,
            fixture.title.id.clone(),
            Some(collection.id.clone()),
            if season_number == 0 {
                "special"
            } else {
                "standard"
            }
            .into(),
            Some(episode_number.to_string()),
            Some(season_number.to_string()),
            None,
            Some(format!("Episode {season_number}-{episode_number}")),
            Some(air_date.to_string()),
            None,
            false,
            false,
        )
        .await
        .expect("create episode");
    (collection, episode)
}

pub(super) async fn add_episode_file(
    fixture: &TitleFixture,
    episode: &Episode,
    relative_path: &str,
) -> String {
    let path = fixture.root.join(relative_path);
    write_file(&path);
    let file_id = fixture
        .execution
        .app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: fixture.title.id.clone(),
            file_path: path.to_string_lossy().to_string(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert media file");
    fixture
        .execution
        .app
        .services
        .library
        .media_files
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link media file to episode");
    file_id
}

fn write_file(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("media path parent"))
        .expect("create media parent");
    std::fs::write(path, b"video").expect("write media file");
}

async fn create_destructive_rule(
    fixture: &ExecutionFixture,
    subject_kind: MaintenanceRuleSubjectKind,
    action_kind: MaintenanceActionKind,
    rego_source: &str,
) -> String {
    let created = fixture
        .app
        .create_maintenance_rule_set(
            &fixture.user,
            MaintenanceRuleDraft {
                subject_kind,
                name: "Completion acceptance".into(),
                description: String::new(),
                rego_source: rego_source.to_string(),
                action_definition: crate::maintenance_rules::MaintenanceActionDefinition::Legacy(
                    MaintenanceActionSpec::new(action_kind),
                ),
                grace_days: 0,
                storage_root_id: None,
                library_ids: vec![],
                evaluation_mode: None,
            },
        )
        .await
        .expect("create maintenance rule");
    fixture
        .app
        .set_maintenance_rule_evaluation_mode(
            &fixture.user,
            &created.rule_set.id,
            MaintenanceEvaluationMode::Observe,
        )
        .await
        .expect("enable observation");
    fixture
        .app
        .set_maintenance_instance_gates(
            &fixture.user,
            MaintenanceGatesUpdate {
                evaluation_enabled: Some(true),
                result_display_enabled: Some(true),
                destructive_effects_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("open maintenance gates");
    fixture
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("evaluate rule");
    let candidates = fixture
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .filter(|candidate| {
            candidate.rule_set_id == created.rule_set.id && !candidate.state.is_terminal()
        })
        .count();
    fixture
        .app
        .set_maintenance_rule_arming(
            &fixture.user,
            &created.rule_set.id,
            MaintenanceEffectArming::Destructive,
            Some(candidates as i64),
        )
        .await
        .expect("arm destructive rule");
    created.rule_set.id
}

#[tokio::test]
async fn title_file_only_deletion_retains_show_records_and_linked_or_sibling_files() {
    let fixture = title_fixture("Emberfall", MediaFacet::Series).await;
    let (season, aired) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let (_, future) = season_and_episode(&fixture, 2, 1, "2099-01-01").await;
    let removed_path = fixture.root.join("Emberfall/Season 01/episode.mkv");
    add_episode_file(&fixture, &aired, "Emberfall/Season 01/episode.mkv").await;

    let link = fixture
        .execution
        .app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &fixture.title.id,
            "Emberfall: Side Story",
            Some(2024),
            None,
            None,
        ))
        .await
        .expect("create linked movie");
    let linked_path = fixture.root.join("Emberfall/Specials/side-story.mkv");
    write_file(&linked_path);
    let linked_file = fixture
        .execution
        .app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: fixture.title.id.clone(),
            file_path: linked_path.to_string_lossy().to_string(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert linked movie file");
    fixture
        .execution
        .app
        .services
        .library
        .media_files
        .link_file_to_series_movie(&linked_file, &link.id)
        .await
        .expect("associate linked movie file");

    let sibling = fixture
        .execution
        .app
        .add_title(
            &fixture.execution.user,
            NewTitle {
                name: "Untouched Sibling".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                ..Default::default()
            },
        )
        .await
        .expect("create sibling title");
    let sibling_path = fixture.root.join("Sibling/sibling.mkv");
    write_file(&sibling_path);
    fixture
        .execution
        .app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: sibling.id.clone(),
            file_path: sibling_path.to_string_lossy().to_string(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert sibling file");

    let title_matcher = format!(
        "match if {{ input.subject.title_id == {:?}; input.facts.monitored }}",
        fixture.title.id
    );
    let rule = create_destructive_rule(
        &fixture.execution,
        MaintenanceRuleSubjectKind::Title,
        MaintenanceActionKind::UnmonitorTitleDeleteAllFiles,
        &title_matcher,
    )
    .await;
    let preview = fixture
        .execution
        .app
        .preview_maintenance_rule(
            &fixture.execution.user,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Stored {
                    rule_set_id: rule.clone(),
                },
                selection: MaintenancePreviewSelection::Titles(vec![fixture.title.id.clone()]),
            },
        )
        .await
        .expect("preview title deletion");
    assert_eq!(preview.titles.len(), 1);
    assert_eq!(
        preview.titles[0].file_count, 2,
        "ordinary subject facts still include the linked movie file"
    );
    assert_eq!(preview.titles[0].outcome, Some(MaintenanceOutcome::Match));

    let report = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("execute deletion");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(!removed_path.exists(), "owned episode file is deleted");
    assert!(linked_path.exists(), "linked movie file stays intact");
    assert!(sibling_path.exists(), "sibling title file stays intact");

    let retained = fixture
        .execution
        .app
        .services
        .catalog
        .titles
        .get_by_id(&fixture.title.id)
        .await
        .expect("read series")
        .expect("series retained");
    assert!(!retained.monitored);
    assert!(
        fixture
            .execution
            .app
            .services
            .catalog
            .shows
            .get_collection_by_id(&season.id)
            .await
            .expect("read season")
            .is_some_and(|collection| !collection.monitored)
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .catalog
            .shows
            .get_episode_by_id(&aired.id)
            .await
            .expect("read aired episode")
            .is_some_and(|episode| !episode.monitored)
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .catalog
            .shows
            .get_episode_by_id(&future.id)
            .await
            .expect("read future episode")
            .is_some_and(|episode| !episode.monitored),
        "future discovery inherits the title's unmonitored state"
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .catalog
            .titles
            .get_by_id(&sibling.id)
            .await
            .expect("read sibling")
            .is_some_and(|title| title.monitored)
    );
    let action_runs = fixture.execution.evaluation.all_action_runs().await;
    assert_eq!(action_runs.len(), 1);
    assert_eq!(action_runs[0].status, LifecycleActionRunStatus::Succeeded);
    let detail: serde_json::Value =
        serde_json::from_str(&action_runs[0].detail).expect("parse deletion detail");
    assert_eq!(detail["files"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn movie_title_file_only_deletion_retains_title_and_metadata_records() {
    let fixture = title_fixture("Last Picture", MediaFacet::Movie).await;
    let movie_path = fixture.root.join("Last Picture/last-picture.mkv");
    write_file(&movie_path);
    let movie_collection = fixture
        .execution
        .app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: fixture.title.id.clone(),
            collection_type: CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("Last Picture".to_string()),
            ordered_path: Some(movie_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("seed movie collection");
    let movie_episode = fixture
        .execution
        .app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: fixture.title.id.clone(),
            collection_id: Some(movie_collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("Last Picture".to_string()),
            title: Some("Last Picture metadata".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: Some("must survive file-only maintenance".to_string()),
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("seed movie metadata episode");
    fixture
        .execution
        .app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: fixture.title.id.clone(),
            file_path: movie_path.to_string_lossy().to_string(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert movie file");

    let movie_matcher = format!(
        "match if {{ input.subject.title_id == {:?}; input.facts.monitored }}",
        fixture.title.id
    );
    create_destructive_rule(
        &fixture.execution,
        MaintenanceRuleSubjectKind::Title,
        MaintenanceActionKind::UnmonitorTitleDeleteAllFiles,
        &movie_matcher,
    )
    .await;
    let report = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("execute movie deletion");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(!movie_path.exists());
    assert!(
        fixture
            .execution
            .app
            .services
            .catalog
            .titles
            .get_by_id(&fixture.title.id)
            .await
            .expect("read movie")
            .is_some_and(|movie| !movie.monitored),
        "file-only deletion retains and unmonitors the movie record"
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .list_media_files_for_title(&fixture.title.id)
            .await
            .expect("list movie files")
            .is_empty(),
        "the catalog no longer claims the deleted media file"
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .catalog
            .shows
            .get_collection_by_id(&movie_collection.id)
            .await
            .expect("read movie collection")
            .is_some(),
        "file-only deletion preserves the ordered-path movie collection"
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .catalog
            .shows
            .get_episode_by_id(&movie_episode.id)
            .await
            .expect("read movie metadata episode")
            .is_some(),
        "file-only deletion preserves movie metadata episodes"
    );
    let action_runs = fixture.execution.evaluation.all_action_runs().await;
    assert_eq!(action_runs.len(), 1);
    assert_eq!(action_runs[0].status, LifecycleActionRunStatus::Succeeded);
}

#[tokio::test]
async fn episode_retention_preview_grace_and_execution_keep_only_the_newest_downloaded_episode() {
    let fixture = title_fixture("Retention Clock", MediaFacet::Series).await;
    let (_, oldest) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let (_, middle) = season_and_episode(&fixture, 1, 2, "2020-01-02").await;
    let (_, newest) = season_and_episode(&fixture, 1, 3, "2020-01-03").await;
    let oldest_path = fixture.root.join("Retention/Season 01/episode-1.mkv");
    let middle_path = fixture.root.join("Retention/Season 01/episode-2.mkv");
    let newest_path = fixture.root.join("Retention/Season 01/episode-3.mkv");
    add_episode_file(&fixture, &oldest, "Retention/Season 01/episode-1.mkv").await;
    add_episode_file(&fixture, &middle, "Retention/Season 01/episode-2.mkv").await;
    add_episode_file(&fixture, &newest, "Retention/Season 01/episode-3.mkv").await;

    let preview = fixture
        .execution
        .app
        .preview_maintenance_rule(
            &fixture.execution.user,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Inline {
                    library_ids: vec![],
                    subject_kind: MaintenanceRuleSubjectKind::Episode,
                    rego_source: RETAIN_NEWEST_EPISODE.to_string(),
                    action_definition:
                        crate::maintenance_rules::MaintenanceActionDefinition::Legacy(
                            MaintenanceActionSpec::new(
                                MaintenanceActionKind::UnmonitorScopeDeleteFiles,
                            ),
                        ),
                    grace_days: 0,
                    storage_root_id: None,
                },
                selection: MaintenancePreviewSelection::Titles(vec![fixture.title.id.clone()]),
            },
        )
        .await
        .expect("preview retention rule");
    let matched = preview
        .titles
        .iter()
        .filter(|row| row.outcome == Some(MaintenanceOutcome::Match))
        .map(|row| row.subject_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(matched.len(), 2);
    assert!(matched.contains(&oldest.id.as_str()));
    assert!(matched.contains(&middle.id.as_str()));
    assert!(!matched.contains(&newest.id.as_str()));

    let rule = create_destructive_rule(
        &fixture.execution,
        MaintenanceRuleSubjectKind::Episode,
        MaintenanceActionKind::UnmonitorScopeDeleteFiles,
        RETAIN_NEWEST_EPISODE,
    )
    .await;
    let candidates = fixture.execution.evaluation.all_candidates().await;
    let active = candidates
        .iter()
        .filter(|candidate| candidate.rule_set_id == rule && !candidate.state.is_terminal())
        .collect::<Vec<_>>();
    assert_eq!(active.len(), 2);
    assert!(
        active.iter().all(|candidate| {
            candidate.state == MaintenanceCandidateState::Observing
                && candidate.due_at == candidate.first_matched_at
        }),
        "{active:?}"
    );

    let report = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("execute retention");
    assert_eq!(report.executed, 2, "{report:?}");
    assert!(!oldest_path.exists());
    assert!(!middle_path.exists());
    assert!(
        newest_path.exists(),
        "rank one remains after sibling execution"
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .catalog
            .shows
            .get_episode_by_id(&newest.id)
            .await
            .expect("read newest episode")
            .is_some_and(|episode| episode.monitored)
    );
    assert_eq!(
        fixture.execution.evaluation.all_action_runs().await.len(),
        2
    );
}

fn linked_account(user_id: &str, external_user_id: &str) -> UserExternalAccount {
    UserExternalAccount {
        id: format!("link-{user_id}"),
        user_id: user_id.to_string(),
        provider: ExternalAccountProvider::Jellyfin,
        connection_id: CONNECTION_ID.to_string(),
        external_user_id: Some(external_user_id.to_string()),
        username: external_user_id.to_string(),
        display_name: None,
        avatar_url: None,
        status: ExternalAccountStatus::Active,
        verified_at: Some(Utc::now()),
        last_login_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

async fn record_complete_sweep(signals: &InMemorySignalRepo) {
    signals
        .upsert_signal_sync_state(&MediaServerSignalSyncState {
            connection_id: CONNECTION_ID.to_string(),
            provider: MediaServerProvider::Jellyfin,
            enabled: true,
            last_started_at: Some(Utc::now()),
            last_success_at: Some(Utc::now()),
            last_error: None,
            participant_count: 2,
            signal_count: 2,
            updated_at: Utc::now(),
        })
        .await
        .expect("record completed sweep");
}

async fn record_episode_plays(
    signals: &InMemorySignalRepo,
    external_user_id: &str,
    scryer_user_id: &str,
    title_id: &str,
    episode_ids: &[String],
) {
    let plays = episode_ids
        .iter()
        .map(|episode_id| NewUserMediaSignal {
            provider: MediaServerProvider::Jellyfin,
            scryer_user_id: Some(scryer_user_id.to_string()),
            provider_item_id: format!("jf-{episode_id}"),
            kind: MediaServerSignalKind::Episode,
            scryer_title_id: Some(title_id.to_string()),
            scryer_episode_id: Some(episode_id.clone()),
            played: true,
            play_count: 1,
            last_played_at: Some(Utc::now()),
            observed_at: Utc::now(),
        })
        .collect::<Vec<_>>();
    signals
        .replace_participant_signals(CONNECTION_ID, external_user_id, &plays)
        .await
        .expect("record episode plays");
}

async fn preview_whole_show_completion(
    app: &AppUseCase,
    user: &User,
    title_id: &str,
) -> crate::maintenance_rules::MaintenancePreviewResult {
    app.preview_maintenance_rule(
        user,
        MaintenancePreviewRequest {
            matcher: MaintenancePreviewMatcher::Inline {
                library_ids: vec![],
                subject_kind: MaintenanceRuleSubjectKind::Title,
                rego_source: COMPLETE_SHOW_WATCH.to_string(),
                action_definition: crate::maintenance_rules::MaintenanceActionDefinition::Legacy(
                    MaintenanceActionSpec::new(MaintenanceActionKind::UnmonitorScopeKeepFiles),
                ),
                grace_days: 0,
                storage_root_id: None,
            },
            selection: MaintenancePreviewSelection::Titles(vec![title_id.to_string()]),
        },
    )
    .await
    .expect("preview whole-show completion")
}

#[tokio::test]
async fn whole_show_watch_preview_requires_one_verified_person_for_all_aired_episodes() {
    let execution = execution_app(None);
    let user = execution.user;
    let signals = Arc::new(InMemorySignalRepo::default());
    let app = execution.app.with_test_overrides(|services| {
        services
            .with_media_server_connection_store(Arc::new(StubSignalConnections {
                connections: vec![jellyfin_connection(true)],
                fail_list: false,
            }))
            .with_external_account_store(Arc::new(StubExternalAccounts {
                accounts: vec![
                    linked_account("viewer-a", "jf-a"),
                    linked_account("viewer-b", "jf-b"),
                ],
            }))
            .with_media_server_signal_store(signals.clone())
    });
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Shared Viewing".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                ..Default::default()
            },
        )
        .await
        .expect("create show");
    let season = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create season");
    let aired = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(season.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            None,
            Some("Aired".into()),
            Some("2020-01-01".into()),
            None,
            false,
            false,
        )
        .await
        .expect("create aired episode");
    let special = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(season.id.clone()),
            "special".into(),
            Some("2".into()),
            Some("0".into()),
            None,
            Some("Aired special".into()),
            Some("2020-01-02".into()),
            None,
            false,
            false,
        )
        .await
        .expect("create aired special");
    let future = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(season.id.clone()),
            "standard".into(),
            Some("3".into()),
            Some("1".into()),
            None,
            Some("Future".into()),
            Some("2099-01-01".into()),
            None,
            false,
            false,
        )
        .await
        .expect("create future episode");
    record_complete_sweep(&signals).await;
    record_episode_plays(
        &signals,
        "jf-a",
        "viewer-a",
        &title.id,
        std::slice::from_ref(&aired.id),
    )
    .await;
    record_episode_plays(
        &signals,
        "jf-b",
        "viewer-b",
        &title.id,
        std::slice::from_ref(&special.id),
    )
    .await;

    let split_viewing = preview_whole_show_completion(&app, &user, &title.id).await;
    assert_eq!(split_viewing.titles.len(), 1);
    assert_eq!(
        split_viewing.titles[0].outcome,
        Some(MaintenanceOutcome::NoMatch)
    );

    record_episode_plays(
        &signals,
        "jf-a",
        "viewer-a",
        &title.id,
        &[aired.id.clone(), special.id.clone()],
    )
    .await;
    record_episode_plays(&signals, "jf-b", "viewer-b", &title.id, &[]).await;
    let complete_viewing = preview_whole_show_completion(&app, &user, &title.id).await;
    assert_eq!(
        complete_viewing.titles[0].outcome,
        Some(MaintenanceOutcome::Match)
    );
    assert!(
        complete_viewing.titles[0]
            .reason_codes
            .iter()
            .all(|reason| !reason.contains(&future.id)),
        "unplayed future episode is excluded from the completion inventory"
    );
}
