//! Lifecycle claims as maintenance sees them (spec 0003 FR-041..FR-044).
//!
//! A lease is only a time to delete: it reaches maintenance through the facts
//! a rule reads, and preview and the scheduled pass have to read the same
//! ones. It never holds an action by itself.

use super::*;

use crate::lib_tests::maintenance_evaluation::InMemoryMaintenanceEvaluationRepo;
use crate::lib_tests::maintenance_rules::InMemoryMaintenanceRuleRepo;
use crate::lib_tests::request_rules_support::InMemoryLifecycleClaimRepo;
use crate::maintenance_rules::{
    MaintenanceActionKind, MaintenanceActionSpec, MaintenancePreviewMatcher,
    MaintenancePreviewRequest, MaintenancePreviewSelection, MaintenanceRuleDraft, execution_reason,
};
use scryer_domain::{
    LifecycleClaim, LifecycleClaimKind, LifecycleClaimProducer, LifecycleClaimState,
    MaintenanceCandidateState, MaintenanceEffectArming, MaintenanceRuleState,
};
use scryer_rules::maintenance::MaintenanceOutcome;

/// The shipped `expired-request-leases` template, byte-for-byte minus the
/// package line the authoring path rewrites.
const EXPIRED_LEASE_MATCHER: &str = "match if {\n\
     \tinput.facts.request_lease_state == \"expired\"\n\
     }\n";

/// Matches every title, so the executor's rechecks are what decide.
const ALWAYS_MATCHER: &str = "match := true\n";

struct ClaimFixture {
    app: AppUseCase,
    user: User,
    evaluation: Arc<InMemoryMaintenanceEvaluationRepo>,
    claims: Arc<InMemoryLifecycleClaimRepo>,
    media_files: Arc<MockMediaFileRepo>,
}

fn claims_app() -> ClaimFixture {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryMaintenanceRuleRepo::default());
    let evaluation = Arc::new(InMemoryMaintenanceEvaluationRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let claims = Arc::new(InMemoryLifecycleClaimRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_maintenance_rule_set_store(rules.clone())
            .with_maintenance_evaluation_store(evaluation.clone())
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
    async fn rule(&self, draft: MaintenanceRuleDraft, state: MaintenanceRuleState) -> String {
        let created = self
            .app
            .create_maintenance_rule_set(&self.user, draft)
            .await
            .expect("create rule set");
        super::maintenance_rules::set_rule_state(
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
        super::maintenance_rules::open_maintenance_gates(&self.app, reversible, destructive).await;
    }

    async fn arm(&self, rule_set_id: &str, arming: MaintenanceEffectArming, ack: Option<i64>) {
        super::maintenance_rules::arm_rule(&self.app, &self.user, rule_set_id, arming, ack).await;
    }

    async fn evaluate(&self) -> crate::maintenance_rules::MaintenanceEvaluationReport {
        self.app
            .run_maintenance_rule_evaluation_job()
            .await
            .expect("evaluation pass")
    }

    async fn handle(&self) -> crate::maintenance_rules::MaintenanceActionHandlingReport {
        self.app
            .run_lifecycle_action_handling_job()
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

fn draft(rego_source: &str, kind: MaintenanceActionKind) -> MaintenanceRuleDraft {
    MaintenanceRuleDraft {
        subject_kind: scryer_domain::MaintenanceRuleSubjectKind::Title,
        name: "Lease rule".to_string(),
        description: String::new(),
        rego_source: rego_source.to_string(),
        action_definition: crate::maintenance_rules::MaintenanceActionDefinition::Legacy(
            MaintenanceActionSpec::new(kind),
        ),
        grace_days: 0,
        storage_root_id: None,
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
            draft(ALWAYS_MATCHER, MaintenanceActionKind::DeleteTitleAndFiles),
            MaintenanceRuleState::DryRun,
        )
        .await;
    fixture.open_gates(true, true).await;
    let root = tempfile::tempdir().expect("claim fixture root");
    fixture
        .app
        .update_media_settings(
            &fixture.user,
            MediaFacet::Movie,
            empty_update_media_settings_with_roots(vec![build_root_folder_entry(
                root.path(),
                true,
            )]),
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
    super::maintenance_title_completion::record_fixture_title_folder(
        &fixture.app,
        &title.id,
        &folder,
    )
    .await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Destructive, Some(1))
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
            draft(ALWAYS_MATCHER, MaintenanceActionKind::DeleteTitleAndFiles),
            MaintenanceRuleState::DryRun,
        )
        .await;
    fixture.open_gates(true, true).await;
    let title = seed_title(&fixture.app, &fixture.user, "Guarded lease").await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Destructive, Some(1))
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
            draft(ALWAYS_MATCHER, MaintenanceActionKind::DeleteTitleAndFiles),
            MaintenanceRuleState::DryRun,
        )
        .await;
    fixture.open_gates(true, true).await;
    let title = seed_title(&fixture.app, &fixture.user, "Unknowable").await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Destructive, Some(1))
        .await;
    fixture.claims.set_unreadable(true);

    let report = fixture.handle().await;
    assert_eq!(report.held, 1, "{report:?}");
    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.state, MaintenanceCandidateState::Blocked);
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
                ALWAYS_MATCHER,
                MaintenanceActionKind::UnmonitorScopeKeepFiles,
            ),
            MaintenanceRuleState::DryRun,
        )
        .await;
    fixture.open_gates(true, false).await;
    let title = seed_title(&fixture.app, &fixture.user, "Leased").await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
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
                EXPIRED_LEASE_MATCHER,
                MaintenanceActionKind::DeleteTitleAndFiles,
            ),
            MaintenanceRuleState::DryRun,
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
                EXPIRED_LEASE_MATCHER,
                MaintenanceActionKind::DeleteTitleAndFiles,
            ),
            MaintenanceRuleState::DryRun,
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
                EXPIRED_LEASE_MATCHER,
                MaintenanceActionKind::DeleteTitleAndFiles,
            ),
            MaintenanceRuleState::DryRun,
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
                EXPIRED_LEASE_MATCHER,
                MaintenanceActionKind::DeleteTitleAndFiles,
            ),
            MaintenanceRuleState::DryRun,
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
        .preview_maintenance_rule(
            &fixture.user,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Inline {
                    library_ids: vec![],
                    subject_kind: scryer_domain::MaintenanceRuleSubjectKind::Title,
                    rego_source: EXPIRED_LEASE_MATCHER.to_string(),
                    action_definition:
                        crate::maintenance_rules::MaintenanceActionDefinition::Legacy(
                            MaintenanceActionSpec::new(MaintenanceActionKind::DeleteTitleAndFiles),
                        ),
                    grace_days: 0,
                    storage_root_id: None,
                },
                selection: MaintenancePreviewSelection::Titles(vec![
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
        .filter(|result| result.outcome == Some(MaintenanceOutcome::Match))
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
    async fn shipped_rule(&self) -> crate::maintenance_rules::MaintenanceRuleSetDetail {
        let rules = self
            .app
            .list_maintenance_rule_set_details(&self.user)
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
        rule.system_owned && !rule.enabled,
        "nothing is enabled by default"
    );
    assert!(rule.library_ids.is_empty(), "every library");
    assert_eq!(
        rule.subject_kind,
        scryer_domain::MaintenanceRuleSubjectKind::Title
    );
    assert_eq!(
        crate::maintenance_rules::maintenance_rule_state(&detail),
        MaintenanceRuleState::Disabled
    );
    assert_eq!(rule.effect_arming, MaintenanceEffectArming::None);
    assert!(!rule.destructive_rearm_required);
    assert_eq!(detail.revision.grace_days, 7);
    assert_eq!(detail.revision.created_by, None);
    assert_eq!(
        detail.revision.action_spec_json,
        r#"{"schema_version":2,"steps":[{"id":"legacy-delete-title","kind":"delete_title_and_files"}]}"#
    );
    assert_eq!(
        scryer_rules::strip_editor_source(&detail.revision.rego_source),
        EXPIRED_LEASE_MATCHER
    );
}

#[test]
fn the_shipped_rule_matches_the_web_template() {
    let template =
        include_str!("../../../../apps/scryer-web/lib/constants/maintenance-rule-templates.ts");
    let escaped = crate::maintenance_rules::EXPIRED_REQUEST_LEASES_REGO
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t");
    assert!(
        template.contains(&format!("\"{escaped}\"")),
        "the expired-request-leases template and the shipped rule must carry the same source"
    );
}

#[tokio::test]
async fn the_shipped_rule_keeps_its_matcher_and_name_but_not_its_grace_or_scope() {
    let fixture = claims_app();
    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("seed");
    let id = fixture.shipped_rule().await.rule_set.id;
    // The rule ships Disabled. Enable it as an operator would, acknowledging
    // the (empty) candidate list, so the edits below start from Enabled.
    super::maintenance_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &id,
        MaintenanceRuleState::Enabled,
        Some(0),
    )
    .await;
    let detail = fixture.shipped_rule().await;
    assert_eq!(
        detail.rule_set.effect_arming,
        MaintenanceEffectArming::Destructive
    );
    let stored_source = scryer_rules::strip_editor_source(&detail.revision.rego_source);
    let matcher = |rego_source: &str, action_definition, grace_days| {
        crate::maintenance_rules::MaintenanceMatcherDraft {
            rego_source: rego_source.to_string(),
            action_definition,
            grace_days,
            storage_root_id: None,
        }
    };
    let refused = |result: AppResult<_>| matches!(result, Err(AppError::Validation(_)));

    assert!(refused(
        fixture
            .app
            .delete_maintenance_rule_set(&fixture.user, &id)
            .await
    ));
    assert!(refused(
        fixture
            .app
            .update_maintenance_rule_metadata(
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
            .update_maintenance_rule_matcher(
                &fixture.user,
                &id,
                matcher(ALWAYS_MATCHER, detail.action_definition.clone(), 7),
            )
            .await
            .map(|_| ())
    ));
    assert!(refused(
        fixture
            .app
            .update_maintenance_rule_matcher(
                &fixture.user,
                &id,
                matcher(
                    &stored_source,
                    crate::maintenance_rules::MaintenanceActionDefinition::Legacy(
                        MaintenanceActionSpec::new(MaintenanceActionKind::UnmonitorScopeKeepFiles),
                    ),
                    7,
                ),
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
        crate::maintenance_rules::maintenance_rule_state(&unchanged),
        MaintenanceRuleState::Enabled
    );

    // Grace and libraries are edited exactly as on any rule, including the
    // drop to Dry run that every matcher or scope edit of an enabled rule has.
    let regraced = fixture
        .app
        .update_maintenance_rule_matcher(
            &fixture.user,
            &id,
            matcher(&stored_source, detail.action_definition.clone(), 14),
        )
        .await
        .expect("grace period is editable");
    assert_eq!(regraced.revision.grace_days, 14);
    let rescoped = fixture
        .app
        .update_maintenance_rule_metadata(
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
        crate::maintenance_rules::maintenance_rule_state(&edited),
        MaintenanceRuleState::DryRun
    );
    assert!(edited.rule_set.destructive_rearm_required);
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
    super::maintenance_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &id,
        MaintenanceRuleState::Enabled,
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
async fn startup_replaces_the_previously_shipped_matcher() {
    let fixture = claims_app();
    fixture
        .app
        .ensure_expired_request_leases_rule()
        .await
        .expect("seed");
    let seeded = fixture.shipped_rule().await;
    let id = seeded.rule_set.id.clone();
    let store = &fixture.app.services.customization.maintenance_rule_sets;
    let previous_source = scryer_rules::maintenance::rewrite_package_declaration(
        "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.request_lease_state == \"expired\"\n\tnot input.facts.keep_claim_active\n}\n",
        &id,
    );
    let now = Utc::now();
    let previous = scryer_domain::MaintenanceRuleRevision {
        id: "previous-revision".to_string(),
        revision_number: 2,
        rego_source: previous_source.clone(),
        grace_days: 10,
        matcher_content_hash: scryer_rules::runtime::content_hash(&previous_source),
        created_at: now,
        ..seeded.revision.clone()
    };
    store
        .add_revision(&previous, now)
        .await
        .expect("old revision");
    store
        .update_rule_set_metadata(
            &id,
            &seeded.rule_set.name,
            "Deletes a requested title and its files once its request lease has expired, unless someone keeps it.",
            &[],
            false,
            now,
        )
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
    assert_eq!(
        upgraded.rule_set.effect_arming,
        MaintenanceEffectArming::None
    );

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
