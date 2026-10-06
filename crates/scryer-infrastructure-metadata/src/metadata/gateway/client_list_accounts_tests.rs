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
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"authorize_url":"https://trakt.tv/oauth/authorize?client_id=fixture-client", "expires_in":600})))
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
    assert!(error.contains("relay_unavailable"));
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
        Mock::given(method("POST")).and(path(format!("/{provider}/token")))
            .and(body_json(json!({"grant_type":"authorization_code","code":"fixture-exchange", "client_id":"fixture-client","client_secret":"fixture-secret","redirect_uri":"http://instance.example/lists/oauth/callback"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(reply)).expect(1).mount(&server).await;
        let mut request = complete_request(provider);
        request.app = Some(app());
        assert!(transport.complete(request).await.unwrap().direct);
    }
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
            .credential("trakt", false, TokenGrant::Exchange)
            .is_err()
    );
    let mut reply = token_reply("simkl");
    reply["scope"] = json!("media:write");
    assert!(
        serde_json::from_value::<Tokens>(reply)
            .unwrap()
            .credential("simkl", false, TokenGrant::Exchange)
            .is_err()
    );
    let mut reply = token_reply("trakt");
    reply["expires_in"] = json!(i64::MAX);
    assert!(
        serde_json::from_value::<Tokens>(reply)
            .unwrap()
            .credential("trakt", false, TokenGrant::Exchange)
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
        .credential_at("trakt", false, TokenGrant::Exchange, now)
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
                .credential_at("trakt", false, TokenGrant::Exchange, now)
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

fn simkl_reply_with_refresh_lifetime(refresh_expires_in: Option<i64>) -> Tokens {
    let mut reply = token_reply("simkl");
    match refresh_expires_in {
        Some(secs) => reply["refresh_expires_in"] = json!(secs),
        None => {
            reply.as_object_mut().unwrap().remove("refresh_expires_in");
        }
    }
    serde_json::from_value(reply).unwrap()
}

#[test]
fn simkl_renew_keeps_a_shortened_refresh_deadline_but_exchange_requires_the_full_lifetime() {
    let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let full = SIMKL_REFRESH_LIFETIME_SECS;
    let remaining = 30 * 24 * 60 * 60;

    for secs in [1, remaining, full] {
        let credential = simkl_reply_with_refresh_lifetime(Some(secs))
            .credential_at("simkl", false, TokenGrant::Renew, now)
            .unwrap();
        assert_eq!(
            credential.refresh_expires_at,
            Some(now + chrono::Duration::seconds(secs)),
            "renew stores the deadline the relay reported"
        );
    }
    for secs in [Some(0), Some(-1), Some(full + 1), None] {
        assert!(
            simkl_reply_with_refresh_lifetime(secs)
                .credential_at("simkl", false, TokenGrant::Renew, now)
                .is_err(),
            "renew rejects refresh lifetime {secs:?}"
        );
    }

    let credential = simkl_reply_with_refresh_lifetime(Some(full))
        .credential_at("simkl", false, TokenGrant::Exchange, now)
        .unwrap();
    assert_eq!(
        credential.refresh_expires_at,
        Some(now + chrono::Duration::seconds(full))
    );
    for secs in [Some(remaining), Some(0), Some(full + 1), None] {
        assert!(
            simkl_reply_with_refresh_lifetime(secs)
                .credential_at("simkl", false, TokenGrant::Exchange, now)
                .is_err(),
            "exchange rejects refresh lifetime {secs:?}"
        );
    }
}

#[tokio::test]
async fn simkl_relay_renew_accepts_the_remaining_refresh_lifetime() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    let remaining = 30 * 24 * 60 * 60;
    let mut reply = token_reply("simkl");
    reply["refresh_expires_in"] = json!(remaining);
    Mock::given(method("POST"))
        .and(path("/auth/v1/simkl/renew"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reply.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/v1/simkl/exchange"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reply))
        .expect(1)
        .mount(&server)
        .await;
    let stored = ListAccountCredential {
        access_token: "fixture-access".into(),
        refresh_handle: Some("fixture-handle".into()),
        ..Default::default()
    };

    let before = Utc::now();
    let renewed = transport.renew("simkl", &stored, None).await.unwrap();
    let after = Utc::now();
    let deadline = renewed.refresh_expires_at.expect("refresh deadline");
    assert!(deadline >= before + chrono::Duration::seconds(remaining));
    assert!(deadline <= after + chrono::Duration::seconds(remaining));

    let error = transport
        .complete(complete_request("simkl"))
        .await
        .unwrap_err();
    assert_eq!(
        scryer_application::lists::account_transport::auth_failure_code(&error),
        Some("invalid_provider_response"),
        "an exchange must report the full lifetime"
    );
}

#[tokio::test]
async fn relay_unavailable_answers_on_renew_and_revoke_are_transient_not_reconnect() {
    for relay_error in ["relay_key_unavailable", "relay_busy"] {
        let server = MockServer::start().await;
        let transport = transport(&server).await;
        for operation in ["renew", "revoke"] {
            Mock::given(method("POST"))
                .and(path(format!("/auth/v1/simkl/{operation}")))
                .respond_with(
                    ResponseTemplate::new(503)
                        .insert_header("Retry-After", "300")
                        .set_body_json(json!({ "error": relay_error })),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        let stored = ListAccountCredential {
            access_token: "fixture-access".into(),
            refresh_handle: Some("fixture-handle".into()),
            ..Default::default()
        };

        let renew_error = transport.renew("simkl", &stored, None).await.unwrap_err();
        let revoke_error = transport.revoke("simkl", &stored, None).await.unwrap_err();

        // `relay_unavailable` is a transient failure of the relay itself: the
        // account keeps its link, the next sync retries, and it never counts
        // against the grant. It must never read as reconnect_required.
        for (operation, error) in [("renew", renew_error), ("revoke", revoke_error)] {
            assert_eq!(
                scryer_application::lists::account_transport::auth_failure_code(&error),
                Some("relay_unavailable"),
                "{relay_error} on {operation}"
            );
        }
    }
}

fn failure_code(error: &AppError) -> Option<&str> {
    scryer_application::lists::account_transport::auth_failure_code(error)
}
fn tmdb_app() -> ListProviderAppConfig {
    ListProviderAppConfig {
        access_token: Some("fixture-read-token".into()),
        ..Default::default()
    }
}
fn relay_credential() -> ListAccountCredential {
    ListAccountCredential {
        access_token: "fixture-access".into(),
        refresh_handle: Some("fixture-handle".into()),
        ..Default::default()
    }
}
fn direct_credential() -> ListAccountCredential {
    ListAccountCredential {
        access_token: "fixture-access".into(),
        refresh_token: Some("fixture-refresh".into()),
        client_id: Some("fixture-client".into()),
        direct: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn tmdb_refusal_without_pending_status_is_provider_rejected() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST"))
        .and(path("/auth/access_token"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"success":false,"status_code":7})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let error = transport
        .poll("tmdb", "fixture-request", Some(&tmdb_app()))
        .await
        .unwrap_err();
    assert_eq!(failure_code(&error), Some("provider_rejected"));
}

#[tokio::test]
async fn tmdb_pending_approval_still_reads_as_pending() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST"))
        .and(path("/auth/access_token"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"success":false,"status_code":41})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let result = transport
        .poll("tmdb", "fixture-request", Some(&tmdb_app()))
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn tmdb_html_server_error_is_unavailable_not_an_invalid_response() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST"))
        .and(path("/auth/access_token"))
        .respond_with(
            ResponseTemplate::new(503).set_body_string("<html><body>fixture outage</body></html>"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let error = transport
        .poll("tmdb", "fixture-request", Some(&tmdb_app()))
        .await
        .unwrap_err();
    assert_eq!(failure_code(&error), Some("provider_unavailable"));
}

#[tokio::test]
async fn plex_missing_pin_is_provider_rejected() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("GET"))
        .and(path("/pins/42"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let poll = encode(&PlexPoll {
        id: 42,
        code: "fixture-code".into(),
        client_id: "fixture-plex-client".into(),
    })
    .unwrap();
    let error = transport.poll("plex", &poll, None).await.unwrap_err();
    assert_eq!(failure_code(&error), Some("provider_rejected"));
}

#[tokio::test]
async fn rate_limited_answer_keeps_the_requested_pause() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST"))
        .and(path("/auth/v1/simkl/renew"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "120")
                .set_body_json(json!({"error":"rate_limited"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let error = transport
        .renew("simkl", &relay_credential(), None)
        .await
        .unwrap_err();
    assert_eq!(failure_code(&error), Some("rate_limited"));
    match error {
        AppError::TemporaryUnavailable {
            message,
            retry_after,
            ..
        } => {
            assert_eq!(message, "list account authentication failed: rate_limited");
            assert_eq!(retry_after, Some(Duration::from_secs(120)));
        }
        other => panic!("expected a temporary failure, got {other:?}"),
    }
}

/// An error answer body in a renew table.
#[derive(Clone, Copy, Debug)]
enum Answer {
    /// `{"error": <code>}`, the OAuth and relay error shape.
    Code(&'static str),
    /// No body at all.
    Empty,
    /// A proxy, CDN or maintenance page.
    Html,
}
fn answer(status: u16, body: Answer) -> ResponseTemplate {
    let template = ResponseTemplate::new(status);
    match body {
        Answer::Code(error) => template.set_body_json(json!({ "error": error })),
        Answer::Empty => template,
        Answer::Html => template
            .insert_header("content-type", "text/html")
            .set_body_string("<html><body>fixture gateway page</body></html>"),
    }
}

#[tokio::test]
async fn relay_renew_answers_map_to_reconnect_only_for_explicit_grant_codes() {
    use Answer::*;
    use scryer_application::lists::account_transport::renew_requires_reconnect;
    let cases: &[(u16, Answer, &str)] = &[
        (400, Code("invalid_grant"), "reconnect_required"),
        (400, Code("invalid_refresh_handle"), "reconnect_required"),
        (400, Code("expired_token"), "reconnect_required"),
        (400, Code("access_denied"), "access_denied"),
        (400, Code("invalid_scope"), "provider_unavailable"),
        (400, Code("invalid_request"), "invalid_request"),
        (413, Code("request_too_large"), "invalid_request"),
        (
            401,
            Code("instance_auth_required"),
            "instance_auth_required",
        ),
        (404, Code("not_found"), "unsupported_provider"),
        (405, Code("method_not_allowed"), "unsupported_provider"),
        (503, Code("relay_busy"), "relay_unavailable"),
        (503, Code("relay_key_unavailable"), "relay_unavailable"),
        (400, Code("relay_busy"), "relay_unavailable"),
        (500, Code("relay_unavailable"), "relay_unavailable"),
        (500, Empty, "relay_unavailable"),
        (503, Html, "relay_unavailable"),
        (503, Code("relay_not_configured"), "provider_not_configured"),
        (
            503,
            Code("provider_not_configured"),
            "provider_not_configured",
        ),
        (
            503,
            Code("provider_configuration_error"),
            "provider_not_configured",
        ),
        (502, Code("provider_unavailable"), "provider_unavailable"),
        (
            502,
            Code("invalid_provider_response"),
            "provider_unavailable",
        ),
        (502, Code("invalid_provider_scope"), "provider_unavailable"),
        (408, Empty, "relay_unavailable"),
        // SMG's instance authentication answers in free text. None of it says
        // anything about the provider grant.
        (
            401,
            Code("timestamp skew too large: 412s"),
            "instance_auth_required",
        ),
        (401, Code("unknown PQ key"), "instance_auth_required"),
        (
            401,
            Code("instance auth required"),
            "instance_auth_required",
        ),
        (401, Code("nonce already used"), "instance_auth_required"),
        (
            403,
            Code("access denied: client IP does not match enrollment"),
            "instance_auth_required",
        ),
        (403, Code("access denied"), "instance_auth_required"),
        (404, Code("not found"), "relay_unavailable"),
        (403, Empty, "instance_auth_required"),
        (401, Html, "instance_auth_required"),
        (404, Empty, "relay_unavailable"),
        (400, Html, "relay_unavailable"),
        (400, Code("fixture_unknown_code"), "relay_unavailable"),
        (500, Code("fixture_unknown_code"), "relay_unavailable"),
    ];
    for &(status, body, expected) in cases {
        let server = MockServer::start().await;
        let transport = transport(&server).await;
        Mock::given(method("POST"))
            .and(path("/auth/v1/simkl/renew"))
            .respond_with(answer(status, body))
            .expect(1)
            .mount(&server)
            .await;
        let failure = transport
            .renew("simkl", &relay_credential(), None)
            .await
            .unwrap_err();
        let code = failure_code(&failure);
        assert_eq!(code, Some(expected), "relay {status} {body:?}");
        let reconnect = matches!(
            body,
            Code("invalid_grant" | "invalid_refresh_handle" | "expired_token" | "access_denied")
        );
        assert_eq!(
            renew_requires_reconnect(expected),
            reconnect,
            "relay {status} {body:?} reconnect"
        );
    }
}

#[tokio::test]
async fn direct_renew_refusals_require_reconnect_and_outages_do_not() {
    use Answer::*;
    use scryer_application::lists::account_transport::renew_requires_reconnect;
    let cases: &[(u16, Answer, bool, &str)] = &[
        (400, Code("invalid_grant"), true, "reconnect_required"),
        (400, Code("invalid_request"), true, "provider_rejected"),
        (401, Code("invalid_token"), true, "provider_rejected"),
        (502, Code("invalid_grant"), true, "reconnect_required"),
        // The operator's own app refusing its client credentials keeps every
        // account linked through it.
        (
            401,
            Code("invalid_client"),
            false,
            "provider_not_configured",
        ),
        (
            400,
            Code("unauthorized_client"),
            false,
            "provider_not_configured",
        ),
        // A bare or HTML 4xx is a proxy or maintenance page, not a refusal.
        (401, Empty, false, "provider_unavailable"),
        (403, Empty, false, "provider_unavailable"),
        (400, Empty, false, "provider_unavailable"),
        (403, Html, false, "provider_unavailable"),
        (404, Html, false, "provider_unavailable"),
        (500, Empty, false, "provider_unavailable"),
        (503, Html, false, "provider_unavailable"),
        (408, Empty, false, "provider_unavailable"),
    ];
    for &(status, body, reconnect, expected) in cases {
        let server = MockServer::start().await;
        let transport = transport(&server).await;
        Mock::given(method("POST"))
            .and(path("/trakt/token"))
            .respond_with(answer(status, body))
            .expect(1)
            .mount(&server)
            .await;
        let failure = transport
            .renew("trakt", &direct_credential(), Some(&app()))
            .await
            .unwrap_err();
        let code = failure_code(&failure).unwrap();
        assert_eq!(code, expected, "direct {status} {body:?}");
        assert_eq!(
            renew_requires_reconnect(code),
            reconnect,
            "direct {status} {body:?}"
        );
    }
}

#[tokio::test]
async fn unreachable_endpoint_is_a_transport_failure_not_a_provider_answer() {
    let server = MockServer::start().await;
    let mut transport = transport(&server).await;
    // Port 0 can never accept a connection: it fails before any HTTP answer.
    transport.endpoints.trakt = "http://127.0.0.1:0/trakt/token".into();
    let error = transport
        .renew("trakt", &direct_credential(), Some(&app()))
        .await
        .unwrap_err();
    assert_eq!(failure_code(&error), Some("transport_unavailable"));
}

#[tokio::test]
async fn built_in_mal_client_refusal_is_a_configuration_fault_not_a_reconnect() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    Mock::given(method("POST"))
        .and(path("/mal/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error":"invalid_client"})))
        .expect(1)
        .mount(&server)
        .await;
    let error = transport
        .renew("mal", &direct_credential(), None)
        .await
        .unwrap_err();
    assert_eq!(failure_code(&error), Some("provider_not_configured"));
}

#[tokio::test]
async fn link_exchange_keeps_its_specific_codes() {
    let cases: &[(u16, Option<&str>, &str)] = &[
        (400, Some("invalid_grant"), "reconnect_required"),
        (400, Some("access_denied"), "access_denied"),
        (401, Some("invalid_client"), "provider_not_configured"),
        (400, Some("unauthorized_client"), "provider_not_configured"),
        (400, None, "provider_rejected"),
        (504, None, "provider_unavailable"),
    ];
    for &(status, error, expected) in cases {
        let server = MockServer::start().await;
        let transport = transport(&server).await;
        let body = error.map_or_else(|| json!({}), |error| json!({ "error": error }));
        Mock::given(method("POST"))
            .and(path("/trakt/token"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let mut request = complete_request("trakt");
        request.app = Some(app());
        let failure = transport.complete(request).await.unwrap_err();
        assert_eq!(
            failure_code(&failure),
            Some(expected),
            "exchange {status} {error:?}"
        );
    }
}

#[tokio::test]
async fn an_empty_stored_refresh_credential_requires_reconnect_before_any_request() {
    let server = MockServer::start().await;
    let transport = transport(&server).await;
    let mut relay = relay_credential();
    relay.refresh_handle = Some(String::new());
    let mut direct = direct_credential();
    direct.refresh_token = Some(String::new());
    for (provider, credential, app) in [
        ("simkl", relay.clone(), None),
        ("trakt", direct.clone(), Some(app())),
        (
            "trakt",
            ListAccountCredential {
                refresh_token: None,
                ..direct
            },
            Some(app()),
        ),
        (
            "simkl",
            ListAccountCredential {
                refresh_handle: None,
                ..relay
            },
            None,
        ),
    ] {
        let error = transport
            .renew(provider, &credential, app.as_ref())
            .await
            .unwrap_err();
        assert_eq!(
            failure_code(&error),
            Some("reconnect_required"),
            "{provider}"
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// Callers parse `list account authentication failed: <code>` from these
/// errors, and the set of codes is closed. Emitting, removing or renaming a
/// code must update `AUTH_FAILURE_CODES` and every caller that reads it.
#[test]
fn auth_failure_message_format_and_code_set_are_pinned() {
    use scryer_application::lists::account_transport::{
        AUTH_FAILURE_CODES, auth_failure, rate_limited_failure,
    };
    let source = include_str!("client_list_accounts.rs");
    let source = source.split("#[cfg(test)]").next().unwrap();
    let flat = source.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut emitted = std::collections::BTreeSet::new();
    for marker in ["failure(\"", "=> Some(\"", "=> { Some(\"", "=> \""] {
        for (index, _) in flat.match_indices(marker) {
            let rest = &flat[index + marker.len()..];
            emitted.insert(rest[..rest.find('"').unwrap()].to_string());
        }
    }
    // Codes the gateway emits through named constants.
    for (constant, code) in [
        ("RECONNECT_REQUIRED", "reconnect_required"),
        ("PROVIDER_REJECTED", "provider_rejected"),
        ("rate_limited_failure(", "rate_limited"),
    ] {
        if flat.contains(constant) {
            emitted.insert(code.to_string());
        }
    }
    let declared = AUTH_FAILURE_CODES
        .iter()
        .map(|code| code.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(emitted, declared);

    for code in AUTH_FAILURE_CODES {
        let expected = format!("list account authentication failed: {code}");
        match auth_failure(code) {
            AppError::Validation(message) => assert_eq!(message, expected),
            other => panic!("unexpected {other:?}"),
        }
    }
    match rate_limited_failure(None) {
        AppError::TemporaryUnavailable { message, .. } => {
            assert_eq!(message, "list account authentication failed: rate_limited")
        }
        other => panic!("unexpected {other:?}"),
    }
}
