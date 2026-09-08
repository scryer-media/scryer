use super::*;

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
