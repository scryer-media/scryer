use super::*;
use crate::metadata::gateway::client::{InstanceAuth, MtlsState, SmgEnrollmentConfig};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

async fn transport(server: &MockServer) -> HttpListAccountAuthGateway {
    let gateway = Arc::new(MetadataGatewayClient::new_without_enrollment_store(
        format!("{}/graphql", server.uri()),
        SmgEnrollmentConfig {
            registration_secret: Some("fixture-registration".into()),
        },
    ));
    *gateway.mtls_state.write().await = MtlsState::Enrolled {
        client: scryer_outbound_http::smg_reqwest_client(),
        auth: InstanceAuth::Pq {
            instance_id: Arc::new("fixture-instance".into()),
            seed_b64: Arc::new(base64::engine::general_purpose::STANDARD.encode([7u8; 32])),
            key_id: Arc::new("fixture-key".into()),
            enrollment_generation: Some(1),
        },
    };
    let mut transport = HttpListAccountAuthGateway::new_with_client_identifier(
        gateway,
        "fixture-plex-client".into(),
    )
    .unwrap();
    transport.endpoints = Endpoints {
        relay: server.uri(),
        plex: server.uri(),
        tmdb: server.uri(),
        trakt: format!("{}/trakt/token", server.uri()),
        anilist: format!("{}/anilist/token", server.uri()),
        mal: format!("{}/mal/token", server.uri()),
    };
    transport
}
fn start_request(provider: &str) -> ListAccountStartRequest {
    ListAccountStartRequest {
        provider: provider.into(),
        origin: "http://instance.example".into(),
        state: "fixture-state".into(),
        code_challenge: "c".repeat(43),
        app: None,
    }
}
fn complete_request(provider: &str) -> ListAccountCompleteRequest {
    ListAccountCompleteRequest {
        provider: provider.into(),
        origin: "http://instance.example".into(),
        code: "fixture-exchange".into(),
        code_verifier: "v".repeat(43),
        app: None,
        poll_token: Some(
            encode(&RelayPoll {
                client_id: "fixture-client".into(),
            })
            .unwrap(),
        ),
    }
}
fn token_reply(provider: &str) -> Value {
    let mut value =
        json!({ "access_token": "fixture-access", "token_type": "bearer", "expires_in": 3600 });
    if matches!(provider, "trakt" | "simkl") {
        value["refresh_handle"] = json!("fixture-handle");
    }
    if provider == "simkl" {
        value["scope"] = json!("media:read");
        value["refresh_expires_in"] = json!(180 * 24 * 60 * 60);
    }
    value
}
fn app() -> ListProviderAppConfig {
    ListProviderAppConfig {
        client_id: "fixture-client".into(),
        client_secret: "fixture-secret".into(),
        redirect_uri: "http://instance.example/lists/oauth/callback".into(),
        access_token: None,
    }
}

#[tokio::test]
async fn relay_start_signs_exact_path_raw_body_and_fresh_nonce() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST")).and(path("/auth/v1/trakt/start"))
        .and(body_json(json!({"origin":"http://instance.example","state":"fixture-state","code_challenge":"c".repeat(43)})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"authorize_url":"https://auth.trakt.tv/oauth/authorize?client_id=fixture-client", "expires_in":600})))
        .expect(2).mount(&server).await;
    transport.start(start_request("trakt")).await.unwrap();
    transport.start(start_request("trakt")).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let public_key = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            use aws_lc_rs::signature::KeyPair as _;
            aws_lc_rs::signature::PqdsaKeyPair::from_seed(
                &aws_lc_rs::signature::ML_DSA_65_SIGNING,
                &[7u8; 32],
            )
            .unwrap()
            .public_key()
            .as_ref()
            .to_vec()
        })
        .unwrap()
        .join()
        .unwrap();
    for request in &requests {
        assert_eq!(
            request.headers.get("x-scryer-auth-version").unwrap(),
            "pqsig-v2"
        );
        assert_eq!(
            request.headers.get("x-scryer-key-id").unwrap(),
            "fixture-key"
        );
        let timestamp = request
            .headers
            .get("x-scryer-timestamp")
            .unwrap()
            .to_str()
            .unwrap();
        let nonce = request
            .headers
            .get("x-scryer-nonce")
            .unwrap()
            .to_str()
            .unwrap();
        let host = request.headers.get("host").unwrap().to_str().unwrap();
        let endpoint = url::Url::parse(&server.uri()).unwrap();
        assert_eq!(
            host,
            &endpoint[url::Position::BeforeHost..url::Position::AfterPort]
        );
        let hash = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, &request.body);
        let body_hash = hash
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let message =
            format!("POST\n{host}\n/auth/v1/trakt/start\n{timestamp}\n{nonce}\n{body_hash}")
                .into_bytes();
        let signature = base64::engine::general_purpose::STANDARD
            .decode(request.headers.get("x-scryer-signature").unwrap())
            .unwrap();
        aws_lc_rs::signature::UnparsedPublicKey::new(&aws_lc_rs::signature::ML_DSA_65, &public_key)
            .verify(&message, &signature)
            .unwrap();
    }
    assert_ne!(
        requests[0].headers.get("x-scryer-nonce"),
        requests[1].headers.get("x-scryer-nonce")
    );
}

#[tokio::test]
async fn relay_exchange_uses_receipt_and_keeps_refresh_handle_separate() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    for provider in ["trakt", "anilist", "simkl"] {
        Mock::given(method("POST"))
            .and(path(format!("/auth/v1/{provider}/exchange")))
            .and(body_json(
                json!({"exchange_code":"fixture-exchange", "code_verifier":"v".repeat(43)}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_reply(provider)))
            .expect(1)
            .mount(&server)
            .await;
        let credential = transport
            .complete(complete_request(provider))
            .await
            .unwrap();
        assert_eq!(credential.access_token, "fixture-access");
        assert!(credential.refresh_token.is_none());
        assert!(!credential.direct);
        assert_eq!(
            credential.refresh_handle.as_deref(),
            if provider == "anilist" {
                None
            } else {
                Some("fixture-handle")
            }
        );
        assert_eq!(credential.client_id.as_deref(), Some("fixture-client"));
    }
}

#[tokio::test]
async fn relay_renew_and_simkl_revoke_send_only_refresh_handle() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    for provider in ["trakt", "simkl"] {
        Mock::given(method("POST"))
            .and(path(format!("/auth/v1/{provider}/renew")))
            .and(body_json(json!({"refresh_handle":"fixture-handle"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_reply(provider)))
            .expect(1)
            .mount(&server)
            .await;
        let credential = ListAccountCredential {
            access_token: "fixture-access".into(),
            refresh_handle: Some("fixture-handle".into()),
            ..Default::default()
        };
        transport.renew(provider, &credential, None).await.unwrap();
        if provider == "simkl" {
            Mock::given(method("POST"))
                .and(path("/auth/v1/simkl/revoke"))
                .and(body_json(json!({"refresh_handle":"fixture-handle"})))
                .respond_with(ResponseTemplate::new(204))
                .expect(1)
                .mount(&server)
                .await;
            transport.revoke(provider, &credential, None).await.unwrap();
        }
    }
}

#[tokio::test]
async fn ambiguous_token_post_is_not_retried_and_error_body_is_redacted() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST"))
        .and(path("/auth/v1/trakt/exchange"))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_json(json!({"error":"fixture-secret", "access_token":"fixture-secret"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let error = transport
        .complete(complete_request("trakt"))
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("fixture-secret"));
    assert!(error.contains("provider_unavailable"));
}

#[tokio::test]
async fn token_redirect_does_not_replay_credentials() {
    let server = MockServer::start().await;
    let target = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST"))
        .and(path("/auth/v1/trakt/exchange"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("Location", format!("{}/stolen", target.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(transport.complete(complete_request("trakt")).await.is_err());
    assert!(target.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn successful_and_error_bodies_are_bounded() {
    for status in [200, 400] {
        let server = MockServer::start().await;
        let transport = transport(&server).await;
        Mock::given(method("POST"))
            .and(path("/auth/v1/trakt/exchange"))
            .respond_with(
                ResponseTemplate::new(status).set_body_string("x".repeat(RESPONSE_LIMIT + 1)),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            transport
                .complete(complete_request("trakt"))
                .await
                .unwrap_err()
                .to_string()
                .contains("invalid_provider_response")
        );
    }
}

#[tokio::test]
async fn default_mal_exchanges_and_renews_directly_with_public_client_and_plain_pkce() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST")).and(path("/mal/token")).and(header("content-type", "application/x-www-form-urlencoded"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"fixture-access","refresh_token":"fixture-refresh","token_type":"Bearer","expires_in":3600})))
        .expect(2).mount(&server).await;
    let credential = transport.complete(complete_request("mal")).await.unwrap();
    transport.renew("mal", &credential, None).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let fields: std::collections::BTreeMap<_, _> = url::form_urlencoded::parse(&requests[0].body)
        .into_owned()
        .collect();
    assert_eq!(fields.get("client_id").unwrap(), "fixture-client");
    assert_eq!(fields.get("code_verifier").unwrap(), &"v".repeat(43));
    assert_eq!(
        fields.get("redirect_uri").unwrap(),
        "https://smg.scryer.media/auth/v1/mal/auth"
    );
    assert!(!fields.contains_key("client_secret"));
    let fields: std::collections::BTreeMap<_, _> = url::form_urlencoded::parse(&requests[1].body)
        .into_owned()
        .collect();
    assert_eq!(fields.get("refresh_token").unwrap(), "fixture-refresh");
    assert_eq!(fields.get("grant_type").unwrap(), "refresh_token");
}

#[tokio::test]
async fn byo_trakt_and_anilist_exchange_json_at_fixed_endpoints() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    for provider in ["trakt", "anilist"] {
        let mut reply = token_reply("anilist");
        if provider == "trakt" {
            reply["refresh_token"] = json!("fixture-refresh");
        }
        let mut body = json!({"grant_type":"authorization_code","code":"fixture-exchange", "client_id":"fixture-client","client_secret":"fixture-secret","redirect_uri":"http://instance.example/lists/oauth/callback"});
        if provider == "trakt" {
            body["code_verifier"] = json!("v".repeat(43));
        }
        Mock::given(method("POST"))
            .and(path(format!("/{provider}/token")))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(reply))
            .expect(1)
            .mount(&server)
            .await;
        let mut request = complete_request(provider);
        request.app = Some(app());
        assert!(transport.complete(request).await.unwrap().direct);
    }
}

fn pkce_trakt_app() -> ListProviderAppConfig {
    ListProviderAppConfig {
        client_secret: String::new(),
        ..app()
    }
}

fn direct_trakt_reply() -> Value {
    let mut reply = token_reply("anilist");
    reply["refresh_token"] = json!("fixture-refresh");
    reply
}

#[test]
fn byo_trakt_authorizes_on_the_auth_host_with_an_s256_challenge() {
    for app in [pkce_trakt_app(), app()] {
        let mut request = start_request("trakt");
        request.app = Some(app.clone());
        let url = Url::parse(&direct_authorize_url("trakt", &app, &request).unwrap()).unwrap();
        assert_eq!(url.origin().ascii_serialization(), "https://auth.trakt.tv");
        assert_eq!(url.path(), "/oauth/authorize");
        let query: std::collections::BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("code_challenge").unwrap(), &"c".repeat(43));
        assert_eq!(query.get("code_challenge_method").unwrap(), "S256");
        assert_eq!(query.get("client_id").unwrap(), "fixture-client");
        assert_eq!(query.get("state").unwrap(), "fixture-state");
        assert!(!url.as_str().contains("fixture-secret"));
    }
    // AniList still needs its secret and gets no challenge.
    let mut request = start_request("anilist");
    request.app = Some(pkce_trakt_app());
    assert!(direct_authorize_url("anilist", &pkce_trakt_app(), &request).is_err());
    request.app = Some(app());
    let url = direct_authorize_url("anilist", &app(), &request).unwrap();
    assert!(!url.contains("code_challenge"));
    // The relay is held to the same auth host.
    assert_eq!(
        validate_authorize_url(
            "trakt",
            "https://auth.trakt.tv/oauth/authorize?client_id=fixture-client"
        )
        .unwrap(),
        "fixture-client"
    );
    assert!(
        validate_authorize_url(
            "trakt",
            "https://trakt.tv/oauth/authorize?client_id=fixture-client"
        )
        .is_err()
    );
}

#[tokio::test]
async fn byo_trakt_exchange_sends_the_verifier_and_no_secret_when_the_app_has_none() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST"))
        .and(path("/trakt/token"))
        .and(header("trakt-api-key", "fixture-client"))
        .and(body_json(json!({
            "grant_type": "authorization_code",
            "code": "fixture-exchange",
            "code_verifier": "v".repeat(43),
            "client_id": "fixture-client",
            "redirect_uri": "http://instance.example/lists/oauth/callback",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(direct_trakt_reply()))
        .expect(1)
        .mount(&server)
        .await;
    let mut request = complete_request("trakt");
    request.app = Some(pkce_trakt_app());
    let credential = transport.complete(request).await.unwrap();
    assert!(credential.direct);
    assert_eq!(credential.refresh_token.as_deref(), Some("fixture-refresh"));
    assert_eq!(credential.client_id.as_deref(), Some("fixture-client"));
}

#[tokio::test]
async fn byo_trakt_renew_sends_the_secret_only_when_the_app_has_one() {
    for app in [pkce_trakt_app(), app()] {
        let server = MockServer::start().await;
        let transport = transport(&server).await;
        let mut body = json!({
            "grant_type": "refresh_token",
            "refresh_token": "fixture-refresh",
            "client_id": "fixture-client",
            "redirect_uri": "http://instance.example/lists/oauth/callback",
        });
        if !app.client_secret.is_empty() {
            body["client_secret"] = json!("fixture-secret");
        }
        Mock::given(method("POST"))
            .and(path("/trakt/token"))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(direct_trakt_reply()))
            .expect(1)
            .mount(&server)
            .await;
        let credential = ListAccountCredential {
            access_token: "fixture-access".into(),
            refresh_token: Some("fixture-refresh".into()),
            client_id: Some("fixture-client".into()),
            direct: true,
            ..Default::default()
        };
        let renewed = transport
            .renew("trakt", &credential, Some(&app))
            .await
            .unwrap();
        assert_eq!(renewed.refresh_token.as_deref(), Some("fixture-refresh"));
    }
}

/// Collects every field and message Scryer records so a test can prove what
/// the transport logged.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<std::sync::Mutex<String>>);
impl tracing::field::Visit for CapturedLogs {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let mut logs = self.0.lock().unwrap();
        logs.push_str(&format!("{}={value:?}\n", field.name()));
    }
}
impl tracing::Subscriber for CapturedLogs {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target().starts_with("scryer")
    }
    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }
    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        span.record(&mut self.clone());
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        values.record(&mut self.clone());
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        event.record(&mut self.clone());
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test]
async fn byo_trakt_failed_exchange_never_reports_the_verifier_or_secret() {
    let logs = CapturedLogs::default();
    let _logging = tracing::dispatcher::set_default(&tracing::Dispatch::new(logs.clone()));
    let verifier = "fixture-private-verifier-".repeat(2);
    for (status, expected) in [(400, "reconnect_required"), (503, "provider_unavailable")] {
        let server = MockServer::start().await;
        let transport = transport(&server).await;
        Mock::given(method("POST"))
            .and(path("/trakt/token"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                "error": if status == 400 { "invalid_grant" } else { "fixture-secret" },
                "error_description": format!("{verifier} fixture-secret"),
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut request = complete_request("trakt");
        request.code_verifier = verifier.clone();
        request.app = Some(app());
        let error = transport.complete(request).await.unwrap_err().to_string();
        assert!(error.ends_with(expected), "{error}");
        assert!(!error.contains(&verifier));
        assert!(!error.contains("fixture-secret"));
        // The verifier did reach the provider, so its absence from the
        // report is the redaction, not a missing field.
        let sent = &server.received_requests().await.unwrap()[0].body;
        assert!(String::from_utf8_lossy(sent).contains(&verifier));
    }
    let logs = logs.0.lock().unwrap();
    assert!(logs.contains("list OAuth response"), "{logs}");
    assert!(!logs.contains(&verifier));
    assert!(!logs.contains("fixture-secret"));
    assert!(!logs.contains("fixture-exchange"));
}

#[tokio::test]
async fn plex_pin_creation_and_poll_bind_same_identifier_and_pin() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST"))
        .and(path("/pins"))
        .and(header("x-plex-client-identifier", "fixture-plex-client"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":42,"code":"fixture-code","expiresIn":900})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let started = transport.start(start_request("plex")).await.unwrap();
    assert_eq!(started.expires_in, 600);
    Mock::given(method("GET"))
        .and(path("/pins/42"))
        .and(header("x-plex-client-identifier", "fixture-plex-client"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":42,"code":"fixture-code","authToken":"fixture-access"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        transport
            .poll("plex", started.poll_token.as_deref().unwrap(), None)
            .await
            .unwrap()
            .unwrap()
            .access_token,
        "fixture-access"
    );
}

#[tokio::test]
async fn tmdb_v4_approval_uses_application_bearer_and_keeps_account_id() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    let app = ListProviderAppConfig {
        access_token: Some("fixture-read-token".into()),
        ..Default::default()
    };
    Mock::given(method("POST"))
        .and(path("/auth/request_token"))
        .and(header("authorization", "Bearer fixture-read-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success":true,"request_token":"fixture-request"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut request = start_request("tmdb");
    request.app = Some(app.clone());
    let started = transport.start(request).await.unwrap();
    Mock::given(method("POST")).and(path("/auth/access_token")).and(header("authorization", "Bearer fixture-read-token"))
        .and(body_json(json!({"request_token":"fixture-request"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true,"access_token":"fixture-user-token","account_id":"fixture-account"})))
        .expect(1).mount(&server).await;
    let credential = transport
        .poll("tmdb", started.poll_token.as_deref().unwrap(), Some(&app))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(credential.account_id.as_deref(), Some("fixture-account"));
    assert_eq!(credential.access_token, "fixture-user-token");
}

#[test]
fn token_validation_rejects_raw_relay_refresh_wrong_scope_and_overflow() {
    let mut reply = token_reply("trakt");
    reply["refresh_token"] = json!("fixture-raw-refresh");
    assert!(
        serde_json::from_value::<Tokens>(reply)
            .unwrap()
            .credential("trakt", false)
            .is_err()
    );
    let mut reply = token_reply("simkl");
    reply["scope"] = json!("media:write");
    assert!(
        serde_json::from_value::<Tokens>(reply)
            .unwrap()
            .credential("simkl", false)
            .is_err()
    );
    let mut reply = token_reply("trakt");
    reply["expires_in"] = json!(i64::MAX);
    assert!(
        serde_json::from_value::<Tokens>(reply)
            .unwrap()
            .credential("trakt", false)
            .is_err()
    );
}

#[test]
fn byo_callback_requires_exact_own_origin_and_simkl_has_no_byo() {
    assert!(
        validate_redirect(
            "http://instance.example/media/lists/oauth/callback",
            "http://instance.example"
        )
        .is_ok()
    );
    assert!(
        validate_redirect(
            "http://instance.example/media/lists/oauth/callback/other",
            "http://instance.example"
        )
        .is_err()
    );
    assert!(
        validate_redirect(
            "http://other.example/lists/oauth/callback",
            "http://instance.example"
        )
        .is_err()
    );
    assert!(
        validate_redirect(
            "http://instance.example/lists/oauth/callback?token=bad",
            "http://instance.example"
        )
        .is_err()
    );
    let app = app();
    let mut request = start_request("mal");
    request.app = Some(app.clone());
    let url = direct_authorize_url("mal", &app, &request).unwrap();
    let query: std::collections::BTreeMap<_, _> = Url::parse(&url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert_eq!(query.get("code_challenge_method").unwrap(), "plain");
    assert!(!url.contains("fixture-secret"));
    assert!(
        validate_authorize_url(
            "mal",
            "https://other.example/v1/oauth2/authorize?client_id=bad"
        )
        .is_err()
    );
}

#[test]
fn token_issue_time_accepts_small_positive_skew_but_rejects_stale_and_impossible_dates() {
    let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let mut reply = token_reply("trakt");
    reply["created_at"] = json!((now + chrono::Duration::minutes(2)).timestamp());
    let credential = serde_json::from_value::<Tokens>(reply)
        .unwrap()
        .credential_at("trakt", false, now)
        .unwrap();
    assert_eq!(
        credential.expires_at.unwrap(),
        now + chrono::Duration::seconds(3600)
    );
    for issued in [
        (now - chrono::Duration::hours(2)).timestamp(),
        (now + chrono::Duration::hours(2)).timestamp(),
        i64::MAX,
        -1,
    ] {
        let mut reply = token_reply("trakt");
        reply["created_at"] = json!(issued);
        assert!(
            serde_json::from_value::<Tokens>(reply)
                .unwrap()
                .credential_at("trakt", false, now)
                .is_err()
        );
    }
}

#[tokio::test]
async fn tmdb_pending_status_is_restricted_to_expected_unauthorized_response() {
    for (status, pending) in [(401, true), (503, false), (307, false)] {
        let server = MockServer::start().await;
        let transport = transport(&server).await;
        let app = ListProviderAppConfig {
            access_token: Some("fixture-read-token".into()),
            ..Default::default()
        };
        Mock::given(method("POST"))
            .and(path("/auth/access_token"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({"success":false,"status_code":41})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = transport.poll("tmdb", "fixture-request", Some(&app)).await;
        if pending {
            assert!(result.unwrap().is_none());
        } else {
            assert!(result.is_err());
        }
    }
}

#[tokio::test]
async fn simkl_byo_is_refused_before_network() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    let mut request = start_request("simkl");
    request.app = Some(app());
    assert!(transport.start(request).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
}
