use super::*;
use base64::Engine as _;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct MemorySettings {
    active: Mutex<Option<String>>,
    writes: AtomicUsize,
}

#[async_trait]
impl SettingsRepository for MemorySettings {
    async fn get_setting_json(
        &self,
        _scope: &str,
        key: &str,
        _scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        Ok((key == "smg.clock_skew_notice")
            .then(|| self.active.lock().unwrap().clone())
            .flatten())
    }
    async fn upsert_setting_json(
        &self,
        _scope: &str,
        key: &str,
        _scope_id: Option<String>,
        value: String,
        _source: &str,
        _user: Option<String>,
    ) -> AppResult<()> {
        assert_eq!(key, "smg.clock_skew_notice");
        *self.active.lock().unwrap() = Some(value);
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn delete_setting_value(
        &self,
        _scope: &str,
        _key: &str,
        _scope_id: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn delete_values_for_scope_id(&self, _scope_id: &str) -> AppResult<u32> {
        Ok(0)
    }
}

async fn signed_client(
    server: &MockServer,
    settings: Arc<MemorySettings>,
) -> MetadataGatewayClient {
    let client = MetadataGatewayClient::new_with_enrollment_store(
        format!("{}/graphql", server.uri()),
        settings,
        SmgEnrollmentConfig {
            registration_secret: Some("fixture-secret".into()),
        },
    );
    *client.mtls_state.write().await = MtlsState::Enrolled {
        client: smg_reqwest_client(),
        auth: InstanceAuth::Pq {
            instance_id: Arc::new("fixture-instance".into()),
            seed_b64: Arc::new(base64::engine::general_purpose::STANDARD.encode([7u8; 32])),
            key_id: Arc::new("fixture-key".into()),
            enrollment_generation: Some(1),
        },
    };
    client
}

fn clock_rejection() -> ResponseTemplate {
    let server_date = (chrono::Utc::now() - chrono::Duration::seconds(392))
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    ResponseTemplate::new(401)
        .insert_header("date", server_date.as_str())
        .set_body_json(json!({"error": "Arbitrary provider wording"}))
}

fn assert_clock_error(error: AppError) {
    assert!(matches!(
        error,
        AppError::ValidationRefused {
            reason: scryer_application::error_reasons::metadata_gateway::CLOCK_SKEW,
            ..
        }
    ));
}

#[tokio::test]
async fn clock_skew_post_preserves_enrollment_and_persists_only_transitions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(clock_rejection())
        .expect(2)
        .mount(&server)
        .await;
    let settings = Arc::new(MemorySettings::default());
    let client = signed_client(&server, settings.clone()).await;
    for _ in 0..2 {
        assert_clock_error(
            client
                .execute_graphql::<serde_json::Value>(json!({"query":"{ fixture }"}))
                .await
                .unwrap_err(),
        );
        assert!(matches!(
            *client.mtls_state.read().await,
            MtlsState::Enrolled { .. }
        ));
        assert!(client.last_reenrollment.lock().await.is_none());
    }
    assert_eq!(settings.active.lock().unwrap().as_deref(), Some("true"));
    assert_eq!(settings.writes.load(Ordering::SeqCst), 1);
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"fixture":true}})))
        .expect(2)
        .mount(&server)
        .await;
    for _ in 0..2 {
        client
            .execute_graphql::<serde_json::Value>(json!({"query":"{ fixture }"}))
            .await
            .unwrap();
    }
    assert_eq!(settings.active.lock().unwrap().as_deref(), Some("false"));
    assert_eq!(settings.writes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn clock_skew_apq_get_preserves_enrollment_without_post_retry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(clock_rejection())
        .expect(1)
        .mount(&server)
        .await;
    let client = signed_client(&server, Arc::new(MemorySettings::default())).await;
    assert_clock_error(
        client
            .execute_graphql_apq::<serde_json::Value>(
                "Fixture",
                "query Fixture { fixture }",
                "fixture-hash",
                json!({}),
            )
            .await
            .unwrap_err(),
    );
    assert!(matches!(
        *client.mtls_state.read().await,
        MtlsState::Enrolled { .. }
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn clock_skew_compatibility_check_preserves_enrollment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(METADATA_GATEWAY_VERSION_COMPATIBILITY_PATH))
        .respond_with(clock_rejection())
        .expect(1)
        .mount(&server)
        .await;
    let client = signed_client(&server, Arc::new(MemorySettings::default())).await;
    assert_clock_error(
        client
            .refresh_version_compatibility(false)
            .await
            .unwrap_err(),
    );
    assert!(client.last_reenrollment.lock().await.is_none());
}

#[tokio::test]
async fn clock_skew_stale_notice_clears_on_first_healthy_response_after_restart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{}})))
        .expect(1)
        .mount(&server)
        .await;
    let settings = Arc::new(MemorySettings::default());
    *settings.active.lock().unwrap() = Some("true".into());
    let client = signed_client(&server, settings.clone()).await;
    client
        .execute_graphql::<serde_json::Value>(json!({"query":"{ fixture }"}))
        .await
        .unwrap();
    assert_eq!(settings.active.lock().unwrap().as_deref(), Some("false"));
}

#[tokio::test]
async fn clock_skew_unrelated_auth_rejection_is_not_relabelled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("date", "invalid")
                .set_body_json(json!({"error":"timestamp skew too large: 392s"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = MetadataGatewayClient::new_without_enrollment_store(
        format!("{}/graphql", server.uri()),
        SmgEnrollmentConfig {
            registration_secret: None,
        },
    );
    let error = client
        .execute_graphql::<serde_json::Value>(json!({"query":"{ fixture }"}))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Repository(_)));
}
