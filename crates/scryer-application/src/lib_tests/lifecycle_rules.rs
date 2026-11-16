use super::*;

use crate::lib_tests::lifecycle_evaluation::InMemoryLifecycleEvaluationRepo;
use crate::lifecycle_rules::{
    LIFECYCLE_MAX_GRACE_DAYS, LifecycleActionKind, LifecycleMatcherDraft, LifecyclePreviewMatcher,
    LifecyclePreviewRequest, LifecyclePreviewSelection, LifecycleRuleDraft,
};
use scryer_domain::{
    LifecycleActionRun, LifecycleActionRunStatus, LifecycleCandidate, LifecycleCandidateState,
    LifecycleEffectArming, LifecycleEvaluationMode, LifecycleRuleRevision, LifecycleRuleSet,
    LifecycleRuleSubjectKind,
};
use scryer_rules::lifecycle::LifecycleOutcome;

/// Matches every monitored title. The package line is deliberately wrong so
/// every test also proves the service rewrites it to the assigned rule ID.
const MONITORED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.facts.monitored\n\
     }\n";

/// Matches nothing: the facet never equals this sentinel.
const NEVER_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.subject.facet == \"not-a-facet\"\n\
     }\n";

/// Would match, but needs a fact this wave cannot observe, so it must hold.
///
/// No `unknown` rule of its own: reading `input.facts.last_upgraded_at` is
/// enough, because the engine will not consult a rule whose facts it could not
/// observe for the subject.
const NEEDS_UPGRADE_HISTORY_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.facts.last_upgraded_at != \"\"\n\
     }\n";

/// What a test wants [`InMemoryLifecycleRuleRepo::get_rule_set`] to answer
/// instead of reading storage.
///
/// The executor's safety recheck is the only caller of that read during a
/// handler pass, and it has to tell two failures apart: a rule that is genuinely
/// gone (cancel the candidate — its authorization no longer exists) from a store
/// it merely could not reach (hold, like every other unresolvable signal).
/// Deleting the rule for real cannot exercise this, because a deleted rule is
/// not in the pass's rule listing at all.
#[derive(Clone, Copy, Debug)]
pub(super) enum LifecycleRuleReadFault {
    /// The rule set is gone.
    Missing,
    /// The store could not answer.
    Unreachable,
}

#[derive(Default)]
pub(super) struct InMemoryLifecycleRuleRepo {
    rule_sets: Mutex<Vec<LifecycleRuleSet>>,
    revisions: Mutex<Vec<LifecycleRuleRevision>>,
    read_fault: Mutex<Option<LifecycleRuleReadFault>>,
}

impl InMemoryLifecycleRuleRepo {
    /// Make every subsequent single-rule read fail this way.
    pub(super) async fn fail_rule_set_reads(&self, fault: LifecycleRuleReadFault) {
        *self.read_fault.lock().await = Some(fault);
    }
}

impl InMemoryLifecycleRuleRepo {
    /// Re-point a rule's library scope without the disarm the service writes
    /// alongside it.
    ///
    /// Production cannot do this — `update_lifecycle_rule_metadata` disarms in
    /// the same write — but a test that wants to prove the *executor's* own
    /// scope recheck has to arrive at the handler with a narrowed scope and an
    /// arming still in place. Going through the service instead would only
    /// re-test the disarm.
    pub(super) async fn re_scope_without_disarming(&self, id: &str, library_ids: Vec<String>) {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == id)
            .expect("rule set exists");
        rule_set.library_ids = library_ids;
    }

    /// Rewrite the revision currently in force without appending a new one.
    ///
    /// Production never does this — revisions are immutable — but a test that
    /// wants to change what a rule *decides* without also triggering the
    /// supersede path has no other way to isolate the two behaviours.
    pub(super) async fn replace_revision_in_place(&self, revision: LifecycleRuleRevision) {
        let mut revisions = self.revisions.lock().await;
        revisions.retain(|stored| {
            !(stored.rule_set_id == revision.rule_set_id
                && stored.revision_number == revision.revision_number)
        });
        revisions.push(revision);
    }
}

#[async_trait]
impl LifecycleRuleSetRepository for InMemoryLifecycleRuleRepo {
    async fn list_rule_sets(&self) -> AppResult<Vec<LifecycleRuleSet>> {
        Ok(self.rule_sets.lock().await.clone())
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<LifecycleRuleSet>> {
        match *self.read_fault.lock().await {
            Some(LifecycleRuleReadFault::Missing) => return Ok(None),
            Some(LifecycleRuleReadFault::Unreachable) => {
                return Err(AppError::Repository(
                    "lifecycle rule store is unreachable".to_string(),
                ));
            }
            None => {}
        }
        Ok(self
            .rule_sets
            .lock()
            .await
            .iter()
            .find(|rule_set| rule_set.id == id)
            .cloned())
    }

    async fn create_rule_set(
        &self,
        rule_set: &LifecycleRuleSet,
        revision: &LifecycleRuleRevision,
    ) -> AppResult<()> {
        self.rule_sets.lock().await.push(rule_set.clone());
        self.revisions.lock().await.push(revision.clone());
        Ok(())
    }

    async fn add_revision(
        &self,
        revision: &LifecycleRuleRevision,
        max_titles_per_run: Option<Option<u32>>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == revision.rule_set_id)
            .ok_or_else(|| AppError::NotFound(revision.rule_set_id.clone()))?;
        if rule_set.current_revision_number != revision.revision_number - 1 {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::RULE_REVISION_CONFLICT,
                "the rule was changed by another save; reload it and save again".to_string(),
            ));
        }
        rule_set.current_revision_number = revision.revision_number;
        // The SQL store disarms in the same transaction as the pointer move; a
        // permissive double here would hide an arming that survived a matcher
        // swap.
        rule_set.disarm_after_edit();
        if let Some(max) = max_titles_per_run {
            rule_set.max_titles_per_run = max;
        }
        rule_set.updated_at = updated_at;
        self.revisions.lock().await.push(revision.clone());
        Ok(())
    }

    async fn add_revision_keeping_arming(
        &self,
        revision: &LifecycleRuleRevision,
        effect_arming: scryer_domain::LifecycleEffectArming,
        max_titles_per_run: Option<Option<u32>>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == revision.rule_set_id)
            .ok_or_else(|| AppError::NotFound(revision.rule_set_id.clone()))?;
        // The SQL store moves the pointer only from the revision this one
        // follows.
        if rule_set.current_revision_number != revision.revision_number - 1 {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::RULE_REVISION_CONFLICT,
                "the rule was changed by another save; reload it and save again".to_string(),
            ));
        }
        rule_set.current_revision_number = revision.revision_number;
        // As the SQL store: never above what is stored now.
        let rank = |arming: scryer_domain::LifecycleEffectArming| match arming {
            scryer_domain::LifecycleEffectArming::None => 0,
            scryer_domain::LifecycleEffectArming::Reversible => 1,
            scryer_domain::LifecycleEffectArming::Destructive => 2,
        };
        if rank(effect_arming) < rank(rule_set.effect_arming) {
            rule_set.effect_arming = effect_arming;
        }
        if let Some(max) = max_titles_per_run {
            rule_set.max_titles_per_run = max;
        }
        rule_set.updated_at = updated_at;
        self.revisions.lock().await.push(revision.clone());
        Ok(())
    }

    async fn get_revision(
        &self,
        rule_set_id: &str,
        revision_number: i64,
    ) -> AppResult<Option<LifecycleRuleRevision>> {
        Ok(self
            .revisions
            .lock()
            .await
            .iter()
            .find(|revision| {
                revision.rule_set_id == rule_set_id && revision.revision_number == revision_number
            })
            .cloned())
    }

    async fn list_revisions(&self, rule_set_id: &str) -> AppResult<Vec<LifecycleRuleRevision>> {
        let mut revisions: Vec<LifecycleRuleRevision> = self
            .revisions
            .lock()
            .await
            .iter()
            .filter(|revision| revision.rule_set_id == rule_set_id)
            .cloned()
            .collect();
        revisions.sort_by_key(|revision| std::cmp::Reverse(revision.revision_number));
        Ok(revisions)
    }

    async fn update_rule_set_metadata(
        &self,
        id: &str,
        name: &str,
        description: &str,
        library_ids: &[String],
        schedule: Option<&scryer_domain::ScriptSchedule>,
        max_titles_per_run: Option<Option<u32>>,
        disarm: bool,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        rule_set.name = name.to_string();
        rule_set.description = description.to_string();
        rule_set.library_ids = library_ids.to_vec();
        if let Some(schedule) = schedule {
            rule_set.schedule = schedule.clone();
        }
        if let Some(max) = max_titles_per_run {
            rule_set.max_titles_per_run = max;
        }
        // The SQL store disarms inside the same UPDATE; a permissive double here
        // would hide an arming that survived a scope change.
        if disarm {
            rule_set.disarm_after_edit();
        }
        rule_set.updated_at = updated_at;
        Ok(())
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        self.rule_sets
            .lock()
            .await
            .retain(|rule_set| rule_set.id != id);
        self.revisions
            .lock()
            .await
            .retain(|revision| revision.rule_set_id != id);
        Ok(())
    }

    async fn delete_rule_set_if_unused(&self, id: &str) -> AppResult<()> {
        // Application guard tests use the separate in-memory evaluation store;
        // SQL concurrency and atomic guard behavior are covered by store tests.
        self.delete_rule_set(id).await
    }

    async fn set_rule_set_max_titles_per_run(
        &self,
        id: &str,
        max_titles_per_run: Option<u32>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        rule_set.max_titles_per_run = max_titles_per_run;
        rule_set.updated_at = updated_at;
        Ok(())
    }

    async fn update_rule_set_state(
        &self,
        id: &str,
        mode: LifecycleEvaluationMode,
        arming: scryer_domain::LifecycleEffectArming,
        confirmation: Option<&scryer_domain::LifecycleRuleArmingConfirmation>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut rule_sets = self.rule_sets.lock().await;
        let Some(rule_set) = rule_sets.iter_mut().find(|rule_set| rule_set.id == id) else {
            return Ok(false);
        };
        if arming == scryer_domain::LifecycleEffectArming::Destructive
            && !confirmation.is_some_and(|reviewed| reviewed.matches(rule_set))
        {
            return Ok(false);
        }
        rule_set.evaluation_mode = mode;
        rule_set.enabled = mode != LifecycleEvaluationMode::Disabled;
        rule_set.effect_arming = arming;
        rule_set.destructive_rearm_required = false;
        rule_set.updated_at = updated_at;
        Ok(true)
    }
}

struct LifecycleFixture {
    app: AppUseCase,
    user: User,
    rules: Arc<InMemoryLifecycleRuleRepo>,
    media_files: Arc<MockMediaFileRepo>,
    /// Authoring reads this store to decide what a delete would destroy, so the
    /// real double is wired in rather than the null one, whose empty answers
    /// would make every rule look historyless.
    evaluation: Arc<InMemoryLifecycleEvaluationRepo>,
}

fn lifecycle_app() -> LifecycleFixture {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryLifecycleRuleRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let evaluation = Arc::new(InMemoryLifecycleEvaluationRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_lifecycle_rule_set_store(rules.clone())
            .with_lifecycle_evaluation_store(evaluation.clone())
            .with_media_files(media_files.clone())
    });
    LifecycleFixture {
        app,
        user,
        rules,
        media_files,
        evaluation,
    }
}

/// A finished action run, the evidence a delete must refuse to erase.
fn action_run(rule_set_id: &str, title_id: &str) -> LifecycleActionRun {
    let now = Utc::now();
    LifecycleActionRun {
        id: Id::new().0,
        candidate_id: "candidate-1".to_string(),
        rule_set_id: rule_set_id.to_string(),
        revision_number: 1,
        title_id: title_id.to_string(),
        subject_id: title_id.to_string(),
        subject_kind: "title".to_string(),
        action_kind: LifecycleActionKind::DeleteTitleAndFiles
            .as_wire_str()
            .to_string(),
        match_generation: 1,
        idempotency_key: Id::new().0,
        attempt: 1,
        status: LifecycleActionRunStatus::Succeeded,
        hold_reason: None,
        error: None,
        detail: "{}".to_string(),
        started_at: now,
        finished_at: Some(now),
        created_at: now,
    }
}

/// A live candidate: membership an operator can currently see.
fn active_candidate(rule_set_id: &str, title_id: &str) -> LifecycleCandidate {
    let now = Utc::now();
    LifecycleCandidate {
        id: Id::new().0,
        rule_set_id: rule_set_id.to_string(),
        revision_number: 1,
        matcher_content_hash: "hash".to_string(),
        title_id: title_id.to_string(),
        subject_id: title_id.to_string(),
        library_id: "library-a".to_string(),
        facet: MediaFacet::Movie.as_str().to_string(),
        subject_kind: LifecycleRuleSubjectKind::Title.as_storage_str().to_string(),
        match_generation: 1,
        state: LifecycleCandidateState::Observing,
        state_reason: "first_match".to_string(),
        reason_codes: vec![],
        action_kind: LifecycleActionKind::UnmonitorScopeKeepFiles
            .as_wire_str()
            .to_string(),
        grace_days: 7,
        first_matched_at: now,
        last_matched_at: now,
        due_at: now,
        last_evaluated_at: now,
        held_since: None,
        action_attempts: 0,
        decided_actions_json: None,
        created_at: now,
        updated_at: now,
    }
}

fn draft(rego_source: &str) -> LifecycleRuleDraft {
    LifecycleRuleDraft {
        max_titles_per_run: Some(100),
        subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
        name: "Stale movies".to_string(),
        description: "Unwatched for a long time".to_string(),
        rego_source: crate::lib_tests::legacy_source(
            rego_source,
            crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
        ),
        grace_days: 7,
        storage_root_id: None,
        library_ids: Vec::new(),
    }
}

async fn seed_title(app: &AppUseCase, user: &User, name: &str, monitored: bool) -> Title {
    app.add_title(
        user,
        NewTitle {
            name: name.to_string(),
            facet: MediaFacet::Movie,
            monitored,
            tags: vec![],
            external_ids: vec![],
            ..Default::default()
        },
    )
    .await
    .expect("create title")
}

async fn seed_media_file(media_files: &MockMediaFileRepo, title_id: &str, size_bytes: i64) {
    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.to_string(),
            file_path: format!("/media/{title_id}.mkv"),
            size_bytes,
            quality_label: Some("1080P".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
}

pub(super) async fn arming_confirmation(
    app: &AppUseCase,
    id: &str,
) -> Option<scryer_domain::LifecycleRuleArmingConfirmation> {
    let rule = app
        .services
        .customization
        .lifecycle_rule_sets
        .get_rule_set(id)
        .await
        .unwrap()
        .unwrap();
    Some((&rule).into())
}

/// Moves a rule through the operator state control with the confirmation the
/// web dialog sends: the current revision and scope, and `ack` as the count.
pub(super) async fn set_rule_state(
    app: &AppUseCase,
    user: &User,
    id: &str,
    state: scryer_domain::LifecycleRuleState,
    ack: Option<i64>,
) -> crate::lifecycle_rules::LifecycleRuleSetDetail {
    app.set_lifecycle_rule_state(user, id, state, arming_confirmation(app, id).await, ack)
        .await
        .expect("set rule state")
}

/// Arms a rule through the state control and checks the arming it derived is
/// the one the test meant to grant.
pub(super) async fn arm_rule(
    app: &AppUseCase,
    user: &User,
    id: &str,
    arming: LifecycleEffectArming,
    ack: Option<i64>,
) {
    let state = if arming == LifecycleEffectArming::None {
        scryer_domain::LifecycleRuleState::DryRun
    } else {
        scryer_domain::LifecycleRuleState::Enabled
    };
    let detail = set_rule_state(app, user, id, state, ack).await;
    assert_eq!(detail.rule_set.effect_arming, arming, "derived arming");
}

/// Writes every stored gate directly. Operators move them in pairs through the
/// two switches; executor tests open one effect class without the other.
pub(super) async fn store_lifecycle_gates(
    app: &AppUseCase,
    gates: crate::lifecycle_rules::LifecycleGates,
) {
    use crate::settings::keys::{
        LIFECYCLE_GATE_DESTRUCTIVE_EFFECTS_KEY, LIFECYCLE_GATE_EVALUATION_KEY,
        LIFECYCLE_GATE_RESULT_DISPLAY_KEY, LIFECYCLE_GATE_REVERSIBLE_EFFECTS_KEY,
    };
    let values = [
        (LIFECYCLE_GATE_EVALUATION_KEY, gates.evaluation_enabled),
        (
            LIFECYCLE_GATE_RESULT_DISPLAY_KEY,
            gates.result_display_enabled,
        ),
        (
            LIFECYCLE_GATE_REVERSIBLE_EFFECTS_KEY,
            gates.reversible_effects_enabled,
        ),
        (
            LIFECYCLE_GATE_DESTRUCTIVE_EFFECTS_KEY,
            gates.destructive_effects_enabled,
        ),
    ]
    .map(|(key, value)| (key.to_string(), value.to_string()));
    app.services
        .config
        .settings
        .upsert_global_settings_json(
            crate::SETTINGS_SCOPE_SYSTEM,
            &values,
            crate::settings::keys::SETTINGS_SOURCE_TYPED_GRAPHQL,
            None,
        )
        .await
        .expect("store gates");
}

/// Evaluation and result display on, with each effect gate as given.
pub(super) async fn open_lifecycle_gates(app: &AppUseCase, reversible: bool, destructive: bool) {
    store_lifecycle_gates(
        app,
        crate::lifecycle_rules::LifecycleGates {
            evaluation_enabled: true,
            result_display_enabled: true,
            reversible_effects_enabled: reversible,
            destructive_effects_enabled: destructive,
        },
    )
    .await;
}

// ── Authoring ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_persists_a_dormant_rule_set_and_its_first_revision() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();

    let detail = app
        .create_lifecycle_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");

    assert!(!detail.rule_set.enabled, "rule sets ship dark");
    assert_eq!(
        detail.rule_set.evaluation_mode,
        LifecycleEvaluationMode::Disabled
    );
    assert_eq!(
        detail.rule_set.subject_kind,
        LifecycleRuleSubjectKind::Title
    );
    assert_eq!(detail.rule_set.current_revision_number, 1);
    assert_eq!(detail.revision.revision_number, 1);
    assert_eq!(detail.revision.grace_days, 7);
    assert_eq!(
        detail.revision.created_by.as_deref(),
        Some(user.id.as_str())
    );

    let expected_package = format!("package scryer.maintenance.user.{}", detail.rule_set.id);
    assert!(
        detail
            .revision
            .rego_source
            .lines()
            .any(|line| line.trim() == expected_package),
        "stored source must carry the rewritten package: {}",
        detail.revision.rego_source
    );
    assert_eq!(
        detail.revision.matcher_content_hash,
        scryer_rules::runtime::content_hash(&detail.revision.rego_source),
        "the stored hash must be the hash of the stored source"
    );

    assert_eq!(rules.list_rule_sets().await.unwrap().len(), 1);
    assert_eq!(
        rules
            .list_revisions(&detail.rule_set.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn builder_model_rides_each_revision_and_carries_forward_when_kept() {
    use crate::BuilderModelChange;

    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();
    let model = serde_json::json!({
        "version": 2,
        "family": "lifecycle",
        "join": "ALL",
        "conditions": [],
        "outputs": {"UNMONITOR": true}
    });
    let created = app
        .create_lifecycle_rule_set_with_builder_model(
            &user,
            draft(MONITORED_MATCHER),
            BuilderModelChange::Set(model.clone()),
        )
        .await
        .expect("create with builder model");
    let stored = created
        .revision
        .builder_model_json
        .clone()
        .expect("first revision stores the document");
    assert_eq!(
        crate::stored_builder_model(Some(&stored)),
        Some(model.clone())
    );

    let matcher = |grace_days| LifecycleMatcherDraft {
        rego_source: crate::lib_tests::legacy_source(
            MONITORED_MATCHER,
            crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
        ),
        grace_days,
        storage_root_id: None,
        max_titles_per_run: None,
    };

    // Omitted: the new revision carries the current document forward.
    let kept = app
        .update_lifecycle_rule_matcher(&user, &created.rule_set.id, matcher(8))
        .await
        .expect("update keeping model");
    assert_eq!(
        kept.revision.builder_model_json.as_deref(),
        Some(stored.as_str())
    );

    // A hand edit with the field omitted leaves the new revision without one.
    let edited = app
        .update_lifecycle_rule_matcher(
            &user,
            &created.rule_set.id,
            LifecycleMatcherDraft {
                rego_source: crate::lib_tests::legacy_source(
                    NEVER_MATCHER,
                    crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                ),
                grace_days: 8,
                storage_root_id: None,
                max_titles_per_run: None,
            },
        )
        .await
        .expect("hand edit");
    assert_eq!(edited.revision.builder_model_json, None);
    let restored = app
        .update_lifecycle_rule_matcher_with_builder_model(
            &user,
            &created.rule_set.id,
            matcher(8),
            BuilderModelChange::Set(model.clone()),
        )
        .await
        .expect("builder save");
    assert_eq!(
        restored.revision.builder_model_json.as_deref(),
        Some(stored.as_str())
    );

    // A scoring document is refused and no revision is appended.
    let refused = app
        .update_lifecycle_rule_matcher_with_builder_model(
            &user,
            &created.rule_set.id,
            matcher(9),
            BuilderModelChange::Set(serde_json::json!({"version": 2, "family": "scoring"})),
        )
        .await
        .expect_err("wrong family is refused");
    assert_eq!(
        refused.validation_reason(),
        Some(crate::error_reasons::rules::BUILDER_MODEL_INVALID)
    );
    assert_eq!(
        rules
            .list_revisions(&created.rule_set.id)
            .await
            .unwrap()
            .len(),
        4
    );

    // Cleared: the next revision is hand-written; earlier revisions keep theirs.
    let cleared = app
        .update_lifecycle_rule_matcher_with_builder_model(
            &user,
            &created.rule_set.id,
            matcher(10),
            BuilderModelChange::Clear,
        )
        .await
        .expect("clear model");
    assert_eq!(cleared.revision.builder_model_json, None);
    let first = rules
        .get_revision(&created.rule_set.id, 1)
        .await
        .unwrap()
        .expect("revision 1");
    assert_eq!(first.builder_model_json.as_deref(), Some(stored.as_str()));
}

#[tokio::test]
async fn update_matcher_appends_a_revision_and_leaves_the_previous_one_untouched() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");
    let first = rules
        .get_revision(&created.rule_set.id, 1)
        .await
        .unwrap()
        .expect("revision 1");

    let updated = app
        .update_lifecycle_rule_matcher(
            &user,
            &created.rule_set.id,
            LifecycleMatcherDraft {
                rego_source: crate::lib_tests::legacy_source(
                    NEVER_MATCHER,
                    crate::lifecycle_rules::LifecycleActionKind::DeleteTitleAndFiles,
                ),
                grace_days: 30,
                storage_root_id: None,
                max_titles_per_run: None,
            },
        )
        .await
        .expect("update matcher");

    assert_eq!(updated.rule_set.current_revision_number, 2);
    assert_eq!(updated.revision.revision_number, 2);
    assert_eq!(updated.revision.grace_days, 30);
    assert_ne!(
        updated.revision.matcher_content_hash,
        first.matcher_content_hash
    );

    let stored_first = rules
        .get_revision(&created.rule_set.id, 1)
        .await
        .unwrap()
        .expect("revision 1 survives");
    assert_eq!(stored_first, first, "revisions are immutable");
    assert_eq!(
        rules
            .list_revisions(&created.rule_set.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn matcher_update_preserves_an_omitted_storage_root_and_explicitly_clears_it() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");
    let movie_root_id = scryer_domain::root_folder_id_for_path("/data/movies");
    let mut stored = rules
        .get_revision(&created.rule_set.id, 1)
        .await
        .unwrap()
        .expect("revision 1");
    stored.storage_root_id = Some(movie_root_id.clone());
    rules.replace_revision_in_place(stored).await;

    let preserved = app
        .update_lifecycle_rule_matcher(
            &user,
            &created.rule_set.id,
            LifecycleMatcherDraft {
                rego_source: crate::lib_tests::legacy_source(
                    NEVER_MATCHER,
                    crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                ),
                grace_days: 7,
                storage_root_id: None,
                max_titles_per_run: None,
            },
        )
        .await
        .expect("legacy update preserves the selected root");
    assert_eq!(
        preserved.revision.storage_root_id.as_deref(),
        Some(movie_root_id.as_str())
    );

    let cleared = app
        .update_lifecycle_rule_matcher(
            &user,
            &created.rule_set.id,
            LifecycleMatcherDraft {
                rego_source: crate::lib_tests::legacy_source(
                    NEVER_MATCHER,
                    crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                ),
                grace_days: 7,
                storage_root_id: Some(None),
                max_titles_per_run: None,
            },
        )
        .await
        .expect("explicit null clears the selected root");
    assert_eq!(cleared.revision.storage_root_id, None);
}

#[tokio::test]
async fn metadata_edits_do_not_create_a_revision() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");

    let updated = app
        .update_lifecycle_rule_metadata(
            &user,
            &created.rule_set.id,
            "Renamed".to_string(),
            "New description".to_string(),
            vec!["library-a".to_string()],
        )
        .await
        .expect("update metadata");

    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.library_ids, vec!["library-a".to_string()]);
    assert_eq!(updated.current_revision_number, 1);
    assert_eq!(
        rules
            .list_revisions(&created.rule_set.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Scope is blast radius, so re-scoping invalidates the acknowledgement that
/// armed the rule, exactly as appending a revision does.
#[tokio::test]
async fn changing_the_library_scope_disarms_the_rule() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");
    super::lifecycle_rules::arm_rule(
        &app,
        &user,
        &created.rule_set.id,
        LifecycleEffectArming::Reversible,
        None,
    )
    .await;

    let updated = app
        .update_lifecycle_rule_metadata(
            &user,
            &created.rule_set.id,
            created.rule_set.name.clone(),
            created.rule_set.description.clone(),
            vec!["library-a".to_string()],
        )
        .await
        .expect("re-scope the rule");

    assert_eq!(
        updated.effect_arming,
        LifecycleEffectArming::None,
        "a scope change must disarm the rule it re-scoped"
    );
    // The returned rule must not claim an arming the store no longer holds.
    let stored = rules
        .get_rule_set(&created.rule_set.id)
        .await
        .unwrap()
        .expect("rule set exists");
    assert_eq!(stored.effect_arming, LifecycleEffectArming::None);
    assert_eq!(stored.library_ids, vec!["library-a".to_string()]);
}

/// A rename does not move the matcher, the action, or the scope, so the
/// acknowledgement still describes the same blast radius. Neither does
/// re-saving the same libraries in a different order.
#[tokio::test]
async fn renaming_or_reordering_the_same_scope_leaves_arming_alone() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();
    let created = app
        .create_lifecycle_rule_set(
            &user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                library_ids: vec!["library-a".to_string(), "library-b".to_string()],
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect("create rule set");
    super::lifecycle_rules::arm_rule(
        &app,
        &user,
        &created.rule_set.id,
        LifecycleEffectArming::Reversible,
        None,
    )
    .await;

    let updated = app
        .update_lifecycle_rule_metadata(
            &user,
            &created.rule_set.id,
            "Renamed".to_string(),
            "New description".to_string(),
            vec!["library-b".to_string(), "library-a".to_string()],
        )
        .await
        .expect("rename the rule");

    assert_eq!(updated.name, "Renamed");
    assert_eq!(
        updated.effect_arming,
        LifecycleEffectArming::Reversible,
        "a rename is not a scope change and must not disarm"
    );
    let stored = rules
        .get_rule_set(&created.rule_set.id)
        .await
        .unwrap()
        .expect("rule set exists");
    assert_eq!(stored.effect_arming, LifecycleEffectArming::Reversible);
}

/// The action runs are the only record that the rule ever deleted anything, and
/// the rule set's tables cascade, so deleting it would erase them.
#[tokio::test]
async fn deleting_a_rule_that_has_executed_is_refused() {
    let LifecycleFixture {
        app,
        user,
        rules,
        evaluation,
        ..
    } = lifecycle_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");
    evaluation
        .start_action_run(&action_run(&created.rule_set.id, "title-1"))
        .await
        .expect("record an executed action");

    let error = app
        .delete_lifecycle_rule_set(&user, &created.rule_set.id)
        .await
        .expect_err("a rule with an execution history must not be deletable");

    assert!(
        matches!(
            error,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("Set it to Disabled"),
        "the refusal must name the way out: {error}"
    );
    assert_eq!(
        rules.list_rule_sets().await.unwrap().len(),
        1,
        "the refused delete must leave the rule in place"
    );
}

/// Disabling a rule stops it without touching its history, so a disabled rule
/// that once executed is refused for exactly the same reason.
#[tokio::test]
async fn a_disabled_rule_with_a_history_is_still_refused() {
    let LifecycleFixture {
        app,
        user,
        evaluation,
        ..
    } = lifecycle_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");
    evaluation
        .start_action_run(&action_run(&created.rule_set.id, "title-1"))
        .await
        .expect("record an executed action");
    super::lifecycle_rules::set_rule_state(
        &app,
        &user,
        &created.rule_set.id,
        scryer_domain::LifecycleRuleState::Disabled,
        None,
    )
    .await;

    let error = app
        .delete_lifecycle_rule_set(&user, &created.rule_set.id)
        .await
        .expect_err("disabling a rule does not make its history disposable");
    assert!(
        matches!(
            error,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{error:?}"
    );
}

/// Live candidates are membership an operator can currently see; the cascade
/// would drop them without a trace.
#[tokio::test]
async fn deleting_a_rule_with_live_candidates_is_refused() {
    let LifecycleFixture {
        app,
        user,
        rules,
        evaluation,
        ..
    } = lifecycle_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");
    evaluation
        .create_candidate(&active_candidate(&created.rule_set.id, "title-1"))
        .await
        .expect("open a candidate");

    let error = app
        .delete_lifecycle_rule_set(&user, &created.rule_set.id)
        .await
        .expect_err("a rule still tracking subjects must not be deletable");

    assert!(
        matches!(
            error,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{error:?}"
    );
    assert!(error.to_string().contains("tracking 1"), "{error}");
    assert_eq!(rules.list_rule_sets().await.unwrap().len(), 1);
}

#[tokio::test]
async fn delete_removes_the_rule_set_and_its_revisions() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");

    app.delete_lifecycle_rule_set(&user, &created.rule_set.id)
        .await
        .expect("delete rule set");

    assert!(rules.list_rule_sets().await.unwrap().is_empty());
    assert!(
        rules
            .list_revisions(&created.rule_set.id)
            .await
            .unwrap()
            .is_empty()
    );
}

// ── Rejections ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_matcher_that_does_not_compile_is_rejected() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();

    let error = app
        .create_lifecycle_rule_set(&user, draft("match if { this is not rego"))
        .await
        .expect_err("invalid rego must be rejected");

    assert!(
        matches!(
            error,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{error:?}"
    );
    assert!(rules.list_rule_sets().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_matcher_reading_an_undocumented_fact_is_rejected() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();

    let error = app
        .create_lifecycle_rule_set(
            &user,
            draft("match if {\n\tinput.facts.plex_watch_count > 0\n}\n"),
        )
        .await
        .expect_err("unknown input paths must be rejected");

    assert!(
        matches!(
            error,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{error:?}"
    );
    assert!(rules.list_rule_sets().await.unwrap().is_empty());
}

/// A season or episode rule acts on one subject's monitoring and files only,
/// so a head that needs the whole title is refused at save, pointing at the
/// head, on creation and on a matcher replacement alike.
#[tokio::test]
async fn a_head_the_scope_cannot_run_is_refused_with_its_position() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();
    let source = "package whatever\nimport rego.v1\n\nunmonitor := true\n\nsearch if input.facts.monitored\n";

    let error = app
        .create_lifecycle_rule_set(
            &user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                subject_kind: scryer_domain::LifecycleRuleSubjectKind::Season,
                rego_source: source.to_string(),
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect_err("a season rule cannot search");
    assert_eq!(
        error.validation_reason(),
        Some(crate::error_reasons::lifecycle::ACTION_HEAD_UNSUPPORTED_FOR_SCOPE),
        "{error:?}"
    );
    let AppError::ValidationRefused { diagnostics, .. } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.line, diagnostic.column))
            .collect::<Vec<_>>(),
        vec![(Some(6), Some(1))]
    );
    assert!(rules_are_empty(&app, &user).await);

    let created = app
        .create_lifecycle_rule_set(
            &user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                subject_kind: scryer_domain::LifecycleRuleSubjectKind::Season,
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect("a season rule may unmonitor");
    let refused = app
        .update_lifecycle_rule_matcher(
            &user,
            &created.rule_set.id,
            LifecycleMatcherDraft {
                rego_source: crate::lib_tests::legacy_source(
                    MONITORED_MATCHER,
                    crate::lifecycle_rules::LifecycleActionKind::DeleteTitleAndFiles,
                ),
                grace_days: 7,
                storage_root_id: None,
                max_titles_per_run: None,
            },
        )
        .await
        .expect_err("a revision cannot smuggle in an unrunnable action either");
    assert_eq!(
        refused.validation_reason(),
        Some(crate::error_reasons::lifecycle::ACTION_HEAD_UNSUPPORTED_FOR_SCOPE),
        "{refused:?}"
    );
}

/// A source with no action head could never do anything, so it is refused
/// at save with its own code; `match` is an ordinary helper now.
#[tokio::test]
async fn a_source_without_an_action_head_is_refused() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();

    let error = app
        .create_lifecycle_rule_set(
            &user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                rego_source: MONITORED_MATCHER.to_string(),
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect_err("a rule with only `match` does nothing");
    assert_eq!(
        error.validation_reason(),
        Some(crate::error_reasons::lifecycle::ACTION_HEAD_REQUIRED),
        "{error:?}"
    );
    assert!(rules_are_empty(&app, &user).await);
}

async fn rules_are_empty(app: &AppUseCase, user: &User) -> bool {
    app.list_lifecycle_rule_sets(user)
        .await
        .expect("list rules")
        .is_empty()
}

#[tokio::test]
async fn a_negative_grace_period_is_rejected() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();

    let error = app
        .create_lifecycle_rule_set(
            &user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                grace_days: -1,
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect_err("negative grace must be rejected");

    assert!(
        matches!(
            error,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{error:?}"
    );
}

/// The grace period is materialized into `due_at` as a date addition, and
/// `chrono` panics rather than saturating once that leaves its range, so the
/// bound is a real one and not a style preference.
#[tokio::test]
async fn the_grace_period_is_bounded_at_ten_years() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();

    let accepted = app
        .create_lifecycle_rule_set(
            &user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                grace_days: LIFECYCLE_MAX_GRACE_DAYS,
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect("the boundary itself must be accepted");
    assert_eq!(accepted.revision.grace_days, LIFECYCLE_MAX_GRACE_DAYS);

    let error = app
        .create_lifecycle_rule_set(
            &user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                grace_days: LIFECYCLE_MAX_GRACE_DAYS + 1,
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect_err("one day past the bound must be rejected");
    assert!(
        matches!(
            error,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("ten years"),
        "the refusal should say what the limit is: {error}"
    );

    // A revision cannot smuggle one past the bound either.
    let refused = app
        .update_lifecycle_rule_matcher(
            &user,
            &accepted.rule_set.id,
            LifecycleMatcherDraft {
                rego_source: crate::lib_tests::legacy_source(
                    MONITORED_MATCHER,
                    crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                ),
                grace_days: LIFECYCLE_MAX_GRACE_DAYS + 1,
                storage_root_id: None,
                max_titles_per_run: None,
            },
        )
        .await
        .expect_err("the bound applies to matcher replacements too");
    assert!(
        matches!(
            refused,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_non_privileged_actor_cannot_author_or_preview() {
    let LifecycleFixture { app, .. } = lifecycle_app();
    let mut outsider = User::new_admin("outsider");
    outsider.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::default(),
        loaded: true,
        ..Default::default()
    };

    let create_error = app
        .create_lifecycle_rule_set(&outsider, draft(MONITORED_MATCHER))
        .await
        .expect_err("create must require ManageCatalogSettings");
    assert!(
        matches!(
            create_error,
            AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
        ),
        "{create_error:?}"
    );

    let list_error = app
        .list_lifecycle_rule_sets(&outsider)
        .await
        .expect_err("list must require ManageCatalogSettings");
    assert!(
        matches!(
            list_error,
            AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
        ),
        "{list_error:?}"
    );

    let preview_error = app
        .preview_lifecycle_rule(
            &outsider,
            LifecyclePreviewRequest {
                matcher: LifecyclePreviewMatcher::Inline {
                    library_ids: vec![],
                    subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
                    rego_source: crate::lib_tests::legacy_source(
                        MONITORED_MATCHER,
                        crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                    storage_root_id: None,
                },
                selection: LifecyclePreviewSelection::Titles(vec![]),
            },
        )
        .await
        .expect_err("preview must require ManageCatalogSettings");
    assert!(
        matches!(
            preview_error,
            AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
        ),
        "{preview_error:?}"
    );
}

// ── Preview ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn preview_separates_match_from_no_match() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();
    let monitored = seed_title(&app, &user, "Monitored Movie", true).await;
    let unmonitored = seed_title(&app, &user, "Unmonitored Movie", false).await;
    let created = app
        .create_lifecycle_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");

    let preview = app
        .preview_lifecycle_rule(
            &user,
            LifecyclePreviewRequest {
                matcher: LifecyclePreviewMatcher::Stored {
                    rule_set_id: created.rule_set.id.clone(),
                },
                selection: LifecyclePreviewSelection::Titles(vec![
                    monitored.id.clone(),
                    unmonitored.id.clone(),
                ]),
            },
        )
        .await
        .expect("preview");

    assert_eq!(preview.rule_set_id, created.rule_set.id);
    assert_eq!(
        preview.matcher_content_hash,
        created.revision.matcher_content_hash
    );
    assert_eq!(preview.titles.len(), 2);

    let by_id: HashMap<&str, &crate::lifecycle_rules::LifecyclePreviewTitleResult> = preview
        .titles
        .iter()
        .map(|result| (result.title_id.as_str(), result))
        .collect();
    assert_eq!(
        by_id[monitored.id.as_str()].outcome,
        Some(LifecycleOutcome::Match)
    );
    assert_eq!(
        by_id[unmonitored.id.as_str()].outcome,
        Some(LifecycleOutcome::NoMatch)
    );
    assert!(preview.titles.iter().all(|result| result.error.is_none()));
    assert!(
        preview
            .titles
            .iter()
            .all(|result| result.error_code.is_none())
    );
}

/// A subject the rule fails on reports the failure by code beside its
/// sentence, and is never shown as a decision.
#[tokio::test]
async fn preview_reports_a_failed_subject_by_code() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();
    let unmonitored = seed_title(&app, &user, "Unmonitored Movie", false).await;
    // The save's dry run sees a monitored subject, where this head stays
    // undefined; on an unmonitored title it is a string, not a boolean.
    let created = app
        .create_lifecycle_rule_set(
            &user,
            LifecycleRuleDraft {
                max_titles_per_run: Some(100),
                rego_source: "package rules\nimport rego.v1\n\nunmonitor := \"yes\" if {\n\tnot input.facts.monitored\n}\n"
                    .to_string(),
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect("create rule set");

    let preview = app
        .preview_lifecycle_rule(
            &user,
            LifecyclePreviewRequest {
                matcher: LifecyclePreviewMatcher::Stored {
                    rule_set_id: created.rule_set.id.clone(),
                },
                selection: LifecyclePreviewSelection::Titles(vec![unmonitored.id.clone()]),
            },
        )
        .await
        .expect("preview");

    assert_eq!(preview.titles.len(), 1);
    let result = &preview.titles[0];
    assert_eq!(result.outcome, None, "{result:?}");
    assert!(result.error.is_some(), "{result:?}");
    assert_eq!(
        result.error_code,
        Some(scryer_rules::diagnostic_codes::RULE_OUTPUT_INVALID)
    );
}

#[tokio::test]
async fn preview_holds_when_selected_root_membership_cannot_be_proved() {
    let LifecycleFixture {
        app,
        user,
        media_files,
        ..
    } = lifecycle_app();
    let title = seed_title(&app, &user, "Root coverage unknown", true).await;
    // The catalog deliberately knows this path while the filesystem does not,
    // so a configured root cannot prove physical ownership.
    seed_media_file(&media_files, &title.id, 1).await;

    let preview = app
        .preview_lifecycle_rule(
            &user,
            LifecyclePreviewRequest {
                matcher: LifecyclePreviewMatcher::Inline {
                    library_ids: vec![],
                    subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
                    rego_source: crate::lib_tests::legacy_source(
                        MONITORED_MATCHER,
                        crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                    storage_root_id: Some(scryer_domain::root_folder_id_for_path("/data/movies")),
                },
                selection: LifecyclePreviewSelection::Titles(vec![title.id]),
            },
        )
        .await
        .expect("preview returns an unknown result rather than matching");

    assert_eq!(preview.titles.len(), 1);
    assert_eq!(preview.titles[0].outcome, Some(LifecycleOutcome::Unknown));
    assert_eq!(
        preview.titles[0].reason_codes,
        vec!["storage_root_membership_unavailable".to_string()]
    );
    assert_eq!(preview.titles[0].storage_root_file_count, None);
}

/// The fail-closed property the RFC turns on: a rule that needs a fact Scryer
/// does not collect holds instead of matching — and it holds without the author
/// having written a single guard, which is the whole point of the host deriving
/// unknownness from the facts a rule references.
#[tokio::test]
async fn a_rule_needing_an_uncollected_fact_reports_unknown() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();
    let title = seed_title(&app, &user, "Any Movie", true).await;

    let preview = app
        .preview_lifecycle_rule(
            &user,
            LifecyclePreviewRequest {
                matcher: LifecyclePreviewMatcher::Inline {
                    library_ids: vec![],
                    subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
                    rego_source: crate::lib_tests::legacy_source(
                        NEEDS_UPGRADE_HISTORY_MATCHER,
                        crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                    storage_root_id: None,
                },
                selection: LifecyclePreviewSelection::Titles(vec![title.id.clone()]),
            },
        )
        .await
        .expect("preview");

    assert_eq!(preview.titles.len(), 1);
    assert_eq!(
        preview.titles[0].outcome,
        Some(LifecycleOutcome::Unknown),
        "a fact Scryer cannot observe must hold the subject, with no guard written"
    );
    assert_eq!(
        preview.titles[0].reason_codes,
        vec!["not_yet_collected".to_string()],
        "the reason is the observation's own code, so the operator sees why"
    );
}

#[tokio::test]
async fn an_inline_preview_persists_nothing() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();
    let title = seed_title(&app, &user, "Any Movie", true).await;

    app.preview_lifecycle_rule(
        &user,
        LifecyclePreviewRequest {
            matcher: LifecyclePreviewMatcher::Inline {
                library_ids: vec![],
                subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
                rego_source: crate::lib_tests::legacy_source(
                    MONITORED_MATCHER,
                    crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                ),
                grace_days: 0,
                storage_root_id: None,
            },
            selection: LifecyclePreviewSelection::Titles(vec![title.id]),
        },
    )
    .await
    .expect("preview");

    assert!(rules.list_rule_sets().await.unwrap().is_empty());
}

#[tokio::test]
async fn preview_refuses_a_selection_above_the_cap() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();
    let title_ids: Vec<String> = (0..=crate::lifecycle_rules::LIFECYCLE_PREVIEW_MAX_TITLES)
        .map(|index| format!("title-{index}"))
        .collect();

    let error = app
        .preview_lifecycle_rule(
            &user,
            LifecyclePreviewRequest {
                matcher: LifecyclePreviewMatcher::Inline {
                    library_ids: vec![],
                    subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
                    rego_source: crate::lib_tests::legacy_source(
                        MONITORED_MATCHER,
                        crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                    storage_root_id: None,
                },
                selection: LifecyclePreviewSelection::Titles(title_ids),
            },
        )
        .await
        .expect_err("over-cap selections must be rejected");

    assert!(
        matches!(
            error,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{error:?}"
    );
}

// ── Stateless validation ────────────────────────────────────────────────────

#[tokio::test]
async fn validate_reports_errors_without_storing_anything() {
    let LifecycleFixture {
        app, user, rules, ..
    } = lifecycle_app();

    let valid = app
        .validate_lifecycle_rule_source(
            &user,
            &crate::lib_tests::legacy_source(
                MONITORED_MATCHER,
                crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
            ),
        )
        .await
        .expect("validate");
    assert!(valid.result.valid, "{:?}", valid.result.errors);
    assert!(valid.declared_actions.is_some());

    // A matcher with no action head compiles but can never decide anything.
    let invalid = app
        .validate_lifecycle_rule_source(&user, "reasons := [\"stale\"]\n")
        .await
        .expect("validate");
    assert!(!invalid.result.valid);
    assert_eq!(
        invalid
            .result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>(),
        vec![scryer_rules::diagnostic_codes::RULE_ACTION_REQUIRED]
    );

    assert!(rules.list_rule_sets().await.unwrap().is_empty());
}

/// File facts come from one batched load, and an empty file set is a confirmed
/// answer rather than an unknown.
#[tokio::test]
async fn file_facts_distinguish_having_files_from_having_none() {
    let LifecycleFixture {
        app,
        user,
        media_files,
        ..
    } = lifecycle_app();
    let with_file = seed_title(&app, &user, "Has A File", true).await;
    let without_file = seed_title(&app, &user, "No Files", true).await;
    seed_media_file(&media_files, &with_file.id, 4_000_000_000).await;

    let matcher = "package whatever\n\
         import rego.v1\n\n\
         match if {\n\
         \tinput.facts.has_file\n\
         \tinput.facts.file_count == 1\n\
         \tinput.facts.total_file_size_bytes == 4000000000\n\
         \tinput.facts.first_imported_at != \"\"\n\
         }\n\n\
         match if {\n\
         \tinput.facts.has_file == false\n\
         \tinput.facts.file_count == 0\n\
         \tnot input.facts.first_imported_at\n\
         }\n";

    let preview = app
        .preview_lifecycle_rule(
            &user,
            LifecyclePreviewRequest {
                matcher: LifecyclePreviewMatcher::Inline {
                    library_ids: vec![],
                    subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
                    rego_source: crate::lib_tests::legacy_source(
                        matcher,
                        crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                    storage_root_id: None,
                },
                selection: LifecyclePreviewSelection::Titles(vec![
                    with_file.id.clone(),
                    without_file.id.clone(),
                ]),
            },
        )
        .await
        .expect("preview");

    assert!(
        preview
            .titles
            .iter()
            .all(|result| result.outcome == Some(LifecycleOutcome::Match)),
        "{:?}",
        preview.titles
    );
}

/// Movies have no episodes; that is confirmed absence, not a gap Scryer failed
/// to fill, so a rule testing for it decides rather than holding.
#[tokio::test]
async fn episode_counts_are_absent_for_movies() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();
    let movie = seed_title(&app, &user, "A Movie", true).await;

    let preview = app
        .preview_lifecycle_rule(
            &user,
            LifecyclePreviewRequest {
                matcher: LifecyclePreviewMatcher::Inline {
                    library_ids: vec![],
                    subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
                    rego_source: "package whatever\n\
                         import rego.v1\n\n\
                         unmonitor if {\n\
                         \tnot input.facts.episode_count\n\
                         }\n"
                    .to_string(),
                    grace_days: 0,
                    storage_root_id: None,
                },
                selection: LifecyclePreviewSelection::Titles(vec![movie.id]),
            },
        )
        .await
        .expect("preview");

    assert_eq!(preview.titles[0].outcome, Some(LifecycleOutcome::Match));
}

/// A show subject carries its series movies, tags and all; a movie subject does
/// not carry the key at all. Both halves matter: the first is what a rule reads,
/// and the second is what stops `not input.facts.series_movies` from holding
/// every movie in the library.
#[tokio::test]
async fn series_movie_facts_are_present_for_shows_and_absent_for_movies() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();
    let movie = seed_title(&app, &user, "A Movie", true).await;
    let series = app
        .add_title(
            &user,
            NewTitle {
                name: "A Series".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                ..Default::default()
            },
        )
        .await
        .expect("create series");
    app.create_title_tag_definition(&user, "keep", None)
        .await
        .expect("define tag");
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &series.id,
            "A Series Movie",
            Some(2024),
            None,
            None,
        ))
        .await
        .expect("link a series movie");
    app.update_series_movie_tags(
        &user,
        std::slice::from_ref(&link.id),
        &["keep".to_string()],
        &[],
    )
    .await
    .expect("tag the series movie");

    let matcher = "package whatever\n\
         import rego.v1\n\n\
         match if {\n\
         \tsome series_movie in input.facts.series_movies\n\
         \tseries_movie.name == \"A Series Movie\"\n\
         \tseries_movie.year == 2024\n\
         \tseries_movie.monitored\n\
         \tseries_movie.has_file == false\n\
         \t\"keep\" in series_movie.tags\n\
         }\n\n\
         match if {\n\
         \tnot input.facts.series_movies\n\
         }\n";

    let preview = app
        .preview_lifecycle_rule(
            &user,
            LifecyclePreviewRequest {
                matcher: LifecyclePreviewMatcher::Inline {
                    library_ids: vec![],
                    subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
                    rego_source: crate::lib_tests::legacy_source(
                        matcher,
                        crate::lifecycle_rules::LifecycleActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                    storage_root_id: None,
                },
                selection: LifecyclePreviewSelection::Titles(vec![
                    series.id.clone(),
                    movie.id.clone(),
                ]),
            },
        )
        .await
        .expect("preview");

    // The show matches through its movie, the movie through the absent key.
    for result in &preview.titles {
        assert_eq!(
            result.outcome,
            Some(LifecycleOutcome::Match),
            "{:?}",
            preview.titles
        );
    }
}

/// A move names a library or root by what the operator sees. Save refuses a
/// name that is not exactly one library, a library of another kind than the
/// rule's libraries, and a path that is not a configured root, each pointing
/// at the value.
#[tokio::test]
async fn a_move_target_that_does_not_exist_is_refused_with_its_position() {
    let LifecycleFixture { app, user, .. } = lifecycle_app();
    let libraries = app.services.catalog.libraries.list(None).await.unwrap();
    let movies = libraries
        .iter()
        .find(|library| library.facet == MediaFacet::Movie)
        .expect("a movie library");
    let series = libraries
        .iter()
        .find(|library| library.facet != MediaFacet::Movie)
        .expect("a library of another kind");
    let movie_root = movies
        .roots
        .first()
        .map(|root| root.path.clone())
        .expect("the movie library has a root");

    let refused = |source: String, library_ids: Vec<String>| {
        let app = &app;
        let user = &user;
        async move {
            app.create_lifecycle_rule_set(
                user,
                LifecycleRuleDraft {
                    max_titles_per_run: Some(100),
                    rego_source: source,
                    library_ids,
                    ..draft(MONITORED_MATCHER)
                },
            )
            .await
        }
    };
    let source = |head: &str, value: &str| {
        format!(
            "package whatever\nimport rego.v1\n\n{head} := {} if input.facts.monitored\n",
            serde_json::to_string(value).unwrap()
        )
    };
    let position = |error: &AppError| {
        let AppError::ValidationRefused { diagnostics, .. } = error else {
            panic!("{error:?}");
        };
        diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.line, diagnostic.column))
            .collect::<Vec<_>>()
    };

    for (head, value, library_ids, code) in [
        (
            "move_to_library",
            "Synthetic Missing Library",
            Vec::new(),
            crate::error_reasons::lifecycle::MOVE_LIBRARY_NOT_FOUND,
        ),
        (
            "move_to_library",
            series.name.as_str(),
            vec![movies.id.clone()],
            crate::error_reasons::lifecycle::MOVE_LIBRARY_FACET_MISMATCH,
        ),
        (
            "move_to_root",
            "/synthetic/not/a/root",
            Vec::new(),
            crate::error_reasons::lifecycle::MOVE_ROOT_NOT_FOUND,
        ),
    ] {
        let error = refused(source(head, value), library_ids)
            .await
            .expect_err("a move target that does not exist is refused");
        assert_eq!(error.validation_reason(), Some(code), "{error:?}");
        // The position is the value's, after `<head> := `.
        let column = head.len() + 5;
        assert_eq!(
            position(&error),
            vec![(Some(4), Some(column))],
            "{head} {value}"
        );
    }
    assert!(rules_are_empty(&app, &user).await);

    refused(
        source("move_to_library", movies.name.as_str()),
        vec![movies.id.clone()],
    )
    .await
    .expect("a library of the rule's kind is accepted");
    refused(
        source("move_to_root", &format!("{movie_root}/")),
        Vec::new(),
    )
    .await
    .expect("a configured root is accepted, trailing separator or not");

    // A scope change alone is checked against the same targets: a rule that
    // moves to the movie library cannot be confined to another kind.
    let unscoped = refused(source("move_to_library", movies.name.as_str()), Vec::new())
        .await
        .expect("an unscoped rule may name any library");
    let rule_set_id = unscoped.rule_set.id.clone();
    let error = app
        .update_lifecycle_rule_metadata(
            &user,
            &rule_set_id,
            unscoped.rule_set.name.clone(),
            String::new(),
            vec![series.id.clone()],
        )
        .await
        .expect_err("the new scope holds another kind of media");
    assert_eq!(
        error.validation_reason(),
        Some(crate::error_reasons::lifecycle::MOVE_LIBRARY_FACET_MISMATCH),
        "{error:?}"
    );
    assert_eq!(position(&error), vec![(Some(4), Some(20))]);
    let kept = app
        .services
        .customization
        .lifecycle_rule_sets
        .get_rule_set(&rule_set_id)
        .await
        .unwrap()
        .expect("rule set");
    assert!(
        kept.library_ids.is_empty(),
        "a refused scope is not written"
    );
    app.update_lifecycle_rule_metadata(
        &user,
        &rule_set_id,
        unscoped.rule_set.name.clone(),
        String::new(),
        vec![movies.id.clone()],
    )
    .await
    .expect("a scope of the target's kind is accepted");
}
