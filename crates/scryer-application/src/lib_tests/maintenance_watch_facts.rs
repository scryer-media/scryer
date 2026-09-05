//! Watch-signal maintenance facts (RFC 137 §7.3, WP-N).
//!
//! Two things are specified here, and they are deliberately separate concerns:
//!
//! * **The freshness gate.** Whether Scryer will report watch facts at all is
//!   an instance-wide, unanimous judgement resolved once per run. Every test
//!   below asserts what a rule *decides*, not what a helper returns, because
//!   the property that matters is that an incomplete watch picture holds a
//!   subject rather than deleting it.
//! * **The authoring bar on person-targeted facts.** Writing — or previewing —
//!   a rule that reads who added, requested, or watched something needs the
//!   instance's permission-management authority; running an already-stored one
//!   does not, because the executor and the scheduler have no actor to ask.

use super::*;

use crate::lib_tests::maintenance_evaluation::InMemoryMaintenanceEvaluationRepo;
use crate::lib_tests::maintenance_rules::InMemoryMaintenanceRuleRepo;
use crate::lib_tests::media_server_signals::{
    CONNECTION_ID, InMemorySignalRepo, StubExternalAccounts, StubSignalConnections,
    jellyfin_connection, named_jellyfin_connection,
};
use crate::maintenance_rules::{
    MaintenanceActionKind, MaintenanceActionSpec, MaintenanceGatesUpdate, MaintenanceMatcherDraft,
    MaintenancePreviewMatcher, MaintenancePreviewRequest, MaintenancePreviewResult,
    MaintenancePreviewSelection, MaintenanceRuleDraft,
};
use crate::ports::MediaServerSignalRepository;
use scryer_domain::{
    AppPermissionMask, ExternalAccountProvider, ExternalAccountStatus, MaintenanceEvaluationMode,
    MediaRequest, MediaServerConnection, MediaServerProvider, MediaServerSignalKind,
    MediaServerSignalSyncState, NewUserMediaSignal, UserExternalAccount,
};
use scryer_rules::maintenance::MaintenanceOutcome;

/// Matches a movie every one of its requesters has played — the shipped
/// "Remove watched, requested movies" template, in matcher form.
const WATCHED_BY_ALL_REQUESTERS_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.facts.requested\n\
     \tinput.facts.watched_by_all_requesters\n\
     }\n";

/// Matches a movie nobody has played. Reads only the watcher list, so it is the
/// cleanest probe of the gate: with the gate open and no rows, the list is a
/// known-empty one and this decides.
const NOBODY_WATCHED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tcount(input.facts.watched_by_user_ids) == 0\n\
     }\n";

/// Matches a movie with no recorded play at all, through the anonymous
/// timestamp rather than the watcher list.
const NEVER_WATCHED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tnot input.facts.last_watched_at\n\
     }\n";

/// The other requester rollup, so the gate can be probed through all four watch
/// facts rather than just the two the shipped template reads.
const WATCHED_BY_ANY_REQUESTER_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.facts.watched_by_any_requester\n\
     }\n";

/// Reads no person-targeted fact at all, so the authoring bar must not apply.
const MONITORED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.facts.monitored\n\
     }\n";

const VIEWER_EXTERNAL_ID: &str = "jf-viewer-two";
const OTHER_VIEWER_EXTERNAL_ID: &str = "jf-viewer-three";

// ── Fixture ─────────────────────────────────────────────────────────────────

struct WatchFixture {
    app: AppUseCase,
    user: User,
    signals: Arc<InMemorySignalRepo>,
    media_requests: Arc<MockMediaRequestRepo>,
    evaluation: Arc<InMemoryMaintenanceEvaluationRepo>,
}

fn watch_app(
    connections: Vec<MediaServerConnection>,
    accounts: Vec<UserExternalAccount>,
) -> WatchFixture {
    let (app, user) = bootstrap();
    let signals = Arc::new(InMemorySignalRepo::default());
    let media_requests = Arc::new(MockMediaRequestRepo::default());
    let evaluation = Arc::new(InMemoryMaintenanceEvaluationRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_maintenance_rule_set_store(Arc::new(InMemoryMaintenanceRuleRepo::default()))
            .with_maintenance_evaluation_store(evaluation.clone())
            .with_media_files(Arc::new(MockMediaFileRepo::default()))
            .with_media_requests(media_requests.clone())
            .with_media_server_connection_store(Arc::new(StubSignalConnections {
                connections,
                fail_list: false,
            }))
            .with_external_account_store(Arc::new(StubExternalAccounts { accounts }))
            .with_media_server_signal_store(signals.clone())
    });
    WatchFixture {
        app,
        user,
        signals,
        media_requests,
        evaluation,
    }
}

fn link(user_id: &str, connection_id: &str, external_user_id: &str) -> UserExternalAccount {
    UserExternalAccount {
        id: format!("link-{user_id}"),
        user_id: user_id.to_string(),
        provider: ExternalAccountProvider::Jellyfin,
        connection_id: connection_id.to_string(),
        external_user_id: Some(external_user_id.to_string()),
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

/// Record one connection's sync health. `last_success_at` of `None` is a
/// connection that has never completed a clean sweep.
async fn record_sweep(
    signals: &InMemorySignalRepo,
    connection_id: &str,
    last_success_at: Option<DateTime<Utc>>,
) {
    signals
        .upsert_signal_sync_state(&MediaServerSignalSyncState {
            connection_id: connection_id.to_string(),
            provider: MediaServerProvider::Jellyfin,
            enabled: true,
            last_started_at: Some(Utc::now()),
            last_success_at,
            last_error: None,
            participant_count: 1,
            signal_count: 1,
            updated_at: Utc::now(),
        })
        .await
        .expect("record sync state");
}

async fn record_play(
    signals: &InMemorySignalRepo,
    title_id: &str,
    scryer_user_id: &str,
    external_user_id: &str,
) {
    signals
        .replace_participant_signals(
            CONNECTION_ID,
            external_user_id,
            &[NewUserMediaSignal {
                provider: MediaServerProvider::Jellyfin,
                scryer_user_id: Some(scryer_user_id.to_string()),
                provider_item_id: format!("jf-{title_id}"),
                kind: MediaServerSignalKind::Movie,
                scryer_title_id: Some(title_id.to_string()),
                scryer_episode_id: None,
                played: true,
                play_count: 1,
                last_played_at: Some(Utc::now()),
                observed_at: Utc::now(),
            }],
        )
        .await
        .expect("record play");
}

fn draft(rego_source: &str) -> MaintenanceRuleDraft {
    MaintenanceRuleDraft {
        name: "Watched movies".to_string(),
        description: String::new(),
        rego_source: rego_source.to_string(),
        action_spec: MaintenanceActionSpec::new(MaintenanceActionKind::UnmonitorScopeKeepFiles),
        grace_days: 0,
        library_ids: Vec::new(),
        evaluation_mode: None,
    }
}

async fn seed_title(app: &AppUseCase, user: &User, name: &str) -> Title {
    app.add_title(
        user,
        NewTitle {
            name: name.to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            ..Default::default()
        },
    )
    .await
    .expect("create title")
}

/// A request that created `title_id`, asked for by exactly `requester`.
fn request_for(title_id: &str, requester: &str) -> MediaRequest {
    let now = Utc::now();
    MediaRequest {
        background_url: None,
        requested_monitor_selection: None,
        requested_lease_days: None,
        approved_lease_days: None,
        decision_id: None,
        decided_by_rule_set_ids: Vec::new(),
        policy_tags: Vec::new(),
        metadata_snapshot_json: "{}".to_string(),
        rating_summary: scryer_domain::TitleRatingSummary::default(),
        id: Id::new().0,
        library_id: "library-1".to_string(),
        facet: MediaFacet::Movie,
        status: MediaRequestStatus::Approved,
        identity_fingerprint: format!("fingerprint-{title_id}"),
        title: "Requested Movie".to_string(),
        sort_title: None,
        slug: None,
        poster_url: None,
        year: None,
        overview: None,
        runtime_minutes: None,
        language: None,
        content_status: None,
        requested_quality_profile_id: None,
        requested_quality_profile_name: None,
        requested_monitor_type: None,
        resolved_by_user_id: None,
        resolved_at: Some(now),
        created_title_id: Some(title_id.to_string()),
        approved_quality_profile_id: None,
        approved_quality_profile_name: None,
        external_ids: Vec::new(),
        requesters: vec![MediaRequestRequester {
            user_id: requester.to_string(),
            username: "viewer-two".to_string(),
            avatar_url: None,
            requested_at: now,
        }],
        created_by_user_id: requester.to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn inline_matcher(rego_source: &str) -> MaintenancePreviewMatcher {
    MaintenancePreviewMatcher::Inline {
        rego_source: rego_source.to_string(),
        action_spec: MaintenanceActionSpec::new(MaintenanceActionKind::UnmonitorScopeKeepFiles),
        grace_days: 0,
    }
}

impl WatchFixture {
    /// Run one preview as a named actor, without asserting it succeeded.
    async fn preview_as(
        &self,
        actor: &User,
        matcher: MaintenancePreviewMatcher,
        title_id: &str,
    ) -> AppResult<MaintenancePreviewResult> {
        self.app
            .preview_maintenance_rule(
                actor,
                MaintenancePreviewRequest {
                    matcher,
                    selection: MaintenancePreviewSelection::Titles(vec![title_id.to_string()]),
                },
            )
            .await
    }

    /// Preview one matcher over one title and return what it decided.
    async fn decide(
        &self,
        rego_source: &str,
        title_id: &str,
    ) -> (Option<MaintenanceOutcome>, Vec<String>) {
        let preview = self
            .preview_as(&self.user, inline_matcher(rego_source), title_id)
            .await
            .expect("preview");
        assert_eq!(preview.titles.len(), 1);
        assert!(preview.titles[0].error.is_none(), "{:?}", preview.titles[0]);
        (
            preview.titles[0].outcome,
            preview.titles[0].reason_codes.clone(),
        )
    }
}

// ── The freshness gate ──────────────────────────────────────────────────────

#[tokio::test]
async fn watch_facts_hold_without_a_connected_media_server() {
    let fixture = watch_app(Vec::new(), Vec::new());
    let title = seed_title(&fixture.app, &fixture.user, "Unwatched Movie").await;

    let (outcome, reasons) = fixture.decide(NOBODY_WATCHED_MATCHER, &title.id).await;

    // Nothing could have reported a play, so "nobody watched it" is not
    // something Scryer knows — it is something it never asked.
    assert_eq!(outcome, Some(MaintenanceOutcome::Unknown));
    assert_eq!(reasons, vec!["no_media_server_connection".to_string()]);
}

#[tokio::test]
async fn watch_facts_hold_until_a_connection_has_swept_cleanly() {
    let fixture = watch_app(vec![jellyfin_connection(true)], Vec::new());
    let title = seed_title(&fixture.app, &fixture.user, "Unwatched Movie").await;
    // A state row exists, but no sweep ever finished cleanly.
    record_sweep(&fixture.signals, CONNECTION_ID, None).await;

    let (outcome, reasons) = fixture.decide(NOBODY_WATCHED_MATCHER, &title.id).await;

    assert_eq!(outcome, Some(MaintenanceOutcome::Unknown));
    assert_eq!(reasons, vec!["signal_sync_never_succeeded".to_string()]);
}

#[tokio::test]
async fn a_connection_with_no_state_row_at_all_reads_as_never_swept() {
    let fixture = watch_app(vec![jellyfin_connection(true)], Vec::new());
    let title = seed_title(&fixture.app, &fixture.user, "Unwatched Movie").await;

    let (outcome, reasons) = fixture.decide(NOBODY_WATCHED_MATCHER, &title.id).await;

    assert_eq!(outcome, Some(MaintenanceOutcome::Unknown));
    assert_eq!(reasons, vec!["signal_sync_never_succeeded".to_string()]);
}

#[tokio::test]
async fn watch_facts_hold_when_the_newest_clean_sweep_is_stale() {
    let fixture = watch_app(vec![jellyfin_connection(true)], Vec::new());
    let title = seed_title(&fixture.app, &fixture.user, "Unwatched Movie").await;
    record_sweep(
        &fixture.signals,
        CONNECTION_ID,
        Some(Utc::now() - chrono::Duration::hours(49)),
    )
    .await;

    let (outcome, reasons) = fixture.decide(NOBODY_WATCHED_MATCHER, &title.id).await;

    assert_eq!(outcome, Some(MaintenanceOutcome::Unknown));
    assert_eq!(reasons, vec!["signals_stale".to_string()]);
}

#[tokio::test]
async fn a_fresh_sweep_with_no_plays_is_a_decisive_nobody_watched_it() {
    // Somebody is linked, so Scryer is observing an audience and can say
    // truthfully that none of it played the movie.
    let fixture = watch_app(
        vec![jellyfin_connection(true)],
        vec![link("user-viewer", CONNECTION_ID, VIEWER_EXTERNAL_ID)],
    );
    let title = seed_title(&fixture.app, &fixture.user, "Unwatched Movie").await;
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;

    let (outcome, reasons) = fixture.decide(NOBODY_WATCHED_MATCHER, &title.id).await;

    assert_eq!(
        outcome,
        Some(MaintenanceOutcome::Match),
        "an empty signal set behind a fresh sweep is an answer, not a gap"
    );
    assert!(reasons.is_empty(), "{reasons:?}");
}

#[tokio::test]
async fn a_recorded_play_stops_the_nobody_watched_rule_matching() {
    let fixture = watch_app(
        vec![jellyfin_connection(true)],
        vec![link("user-viewer", CONNECTION_ID, VIEWER_EXTERNAL_ID)],
    );
    let title = seed_title(&fixture.app, &fixture.user, "Watched Movie").await;
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;
    record_play(
        &fixture.signals,
        &title.id,
        "user-viewer",
        VIEWER_EXTERNAL_ID,
    )
    .await;

    let (outcome, _) = fixture.decide(NOBODY_WATCHED_MATCHER, &title.id).await;

    assert_eq!(outcome, Some(MaintenanceOutcome::NoMatch));
}

/// A server the operator turned off is a decision, not a gap: it must not
/// silently hold every watch rule on the instance forever.
#[tokio::test]
async fn a_disabled_connection_never_poisons_the_gate() {
    let fixture = watch_app(
        vec![
            jellyfin_connection(true),
            named_jellyfin_connection("conn-retired", false),
        ],
        vec![link("user-viewer", CONNECTION_ID, VIEWER_EXTERNAL_ID)],
    );
    let title = seed_title(&fixture.app, &fixture.user, "Unwatched Movie").await;
    // Only the enabled connection has ever swept.
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;

    let (outcome, _) = fixture.decide(NOBODY_WATCHED_MATCHER, &title.id).await;

    assert_eq!(outcome, Some(MaintenanceOutcome::Match));
}

/// The gate is unanimous on purpose: signals from the fresh server are a
/// partial watch picture while the other one is stale, and a partial picture
/// reported as a complete one is what deletes something somebody watched.
#[tokio::test]
async fn one_stale_connection_holds_every_subject() {
    let fixture = watch_app(
        vec![
            jellyfin_connection(true),
            named_jellyfin_connection("conn-second", true),
        ],
        Vec::new(),
    );
    let title = seed_title(&fixture.app, &fixture.user, "Unwatched Movie").await;
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;
    record_sweep(
        &fixture.signals,
        "conn-second",
        Some(Utc::now() - chrono::Duration::hours(72)),
    )
    .await;

    let (outcome, reasons) = fixture.decide(NOBODY_WATCHED_MATCHER, &title.id).await;

    assert_eq!(outcome, Some(MaintenanceOutcome::Unknown));
    assert_eq!(reasons, vec!["signals_stale".to_string()]);
}

/// A clean sweep over nobody is not a watch picture: with no verified link on
/// any enabled connection, Scryer observes no audience at all, so an empty set
/// of plays says nothing about whether anyone watched.
#[tokio::test]
async fn a_fresh_sweep_with_nobody_linked_holds_every_watch_fact() {
    let fixture = watch_app(vec![jellyfin_connection(true)], Vec::new());
    let title = seed_title(&fixture.app, &fixture.user, "Unwatched Movie").await;
    fixture
        .media_requests
        .requests
        .lock()
        .await
        .push(request_for(&title.id, "user-viewer"));
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;

    for (fact, matcher) in [
        ("watched_by_user_ids", NOBODY_WATCHED_MATCHER),
        ("last_watched_at", NEVER_WATCHED_MATCHER),
        ("watched_by_any_requester", WATCHED_BY_ANY_REQUESTER_MATCHER),
        (
            "watched_by_all_requesters",
            WATCHED_BY_ALL_REQUESTERS_MATCHER,
        ),
    ] {
        let (outcome, reasons) = fixture.decide(matcher, &title.id).await;

        assert_eq!(outcome, Some(MaintenanceOutcome::Unknown), "{fact}");
        assert_eq!(
            reasons,
            vec!["no_linked_participants".to_string()],
            "{fact}"
        );
    }
}

/// One verified link is the whole difference: there is now an audience to have
/// watched or not watched the subject, so the same facts are answers again.
#[tokio::test]
async fn one_linked_participant_opens_the_gate() {
    let fixture = watch_app(
        vec![jellyfin_connection(true)],
        vec![link("user-viewer", CONNECTION_ID, VIEWER_EXTERNAL_ID)],
    );
    let title = seed_title(&fixture.app, &fixture.user, "Unwatched Movie").await;
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;

    for (fact, matcher) in [
        ("watched_by_user_ids", NOBODY_WATCHED_MATCHER),
        ("last_watched_at", NEVER_WATCHED_MATCHER),
    ] {
        let (outcome, reasons) = fixture.decide(matcher, &title.id).await;

        assert_eq!(outcome, Some(MaintenanceOutcome::Match), "{fact}");
        assert!(reasons.is_empty(), "{fact}: {reasons:?}");
    }
}

// ── Requester rollups end to end ────────────────────────────────────────────

#[tokio::test]
async fn a_requested_movie_its_requester_watched_matches_the_shipped_template() {
    let fixture = watch_app(
        vec![jellyfin_connection(true)],
        vec![link("user-viewer", CONNECTION_ID, VIEWER_EXTERNAL_ID)],
    );
    let title = seed_title(&fixture.app, &fixture.user, "Requested Movie").await;
    fixture
        .media_requests
        .requests
        .lock()
        .await
        .push(request_for(&title.id, "user-viewer"));
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;
    record_play(
        &fixture.signals,
        &title.id,
        "user-viewer",
        VIEWER_EXTERNAL_ID,
    )
    .await;

    let (outcome, _) = fixture
        .decide(WATCHED_BY_ALL_REQUESTERS_MATCHER, &title.id)
        .await;

    assert_eq!(outcome, Some(MaintenanceOutcome::Match));
}

#[tokio::test]
async fn an_unlinked_requester_holds_the_shipped_template() {
    // The connection is fresh, somebody else is linked so the instance-wide
    // gate is open, and the movie was played — but the requester never linked
    // an account, so Scryer cannot say whether *they* watched it.
    let fixture = watch_app(
        vec![jellyfin_connection(true)],
        vec![link("user-other", CONNECTION_ID, OTHER_VIEWER_EXTERNAL_ID)],
    );
    let title = seed_title(&fixture.app, &fixture.user, "Requested Movie").await;
    fixture
        .media_requests
        .requests
        .lock()
        .await
        .push(request_for(&title.id, "user-viewer"));
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;
    record_play(
        &fixture.signals,
        &title.id,
        "user-viewer",
        VIEWER_EXTERNAL_ID,
    )
    .await;

    let (outcome, reasons) = fixture
        .decide(WATCHED_BY_ALL_REQUESTERS_MATCHER, &title.id)
        .await;

    assert_eq!(outcome, Some(MaintenanceOutcome::Unknown));
    assert_eq!(reasons, vec!["requester_not_linked".to_string()]);
}

#[tokio::test]
async fn an_unrequested_movie_never_matches_the_shipped_template() {
    let fixture = watch_app(
        vec![jellyfin_connection(true)],
        vec![link("user-viewer", CONNECTION_ID, VIEWER_EXTERNAL_ID)],
    );
    let title = seed_title(&fixture.app, &fixture.user, "Scanned Movie").await;
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;
    record_play(
        &fixture.signals,
        &title.id,
        "user-viewer",
        VIEWER_EXTERNAL_ID,
    )
    .await;

    let (outcome, reasons) = fixture
        .decide(WATCHED_BY_ALL_REQUESTERS_MATCHER, &title.id)
        .await;

    // Absent, not unknown: the rollups simply do not apply, so the rule decides
    // "no" rather than being held.
    assert_eq!(outcome, Some(MaintenanceOutcome::NoMatch));
    assert!(reasons.is_empty(), "{reasons:?}");
}

// ── The person-fact authoring bar ───────────────────────────────────────────

fn catalog_only_author() -> User {
    test_user_with_app_permissions(
        "catalog-operator",
        AppPermissionMask::from_permissions([AppPermission::ManageCatalogSettings]),
    )
}

#[tokio::test]
async fn authoring_a_watch_rule_needs_permission_management() {
    let fixture = watch_app(Vec::new(), Vec::new());

    let error = fixture
        .app
        .create_maintenance_rule_set(
            &catalog_only_author(),
            draft(WATCHED_BY_ALL_REQUESTERS_MATCHER),
        )
        .await
        .expect_err("a rule about people needs permission-management authority");

    assert!(matches!(error, AppError::Unauthorized(_)), "{error:?}");
    let message = error.to_string();
    assert!(
        message.contains("watched_by_all_requesters"),
        "the error must name the facts that triggered it: {message}"
    );
    assert!(
        message.contains("manage permissions"),
        "the error must say what is missing in plain English: {message}"
    );
}

#[tokio::test]
async fn every_person_targeted_fact_trips_the_bar_and_is_named() {
    let fixture = watch_app(Vec::new(), Vec::new());

    for fact in scryer_rules::maintenance::PERSON_TARGETED_MAINTENANCE_FACTS {
        // Read the fact in a shape that is legal for its type: a bare truth
        // test works for booleans and for the string and list facts alike.
        let source =
            format!("package whatever\nimport rego.v1\n\nmatch if {{\n\tinput.facts.{fact}\n}}\n");
        let error = fixture
            .app
            .create_maintenance_rule_set(&catalog_only_author(), draft(&source))
            .await
            .err()
            .unwrap_or_else(|| panic!("{fact} must be gated"));

        assert!(
            matches!(error, AppError::Unauthorized(_)),
            "{fact}: {error:?}"
        );
        assert!(error.to_string().contains(fact), "{fact}: {error}");
    }
}

#[tokio::test]
async fn a_rule_about_media_rather_than_people_is_unaffected() {
    let fixture = watch_app(Vec::new(), Vec::new());

    fixture
        .app
        .create_maintenance_rule_set(&catalog_only_author(), draft(MONITORED_MATCHER))
        .await
        .expect("catalog-settings management is enough for a rule about media");
}

/// The anonymous aggregates are deliberately outside the bar: they say
/// something happened, never who did it.
#[tokio::test]
async fn the_anonymous_watch_aggregate_is_not_gated() {
    let fixture = watch_app(Vec::new(), Vec::new());
    const LAST_WATCHED_MATCHER: &str = "package whatever\n\
         import rego.v1\n\n\
         match if {\n\
         \tinput.facts.requested\n\
         \tnot input.facts.last_watched_at\n\
         }\n";

    fixture
        .app
        .create_maintenance_rule_set(&catalog_only_author(), draft(LAST_WATCHED_MATCHER))
        .await
        .expect("requested and last_watched_at name nobody");
}

#[tokio::test]
async fn a_privileged_author_may_save_a_person_fact_rule() {
    let fixture = watch_app(Vec::new(), Vec::new());

    fixture
        .app
        .create_maintenance_rule_set(&fixture.user, draft(WATCHED_BY_ALL_REQUESTERS_MATCHER))
        .await
        .expect("an administrator may author a rule about people");
}

#[tokio::test]
async fn the_bar_applies_to_replacing_a_matcher_too() {
    let fixture = watch_app(Vec::new(), Vec::new());
    let created = fixture
        .app
        .create_maintenance_rule_set(&fixture.user, draft(MONITORED_MATCHER))
        .await
        .expect("create a rule about media");

    let error = fixture
        .app
        .update_maintenance_rule_matcher(
            &catalog_only_author(),
            &created.rule_set.id,
            MaintenanceMatcherDraft {
                rego_source: WATCHED_BY_ALL_REQUESTERS_MATCHER.to_string(),
                action_spec: MaintenanceActionSpec::new(
                    MaintenanceActionKind::UnmonitorScopeKeepFiles,
                ),
                grace_days: 0,
            },
        )
        .await
        .expect_err("a revision is authoring, and the bar is on authoring");

    assert!(matches!(error, AppError::Unauthorized(_)), "{error:?}");
    assert!(
        error.to_string().contains("watched_by_all_requesters"),
        "{error}"
    );
}

/// Preview answers the same question authoring does, one real subject at a
/// time: an ungated preview is a person-fact oracle — ask "did this user watch
/// it" title by title and read the answers off the outcomes.
#[tokio::test]
async fn previewing_a_person_fact_draft_needs_permission_management() {
    let fixture = watch_app(
        vec![jellyfin_connection(true)],
        vec![link("user-viewer", CONNECTION_ID, VIEWER_EXTERNAL_ID)],
    );
    let title = seed_title(&fixture.app, &fixture.user, "Requested Movie").await;

    let error = fixture
        .preview_as(
            &catalog_only_author(),
            inline_matcher(WATCHED_BY_ALL_REQUESTERS_MATCHER),
            &title.id,
        )
        .await
        .expect_err("previewing a rule about people needs the same authority as saving one");

    assert!(matches!(error, AppError::Unauthorized(_)), "{error:?}");
    let message = error.to_string();
    assert!(
        message.contains("watched_by_all_requesters"),
        "the error must name the facts that triggered it: {message}"
    );
}

#[tokio::test]
async fn a_privileged_actor_may_preview_a_person_fact_rule() {
    let fixture = watch_app(
        vec![jellyfin_connection(true)],
        vec![link("user-viewer", CONNECTION_ID, VIEWER_EXTERNAL_ID)],
    );
    let title = seed_title(&fixture.app, &fixture.user, "Requested Movie").await;
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;

    let preview = fixture
        .preview_as(
            &fixture.user,
            inline_matcher(WATCHED_BY_ALL_REQUESTERS_MATCHER),
            &title.id,
        )
        .await
        .expect("an administrator may preview a rule about people");

    assert_eq!(preview.titles.len(), 1);
}

#[tokio::test]
async fn previewing_a_rule_about_media_is_unaffected() {
    let fixture = watch_app(Vec::new(), Vec::new());
    let title = seed_title(&fixture.app, &fixture.user, "Scanned Movie").await;

    let preview = fixture
        .preview_as(
            &catalog_only_author(),
            inline_matcher(MONITORED_MATCHER),
            &title.id,
        )
        .await
        .expect("catalog-settings management is enough to preview a rule about media");

    assert_eq!(preview.titles.len(), 1);
}

/// Storing the rule first changes nothing: the preview still reads the same
/// person facts about the same real subjects on the caller's behalf.
#[tokio::test]
async fn previewing_a_stored_person_fact_rule_is_gated_too() {
    let fixture = watch_app(Vec::new(), Vec::new());
    let title = seed_title(&fixture.app, &fixture.user, "Requested Movie").await;
    let created = fixture
        .app
        .create_maintenance_rule_set(&fixture.user, draft(WATCHED_BY_ALL_REQUESTERS_MATCHER))
        .await
        .expect("an administrator may author a rule about people");

    let error = fixture
        .preview_as(
            &catalog_only_author(),
            MaintenancePreviewMatcher::Stored {
                rule_set_id: created.rule_set.id.clone(),
            },
            &title.id,
        )
        .await
        .expect_err("a stored rule is no less of an oracle than a draft");

    assert!(matches!(error, AppError::Unauthorized(_)), "{error:?}");
    assert!(
        error.to_string().contains("watched_by_all_requesters"),
        "{error}"
    );
}

/// The bar is on authoring, never on running: revoking the author's permission
/// must not silently stop a rule an operator armed, and the scheduler has no
/// actor to ask in any case.
#[tokio::test]
async fn a_stored_person_fact_rule_still_evaluates_under_the_system_principal() {
    let fixture = watch_app(
        vec![jellyfin_connection(true)],
        vec![link("user-viewer", CONNECTION_ID, VIEWER_EXTERNAL_ID)],
    );
    let title = seed_title(&fixture.app, &fixture.user, "Requested Movie").await;
    fixture
        .media_requests
        .requests
        .lock()
        .await
        .push(request_for(&title.id, "user-viewer"));
    record_sweep(&fixture.signals, CONNECTION_ID, Some(Utc::now())).await;
    record_play(
        &fixture.signals,
        &title.id,
        "user-viewer",
        VIEWER_EXTERNAL_ID,
    )
    .await;

    let created = fixture
        .app
        .create_maintenance_rule_set(&fixture.user, draft(WATCHED_BY_ALL_REQUESTERS_MATCHER))
        .await
        .expect("create rule set");
    fixture
        .app
        .set_maintenance_rule_evaluation_mode(
            &fixture.user,
            &created.rule_set.id,
            MaintenanceEvaluationMode::Shadow,
        )
        .await
        .expect("arm rule");
    fixture
        .app
        .set_maintenance_instance_gates(
            &fixture.user,
            MaintenanceGatesUpdate {
                evaluation_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("arm the evaluation gate");

    // The job body takes no actor at all, which is the point.
    let report = fixture
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("evaluation pass");

    assert_eq!(report.rules_evaluated, 1);
    assert_eq!(report.candidates_created, 1);
    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    assert_eq!(candidates[0].title_id, title.id);
}
