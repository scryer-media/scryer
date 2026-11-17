//! Lifecycle claims as lifecycle sees them (spec 0003 FR-041..FR-044).
//!
//! A lease is only a time to delete: it reaches lifecycle through the facts
//! a rule reads, and preview and the scheduled pass have to read the same
//! ones. It never holds an action by itself.

use super::*;

use crate::lib_tests::lifecycle_evaluation::InMemoryLifecycleEvaluationRepo;
use crate::lib_tests::lifecycle_rules::InMemoryLifecycleRuleRepo;
use crate::lib_tests::request_rules_support::InMemoryLifecycleClaimRepo;
use crate::lifecycle_rules::{
    LifecyclePreviewRequest, LifecyclePreviewSelection, LifecyclePreviewSource, LifecycleRuleDraft,
    execution_reason,
};
use scryer_domain::{
    LifecycleCandidateState, LifecycleClaim, LifecycleClaimKind, LifecycleClaimProducer,
    LifecycleClaimState, LifecycleRuleState,
};
use scryer_rules::lifecycle::LifecycleOutcome;

/// The shipped `expired-request-leases` template, byte-for-byte minus the
/// package line the authoring path rewrites.
const EXPIRED_LEASE_SOURCE: &str = "match if {\n\
     \tinput.facts.request_lease_state == \"expired\"\n\
     }\n";

/// Matches every title, so the executor's rechecks are what decide.
const ALWAYS_SOURCE: &str = "match := true\n";

struct ClaimFixture {
    app: AppUseCase,
    user: User,
    evaluation: Arc<InMemoryLifecycleEvaluationRepo>,
    claims: Arc<InMemoryLifecycleClaimRepo>,
    media_files: Arc<MockMediaFileRepo>,
}

fn claims_app() -> ClaimFixture {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryLifecycleRuleRepo::default());
    let evaluation = Arc::new(InMemoryLifecycleEvaluationRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let claims = Arc::new(InMemoryLifecycleClaimRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_lifecycle_rule_set_store(rules.clone())
            .with_lifecycle_evaluation_store(evaluation.clone())
            .with_media_files(media_files.clone())
            .with_lifecycle_claim_store(claims.clone())
    });
    ClaimFixture {
        app,
        user,
        evaluation,
        claims,
        media_files,
    }
}

impl ClaimFixture {
    async fn rule(&self, draft: LifecycleRuleDraft, state: LifecycleRuleState) -> String {
        let created = self
            .app
            .create_lifecycle_rule_set(&self.user, draft)
            .await
            .expect("create rule set");
        super::lifecycle_rules::set_rule_state(
            &self.app,
            &self.user,
            &created.rule_set.id,
            state,
            None,
        )
        .await;
        created.rule_set.id
    }

    async fn open_gates(&self, reversible: bool, destructive: bool) {
        super::lifecycle_rules::open_lifecycle_gates(&self.app, reversible, destructive).await;
    }

    async fn arm(&self, rule_set_id: &str, state: LifecycleRuleState, ack: Option<i64>) {
        super::lifecycle_rules::arm_rule(&self.app, &self.user, rule_set_id, state, ack).await;
    }

    async fn evaluate(&self) -> crate::lifecycle_rules::LifecycleEvaluationReport {
        self.app
            .run_lifecycle_rule_evaluation_job()
            .await
            .expect("evaluation pass")
    }

    async fn handle(&self) -> crate::lifecycle_rules::LifecycleActionHandlingReport {
        self.app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("handler pass")
    }

    async fn only_candidate(&self) -> scryer_domain::LifecycleCandidate {
        let candidates = self.evaluation.all_candidates().await;
        assert_eq!(candidates.len(), 1, "{candidates:?}");
        candidates[0].clone()
    }

    async fn only_claim(&self) -> LifecycleClaim {
        let claims = self.claims.all().await;
        assert_eq!(claims.len(), 1, "{claims:?}");
        claims[0].clone()
    }
}

fn draft(
    rego_source: &str,
    heads: &[scryer_rules::lifecycle_actions::LifecycleActionHead],
) -> LifecycleRuleDraft {
    LifecycleRuleDraft {
        max_titles_per_run: Some(100),
        name: "Lease rule".to_string(),
        description: String::new(),
        rego_source: crate::lib_tests::heads_source(rego_source, heads),
        grace_days: 0,
        library_ids: Vec::new(),
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

async fn seed_media_file(media_files: &MockMediaFileRepo, title_id: &str) -> DateTime<Utc> {
    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.to_string(),
            file_path: format!("/media/{title_id}.mkv"),
            size_bytes: 1_000,
            quality_label: Some("1080P".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    let files = media_files.store.lock().await;
    let file = files
        .iter()
        .find(|file| file.title_id == title_id)
        .expect("the file was stored");
    DateTime::parse_from_rfc3339(&file.created_at)
        .expect("the mock stamps an RFC3339 timestamp")
        .with_timezone(&Utc)
}

fn lease(id: &str, title_id: &str, state: LifecycleClaimState) -> LifecycleClaim {
    let now = Utc::now();
    LifecycleClaim {
        id: id.to_string(),
        title_id: title_id.to_string(),
        library_id: "library-1".to_string(),
        producer: LifecycleClaimProducer::RequestLease,
        producer_ref: Some(format!("request-{id}")),
        kind: LifecycleClaimKind::RetainUntil,
        state,
        duration_days: Some(30),
        starts_at: None,
        expires_at: None,
        created_by: Some("user-1".to_string()),
        created_at: now,
        updated_at: now,
        released_reason: None,
    }
}

// ── Leases hold nothing ─────────────────────────────────────────────────────

/// An armed destructive rule acts on a leased title whatever the lease's
/// state: deciding that a lease protects a title is the matcher's job.
async fn assert_lease_does_not_hold(state: LifecycleClaimState) {
    let fixture = claims_app();
    let rule_id = fixture
        .rule(
            draft(
                ALWAYS_SOURCE,
                &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
            ),
            LifecycleRuleState::DryRun,
        )
        .await;
    fixture.open_gates(true, true).await;
    let root = tempfile::tempdir().expect("claim fixture root");
    update_default_library_roots(
        &fixture.app,
        &fixture.user,
        MediaFacet::Movie,
        vec![build_root_folder_entry(root.path(), true)],
    )
    .await
    .expect("configure claim fixture root");
    let title = seed_title(&fixture.app, &fixture.user, "Leased").await;
    let folder = root.path().join("Leased");
    std::fs::create_dir(&folder).expect("create claimed title folder");
    let media_path = folder.join("Leased.mkv");
    std::fs::write(&media_path, b"video").expect("write owned media");
    fixture
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: media_path.to_string_lossy().into_owned(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("record owned media");
    let unrelated = root.path().join("unrelated.txt");
    std::fs::write(&unrelated, b"preserved").expect("write unrelated file");
    super::lifecycle_title_completion::record_fixture_title_folder(
        &fixture.app,
        &title.id,
        &folder,
    )
    .await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, LifecycleRuleState::Enabled, Some(1))
        .await;
    fixture
        .claims
        .seed(lease("claim-1", &title.id, state))
        .await;

    let report = fixture.handle().await;
    assert_eq!(report.held, 0, "a lease holds nothing: {report:?}");
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .is_none(),
        "the armed destructive rule deletes the leased title: {report:?}; {:?}",
        fixture.evaluation.all_action_runs().await
    );
    assert!(!media_path.exists(), "the owned file goes with the title");
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"preserved");
}

#[tokio::test]
async fn destructive_execution_reads_claims_under_the_approval_guard() {
    let fixture = claims_app();
    let rule_id = fixture
        .rule(
            draft(
                ALWAYS_SOURCE,
                &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
            ),
            LifecycleRuleState::DryRun,
        )
        .await;
    fixture.open_gates(true, true).await;
    let title = seed_title(&fixture.app, &fixture.user, "Guarded lease").await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, LifecycleRuleState::Enabled, Some(1))
        .await;
    fixture
        .claims
        .seed(lease(
            "guarded-claim",
            &title.id,
            LifecycleClaimState::Active,
        ))
        .await;
    let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    *fixture.claims.read_guard_probe.lock().await = Some((
        fixture
            .app
            .runtime
            .jobs
            .interactive_operation_guards
            .clone(),
        title.id.clone(),
        observed.clone(),
    ));
    fixture.handle().await;
    assert!(observed.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn an_active_lease_does_not_hold_a_destructive_action() {
    assert_lease_does_not_hold(LifecycleClaimState::Active).await;
}

#[tokio::test]
async fn a_dormant_lease_does_not_hold_a_destructive_action() {
    assert_lease_does_not_hold(LifecycleClaimState::Dormant).await;
}

#[tokio::test]
async fn an_expired_lease_does_not_hold_a_destructive_action() {
    assert_lease_does_not_hold(LifecycleClaimState::Expired).await;
}

#[tokio::test]
async fn an_unreadable_claim_store_holds_a_destructive_action() {
    let fixture = claims_app();
    let rule_id = fixture
        .rule(
            draft(
                ALWAYS_SOURCE,
                &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
            ),
            LifecycleRuleState::DryRun,
        )
        .await;
    fixture.open_gates(true, true).await;
    let title = seed_title(&fixture.app, &fixture.user, "Unknowable").await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, LifecycleRuleState::Enabled, Some(1))
        .await;
    fixture.claims.set_unreadable(true);

    let report = fixture.handle().await;
    assert_eq!(report.held, 1, "{report:?}");
    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.state, LifecycleCandidateState::Blocked);
    assert_eq!(
        candidate.state_reason,
        execution_reason::UNKNOWN_AT_EXECUTION,
        "acting on a claim Scryer could not read is acting on evidence it never had"
    );
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .is_some()
    );
}

#[tokio::test]
async fn a_live_lease_does_not_hold_a_reversible_action() {
    let fixture = claims_app();
    let rule_id = fixture
        .rule(
            draft(
                ALWAYS_SOURCE,
                &[scryer_rules::lifecycle_actions::LifecycleActionHead::Unmonitor],
            ),
            LifecycleRuleState::DryRun,
        )
        .await;
    fixture.open_gates(true, false).await;
    let title = seed_title(&fixture.app, &fixture.user, "Leased").await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, LifecycleRuleState::Enabled, None)
        .await;
    fixture
        .claims
        .seed(lease("claim-1", &title.id, LifecycleClaimState::Active))
        .await;

    let report = fixture.handle().await;
    assert_eq!(report.held, 0, "{report:?}");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(
        !fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    );
}

// ── The facts a rule reads (FR-043) ─────────────────────────────────────────

#[tokio::test]
async fn the_shipped_template_matches_an_expired_lease() {
    let fixture = claims_app();
    fixture
        .rule(
            draft(
                EXPIRED_LEASE_SOURCE,
                &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
            ),
            LifecycleRuleState::DryRun,
        )
        .await;
    fixture.open_gates(false, false).await;
    let title = seed_title(&fixture.app, &fixture.user, "Lapsed").await;
    fixture
        .claims
        .seed(lease("claim-1", &title.id, LifecycleClaimState::Expired))
        .await;

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_created, 1, "{report:?}");
}

/// "Keep permanently" ends the lease, which leaves the title with no time to
/// delete at all.
#[tokio::test]
async fn the_shipped_template_spares_a_title_kept_permanently() {
    let fixture = claims_app();
    fixture
        .rule(
            draft(
                EXPIRED_LEASE_SOURCE,
                &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
            ),
            LifecycleRuleState::DryRun,
        )
        .await;
    fixture.open_gates(false, false).await;
    let title = seed_title(&fixture.app, &fixture.user, "Converted").await;
    fixture
        .claims
        .seed(lease("claim-1", &title.id, LifecycleClaimState::Converted))
        .await;

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_created, 0, "{report:?}");
}

#[tokio::test]
async fn the_shipped_template_spares_a_title_whose_lease_is_still_running() {
    let fixture = claims_app();
    fixture
        .rule(
            draft(
                EXPIRED_LEASE_SOURCE,
                &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
            ),
            LifecycleRuleState::DryRun,
        )
        .await;
    fixture.open_gates(false, false).await;
    let title = seed_title(&fixture.app, &fixture.user, "Running").await;
    let mut claim = lease("claim-1", &title.id, LifecycleClaimState::Active);
    claim.starts_at = Some(Utc::now() - chrono::Duration::days(1));
    claim.expires_at = Some(Utc::now() + chrono::Duration::days(29));
    fixture.claims.seed(claim).await;

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_created, 0, "{report:?}");
}

/// The gate in one test: a store Scryer cannot read must hold the rule, not
/// answer "there is no lease" and let it delete.
#[tokio::test]
async fn an_unreadable_claim_store_holds_a_rule_that_reads_a_lease_fact() {
    let fixture = claims_app();
    fixture
        .rule(
            draft(
                EXPIRED_LEASE_SOURCE,
                &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
            ),
            LifecycleRuleState::DryRun,
        )
        .await;
    fixture.open_gates(false, false).await;
    let title = seed_title(&fixture.app, &fixture.user, "Lapsed").await;
    fixture
        .claims
        .seed(lease("claim-1", &title.id, LifecycleClaimState::Expired))
        .await;
    fixture.evaluate().await;
    let before = fixture.only_candidate().await;

    fixture.claims.set_unreadable(true);
    let report = fixture.evaluate().await;

    assert_eq!(report.candidates_held, 1, "{report:?}");
    assert_eq!(
        report.candidates_canceled, 0,
        "an unreadable store is not a no-match"
    );
    let after = fixture.only_candidate().await;
    assert_eq!(after.id, before.id);
    assert!(after.held_since.is_some());
}

/// Preview is what an operator judges a matcher by, so it must not be able to
/// show a match the scheduled pass would refuse to open.
#[tokio::test]
async fn preview_reads_the_same_claim_facts_as_the_pass() {
    let fixture = claims_app();
    let spared = seed_title(&fixture.app, &fixture.user, "Converted").await;
    let doomed = seed_title(&fixture.app, &fixture.user, "Lapsed").await;
    fixture
        .claims
        .seed(lease("claim-1", &spared.id, LifecycleClaimState::Converted))
        .await;
    fixture
        .claims
        .seed(lease("claim-3", &doomed.id, LifecycleClaimState::Expired))
        .await;

    let preview = fixture
        .app
        .preview_lifecycle_rule(
            &fixture.user,
            LifecyclePreviewRequest {
                source: LifecyclePreviewSource::Inline {
                    library_ids: vec![],
                    rego_source: crate::lib_tests::heads_source(
                        EXPIRED_LEASE_SOURCE,
                        &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
                    ),
                    grace_days: 0,
                },
                selection: LifecyclePreviewSelection::Titles(vec![
                    spared.id.clone(),
                    doomed.id.clone(),
                ]),
            },
        )
        .await
        .expect("preview");

    let matched: Vec<&str> = preview
        .titles
        .iter()
        .filter(|result| result.outcome == Some(LifecycleOutcome::Match))
        .map(|result| result.title_id.as_str())
        .collect();
    assert_eq!(matched, vec![doomed.id.as_str()]);
}

// ── Activation, expiry, release (FR-041, FR-044) ────────────────────────────

#[tokio::test]
async fn an_import_starts_the_lease_clock() {
    let fixture = claims_app();
    let title = seed_title(&fixture.app, &fixture.user, "Arriving").await;
    fixture
        .claims
        .seed(lease("claim-1", &title.id, LifecycleClaimState::Dormant))
        .await;

    let started_at = Utc::now();
    fixture
        .app
        .activate_dormant_claims_for_title(&title.id, started_at)
        .await
        .expect("activate");

    let claim = fixture.only_claim().await;
    assert_eq!(claim.state, LifecycleClaimState::Active);
    assert_eq!(claim.starts_at, Some(started_at));
    assert_eq!(
        claim.expires_at,
        Some(started_at + chrono::Duration::days(30)),
        "the window is duration_days from the import, not from the approval"
    );
}

/// Activate one dormant claim seeded with `duration_days` and `expires_at`,
/// and return what the claim stores afterwards.
async fn activate_seeded(
    duration_days: Option<i64>,
    expires_at: Option<DateTime<Utc>>,
    started_at: DateTime<Utc>,
) -> LifecycleClaim {
    let fixture = claims_app();
    let title = seed_title(&fixture.app, &fixture.user, "Extended").await;
    fixture
        .claims
        .seed(LifecycleClaim {
            duration_days,
            expires_at,
            ..lease("claim-1", &title.id, LifecycleClaimState::Dormant)
        })
        .await;
    fixture
        .app
        .activate_dormant_claims_for_title(&title.id, started_at)
        .await
        .expect("activate");
    let claim = fixture.only_claim().await;
    assert_eq!(claim.state, LifecycleClaimState::Active);
    claim
}

#[tokio::test]
async fn activation_keeps_a_stored_expiry_when_the_claim_has_no_duration() {
    let started_at = Utc::now();
    let extended = started_at + chrono::Duration::days(90);
    let claim = activate_seeded(None, Some(extended), started_at).await;
    assert_eq!(claim.expires_at, Some(extended));
}

#[tokio::test]
async fn activation_keeps_the_later_of_a_stored_expiry_and_the_duration() {
    let started_at = Utc::now();
    let window_end = started_at + chrono::Duration::days(30);

    let later = started_at + chrono::Duration::days(90);
    let claim = activate_seeded(Some(30), Some(later), started_at).await;
    assert_eq!(
        claim.expires_at,
        Some(later),
        "the extension outlasts the window"
    );

    let earlier = started_at + chrono::Duration::days(5);
    let claim = activate_seeded(Some(30), Some(earlier), started_at).await;
    assert_eq!(
        claim.expires_at,
        Some(window_end),
        "the window outlasts the extension"
    );
}

#[tokio::test]
async fn activation_without_a_duration_or_a_stored_expiry_never_expires() {
    let claim = activate_seeded(None, None, Utc::now()).await;
    assert_eq!(claim.expires_at, None);
}

/// Activate one dormant claim whose stored expiry is already past, then run
/// the next pass. The claim activates with that stored date and the pass
/// expires it.
async fn assert_a_past_stored_expiry_is_kept_and_expires(duration_days: Option<i64>) {
    let fixture = claims_app();
    let title = seed_title(&fixture.app, &fixture.user, "Synthetic Lapsed Keep").await;
    let started_at = Utc::now();
    let stored_expiry = started_at - chrono::Duration::days(2);
    fixture
        .claims
        .seed(LifecycleClaim {
            duration_days,
            expires_at: Some(stored_expiry),
            ..lease("claim-1", &title.id, LifecycleClaimState::Dormant)
        })
        .await;
    fixture
        .app
        .activate_dormant_claims_for_title(&title.id, started_at)
        .await
        .expect("activate");
    let activated = fixture.only_claim().await;
    assert_eq!(activated.state, LifecycleClaimState::Active);
    assert_eq!(activated.starts_at, Some(started_at));
    assert_eq!(activated.expires_at, Some(stored_expiry));

    let report = fixture.evaluate().await;
    assert_eq!(report.leases_expired, 1, "{report:?}");
    let expired = fixture.only_claim().await;
    assert_eq!(expired.state, LifecycleClaimState::Expired);
    assert_eq!(expired.expires_at, Some(stored_expiry));
}

#[tokio::test]
async fn activation_without_a_duration_keeps_a_past_stored_expiry_and_the_next_pass_expires_it() {
    assert_a_past_stored_expiry_is_kept_and_expires(None).await;
}

#[tokio::test]
async fn activation_with_a_zero_duration_keeps_a_past_stored_expiry_and_the_next_pass_expires_it() {
    assert_a_past_stored_expiry_is_kept_and_expires(Some(0)).await;
}

/// The dormant sweep starts the clock at the title's first import, and a
/// stored expiry already past at that instant is kept and expired by the
/// pass after.
#[tokio::test]
async fn the_sweep_keeps_a_past_stored_expiry_at_the_backdated_import_and_then_expires_it() {
    let fixture = claims_app();
    let title = seed_title(&fixture.app, &fixture.user, "Synthetic Swept Keep").await;
    let first_imported_at = seed_media_file(&fixture.media_files, &title.id).await;
    let stored_expiry = first_imported_at - chrono::Duration::days(2);
    fixture
        .claims
        .seed(LifecycleClaim {
            duration_days: None,
            expires_at: Some(stored_expiry),
            ..lease("claim-1", &title.id, LifecycleClaimState::Dormant)
        })
        .await;

    let report = fixture.evaluate().await;
    assert_eq!(report.leases_activated, 1, "{report:?}");
    assert_eq!(report.leases_expired, 0, "{report:?}");
    let activated = fixture.only_claim().await;
    assert_eq!(activated.state, LifecycleClaimState::Active);
    assert_eq!(
        activated.starts_at.map(|value| value.timestamp()),
        Some(first_imported_at.timestamp()),
        "the clock is backdated to the import"
    );
    assert_eq!(activated.expires_at, Some(stored_expiry));

    let report = fixture.evaluate().await;
    assert_eq!(report.leases_expired, 1, "{report:?}");
    assert_eq!(
        fixture.only_claim().await.state,
        LifecycleClaimState::Expired
    );
}

/// No duration and no stored expiry: activated with no end, and later passes
/// never expire it.
#[tokio::test]
async fn a_claim_without_a_duration_or_a_stored_expiry_survives_later_passes() {
    let fixture = claims_app();
    let title = seed_title(&fixture.app, &fixture.user, "Synthetic Open Keep").await;
    fixture
        .claims
        .seed(LifecycleClaim {
            duration_days: None,
            expires_at: None,
            ..lease("claim-1", &title.id, LifecycleClaimState::Dormant)
        })
        .await;
    fixture
        .app
        .activate_dormant_claims_for_title(&title.id, Utc::now())
        .await
        .expect("activate");
    for _ in 0..2 {
        let report = fixture.evaluate().await;
        assert_eq!(report.leases_expired, 0, "{report:?}");
    }
    let claim = fixture.only_claim().await;
    assert_eq!(claim.state, LifecycleClaimState::Active);
    assert_eq!(claim.expires_at, None);
}

/// The safety net: an import that happened while the hook was missing or
/// failing still starts the clock, and it starts it at the import.
#[tokio::test]
async fn the_pass_activates_a_dormant_lease_whose_title_already_has_a_file() {
    let fixture = claims_app();
    let title = seed_title(&fixture.app, &fixture.user, "Already here").await;
    let first_imported_at = seed_media_file(&fixture.media_files, &title.id).await;
    fixture
        .claims
        .seed(lease("claim-1", &title.id, LifecycleClaimState::Dormant))
        .await;

    let report = fixture.evaluate().await;
    assert_eq!(report.leases_activated, 1, "{report:?}");

    let claim = fixture.only_claim().await;
    assert_eq!(claim.state, LifecycleClaimState::Active);
    assert_eq!(
        claim.starts_at.map(|value| value.timestamp()),
        Some(first_imported_at.timestamp()),
        "the clock is backdated to the import it missed"
    );
    assert_eq!(
        claim.expires_at.map(|value| value.timestamp()),
        Some((first_imported_at + chrono::Duration::days(30)).timestamp())
    );
    assert!(
        claim.updated_at >= first_imported_at,
        "updated_at is the write time, not the backdated start"
    );
}

#[tokio::test]
async fn the_pass_leaves_a_dormant_lease_alone_while_its_title_has_no_file() {
    let fixture = claims_app();
    let title = seed_title(&fixture.app, &fixture.user, "Still waiting").await;
    fixture
        .claims
        .seed(lease("claim-1", &title.id, LifecycleClaimState::Dormant))
        .await;

    let report = fixture.evaluate().await;
    assert_eq!(report.leases_activated, 0, "{report:?}");
    assert_eq!(
        fixture.only_claim().await.state,
        LifecycleClaimState::Dormant,
        "a lease is not late while the title has not arrived; it is waiting"
    );
}

#[tokio::test]
async fn the_pass_expires_a_lease_whose_window_has_elapsed() {
    let fixture = claims_app();
    let title = seed_title(&fixture.app, &fixture.user, "Lapsing").await;
    let mut claim = lease("claim-1", &title.id, LifecycleClaimState::Active);
    claim.starts_at = Some(Utc::now() - chrono::Duration::days(31));
    claim.expires_at = Some(Utc::now() - chrono::Duration::days(1));
    fixture.claims.seed(claim).await;

    let report = fixture.evaluate().await;
    assert_eq!(report.leases_expired, 1, "{report:?}");
    assert_eq!(
        fixture.only_claim().await.state,
        LifecycleClaimState::Expired
    );
}

/// Bookkeeping is not evaluation: an operator who turned rules off did not turn
/// off the clock a requester was promised.
#[tokio::test]
async fn lease_bookkeeping_runs_with_the_evaluation_gate_closed() {
    let fixture = claims_app();
    let title = seed_title(&fixture.app, &fixture.user, "Lapsing").await;
    let mut expiring = lease("claim-1", &title.id, LifecycleClaimState::Active);
    expiring.starts_at = Some(Utc::now() - chrono::Duration::days(31));
    expiring.expires_at = Some(Utc::now() - chrono::Duration::days(1));
    fixture.claims.seed(expiring).await;

    let arriving = seed_title(&fixture.app, &fixture.user, "Arrived").await;
    seed_media_file(&fixture.media_files, &arriving.id).await;
    fixture
        .claims
        .seed(lease("claim-2", &arriving.id, LifecycleClaimState::Dormant))
        .await;

    let report = fixture.evaluate().await;
    assert!(
        !report.gate_enabled,
        "this test is about the pass with rules switched off"
    );
    assert_eq!(report.leases_expired, 1, "{report:?}");
    assert_eq!(report.leases_activated, 1, "{report:?}");
}

#[tokio::test]
async fn deleting_a_title_releases_its_live_claims() {
    let fixture = claims_app();
    let title = seed_title(&fixture.app, &fixture.user, "Doomed").await;
    fixture
        .claims
        .seed(lease("claim-1", &title.id, LifecycleClaimState::Active))
        .await;

    fixture
        .app
        .delete_title(&fixture.user, &title.id, false, None)
        .await
        .expect("delete title");

    let claim = fixture.only_claim().await;
    assert_eq!(claim.state, LifecycleClaimState::Released);
    assert_eq!(claim.released_reason.as_deref(), Some("title_deleted"));
}

// ── The shipped expired-request-leases rule ─────────────────────────────────

impl ClaimFixture {
    async fn shipped_rule(&self) -> crate::lifecycle_rules::LifecycleRuleSetDetail {
        let rules = self
            .app
            .list_lifecycle_rule_set_details(&self.user)
            .await
            .expect("list rule sets");
        assert_eq!(rules.len(), 1, "{rules:?}");
        rules[0].clone()
    }
}

#[tokio::test]
async fn the_shipped_rule_is_created_once_and_starts_disabled() {
    let fixture = claims_app();
    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("seed");
    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("a second startup leaves the rule alone");

    let detail = fixture.shipped_rule().await;
    let rule = &detail.rule_set;
    assert_eq!(rule.name, "Expired request leases");
    assert!(
        rule.system_owned && rule.state == LifecycleRuleState::Disabled,
        "nothing is enabled by default"
    );
    assert!(rule.library_ids.is_empty(), "every library");
    assert_eq!(
        rule.max_titles_per_run,
        Some(10),
        "ships acting on ten a run"
    );
    assert_eq!(
        crate::lifecycle_rules::lifecycle_rule_state(&detail),
        LifecycleRuleState::Disabled
    );
    assert_eq!(rule.destructive_confirmed_revision, None);
    assert!(!rule.edited_since_enabled);
    assert_eq!(detail.revision.grace_days, 7);
    assert_eq!(detail.revision.created_by, None);
    assert_eq!(
        detail.revision.action_spec_json,
        r#"{"schema_version":3,"heads":["delete_title"],"destructive":true,"quality_profiles":[],"add_tags":[],"remove_tags":[]}"#
    );
    assert_eq!(
        scryer_rules::strip_editor_source(&detail.revision.rego_source),
        "delete_title if {\n\tinput.facts.request_lease_state == \"expired\"\n}\n"
    );
}

#[test]
fn the_shipped_rule_matches_the_web_template() {
    let template =
        include_str!("../../../../apps/scryer-web/lib/constants/lifecycle-rule-templates.ts");
    let source = crate::lifecycle_rules::EXPIRED_REQUEST_LEASES_REGO;
    let double_quoted = serde_json::to_string(source).expect("serialize matcher literal");
    let single_quoted = source
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    assert!(
        template.contains(&double_quoted) || template.contains(&format!("'{single_quoted}'")),
        "the expired-request-leases template and the shipped rule must carry the same source"
    );
}

#[tokio::test]
async fn the_shipped_rule_keeps_its_source_and_name_but_not_its_grace_or_scope() {
    let fixture = claims_app();
    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("seed");
    let id = fixture.shipped_rule().await.rule_set.id;
    // The rule ships Disabled. Enable it as an operator would, acknowledging
    // the (empty) candidate list, so the edits below start from Enabled.
    super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &id,
        LifecycleRuleState::Enabled,
        Some(0),
    )
    .await;
    let detail = fixture.shipped_rule().await;
    assert_eq!(detail.rule_set.state, LifecycleRuleState::Enabled);
    assert!(detail.rule_set.destructive_confirmed());
    let stored_source = scryer_rules::strip_editor_source(&detail.revision.rego_source);
    let matcher = |rego_source: &str, grace_days| crate::lifecycle_rules::LifecycleSourceDraft {
        rego_source: rego_source.to_string(),
        grace_days,
        max_titles_per_run: None,
    };
    let refused = |result: AppResult<_>| {
        matches!(
            result,
            Err(AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. })
        )
    };

    assert!(refused(
        fixture
            .app
            .delete_lifecycle_rule_set(&fixture.user, &id)
            .await
    ));
    assert!(refused(
        fixture
            .app
            .update_lifecycle_rule_metadata(
                &fixture.user,
                &id,
                "Renamed".into(),
                String::new(),
                Vec::new(),
            )
            .await
            .map(|_| ())
    ));
    assert!(refused(
        fixture
            .app
            .update_lifecycle_rule_source(&fixture.user, &id, matcher(ALWAYS_SOURCE, 7),)
            .await
            .map(|_| ())
    ));
    assert!(refused(
        fixture
            .app
            .update_lifecycle_rule_source(
                &fixture.user,
                &id,
                // The shipped source with another action declared beside it.
                matcher(&format!("{stored_source}\nunmonitor := true\n"), 7),
            )
            .await
            .map(|_| ())
    ));
    let unchanged = fixture.shipped_rule().await;
    assert_eq!(
        unchanged.rule_set, detail.rule_set,
        "a refused edit writes nothing"
    );
    assert_eq!(
        crate::lifecycle_rules::lifecycle_rule_state(&unchanged),
        LifecycleRuleState::Enabled
    );

    // Grace and libraries are edited exactly as on any rule, including the
    // drop to Dry run that every matcher or scope edit of an enabled rule has.
    let regraced = fixture
        .app
        .update_lifecycle_rule_source(&fixture.user, &id, matcher(&stored_source, 14))
        .await
        .expect("grace period is editable");
    assert_eq!(regraced.revision.grace_days, 14);
    let rescoped = fixture
        .app
        .update_lifecycle_rule_metadata(
            &fixture.user,
            &id,
            detail.rule_set.name.clone(),
            detail.rule_set.description.clone(),
            vec!["library-1".into()],
        )
        .await
        .expect("libraries are editable");
    assert_eq!(rescoped.library_ids, vec!["library-1".to_string()]);
    let edited = fixture.shipped_rule().await;
    assert!(edited.rule_set.system_owned);
    assert_eq!(
        crate::lifecycle_rules::lifecycle_rule_state(&edited),
        LifecycleRuleState::DryRun
    );
    assert!(edited.rule_set.edited_since_enabled);
    assert_eq!(edited.rule_set.destructive_confirmed_revision, None);
}

#[tokio::test]
async fn the_shipped_rule_matches_only_titles_whose_lease_expired() {
    let fixture = claims_app();
    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("seed");
    fixture.open_gates(false, false).await;
    let id = fixture.shipped_rule().await.rule_set.id;
    super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &id,
        LifecycleRuleState::Enabled,
        Some(0),
    )
    .await;

    let lapsed = seed_title(&fixture.app, &fixture.user, "Lapsed").await;
    fixture
        .claims
        .seed(lease("claim-1", &lapsed.id, LifecycleClaimState::Expired))
        .await;
    let kept = seed_title(&fixture.app, &fixture.user, "Kept").await;
    fixture
        .claims
        .seed(lease("claim-2", &kept.id, LifecycleClaimState::Converted))
        .await;
    let running = seed_title(&fixture.app, &fixture.user, "Running").await;
    fixture
        .claims
        .seed(lease("claim-3", &running.id, LifecycleClaimState::Dormant))
        .await;
    seed_title(&fixture.app, &fixture.user, "Unrequested").await;

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_created, 1, "{report:?}");
    assert_eq!(fixture.only_candidate().await.title_id, lapsed.id);
}

/// Instances that seeded the rule while it also checked for a keep claim get
/// the current matcher on the next boot, as a new revision that keeps the
/// operator's grace period and has to be armed again.
#[tokio::test]
async fn startup_replaces_the_previously_shipped_source() {
    let fixture = claims_app();
    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("seed");
    let seeded = fixture.shipped_rule().await;
    let id = seeded.rule_set.id.clone();
    let store = &fixture.app.services.customization.lifecycle_rule_sets;
    let previous_source = scryer_rules::lifecycle::rewrite_package_declaration(
        "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.request_lease_state == \"expired\"\n\tnot input.facts.keep_claim_active\n}\n",
        &id,
    );
    let now = Utc::now();
    let previous = scryer_domain::LifecycleRuleRevision {
        id: "previous-revision".to_string(),
        revision_number: 2,
        rego_source: previous_source.clone(),
        grace_days: 10,
        source_content_hash: scryer_rules::runtime::content_hash(&previous_source),
        created_at: now,
        ..seeded.revision.clone()
    };
    store
        .add_revision(&previous, None, now)
        .await
        .expect("old revision");
    store
        .update_rule_set_metadata(
            &id,
            &seeded.rule_set.name,
            "Deletes a requested title and its files once its request lease has expired, unless someone keeps it.",
            &[],
            None,
            None,
            false,
            now,)
        .await
        .expect("old description");

    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("upgrade");
    let upgraded = fixture.shipped_rule().await;
    assert_eq!(upgraded.revision.revision_number, 3);
    assert_eq!(upgraded.revision.grace_days, 10);
    assert!(
        !upgraded.revision.rego_source.contains("keep_claim_active"),
        "{}",
        upgraded.revision.rego_source
    );
    assert_eq!(
        upgraded.rule_set.description,
        "Deletes a requested title and its files once its request lease has expired."
    );
    assert_eq!(upgraded.rule_set.state, LifecycleRuleState::Disabled);
    assert_eq!(upgraded.rule_set.destructive_confirmed_revision, None);

    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("second boot");
    assert_eq!(
        fixture.shipped_rule().await.revision.revision_number,
        3,
        "a rule already on the current matcher is left alone"
    );
}

/// Instances that seeded the rule while a separate action definition deleted
/// what `match` matched get the current source on the next boot. It deletes
/// exactly the same titles, so the rule keeps its state, arming and grace.
#[tokio::test]
async fn startup_moves_the_match_form_rule_to_the_current_source_keeping_its_state() {
    let fixture = claims_app();
    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("seed");
    let id = fixture.shipped_rule().await.rule_set.id;
    super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &id,
        LifecycleRuleState::Enabled,
        Some(0),
    )
    .await;
    let armed = fixture.shipped_rule().await;
    let store = &fixture.app.services.customization.lifecycle_rule_sets;
    let match_source = scryer_rules::lifecycle::rewrite_package_declaration(
        "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.request_lease_state == \"expired\"\n}\n",
        &id,
    );
    let now = Utc::now();
    let match_form = scryer_domain::LifecycleRuleRevision {
        id: "match-form-revision".to_string(),
        revision_number: 2,
        rego_source: match_source.clone(),
        grace_days: 12,
        source_content_hash: scryer_rules::runtime::content_hash(&match_source),
        created_at: now,
        ..armed.revision.clone()
    };
    store
        .add_revision_keeping_state(&match_form, true, true, None, now)
        .await
        .expect("match-form revision");

    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("upgrade");
    let upgraded = fixture.shipped_rule().await;
    assert_eq!(upgraded.rule_set.id, id);
    assert_eq!(upgraded.revision.revision_number, 3);
    assert_eq!(upgraded.revision.grace_days, 12);
    assert_eq!(
        scryer_rules::strip_editor_source(&upgraded.revision.rego_source),
        "delete_title if {\n\tinput.facts.request_lease_state == \"expired\"\n}\n"
    );
    assert_eq!(
        crate::lifecycle_rules::lifecycle_rule_state(&upgraded),
        LifecycleRuleState::Enabled
    );
    assert_eq!(upgraded.rule_set.state, LifecycleRuleState::Enabled);
    assert_eq!(
        upgraded.rule_set.destructive_confirmed_revision,
        Some(3),
        "the confirmation moves to the replacing revision with the pointer"
    );
    assert!(upgraded.rule_set.destructive_confirmed());
    assert!(!upgraded.rule_set.edited_since_enabled);
    assert_eq!(upgraded.rule_set.description, armed.rule_set.description);
}
