//! The live watch re-check before an action that a watch fact allowed.
//!
//! Each test runs the real executor over a rule that matched on stored watch
//! facts, with a media server whose live answer the test chooses, and asserts
//! what happened to the title, the candidate, and the stored facts.

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::lib_tests::maintenance_evaluation::InMemoryMaintenanceEvaluationRepo;
use crate::lib_tests::maintenance_rules::InMemoryMaintenanceRuleRepo;
use crate::lib_tests::media_server_signals::{
    CONNECTION_ID, InMemorySignalRepo, StubExternalAccounts, StubSignalConnections,
    jellyfin_connection, named_jellyfin_connection,
};
use crate::maintenance_rules::{
    MaintenanceActionKind, MaintenanceActionSpec, MaintenanceRuleDraft, execution_reason,
};
use crate::media_server_signals::reasons;
use crate::ports::{
    ConnectionPlaybackActivity, MediaServerPlaybackProbe, MediaServerSignalReader,
    MediaServerSignalRepository, MediaServerSignalSource, PlaybackActivitySnapshot,
    PlaybackProbeStatus, ProviderItemPlay, ProviderItemRecheck, ProviderPlayedItem,
    SignalReadFailure,
};
use scryer_domain::{
    ExternalAccountProvider, ExternalAccountStatus, MaintenanceCandidateState,
    MaintenanceEffectArming, MaintenanceRuleState, MediaServerConnection,
    MediaServerPlaybackEntityKind, MediaServerPlaybackItem, MediaServerProvider,
    MediaServerSignalKind, MediaServerSignalSyncState, NewUserMediaSignal, UserExternalAccount,
};

/// Matches a movie someone has played: a rule that rests on a watch fact.
const SOMEONE_WATCHED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tcount(input.facts.watched_by_user_ids) > 0\n\
     }\n";

/// Matches a movie nobody has played: the rule a missed play would delete.
const NOBODY_WATCHED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tcount(input.facts.watched_by_user_ids) == 0\n\
     }\n";

/// Reads no watch fact, so no re-check is due.
const MONITORED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.facts.monitored\n\
     }\n";

const VIEWER: &str = "user-viewer";
const VIEWER_EXTERNAL_ID: &str = "jf-viewer-two";
const STRANGER_EXTERNAL_ID: &str = "jf-unlinked-account";

struct IdlePlayback;

#[async_trait]
impl MediaServerPlaybackProbe for IdlePlayback {
    async fn active_playback(&self) -> AppResult<PlaybackActivitySnapshot> {
        Ok(PlaybackActivitySnapshot {
            connections: vec![ConnectionPlaybackActivity {
                connection_id: CONNECTION_ID.to_string(),
                provider: MediaServerProvider::Jellyfin,
                status: PlaybackProbeStatus::Idle,
            }],
            observed_at: Utc::now(),
        })
    }
}

/// A server whose live re-check answer the test sets for every item asked
/// about, counting the reads and recording the items.
struct LiveServer {
    answer: Result<ProviderItemRecheck, SignalReadFailure>,
    rechecks: Arc<AtomicUsize>,
    asked: Arc<std::sync::Mutex<Vec<String>>>,
}

struct LiveReader {
    answer: Result<ProviderItemRecheck, SignalReadFailure>,
    rechecks: Arc<AtomicUsize>,
    asked: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait]
impl MediaServerSignalSource for LiveServer {
    async fn open(
        &self,
        _: &MediaServerConnection,
    ) -> Result<Box<dyn MediaServerSignalReader>, SignalReadFailure> {
        Ok(Box::new(LiveReader {
            answer: self.answer.clone(),
            rechecks: self.rechecks.clone(),
            asked: self.asked.clone(),
        }))
    }
}

#[async_trait]
impl MediaServerSignalReader for LiveReader {
    async fn fetch_played_items(
        &self,
        _: &str,
    ) -> Result<Vec<ProviderPlayedItem>, SignalReadFailure> {
        Ok(Vec::new())
    }

    async fn recheck_items(
        &self,
        provider_item_ids: &[String],
        _: &[String],
    ) -> Result<std::collections::BTreeMap<String, ProviderItemRecheck>, SignalReadFailure> {
        self.rechecks.fetch_add(1, Ordering::SeqCst);
        self.asked
            .lock()
            .expect("asked")
            .extend(provider_item_ids.iter().cloned());
        let answer = self.answer.clone()?;
        Ok(provider_item_ids
            .iter()
            .map(|id| (id.clone(), answer.clone()))
            .collect())
    }
}

struct RecheckFixture {
    app: AppUseCase,
    user: User,
    signals: Arc<InMemorySignalRepo>,
    evaluation: Arc<InMemoryMaintenanceEvaluationRepo>,
    rechecks: Arc<AtomicUsize>,
    asked: Arc<std::sync::Mutex<Vec<String>>>,
    connections: Arc<StubSignalConnections>,
    media_files: Arc<MockMediaFileRepo>,
    /// When the viewer's stored play happened.
    stored_at: DateTime<Utc>,
}

/// What the title looks like when the rule is evaluated.
#[derive(Clone, Copy)]
struct Seed {
    /// The sweep stored a play of it by the viewer.
    stored_play: bool,
    /// It has a file on disk.
    file: bool,
    /// The library mapping knows its item on the connection.
    mapped: bool,
}

impl Seed {
    const STORED_PLAY: Self = Self {
        stored_play: true,
        file: false,
        mapped: false,
    };
}

fn viewer_link() -> UserExternalAccount {
    UserExternalAccount {
        id: format!("link-{VIEWER}"),
        user_id: VIEWER.to_string(),
        provider: ExternalAccountProvider::Jellyfin,
        connection_id: CONNECTION_ID.to_string(),
        external_user_id: Some(VIEWER_EXTERNAL_ID.to_string()),
        username: "viewer-two".to_string(),
        display_name: None,
        avatar_url: None,
        status: ExternalAccountStatus::Active,
        verified_at: Some(Utc::now()),
        last_login_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn recheck_app(answer: Result<ProviderItemRecheck, SignalReadFailure>) -> RecheckFixture {
    recheck_app_with(
        answer,
        vec![jellyfin_connection(true)],
        vec![viewer_link()],
        &[viewer_link()],
    )
}

/// As [`recheck_app`], over `connections`, with `accounts` linked and only
/// `swept` among them read by a sweep.
fn recheck_app_with(
    answer: Result<ProviderItemRecheck, SignalReadFailure>,
    connections: Vec<MediaServerConnection>,
    accounts: Vec<UserExternalAccount>,
    swept: &[UserExternalAccount],
) -> RecheckFixture {
    let (app, user) = bootstrap();
    let signals = Arc::new(InMemorySignalRepo::swept(swept));
    let evaluation = Arc::new(InMemoryMaintenanceEvaluationRepo::default());
    let rechecks = Arc::new(AtomicUsize::new(0));
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let connections = Arc::new(StubSignalConnections {
        connections,
        fail_list: false,
        ..Default::default()
    });
    let media_files = Arc::new(MockMediaFileRepo {
        scoped_rows_without_episode_id: true,
        ..Default::default()
    });
    let job_runs = Arc::new(RecordingJobRunRepo::with_maintenance_action_job_receipts(
        evaluation.action_job_receipts.clone(),
    ));
    let app = app.with_test_overrides(|services| {
        services
            .with_maintenance_rule_set_store(Arc::new(InMemoryMaintenanceRuleRepo::default()))
            .with_maintenance_evaluation_store(evaluation.clone())
            .with_media_files(media_files.clone())
            .with_shows(Arc::new(MockShowRepo::default()))
            .with_job_runs(job_runs.clone())
            .with_media_server_connection_store(connections.clone())
            .with_external_account_store(Arc::new(StubExternalAccounts { accounts }))
            .with_media_server_signal_store(signals.clone())
            .with_media_server_signal_source(Arc::new(LiveServer {
                answer,
                rechecks: rechecks.clone(),
                asked: asked.clone(),
            }))
            .with_media_server_playback_probe(Arc::new(IdlePlayback))
    });
    RecheckFixture {
        app,
        user,
        signals,
        evaluation,
        rechecks,
        asked,
        connections,
        media_files,
        stored_at: hours_ago(48),
    }
}

fn hours_ago(hours: i64) -> DateTime<Utc> {
    Utc::now() - chrono::Duration::hours(hours)
}

fn item_id(title_id: &str) -> String {
    format!("jf-{title_id}")
}

fn live(account: &str, played: bool, last_played_at: Option<DateTime<Utc>>) -> ProviderItemPlay {
    ProviderItemPlay {
        external_user_id: account.to_string(),
        played,
        last_played_at,
    }
}

impl RecheckFixture {
    /// A clean, current sweep of every connection.
    async fn swept(&self, signal_count: i64) {
        for connection in &self.connections.connections {
            self.swept_connection(&connection.id, signal_count).await;
        }
    }

    async fn swept_connection(&self, connection_id: &str, signal_count: i64) {
        self.signals
            .upsert_signal_sync_state(&MediaServerSignalSyncState {
                connection_id: connection_id.to_string(),
                provider: MediaServerProvider::Jellyfin,
                enabled: true,
                last_started_at: Some(Utc::now()),
                last_success_at: Some(Utc::now()),
                last_error: None,
                last_error_code: None,
                participant_count: 1,
                signal_count,
                updated_at: Utc::now(),
            })
            .await
            .expect("record sync state");
    }

    /// A clean, current sweep that stored one play of `title_id` by the
    /// viewer, `played_at`.
    async fn stored_play(&self, title_id: &str, played_at: DateTime<Utc>) {
        self.swept(1).await;
        self.signals
            .replace_participant_signals(
                CONNECTION_ID,
                VIEWER_EXTERNAL_ID,
                &[NewUserMediaSignal {
                    provider: MediaServerProvider::Jellyfin,
                    scryer_user_id: Some(VIEWER.to_string()),
                    provider_item_id: item_id(title_id),
                    kind: MediaServerSignalKind::Movie,
                    scryer_title_id: Some(title_id.to_string()),
                    scryer_episode_id: None,
                    played: true,
                    play_count: 1,
                    last_played_at: Some(played_at),
                    is_favorite: false,
                    observed_at: Utc::now(),
                }],
                None,
            )
            .await
            .expect("record play");
    }

    async fn stored_facts(&self, title_id: &str) -> Vec<scryer_domain::UserMediaSignal> {
        self.signals
            .movie_signals_for_titles(&[title_id.to_string()])
            .await
            .expect("read signals")
            .remove(title_id)
            .unwrap_or_default()
    }

    /// Arm an unmonitor rule with `matcher`, evaluate, and run one handler
    /// pass over a seeded, monitored movie. Returns the title id and its
    /// stored watch facts as they were just before the handler ran.
    async fn run(&self, matcher: &str) -> (String, Vec<scryer_domain::UserMediaSignal>) {
        self.run_with(matcher, Seed::STORED_PLAY).await
    }

    /// As [`Self::run`], over a title seeded as `seed` describes.
    async fn run_with(
        &self,
        matcher: &str,
        seed: Seed,
    ) -> (String, Vec<scryer_domain::UserMediaSignal>) {
        let title = self
            .app
            .add_title(
                &self.user,
                NewTitle {
                    name: "Synthetic Feature".to_string(),
                    facet: MediaFacet::Movie,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    ..Default::default()
                },
            )
            .await
            .expect("create title");
        if seed.stored_play {
            self.stored_play(&title.id, self.stored_at).await;
        } else {
            self.swept(0).await;
        }
        if seed.file {
            self.media_files
                .insert_media_file(&InsertMediaFileInput {
                    title_id: title.id.clone(),
                    file_path: format!("/media/{}.mkv", title.id),
                    size_bytes: 1_000,
                    ..Default::default()
                })
                .await
                .expect("insert media file");
        }
        if seed.mapped {
            self.connections
                .playback_items
                .lock()
                .expect("playback items")
                .push(MediaServerPlaybackItem {
                    connection_id: CONNECTION_ID.to_string(),
                    entity_kind: MediaServerPlaybackEntityKind::Title,
                    entity_id: title.id.clone(),
                    provider_item_id: item_id(&title.id),
                    last_seen_at: Utc::now(),
                });
        }
        self.arm_and_evaluate(scryer_domain::MaintenanceRuleSubjectKind::Title, matcher)
            .await;
        let before = self.stored_facts(&title.id).await;
        self.app
            .run_lifecycle_action_handling_job()
            .await
            .expect("handler pass");
        (title.id, before)
    }

    /// Map `entity` to `provider_item_id` on `connection_id`.
    fn map_item(
        &self,
        connection_id: &str,
        entity_kind: MediaServerPlaybackEntityKind,
        entity_id: &str,
        provider_item_id: String,
    ) {
        self.connections
            .playback_items
            .lock()
            .expect("playback items")
            .push(MediaServerPlaybackItem {
                connection_id: connection_id.to_string(),
                entity_kind,
                entity_id: entity_id.to_string(),
                provider_item_id,
                last_seen_at: Utc::now(),
            });
    }

    /// A monitored series with one season of two episodes, each with its own
    /// file and a swept, unplayed watch row, the connection's mapping knowing
    /// each episode's item. Arms an
    /// unmonitor rule with `matcher` over `scope`, evaluates, and runs one
    /// handler pass. Returns the episodes.
    async fn run_scoped(
        &self,
        scope: scryer_domain::MaintenanceRuleSubjectKind,
        matcher: &str,
    ) -> Vec<scryer_domain::Episode> {
        let title = self
            .app
            .add_title(
                &self.user,
                NewTitle {
                    name: "Synthetic Series".to_string(),
                    facet: MediaFacet::Series,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    ..Default::default()
                },
            )
            .await
            .expect("create title");
        self.swept(0).await;
        let season = self
            .app
            .create_collection(
                &self.user,
                title.id.clone(),
                "season".into(),
                "1".into(),
                None,
                None,
                None,
                None,
            )
            .await
            .expect("season");
        let mut episodes = Vec::new();
        for number in 1..=2 {
            let episode = self
                .app
                .create_episode(
                    &self.user,
                    title.id.clone(),
                    Some(season.id.clone()),
                    "standard".into(),
                    Some(number.to_string()),
                    Some("1".into()),
                    None,
                    Some(format!("Episode {number}")),
                    Some("2020-01-01".into()),
                    None,
                    false,
                    false,
                )
                .await
                .expect("episode");
            let file_id = self
                .media_files
                .insert_media_file(&InsertMediaFileInput {
                    title_id: title.id.clone(),
                    file_path: format!("/media/{}/s01e{number:02}.mkv", title.id),
                    size_bytes: 1_000,
                    ..Default::default()
                })
                .await
                .expect("insert media file");
            self.media_files
                .store
                .lock()
                .await
                .iter_mut()
                .find(|row| row.id == file_id)
                .expect("seeded file")
                .episode_id = Some(episode.id.clone());
            self.map_item(
                CONNECTION_ID,
                MediaServerPlaybackEntityKind::Episode,
                &episode.id,
                item_id(&episode.id),
            );
            episodes.push(episode);
        }
        // The sweep read every episode and found it unplayed.
        let unplayed: Vec<NewUserMediaSignal> = episodes
            .iter()
            .map(|episode| NewUserMediaSignal {
                provider: MediaServerProvider::Jellyfin,
                scryer_user_id: Some(VIEWER.to_string()),
                provider_item_id: item_id(&episode.id),
                kind: MediaServerSignalKind::Episode,
                scryer_title_id: Some(title.id.clone()),
                scryer_episode_id: Some(episode.id.clone()),
                played: false,
                play_count: 0,
                last_played_at: None,
                is_favorite: false,
                observed_at: Utc::now(),
            })
            .collect();
        self.signals
            .replace_participant_signals(CONNECTION_ID, VIEWER_EXTERNAL_ID, &unplayed, None)
            .await
            .expect("record unplayed episodes");
        self.arm_and_evaluate(scope, matcher).await;
        self.app
            .run_lifecycle_action_handling_job()
            .await
            .expect("handler pass");
        episodes
    }

    /// Arm an unmonitor rule with `matcher` over `scope` and evaluate.
    async fn arm_and_evaluate(
        &self,
        scope: scryer_domain::MaintenanceRuleSubjectKind,
        matcher: &str,
    ) {
        let created = self
            .app
            .create_maintenance_rule_set(
                &self.user,
                MaintenanceRuleDraft {
                    subject_kind: scope,
                    name: "Watched subjects".to_string(),
                    description: String::new(),
                    rego_source: matcher.to_string(),
                    action_definition:
                        crate::maintenance_rules::MaintenanceActionDefinition::Legacy(
                            MaintenanceActionSpec::new(
                                MaintenanceActionKind::UnmonitorScopeKeepFiles,
                            ),
                        ),
                    grace_days: 0,
                    storage_root_id: None,
                    library_ids: Vec::new(),
                },
            )
            .await
            .expect("create rule set");
        let rule_id = created.rule_set.id;
        super::maintenance_rules::set_rule_state(
            &self.app,
            &self.user,
            &rule_id,
            MaintenanceRuleState::DryRun,
            None,
        )
        .await;
        super::maintenance_rules::open_maintenance_gates(&self.app, true, false).await;
        super::maintenance_rules::arm_rule(
            &self.app,
            &self.user,
            &rule_id,
            MaintenanceEffectArming::Reversible,
            None,
        )
        .await;
        self.app
            .run_maintenance_rule_evaluation_job()
            .await
            .expect("evaluation pass");
    }

    /// Arm a title rule with `matcher`, evaluate, and run one handler pass
    /// over a title the test has already seeded.
    async fn arm_and_run(&self, _title_id: &str, matcher: &str) {
        self.arm_and_evaluate(scryer_domain::MaintenanceRuleSubjectKind::Title, matcher)
            .await;
        self.app
            .run_lifecycle_action_handling_job()
            .await
            .expect("handler pass");
    }

    async fn monitored(&self, title_id: &str) -> bool {
        self.app
            .get_title(&self.user, title_id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    }

    async fn assert_held(&self, title_id: &str, reason: &str) {
        assert!(self.monitored(title_id).await, "a held action must not act");
        let candidates = self.evaluation.all_candidates().await;
        assert_eq!(candidates.len(), 1, "{candidates:?}");
        assert_eq!(candidates[0].state, MaintenanceCandidateState::Blocked);
        assert_eq!(candidates[0].state_reason, reason);
        let runs = self.evaluation.all_action_runs().await;
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].hold_reason.as_deref(), Some(reason));
    }
}

#[tokio::test]
async fn a_play_newer_than_the_stored_facts_holds_the_action() {
    let fixture = recheck_app(Ok(ProviderItemRecheck {
        participants: vec![live(VIEWER_EXTERNAL_ID, true, Some(hours_ago(1)))],
        history: Vec::new(),
    }));
    let (title_id, _) = fixture.run(SOMEONE_WATCHED_MATCHER).await;

    assert_eq!(fixture.rechecks.load(Ordering::SeqCst), 1);
    fixture
        .assert_held(&title_id, execution_reason::WATCH_RECHECK_NEWER_PLAY)
        .await;
}

#[tokio::test]
async fn a_play_no_stored_fact_credits_holds_the_action() {
    let fixture = recheck_app(Ok(ProviderItemRecheck {
        participants: vec![live(VIEWER_EXTERNAL_ID, true, Some(hours_ago(48)))],
        history: vec![live(STRANGER_EXTERNAL_ID, true, Some(hours_ago(72)))],
    }));
    let (title_id, _) = fixture.run(SOMEONE_WATCHED_MATCHER).await;

    fixture
        .assert_held(&title_id, execution_reason::WATCH_RECHECK_UNCREDITED_PLAY)
        .await;
}

#[tokio::test]
async fn a_recheck_that_cannot_be_made_holds_the_action() {
    let fixture = recheck_app(Err(SignalReadFailure::new(
        reasons::REQUEST_FAILED,
        "request timed out",
    )));
    let (title_id, _) = fixture.run(SOMEONE_WATCHED_MATCHER).await;

    fixture
        .assert_held(&title_id, execution_reason::WATCH_RECHECK_UNAVAILABLE)
        .await;
}

#[tokio::test]
async fn a_clean_recheck_lets_the_action_run_and_leaves_the_stored_facts_alone() {
    let fixture = recheck_app(Ok(ProviderItemRecheck {
        participants: vec![live(VIEWER_EXTERNAL_ID, true, Some(hours_ago(48)))],
        history: Vec::new(),
    }));
    let (title_id, before) = fixture.run(SOMEONE_WATCHED_MATCHER).await;

    assert_eq!(fixture.rechecks.load(Ordering::SeqCst), 1);
    assert!(!fixture.monitored(&title_id).await, "the action ran");
    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates[0].state, MaintenanceCandidateState::Succeeded);
    let after = fixture.stored_facts(&title_id).await;
    assert_eq!(before, after, "a re-check never writes watch facts");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].sync_generation, before[0].sync_generation);
}

#[tokio::test]
async fn a_rule_that_reads_no_watch_fact_is_not_rechecked() {
    // The live server would fail every re-check; a rule that never read a
    // watch fact must not be held by it.
    let fixture = recheck_app(Err(SignalReadFailure::new(
        reasons::REQUEST_FAILED,
        "request timed out",
    )));
    let (title_id, _) = fixture.run(MONITORED_MATCHER).await;

    assert_eq!(fixture.rechecks.load(Ordering::SeqCst), 0);
    assert!(!fixture.monitored(&title_id).await, "the action ran");
}

// ── A subject nobody had played at the sweep ────────────────────────────────

/// No stored row names the movie's item, so the re-check finds it through the
/// library mapping and reads it: a play since the sweep holds the action.
#[tokio::test]
async fn a_title_with_no_stored_plays_is_rechecked_through_the_library_mapping() {
    let fixture = recheck_app(Ok(ProviderItemRecheck {
        participants: vec![live(VIEWER_EXTERNAL_ID, true, Some(hours_ago(1)))],
        history: Vec::new(),
    }));
    let (title_id, before) = fixture
        .run_with(
            NOBODY_WATCHED_MATCHER,
            Seed {
                stored_play: false,
                file: true,
                mapped: true,
            },
        )
        .await;

    assert!(before.is_empty(), "the sweep stored no play of it");
    assert_eq!(fixture.rechecks.load(Ordering::SeqCst), 1);
    assert_eq!(*fixture.asked.lock().unwrap(), vec![item_id(&title_id)]);
    fixture
        .assert_held(&title_id, execution_reason::WATCH_RECHECK_NEWER_PLAY)
        .await;
}

/// Someone no Scryer user is linked to played it since the sweep: the
/// server's own record of that play holds the action too.
#[tokio::test]
async fn an_unlinked_accounts_play_of_a_title_with_no_stored_plays_holds_the_action() {
    let fixture = recheck_app(Ok(ProviderItemRecheck {
        participants: vec![live(VIEWER_EXTERNAL_ID, false, None)],
        history: vec![live(STRANGER_EXTERNAL_ID, true, Some(hours_ago(1)))],
    }));
    let (title_id, _) = fixture
        .run_with(
            NOBODY_WATCHED_MATCHER,
            Seed {
                stored_play: false,
                file: true,
                mapped: true,
            },
        )
        .await;

    fixture
        .assert_held(&title_id, execution_reason::WATCH_RECHECK_UNCREDITED_PLAY)
        .await;
}

/// A title with a file but no item the mapping can find on the connection
/// cannot be shown unplayed there, so it is held without a read.
#[tokio::test]
async fn a_title_with_no_mapped_item_is_held() {
    let fixture = recheck_app(Ok(ProviderItemRecheck {
        participants: vec![live(VIEWER_EXTERNAL_ID, false, None)],
        history: Vec::new(),
    }));
    let (title_id, _) = fixture
        .run_with(
            NOBODY_WATCHED_MATCHER,
            Seed {
                stored_play: false,
                file: true,
                mapped: false,
            },
        )
        .await;

    assert_eq!(fixture.rechecks.load(Ordering::SeqCst), 0);
    fixture
        .assert_held(&title_id, execution_reason::WATCH_RECHECK_UNMAPPED)
        .await;
}

#[tokio::test]
async fn a_mapped_title_still_unplayed_lets_the_action_run() {
    let fixture = recheck_app(Ok(ProviderItemRecheck {
        participants: vec![live(VIEWER_EXTERNAL_ID, false, None)],
        history: Vec::new(),
    }));
    let (title_id, _) = fixture
        .run_with(
            NOBODY_WATCHED_MATCHER,
            Seed {
                stored_play: false,
                file: true,
                mapped: true,
            },
        )
        .await;

    assert_eq!(fixture.rechecks.load(Ordering::SeqCst), 1);
    assert!(!fixture.monitored(&title_id).await, "the action ran");
}

#[tokio::test]
async fn a_failed_read_of_a_title_with_no_stored_plays_holds_the_action() {
    let fixture = recheck_app(Err(SignalReadFailure::new(
        reasons::REQUEST_FAILED,
        "one user's read timed out",
    )));
    let (title_id, _) = fixture
        .run_with(
            NOBODY_WATCHED_MATCHER,
            Seed {
                stored_play: false,
                file: true,
                mapped: true,
            },
        )
        .await;

    fixture
        .assert_held(&title_id, execution_reason::WATCH_RECHECK_UNAVAILABLE)
        .await;
}

/// A second server whose library mapping has no item for the title: the
/// title cannot be shown unplayed there, so it holds, under its own reason
/// rather than as a failed read.
#[tokio::test]
async fn a_second_server_without_the_title_holds_it_as_unmapped() {
    const OTHER_CONNECTION_ID: &str = "conn-jellyfin-second";
    let fixture = recheck_app_with(
        Ok(ProviderItemRecheck {
            participants: vec![live(VIEWER_EXTERNAL_ID, false, None)],
            history: Vec::new(),
        }),
        vec![
            jellyfin_connection(true),
            named_jellyfin_connection(OTHER_CONNECTION_ID, true),
        ],
        vec![viewer_link()],
        &[viewer_link()],
    );
    let (title_id, _) = fixture
        .run_with(
            NOBODY_WATCHED_MATCHER,
            Seed {
                stored_play: false,
                file: true,
                mapped: true,
            },
        )
        .await;

    assert_eq!(fixture.rechecks.load(Ordering::SeqCst), 0);
    fixture
        .assert_held(&title_id, execution_reason::WATCH_RECHECK_UNMAPPED)
        .await;
}

/// The same two servers, each with a mapped item for the title: nothing is
/// unmapped, both are read, and an unplayed answer lets the action run.
#[tokio::test]
async fn a_title_mapped_on_every_server_is_read_on_each_and_runs() {
    const OTHER_CONNECTION_ID: &str = "conn-jellyfin-second";
    let fixture = recheck_app_with(
        Ok(ProviderItemRecheck {
            participants: vec![live(VIEWER_EXTERNAL_ID, false, None)],
            history: Vec::new(),
        }),
        vec![
            jellyfin_connection(true),
            named_jellyfin_connection(OTHER_CONNECTION_ID, true),
        ],
        vec![viewer_link()],
        &[viewer_link()],
    );
    let title = fixture
        .app
        .add_title(
            &fixture.user,
            NewTitle {
                name: "Synthetic Feature".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    fixture.swept(0).await;
    fixture
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: format!("/media/{}.mkv", title.id),
            size_bytes: 1_000,
            ..Default::default()
        })
        .await
        .expect("insert media file");
    for connection_id in [CONNECTION_ID, OTHER_CONNECTION_ID] {
        fixture.map_item(
            connection_id,
            MediaServerPlaybackEntityKind::Title,
            &title.id,
            format!("{connection_id}-{}", title.id),
        );
    }
    fixture
        .arm_and_evaluate(
            scryer_domain::MaintenanceRuleSubjectKind::Title,
            NOBODY_WATCHED_MATCHER,
        )
        .await;
    fixture
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("handler pass");

    assert_eq!(fixture.rechecks.load(Ordering::SeqCst), 2);
    assert!(!fixture.monitored(&title.id).await, "the action ran");
}

// ── Season and episode subjects ─────────────────────────────────────────────

/// A season's file rows carry no episode of their own; the episodes come
/// from the inventory's links, so the season's mapped items are read and an
/// unplayed answer lets the action run.
#[tokio::test]
async fn a_season_subject_is_rechecked_through_its_episodes_items() {
    let fixture = recheck_app(Ok(ProviderItemRecheck {
        participants: vec![live(VIEWER_EXTERNAL_ID, false, None)],
        history: Vec::new(),
    }));
    let episodes = fixture
        .run_scoped(
            scryer_domain::MaintenanceRuleSubjectKind::Season,
            NOBODY_WATCHED_MATCHER,
        )
        .await;

    let mut asked = fixture.asked.lock().unwrap().clone();
    asked.sort();
    let mut expected: Vec<String> = episodes
        .iter()
        .map(|episode| item_id(&episode.id))
        .collect();
    expected.sort();
    assert_eq!(asked, expected);
    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    assert_eq!(
        candidates[0].state,
        MaintenanceCandidateState::Succeeded,
        "{candidates:?}"
    );
}

#[tokio::test]
async fn an_episode_subject_is_rechecked_through_its_own_item() {
    let fixture = recheck_app(Ok(ProviderItemRecheck {
        participants: vec![live(VIEWER_EXTERNAL_ID, false, None)],
        history: Vec::new(),
    }));
    let episodes = fixture
        .run_scoped(
            scryer_domain::MaintenanceRuleSubjectKind::Episode,
            NOBODY_WATCHED_MATCHER,
        )
        .await;

    // One candidate per episode, each reading only its own item.
    let mut asked = fixture.asked.lock().unwrap().clone();
    asked.sort();
    let mut expected: Vec<String> = episodes
        .iter()
        .map(|episode| item_id(&episode.id))
        .collect();
    expected.sort();
    assert_eq!(asked, expected);
    assert_eq!(fixture.rechecks.load(Ordering::SeqCst), episodes.len());
    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates.len(), episodes.len(), "{candidates:?}");
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.state == MaintenanceCandidateState::Succeeded),
        "{candidates:?}"
    );
}

// ── The per-user gate closed, the audience open ─────────────────────────────

/// Matches a title someone, anyone, has played: an anonymous fact only.
const ANYONE_WATCHED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.facts.watched_by_anyone\n\
     }\n";

/// A link no sweep has read closes the per-user facts, but a rule on the
/// anonymous facts still runs. The re-check compares against what the sweep
/// stored for the accounts it did read: an old play is not a new one.
#[tokio::test]
async fn an_old_stored_play_is_not_a_newer_play_while_a_link_awaits_its_sweep() {
    let played_at = Utc::now() - chrono::Duration::days(200);
    let late = UserExternalAccount {
        id: "link-user-late".to_string(),
        user_id: "user-late".to_string(),
        external_user_id: Some("jf-viewer-late".to_string()),
        username: "viewer-late".to_string(),
        ..viewer_link()
    };
    let mut fixture = recheck_app_with(
        Ok(ProviderItemRecheck {
            participants: vec![live(VIEWER_EXTERNAL_ID, true, Some(played_at))],
            history: vec![live(VIEWER_EXTERNAL_ID, true, Some(played_at))],
        }),
        vec![jellyfin_connection(true)],
        vec![viewer_link(), late],
        &[viewer_link()],
    );
    fixture.stored_at = played_at;
    // The audience read, every account's plays, is current.
    let title = fixture
        .app
        .add_title(
            &fixture.user,
            NewTitle {
                name: "Synthetic Feature".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    fixture
        .signals
        .replace_connection_audience(
            CONNECTION_ID,
            &[crate::ports::AudiencePlay {
                connection_id: CONNECTION_ID.to_string(),
                external_account_id: VIEWER_EXTERNAL_ID.to_string(),
                provider_item_id: item_id(&title.id),
                kind: MediaServerSignalKind::Movie,
                scryer_title_id: Some(title.id.clone()),
                scryer_episode_id: None,
                play_count: 1,
                last_played_at: Some(played_at),
            }],
            Utc::now(),
        )
        .await
        .expect("record audience");
    fixture.stored_play(&title.id, played_at).await;
    let context = fixture.app.maintenance_watch_context().await.unwrap();
    assert_eq!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Unavailable(
            crate::maintenance_rules::facts::unknown_reason::PARTICIPANTS_NOT_YET_SWEPT
        )
    );
    assert_eq!(
        context.audience,
        crate::maintenance_rules::facts::WatchSignalFreshness::Fresh
    );

    fixture.arm_and_run(&title.id, ANYONE_WATCHED_MATCHER).await;

    assert_eq!(fixture.rechecks.load(Ordering::SeqCst), 1);
    let runs = fixture.evaluation.all_action_runs().await;
    assert!(
        runs.iter()
            .all(|run| run.hold_reason.as_deref()
                != Some(execution_reason::WATCH_RECHECK_NEWER_PLAY)),
        "{runs:?}"
    );
    assert!(!fixture.monitored(&title.id).await, "the action ran");
}
