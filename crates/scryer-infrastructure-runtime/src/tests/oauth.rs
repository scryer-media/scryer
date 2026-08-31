use super::*;

#[tokio::test]
async fn revoke_authless_refresh_grants_revokes_only_authless_grants_and_tokens() {
    let (services, db) = temp_services("scryer_oauth_revoke_authless").await;
    let oauth = oauth_store(&services);
    let user = UserRepository::list_all(&user_store(&services))
        .await
        .expect("users should load")
        .into_iter()
        .next()
        .expect("default user should exist");
    let now = Utc::now();

    create_grant_with_token(
        &oauth,
        &user.id,
        "grant-authless-active",
        "family-authless-active",
        scryer_application::OAuthAuthorizationSource::Authless,
        None,
    )
    .await;
    create_grant_with_token(
        &oauth,
        &user.id,
        "grant-authless-already-revoked",
        "family-authless-already-revoked",
        scryer_application::OAuthAuthorizationSource::Authless,
        Some((now - chrono::Duration::minutes(5), "manual_revoked")),
    )
    .await;
    create_grant_with_token(
        &oauth,
        &user.id,
        "grant-authenticated-active",
        "family-authenticated-active",
        scryer_application::OAuthAuthorizationSource::Authenticated,
        None,
    )
    .await;

    let revoked_rows = oauth
        .revoke_authless_refresh_grants(now, "form_login_enabled")
        .await
        .expect("authless grants should revoke");

    assert_eq!(revoked_rows, 1);
    let authless_revoked_grants: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM oauth_refresh_grants
          WHERE authorization_source = 'authless'
            AND revoked_at IS NOT NULL",
    )
    .fetch_one(services.pool())
    .await
    .expect("authless revoked grants should count");
    assert_eq!(authless_revoked_grants, 2);

    let form_login_revoked_grants: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM oauth_refresh_grants
          WHERE authorization_source = 'authless'
            AND revoked_reason = 'form_login_enabled'",
    )
    .fetch_one(services.pool())
    .await
    .expect("form-login revoked grants should count");
    assert_eq!(form_login_revoked_grants, 1);

    let authless_revoked_tokens: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM oauth_refresh_tokens
          WHERE grant_id IN ('grant-authless-active', 'grant-authless-already-revoked')
            AND revoked_at IS NOT NULL",
    )
    .fetch_one(services.pool())
    .await
    .expect("authless tokens should count");
    assert_eq!(authless_revoked_tokens, 2);

    let authenticated_active_grants: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM oauth_refresh_grants
          WHERE id = 'grant-authenticated-active'
            AND revoked_at IS NULL",
    )
    .fetch_one(services.pool())
    .await
    .expect("authenticated grant should count");
    assert_eq!(authenticated_active_grants, 1);

    let authenticated_active_tokens: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM oauth_refresh_tokens
          WHERE grant_id = 'grant-authenticated-active'
            AND revoked_at IS NULL",
    )
    .fetch_one(services.pool())
    .await
    .expect("authenticated token should count");
    assert_eq!(authenticated_active_tokens, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn custom_client_grant_creation_requires_an_enabled_registration() {
    let (services, db) = temp_services("scryer_oauth_disabled_client_grant").await;
    let oauth = oauth_store(&services);
    let user = UserRepository::list_all(&user_store(&services))
        .await
        .expect("users should load")
        .into_iter()
        .next()
        .expect("default user should exist");
    let now = Utc::now();
    let client_id = "oauth-client-disabled";

    oauth
        .create_client_registration(scryer_application::OAuthClientRegistrationRecord {
            client_id: client_id.to_string(),
            display_name: "Disabled client".to_string(),
            redirect_uris: vec!["https://example.test/oauth/callback".to_string()],
            enabled: false,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("disabled client registration should insert");

    let authorization_code = scryer_application::OAuthAuthorizationCodeRecord {
        id: "code-disabled-client".to_string(),
        code_hash: "hash-code-disabled-client".to_string(),
        client_id: client_id.to_string(),
        user_id: user.id.clone(),
        auth_session_version: "1".to_string(),
        authorization_source: scryer_application::OAuthAuthorizationSource::Authenticated,
        redirect_uri: "https://example.test/oauth/callback".to_string(),
        scope: "library".to_string(),
        code_challenge: "challenge-disabled-client".to_string(),
        code_challenge_method: "S256".to_string(),
        created_at: now,
        expires_at: now + chrono::Duration::minutes(5),
        consumed_at: None,
    };
    oauth
        .create_authorization_code(authorization_code.clone())
        .await
        .expect("authorization code should insert");

    let grant = scryer_application::OAuthRefreshGrantRecord {
        id: "grant-disabled-client".to_string(),
        family_id: "family-disabled-client".to_string(),
        user_id: user.id.clone(),
        authorization_source: scryer_application::OAuthAuthorizationSource::Authenticated,
        client_id: client_id.to_string(),
        scope: "library".to_string(),
        auth_session_version: "1".to_string(),
        created_at: now,
        updated_at: now,
        last_used_at: None,
        revoked_at: None,
        revoked_reason: None,
    };
    let token = scryer_application::OAuthRefreshTokenRecord {
        id: "token-disabled-client".to_string(),
        grant_id: grant.id.clone(),
        family_id: grant.family_id.clone(),
        token_hash: "hash-disabled-client".to_string(),
        created_at: now,
        consumed_at: None,
        revoked_at: None,
    };

    let error = oauth
        .create_refresh_grant(grant, token, true)
        .await
        .expect_err("disabled client must not receive a grant");
    assert!(matches!(
        error,
        scryer_application::AppError::Unauthorized(_)
    ));

    let exchange_grant = scryer_application::OAuthRefreshGrantRecord {
        id: "grant-disabled-client-exchange".to_string(),
        family_id: "family-disabled-client-exchange".to_string(),
        user_id: user.id.clone(),
        authorization_source: scryer_application::OAuthAuthorizationSource::Authenticated,
        client_id: client_id.to_string(),
        scope: "library".to_string(),
        auth_session_version: "1".to_string(),
        created_at: now,
        updated_at: now,
        last_used_at: None,
        revoked_at: None,
        revoked_reason: None,
    };
    let exchange_token = scryer_application::OAuthRefreshTokenRecord {
        id: "token-disabled-client-exchange".to_string(),
        grant_id: exchange_grant.id.clone(),
        family_id: exchange_grant.family_id.clone(),
        token_hash: "hash-disabled-client-exchange".to_string(),
        created_at: now,
        consumed_at: None,
        revoked_at: None,
    };
    let error = oauth
        .consume_authorization_code_and_create_refresh_grant(
            authorization_code,
            now,
            exchange_grant,
            exchange_token,
            true,
        )
        .await
        .expect_err("disabled client must not consume an authorization code");
    assert!(matches!(
        error,
        scryer_application::AppError::Unauthorized(_)
    ));
    let code = oauth
        .get_authorization_code("code-disabled-client")
        .await
        .expect("authorization code should load")
        .expect("authorization code should remain");
    assert!(code.consumed_at.is_none());

    let grant_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM oauth_refresh_grants WHERE id = 'grant-disabled-client'",
    )
    .fetch_one(services.pool())
    .await
    .expect("grant count should load");
    assert_eq!(grant_count, 0);

    let exchange_grant_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM oauth_refresh_grants WHERE id = 'grant-disabled-client-exchange'",
    )
    .fetch_one(services.pool())
    .await
    .expect("exchange grant count should load");
    assert_eq!(exchange_grant_count, 0);

    let _ = std::fs::remove_file(db);
}

async fn create_grant_with_token(
    oauth: &OAuthStore,
    user_id: &str,
    grant_id: &str,
    family_id: &str,
    authorization_source: scryer_application::OAuthAuthorizationSource,
    revoked: Option<(chrono::DateTime<chrono::Utc>, &str)>,
) {
    let now = Utc::now();
    let (revoked_at, revoked_reason) = revoked
        .map(|(revoked_at, reason)| (Some(revoked_at), Some(reason.to_string())))
        .unwrap_or((None, None));
    let grant = scryer_application::OAuthRefreshGrantRecord {
        id: grant_id.to_string(),
        family_id: family_id.to_string(),
        user_id: user_id.to_string(),
        authorization_source,
        client_id: "generic-native".to_string(),
        scope: "library".to_string(),
        auth_session_version: "1".to_string(),
        created_at: now,
        updated_at: now,
        last_used_at: None,
        revoked_at,
        revoked_reason,
    };
    let token = scryer_application::OAuthRefreshTokenRecord {
        id: format!("token-{grant_id}"),
        grant_id: grant_id.to_string(),
        family_id: family_id.to_string(),
        token_hash: format!("hash-{grant_id}"),
        created_at: now,
        consumed_at: None,
        revoked_at: None,
    };

    oauth
        .create_refresh_grant(grant, token, false)
        .await
        .expect("grant should insert");
}
