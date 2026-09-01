use super::*;

use crate::maintenance_rules::{
    MaintenanceActionKind, MaintenanceActionSpec, MaintenanceMatcherDraft,
    MaintenancePreviewMatcher, MaintenancePreviewRequest, MaintenancePreviewSelection,
    MaintenanceRuleDraft,
};
use scryer_domain::{
    MaintenanceEvaluationMode, MaintenanceRuleRevision, MaintenanceRuleSet,
    MaintenanceRuleSubjectKind,
};
use scryer_rules::maintenance::MaintenanceOutcome;

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

/// What a test wants [`InMemoryMaintenanceRuleRepo::get_rule_set`] to answer
/// instead of reading storage.
///
/// The executor's safety recheck is the only caller of that read during a
/// handler pass, and it has to tell two failures apart: a rule that is genuinely
/// gone (cancel the candidate — its authorization no longer exists) from a store
/// it merely could not reach (hold, like every other unresolvable signal).
/// Deleting the rule for real cannot exercise this, because a deleted rule is
/// not in the pass's rule listing at all.
#[derive(Clone, Copy, Debug)]
pub(super) enum MaintenanceRuleReadFault {
    /// The rule set is gone.
    Missing,
    /// The store could not answer.
    Unreachable,
}

#[derive(Default)]
pub(super) struct InMemoryMaintenanceRuleRepo {
    rule_sets: Mutex<Vec<MaintenanceRuleSet>>,
    revisions: Mutex<Vec<MaintenanceRuleRevision>>,
    read_fault: Mutex<Option<MaintenanceRuleReadFault>>,
}

impl InMemoryMaintenanceRuleRepo {
    /// Make every subsequent single-rule read fail this way.
    pub(super) async fn fail_rule_set_reads(&self, fault: MaintenanceRuleReadFault) {
        *self.read_fault.lock().await = Some(fault);
    }
}

impl InMemoryMaintenanceRuleRepo {
    /// Rewrite the revision currently in force without appending a new one.
    ///
    /// Production never does this — revisions are immutable — but a test that
    /// wants to change what a rule *decides* without also triggering the
    /// supersede path has no other way to isolate the two behaviours.
    pub(super) async fn replace_revision_in_place(&self, revision: MaintenanceRuleRevision) {
        let mut revisions = self.revisions.lock().await;
        revisions.retain(|stored| {
            !(stored.rule_set_id == revision.rule_set_id
                && stored.revision_number == revision.revision_number)
        });
        revisions.push(revision);
    }
}

#[async_trait]
impl MaintenanceRuleSetRepository for InMemoryMaintenanceRuleRepo {
    async fn list_rule_sets(&self) -> AppResult<Vec<MaintenanceRuleSet>> {
        Ok(self.rule_sets.lock().await.clone())
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<MaintenanceRuleSet>> {
        match *self.read_fault.lock().await {
            Some(MaintenanceRuleReadFault::Missing) => return Ok(None),
            Some(MaintenanceRuleReadFault::Unreachable) => {
                return Err(AppError::Repository(
                    "maintenance rule store is unreachable".to_string(),
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
        rule_set: &MaintenanceRuleSet,
        revision: &MaintenanceRuleRevision,
    ) -> AppResult<()> {
        self.rule_sets.lock().await.push(rule_set.clone());
        self.revisions.lock().await.push(revision.clone());
        Ok(())
    }

    async fn add_revision(
        &self,
        revision: &MaintenanceRuleRevision,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == revision.rule_set_id)
            .ok_or_else(|| AppError::NotFound(revision.rule_set_id.clone()))?;
        rule_set.current_revision_number = revision.revision_number;
        // The SQL store disarms in the same transaction as the pointer move; a
        // permissive double here would hide an arming that survived a matcher
        // swap.
        rule_set.effect_arming = scryer_domain::MaintenanceEffectArming::None;
        rule_set.updated_at = updated_at;
        self.revisions.lock().await.push(revision.clone());
        Ok(())
    }

    async fn get_revision(
        &self,
        rule_set_id: &str,
        revision_number: i64,
    ) -> AppResult<Option<MaintenanceRuleRevision>> {
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

    async fn list_revisions(&self, rule_set_id: &str) -> AppResult<Vec<MaintenanceRuleRevision>> {
        let mut revisions: Vec<MaintenanceRuleRevision> = self
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

    async fn update_rule_set_evaluation_mode(
        &self,
        id: &str,
        mode: MaintenanceEvaluationMode,
        enabled: bool,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        rule_set.evaluation_mode = mode;
        rule_set.enabled = enabled;
        rule_set.updated_at = updated_at;
        Ok(())
    }

    async fn update_rule_set_arming(
        &self,
        id: &str,
        arming: scryer_domain::MaintenanceEffectArming,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        rule_set.effect_arming = arming;
        rule_set.updated_at = updated_at;
        Ok(())
    }
}

struct MaintenanceFixture {
    app: AppUseCase,
    user: User,
    rules: Arc<InMemoryMaintenanceRuleRepo>,
    media_files: Arc<MockMediaFileRepo>,
}

fn maintenance_app() -> MaintenanceFixture {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryMaintenanceRuleRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_maintenance_rule_set_store(rules.clone())
            .with_media_files(media_files.clone())
    });
    MaintenanceFixture {
        app,
        user,
        rules,
        media_files,
    }
}

fn draft(rego_source: &str) -> MaintenanceRuleDraft {
    MaintenanceRuleDraft {
        name: "Stale movies".to_string(),
        description: "Unwatched for a long time".to_string(),
        rego_source: rego_source.to_string(),
        action_spec: MaintenanceActionSpec::new(MaintenanceActionKind::UnmonitorScopeKeepFiles),
        grace_days: 7,
        library_ids: Vec::new(),
        evaluation_mode: None,
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

// ── Authoring ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_persists_a_dormant_rule_set_and_its_first_revision() {
    let MaintenanceFixture {
        app, user, rules, ..
    } = maintenance_app();

    let detail = app
        .create_maintenance_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");

    assert!(!detail.rule_set.enabled, "rule sets ship dark");
    assert_eq!(
        detail.rule_set.evaluation_mode,
        MaintenanceEvaluationMode::Disabled
    );
    assert_eq!(
        detail.rule_set.subject_kind,
        MaintenanceRuleSubjectKind::Title
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
async fn update_matcher_appends_a_revision_and_leaves_the_previous_one_untouched() {
    let MaintenanceFixture {
        app, user, rules, ..
    } = maintenance_app();
    let created = app
        .create_maintenance_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");
    let first = rules
        .get_revision(&created.rule_set.id, 1)
        .await
        .unwrap()
        .expect("revision 1");

    let updated = app
        .update_maintenance_rule_matcher(
            &user,
            &created.rule_set.id,
            MaintenanceMatcherDraft {
                rego_source: NEVER_MATCHER.to_string(),
                action_spec: MaintenanceActionSpec::new(MaintenanceActionKind::DeleteTitleAndFiles),
                grace_days: 30,
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
async fn metadata_edits_do_not_create_a_revision() {
    let MaintenanceFixture {
        app, user, rules, ..
    } = maintenance_app();
    let created = app
        .create_maintenance_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");

    let updated = app
        .update_maintenance_rule_metadata(
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

#[tokio::test]
async fn delete_removes_the_rule_set_and_its_revisions() {
    let MaintenanceFixture {
        app, user, rules, ..
    } = maintenance_app();
    let created = app
        .create_maintenance_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");

    app.delete_maintenance_rule_set(&user, &created.rule_set.id)
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
    let MaintenanceFixture {
        app, user, rules, ..
    } = maintenance_app();

    let error = app
        .create_maintenance_rule_set(&user, draft("match if { this is not rego"))
        .await
        .expect_err("invalid rego must be rejected");

    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    assert!(rules.list_rule_sets().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_matcher_reading_an_undocumented_fact_is_rejected() {
    let MaintenanceFixture {
        app, user, rules, ..
    } = maintenance_app();

    let error = app
        .create_maintenance_rule_set(
            &user,
            draft("match if {\n\tinput.facts.plex_watch_count > 0\n}\n"),
        )
        .await
        .expect_err("unknown input paths must be rejected");

    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    assert!(rules.list_rule_sets().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_action_that_supports_neither_movies_nor_shows_is_rejected() {
    let MaintenanceFixture { app, user, .. } = maintenance_app();

    let error = app
        .create_maintenance_rule_set(
            &user,
            MaintenanceRuleDraft {
                // Season/episode only, so it can never run on a title-scoped rule.
                action_spec: MaintenanceActionSpec::new(
                    MaintenanceActionKind::UnmonitorScopeDeleteFiles,
                ),
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect_err("subject mismatch must be rejected");

    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    assert!(
        error.to_string().contains("title-scoped"),
        "the message should name the scope: {error}"
    );
}

#[tokio::test]
async fn an_action_the_title_executor_cannot_run_is_rejected_at_authoring_time() {
    let MaintenanceFixture { app, user, .. } = maintenance_app();

    // The show-subject delete action passes the descriptor's subject check — a
    // title-scoped rule covers shows — so it used to save cleanly and then hard
    // fail at execution, three attempts into a terminal `Failed` candidate that
    // nothing could rescue. Authoring now refuses it up front.
    let error = app
        .create_maintenance_rule_set(
            &user,
            MaintenanceRuleDraft {
                action_spec: MaintenanceActionSpec::new(
                    MaintenanceActionKind::UnmonitorShowDeleteExistingFiles,
                ),
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect_err("an action the executor refuses must not be savable");

    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("cannot run for a title-scoped rule"),
        "{error}"
    );

    // The same gate applies to a matcher replacement, not just to creation.
    let created = app
        .create_maintenance_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create a rule with a runnable action");
    let refused = app
        .update_maintenance_rule_matcher(
            &user,
            &created.rule_set.id,
            MaintenanceMatcherDraft {
                rego_source: MONITORED_MATCHER.to_string(),
                action_spec: MaintenanceActionSpec::new(
                    MaintenanceActionKind::UnmonitorShowDeleteExistingFiles,
                ),
                grace_days: 7,
            },
        )
        .await
        .expect_err("a revision cannot smuggle in an unrunnable action either");
    assert!(matches!(refused, AppError::Validation(_)), "{refused:?}");
}

#[tokio::test]
async fn a_negative_grace_period_is_rejected() {
    let MaintenanceFixture { app, user, .. } = maintenance_app();

    let error = app
        .create_maintenance_rule_set(
            &user,
            MaintenanceRuleDraft {
                grace_days: -1,
                ..draft(MONITORED_MATCHER)
            },
        )
        .await
        .expect_err("negative grace must be rejected");

    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
}

#[tokio::test]
async fn shadow_and_observe_modes_are_not_yet_available() {
    let MaintenanceFixture { app, user, .. } = maintenance_app();

    for mode in [
        MaintenanceEvaluationMode::Shadow,
        MaintenanceEvaluationMode::Observe,
    ] {
        let error = app
            .create_maintenance_rule_set(
                &user,
                MaintenanceRuleDraft {
                    evaluation_mode: Some(mode),
                    ..draft(MONITORED_MATCHER)
                },
            )
            .await
            .err()
            .unwrap_or_else(|| panic!("{mode:?} must be rejected while nothing evaluates rules"));
        assert!(matches!(error, AppError::Validation(_)), "{error:?}");
        assert!(error.to_string().contains("not yet available"), "{error}");
    }
}

#[tokio::test]
async fn a_non_privileged_actor_cannot_author_or_preview() {
    let MaintenanceFixture { app, .. } = maintenance_app();
    let mut outsider = User::new_admin("outsider");
    outsider.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::default(),
        loaded: true,
        ..Default::default()
    };

    let create_error = app
        .create_maintenance_rule_set(&outsider, draft(MONITORED_MATCHER))
        .await
        .expect_err("create must require ManageCatalogSettings");
    assert!(
        matches!(create_error, AppError::Unauthorized(_)),
        "{create_error:?}"
    );

    let list_error = app
        .list_maintenance_rule_sets(&outsider)
        .await
        .expect_err("list must require ManageCatalogSettings");
    assert!(
        matches!(list_error, AppError::Unauthorized(_)),
        "{list_error:?}"
    );

    let preview_error = app
        .preview_maintenance_rule(
            &outsider,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Inline {
                    rego_source: MONITORED_MATCHER.to_string(),
                    action_spec: MaintenanceActionSpec::new(
                        MaintenanceActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                },
                selection: MaintenancePreviewSelection::Titles(vec![]),
            },
        )
        .await
        .expect_err("preview must require ManageCatalogSettings");
    assert!(
        matches!(preview_error, AppError::Unauthorized(_)),
        "{preview_error:?}"
    );
}

// ── Preview ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn preview_separates_match_from_no_match() {
    let MaintenanceFixture { app, user, .. } = maintenance_app();
    let monitored = seed_title(&app, &user, "Monitored Movie", true).await;
    let unmonitored = seed_title(&app, &user, "Unmonitored Movie", false).await;
    let created = app
        .create_maintenance_rule_set(&user, draft(MONITORED_MATCHER))
        .await
        .expect("create rule set");

    let preview = app
        .preview_maintenance_rule(
            &user,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Stored {
                    rule_set_id: created.rule_set.id.clone(),
                },
                selection: MaintenancePreviewSelection::Titles(vec![
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

    let by_id: HashMap<&str, &crate::maintenance_rules::MaintenancePreviewTitleResult> = preview
        .titles
        .iter()
        .map(|result| (result.title_id.as_str(), result))
        .collect();
    assert_eq!(
        by_id[monitored.id.as_str()].outcome,
        Some(MaintenanceOutcome::Match)
    );
    assert_eq!(
        by_id[unmonitored.id.as_str()].outcome,
        Some(MaintenanceOutcome::NoMatch)
    );
    assert!(preview.titles.iter().all(|result| result.error.is_none()));
}

/// The fail-closed property the RFC turns on: a rule that needs a fact Scryer
/// does not collect holds instead of matching — and it holds without the author
/// having written a single guard, which is the whole point of the host deriving
/// unknownness from the facts a rule references.
#[tokio::test]
async fn a_rule_needing_an_uncollected_fact_reports_unknown() {
    let MaintenanceFixture { app, user, .. } = maintenance_app();
    let title = seed_title(&app, &user, "Any Movie", true).await;

    let preview = app
        .preview_maintenance_rule(
            &user,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Inline {
                    rego_source: NEEDS_UPGRADE_HISTORY_MATCHER.to_string(),
                    action_spec: MaintenanceActionSpec::new(
                        MaintenanceActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                },
                selection: MaintenancePreviewSelection::Titles(vec![title.id.clone()]),
            },
        )
        .await
        .expect("preview");

    assert_eq!(preview.titles.len(), 1);
    assert_eq!(
        preview.titles[0].outcome,
        Some(MaintenanceOutcome::Unknown),
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
    let MaintenanceFixture {
        app, user, rules, ..
    } = maintenance_app();
    let title = seed_title(&app, &user, "Any Movie", true).await;

    app.preview_maintenance_rule(
        &user,
        MaintenancePreviewRequest {
            matcher: MaintenancePreviewMatcher::Inline {
                rego_source: MONITORED_MATCHER.to_string(),
                action_spec: MaintenanceActionSpec::new(
                    MaintenanceActionKind::UnmonitorScopeKeepFiles,
                ),
                grace_days: 0,
            },
            selection: MaintenancePreviewSelection::Titles(vec![title.id]),
        },
    )
    .await
    .expect("preview");

    assert!(rules.list_rule_sets().await.unwrap().is_empty());
}

#[tokio::test]
async fn preview_refuses_a_selection_above_the_cap() {
    let MaintenanceFixture { app, user, .. } = maintenance_app();
    let title_ids: Vec<String> = (0..=crate::maintenance_rules::MAINTENANCE_PREVIEW_MAX_TITLES)
        .map(|index| format!("title-{index}"))
        .collect();

    let error = app
        .preview_maintenance_rule(
            &user,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Inline {
                    rego_source: MONITORED_MATCHER.to_string(),
                    action_spec: MaintenanceActionSpec::new(
                        MaintenanceActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                },
                selection: MaintenancePreviewSelection::Titles(title_ids),
            },
        )
        .await
        .expect_err("over-cap selections must be rejected");

    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
}

// ── Stateless validation ────────────────────────────────────────────────────

#[tokio::test]
async fn validate_reports_errors_without_storing_anything() {
    let MaintenanceFixture {
        app, user, rules, ..
    } = maintenance_app();

    let valid = app
        .validate_maintenance_rule_source(&user, MONITORED_MATCHER)
        .await
        .expect("validate");
    assert!(valid.valid, "{:?}", valid.errors);

    // A matcher with no `match` rule compiles but can never decide anything.
    let invalid = app
        .validate_maintenance_rule_source(&user, "reasons := [\"stale\"]\n")
        .await
        .expect("validate");
    assert!(!invalid.valid);
    assert!(!invalid.errors.is_empty());

    assert!(rules.list_rule_sets().await.unwrap().is_empty());
}

/// File facts come from one batched load, and an empty file set is a confirmed
/// answer rather than an unknown.
#[tokio::test]
async fn file_facts_distinguish_having_files_from_having_none() {
    let MaintenanceFixture {
        app,
        user,
        media_files,
        ..
    } = maintenance_app();
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
        .preview_maintenance_rule(
            &user,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Inline {
                    rego_source: matcher.to_string(),
                    action_spec: MaintenanceActionSpec::new(
                        MaintenanceActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                },
                selection: MaintenancePreviewSelection::Titles(vec![
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
            .all(|result| result.outcome == Some(MaintenanceOutcome::Match)),
        "{:?}",
        preview.titles
    );
}

/// Movies have no episodes; that is confirmed absence, not a gap Scryer failed
/// to fill, so a rule testing for it decides rather than holding.
#[tokio::test]
async fn episode_counts_are_absent_for_movies() {
    let MaintenanceFixture { app, user, .. } = maintenance_app();
    let movie = seed_title(&app, &user, "A Movie", true).await;

    let preview = app
        .preview_maintenance_rule(
            &user,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Inline {
                    rego_source: "package whatever\n\
                         import rego.v1\n\n\
                         match if {\n\
                         \tnot input.facts.episode_count\n\
                         }\n"
                    .to_string(),
                    action_spec: MaintenanceActionSpec::new(
                        MaintenanceActionKind::UnmonitorScopeKeepFiles,
                    ),
                    grace_days: 0,
                },
                selection: MaintenancePreviewSelection::Titles(vec![movie.id]),
            },
        )
        .await
        .expect("preview");

    assert_eq!(preview.titles[0].outcome, Some(MaintenanceOutcome::Match));
}
