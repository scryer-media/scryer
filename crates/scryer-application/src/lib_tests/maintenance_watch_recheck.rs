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
    jellyfin_connection,
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
    MaintenanceEffectArming, MaintenanceRuleState, MediaServerConnection, MediaServerProvider,
    MediaServerSignalKind, MediaServerSignalSyncState, NewUserMediaSignal, UserExternalAccount,
};

/// Matches a movie someone has played: a rule that rests on a watch fact.
const SOMEONE_WATCHED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tcount(input.facts.watched_by_user_ids) > 0\n\
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

/// A server whose live re-check answer the test sets, counting the reads.
struct LiveServer {
    answer: Result<ProviderItemRecheck, SignalReadFailure>,
    rechecks: Arc<AtomicUsize>,
}

struct LiveReader {
    answer: Result<ProviderItemRecheck, SignalReadFailure>,
    rechecks: Arc<AtomicUsize>,
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

    async fn recheck_item(
        &self,
        _: &str,
        _: &[String],
    ) -> Result<ProviderItemRecheck, SignalReadFailure> {
        self.rechecks.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }
}

struct RecheckFixture {
    app: AppUseCase,
    user: User,
    signals: Arc<InMemorySignalRepo>,
    evaluation: Arc<InMemoryMaintenanceEvaluationRepo>,
    rechecks: Arc<AtomicUsize>,
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
    let (app, user) = bootstrap();
    let accounts = vec![viewer_link()];
    let signals = Arc::new(InMemorySignalRepo::swept(&accounts));
    let evaluation = Arc::new(InMemoryMaintenanceEvaluationRepo::default());
    let rechecks = Arc::new(AtomicUsize::new(0));
    let job_runs = Arc::new(RecordingJobRunRepo::with_maintenance_action_job_receipts(
        evaluation.action_job_receipts.clone(),
    ));
    let app = app.with_test_overrides(|services| {
        services
            .with_maintenance_rule_set_store(Arc::new(InMemoryMaintenanceRuleRepo::default()))
            .with_maintenance_evaluation_store(evaluation.clone())
            .with_media_files(Arc::new(MockMediaFileRepo::default()))
            .with_job_runs(job_runs.clone())
            .with_media_server_connection_store(Arc::new(StubSignalConnections {
                connections: vec![jellyfin_connection(true)],
                fail_list: false,
            }))
            .with_external_account_store(Arc::new(StubExternalAccounts { accounts }))
            .with_media_server_signal_store(signals.clone())
            .with_media_server_signal_source(Arc::new(LiveServer {
                answer,
                rechecks: rechecks.clone(),
            }))
            .with_media_server_playback_probe(Arc::new(IdlePlayback))
    });
    RecheckFixture {
        app,
        user,
        signals,
        evaluation,
        rechecks,
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
    /// A clean, current sweep that stored one play of `title_id` by the
    /// viewer, `played_at`.
    async fn stored_play(&self, title_id: &str, played_at: DateTime<Utc>) {
        self.signals
            .upsert_signal_sync_state(&MediaServerSignalSyncState {
                connection_id: CONNECTION_ID.to_string(),
                provider: MediaServerProvider::Jellyfin,
                enabled: true,
                last_started_at: Some(Utc::now()),
                last_success_at: Some(Utc::now()),
                last_error: None,
                last_error_code: None,
                participant_count: 1,
                signal_count: 1,
                updated_at: Utc::now(),
            })
            .await
            .expect("record sync state");
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
        self.stored_play(&title.id, hours_ago(48)).await;
        let created = self
            .app
            .create_maintenance_rule_set(
                &self.user,
                MaintenanceRuleDraft {
                    subject_kind: scryer_domain::MaintenanceRuleSubjectKind::Title,
                    name: "Watched movies".to_string(),
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
        let before = self.stored_facts(&title.id).await;
        self.app
            .run_lifecycle_action_handling_job()
            .await
            .expect("handler pass");
        (title.id, before)
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
