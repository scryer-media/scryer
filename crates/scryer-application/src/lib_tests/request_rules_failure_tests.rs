use super::*;
use crate::lib_tests::request_rules_support::InMemoryRequestRuleRepo;

#[tokio::test]
async fn unreadable_request_rule_tag_registry_holds_the_persisted_request() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve with tags", APPROVE_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;
    harness.titles.fail_tag_reads.store(true, Ordering::SeqCst);
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9047, Some(30)).await;
    let rows = harness.media_requests.requests.lock().await;
    let request = rows
        .iter()
        .find(|request| request.id == request_id)
        .unwrap();
    assert_eq!(request.status, MediaRequestStatus::Pending);
    assert!(request.policy_tags.is_empty());
    assert!(harness.titles.store.lock().await.is_empty());
    let traces = harness.request_rule_decisions.recorded().await;
    let trace = traces.last().unwrap();
    assert_eq!(trace.request_id, request_id);
    assert_eq!(trace.fallback_reason.as_deref(), Some(FALLBACK_ERROR));
    assert!(trace.tags.iter().any(|tag| tag == "auto-approved"));
}

#[tokio::test]
async fn request_rule_timeout_holds_an_armed_request_and_records_the_error() {
    use scryer_rules::request::{RequestPolicy, RequestRulesEngine, rewrite_package_declaration};
    let harness = bootstrap_media_request_app();
    let source = "package rules\nimport rego.v1\napprove if { count([1 | some i in numbers.range(1, 3000); some j in numbers.range(1, 3000); i == j]) > 0 }";
    // Create an accepted rule, then inject an expensive cached engine to
    // exercise submission-time limits independently of authoring validation.
    let detail = create_rule(&harness, "Bounded evaluation", APPROVE_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;
    let mut limits = scryer_rules::runtime::RuntimeLimits::request_defaults();
    limits.max_execution_time = std::time::Duration::from_millis(1);
    limits.timer_check_interval = std::num::NonZeroU32::new(1).unwrap();
    let engine = RequestRulesEngine::build_with_limits(
        &[RequestPolicy {
            id: detail.rule_set.id.clone(),
            name: "Bounded evaluation".into(),
            rego_source: rewrite_package_declaration(source, &detail.rule_set.id),
        }],
        limits,
    )
    .unwrap();
    harness
        .app
        .services
        .customization
        .request_rules_engine
        .write()
        .unwrap()
        .engine = engine;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9047, None).await;
    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Pending
    );
    let traces = harness.request_rule_decisions.recorded().await;
    let trace = traces.last().unwrap();
    assert_eq!(trace.mode, RequestRuleEvaluationMode::Enforce);
    assert_eq!(trace.fallback_reason.as_deref(), Some(FALLBACK_ERROR));
    assert!(trace.votes_json.contains("error"), "{}", trace.votes_json);
    assert!(harness.titles.store.lock().await.is_empty());
}

struct UnreadableRuleGate(Arc<dyn SettingsRepository>);

#[async_trait::async_trait]
impl SettingsRepository for UnreadableRuleGate {
    async fn get_setting_json(
        &self,
        scope: &str,
        key: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        if key == crate::settings::keys::REQUEST_RULE_GATE_EVALUATION_KEY {
            return Err(AppError::Repository("synthetic gate read failure".into()));
        }
        self.0.get_setting_json(scope, key, scope_id).await
    }
    async fn upsert_setting_json(
        &self,
        scope: &str,
        key: &str,
        scope_id: Option<String>,
        value: String,
        source: &str,
        actor: Option<String>,
    ) -> AppResult<()> {
        self.0
            .upsert_setting_json(scope, key, scope_id, value, source, actor)
            .await
    }
    async fn delete_setting_value(
        &self,
        scope: &str,
        key: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.0.delete_setting_value(scope, key, scope_id).await
    }
    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        self.0.delete_values_for_scope_id(scope_id).await
    }
}

#[tokio::test]
async fn unreadable_request_rule_gate_uses_existing_library_permissions() {
    let mut harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;
    let settings = Arc::new(UnreadableRuleGate(
        harness.app.services.config.settings.clone(),
    ));
    harness.app = harness
        .app
        .clone()
        .with_test_overrides(|services| services.with_settings(settings));
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9047, None).await;
    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Pending
    );
    assert!(harness.titles.store.lock().await.is_empty());
}

#[tokio::test]
async fn hiding_experimental_ui_does_not_disarm_existing_request_rules() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;
    harness
        .app
        .services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            crate::settings::keys::EXPERIMENTAL_FEATURES_ENABLED_KEY,
            None,
            "false".into(),
            "test",
            None,
        )
        .await
        .unwrap();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9047, None).await;
    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Rejected
    );
    assert!(harness.titles.store.lock().await.is_empty());
    assert!(
        harness
            .app
            .load_request_rule_gates()
            .await
            .unwrap()
            .evaluation_enabled
    );
}

const APPROVE_WITH_NO_PREVIOUS_REQUEST: &str = r#"package rules
import rego.v1

approve if {
	input.facts.previous_request_count == 0
	input.facts.pending_request_count == 0
}
"#;

const DENY_RESTRICTED_LIBRARY_COPY: &str = r#"package rules
import rego.v1

deny if {
	"restricted-library" in input.facts.exists_in_library_ids
}
"#;

const APPROVE_WITHOUT_LINKED_PROVIDER: &str = r#"package rules
import rego.v1

approve if {
	count(input.requester.linked_providers) == 0
}
"#;

#[tokio::test]
async fn submission_rules_read_history_before_the_new_request_is_persisted() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(
        &harness,
        "Approve a first request",
        APPROVE_WITH_NO_PREVIOUS_REQUEST,
    )
    .await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9047, None).await;

    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Approved
    );
    assert_eq!(harness.titles.store.lock().await.len(), 1);
}

#[tokio::test]
async fn catalog_fact_collects_every_library_with_a_matching_title() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(
        &harness,
        "Deny restricted copies",
        DENY_RESTRICTED_LIBRARY_COPY,
    )
    .await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let mut unrestricted = make_due_hydration_title("unrestricted", MediaFacet::Movie, 9047);
    unrestricted.library_id = "unrestricted-library".to_string();
    let mut restricted = make_due_hydration_title("restricted", MediaFacet::Movie, 9047);
    restricted.library_id = "restricted-library".to_string();
    harness
        .titles
        .store
        .lock()
        .await
        .extend([unrestricted, restricted]);

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9047, None).await;

    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Rejected
    );
    assert_eq!(harness.titles.store.lock().await.len(), 2);
}

struct UnreadableExternalAccounts;

#[async_trait::async_trait]
impl crate::ports::UserExternalAccountRepository for UnreadableExternalAccounts {
    async fn create(
        &self,
        _: scryer_domain::UserExternalAccount,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        Err(AppError::Repository(
            "synthetic external-account read failure".into(),
        ))
    }

    async fn create_or_get_by_provider_identity(
        &self,
        _: scryer_domain::UserExternalAccount,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        Err(AppError::Repository(
            "synthetic external-account read failure".into(),
        ))
    }

    async fn list_by_user_id(&self, _: &str) -> AppResult<Vec<scryer_domain::UserExternalAccount>> {
        Err(AppError::Repository(
            "synthetic external-account read failure".into(),
        ))
    }

    async fn get_by_id(&self, _: &str) -> AppResult<Option<scryer_domain::UserExternalAccount>> {
        Err(AppError::Repository(
            "synthetic external-account read failure".into(),
        ))
    }

    async fn get_by_provider_identity(
        &self,
        _: scryer_domain::ExternalAccountProvider,
        _: &str,
        _: &str,
    ) -> AppResult<Option<scryer_domain::UserExternalAccount>> {
        Err(AppError::Repository(
            "synthetic external-account read failure".into(),
        ))
    }

    async fn get_pending_claim_by_provider_username(
        &self,
        _: scryer_domain::ExternalAccountProvider,
        _: &str,
        _: &str,
    ) -> AppResult<Option<scryer_domain::UserExternalAccount>> {
        Err(AppError::Repository(
            "synthetic external-account read failure".into(),
        ))
    }

    async fn list_verified_by_connection(
        &self,
        _: scryer_domain::ExternalAccountProvider,
        _: &str,
    ) -> AppResult<Vec<scryer_domain::UserExternalAccount>> {
        Err(AppError::Repository(
            "synthetic external-account read failure".into(),
        ))
    }

    async fn update(
        &self,
        _: scryer_domain::UserExternalAccount,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        Err(AppError::Repository(
            "synthetic external-account read failure".into(),
        ))
    }

    async fn create_auto_added_user_with_account(
        &self,
        _: scryer_domain::User,
        _: scryer_domain::AppPermissionMask,
        _: Vec<scryer_domain::LibraryGrant>,
        _: scryer_domain::UserExternalAccount,
    ) -> AppResult<(scryer_domain::User, scryer_domain::UserExternalAccount)> {
        Err(AppError::Repository(
            "synthetic external-account read failure".into(),
        ))
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "synthetic external-account read failure".into(),
        ))
    }
}

#[tokio::test]
async fn unreadable_linked_accounts_hold_an_enforcing_request() {
    let mut harness = bootstrap_media_request_app();
    let detail = create_rule(
        &harness,
        "Approve unlinked requesters",
        APPROVE_WITHOUT_LINKED_PROVIDER,
    )
    .await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;
    harness.app = harness.app.clone().with_test_overrides(|services| {
        services.with_external_account_store(Arc::new(UnreadableExternalAccounts))
    });

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9047, None).await;

    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Pending
    );
    assert!(harness.titles.store.lock().await.is_empty());
    let traces = harness.request_rule_decisions.recorded().await;
    assert_eq!(
        traces.last().unwrap().fallback_reason.as_deref(),
        Some(FALLBACK_ERROR)
    );
}

struct ReadFailingRequestRuleStore {
    inner: Arc<InMemoryRequestRuleRepo>,
    fail_list: std::sync::atomic::AtomicBool,
    block_first_list: std::sync::atomic::AtomicBool,
    first_list_started: Notify,
    release_first_list: Notify,
}

#[async_trait::async_trait]
impl crate::ports::RequestRuleSetRepository for ReadFailingRequestRuleStore {
    async fn list_rule_sets(&self) -> AppResult<Vec<scryer_domain::RequestRuleSet>> {
        if self.block_first_list.swap(false, Ordering::SeqCst) {
            // Capture the old persisted state before the concurrent mutation,
            // then let the test decide when this rebuild may continue.
            let rule_sets = self.inner.list_rule_sets().await?;
            self.first_list_started.notify_one();
            self.release_first_list.notified().await;
            return Ok(rule_sets);
        }
        if self.fail_list.load(Ordering::SeqCst) {
            return Err(AppError::Repository(
                "synthetic request-rule read failure".into(),
            ));
        }
        self.inner.list_rule_sets().await
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<scryer_domain::RequestRuleSet>> {
        self.inner.get_rule_set(id).await
    }

    async fn create_rule_set(
        &self,
        rule_set: &scryer_domain::RequestRuleSet,
        revision: &scryer_domain::RequestRuleRevision,
    ) -> AppResult<()> {
        self.inner.create_rule_set(rule_set, revision).await
    }

    async fn add_revision(
        &self,
        revision: &scryer_domain::RequestRuleRevision,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        self.inner.add_revision(revision, updated_at).await
    }

    async fn get_revision(
        &self,
        rule_set_id: &str,
        revision_number: i64,
    ) -> AppResult<Option<scryer_domain::RequestRuleRevision>> {
        self.inner.get_revision(rule_set_id, revision_number).await
    }

    async fn list_revisions(
        &self,
        rule_set_id: &str,
    ) -> AppResult<Vec<scryer_domain::RequestRuleRevision>> {
        self.inner.list_revisions(rule_set_id).await
    }

    async fn update_rule_set_metadata(
        &self,
        id: &str,
        name: &str,
        description: &str,
        library_ids: &[String],
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        self.inner
            .update_rule_set_metadata(id, name, description, library_ids, updated_at)
            .await
    }

    async fn update_rule_set_evaluation_mode(
        &self,
        id: &str,
        mode: RequestRuleEvaluationMode,
        enabled: bool,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        self.inner
            .update_rule_set_evaluation_mode(id, mode, enabled, updated_at)
            .await
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        self.inner.delete_rule_set(id).await
    }
}

#[tokio::test]
async fn failed_mode_refresh_holds_requests_instead_of_using_the_stale_engine() {
    let mut harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let rules = Arc::new(ReadFailingRequestRuleStore {
        inner: harness.request_rules.clone(),
        fail_list: std::sync::atomic::AtomicBool::new(false),
        block_first_list: std::sync::atomic::AtomicBool::new(false),
        first_list_started: Notify::new(),
        release_first_list: Notify::new(),
    });
    harness.app = harness
        .app
        .clone()
        .with_test_overrides(|services| services.with_request_rule_set_store(rules.clone()));
    rules.fail_list.store(true, Ordering::SeqCst);

    let error = harness
        .app
        .set_request_rule_mode(
            &harness.manager,
            &detail.rule_set.id,
            RequestRuleEvaluationMode::Disabled,
        )
        .await
        .expect_err("the refresh read should fail after persisting the disarm");
    assert!(matches!(error, AppError::Repository(_)));
    assert!(harness.app.request_rules_engine_snapshot().unavailable);

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9047, None).await;

    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Pending
    );
    assert!(harness.titles.store.lock().await.is_empty());
    let traces = harness.request_rule_decisions.recorded().await;
    assert_eq!(
        traces.last().unwrap().fallback_reason.as_deref(),
        Some(FALLBACK_ERROR)
    );
}

#[tokio::test]
async fn an_older_rebuild_cannot_replace_a_failed_newer_disarm_refresh() {
    let mut harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let rules = Arc::new(ReadFailingRequestRuleStore {
        inner: harness.request_rules.clone(),
        fail_list: std::sync::atomic::AtomicBool::new(false),
        block_first_list: std::sync::atomic::AtomicBool::new(true),
        first_list_started: Notify::new(),
        release_first_list: Notify::new(),
    });
    harness.app = harness
        .app
        .clone()
        .with_test_overrides(|services| services.with_request_rule_set_store(rules.clone()));

    let first_refresh = harness.app.rebuild_request_rules_engine();
    tokio::pin!(first_refresh);
    tokio::select! {
        _ = rules.first_list_started.notified() => {}
        result = &mut first_refresh => {
            panic!("the first refresh should wait after reading the old state: {result:?}");
        }
    }

    rules.fail_list.store(true, Ordering::SeqCst);
    let error = harness
        .app
        .set_request_rule_mode(
            &harness.manager,
            &detail.rule_set.id,
            RequestRuleEvaluationMode::Disabled,
        )
        .await
        .expect_err("the newer disarm refresh should fail after persisting the mode");
    assert!(matches!(error, AppError::Repository(_)));
    assert!(harness.app.request_rules_engine_snapshot().unavailable);

    rules.release_first_list.notify_one();
    assert!(
        (&mut first_refresh).await.is_ok(),
        "the old rebuild may finish, but its result must be discarded"
    );
    assert!(harness.app.request_rules_engine_snapshot().unavailable);

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9047, None).await;

    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Pending
    );
    assert!(harness.titles.store.lock().await.is_empty());
    let traces = harness.request_rule_decisions.recorded().await;
    assert_eq!(
        traces.last().unwrap().fallback_reason.as_deref(),
        Some(FALLBACK_ERROR)
    );
}
