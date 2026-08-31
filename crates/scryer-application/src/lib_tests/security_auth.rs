use super::*;

#[test]
fn hash_and_validate_password_round_trip() {
    let (app, _user) = bootstrap();
    let hashed = app
        .hash_password("P@ssw0rd")
        .expect("hash should be generated");
    assert!(app.validate_password_hash(&hashed).is_ok());
    assert!(
        app.validate_password("P@ssw0rd", &hashed)
            .expect("hash should be valid")
    );
    assert!(
        !app.validate_password("wrong", &hashed)
            .expect("hash should validate")
    );
}

#[test]
fn hash_version_is_explicit() {
    let (app, _user) = bootstrap();

    assert!(app.hash_password("abc").expect("hash").starts_with("v2$"));
}

#[test]
fn v1_password_hashes_are_rejected() {
    let (app, _user) = bootstrap();
    // The legacy `v1$<salt>$<sha256(salt+password)>` form is retired. Migration
    // 0191 clears surviving rows; anything still presenting one fails closed.
    let salt = "abcdef0123456789abcdef0123456789";
    let v1_hash =
        format!("v1${salt}$0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
    assert!(app.validate_password_hash(&v1_hash).is_err());
    assert!(app.validate_password("legacy-pass", &v1_hash).is_err());
}

#[test]
fn login_failure_delay_targets_stay_in_configured_ranges() {
    assert_eq!(
        AppUseCase::login_failure_delay_target_for_random(
            LoginFailureTimingClass::PasswordBackedLocal,
            0,
        ),
        Duration::from_millis(400),
    );
    assert_eq!(
        AppUseCase::login_failure_delay_target_for_random(
            LoginFailureTimingClass::PasswordBackedLocal,
            300,
        ),
        Duration::from_millis(700),
    );
    assert_eq!(
        AppUseCase::login_failure_delay_target_for_random(LoginFailureTimingClass::FastMasked, 0,),
        Duration::from_millis(400),
    );
    assert_eq!(
        AppUseCase::login_failure_delay_target_for_random(LoginFailureTimingClass::FastMasked, 300,),
        Duration::from_millis(700),
    );
}

#[test]
fn login_failure_delay_ranges_match_and_do_not_go_negative() {
    assert_eq!(
        AppUseCase::login_failure_delay_target_for_random(
            LoginFailureTimingClass::PasswordBackedLocal,
            123,
        ),
        AppUseCase::login_failure_delay_target_for_random(LoginFailureTimingClass::FastMasked, 123),
    );
    assert_eq!(
        AppUseCase::login_failure_remaining_delay_for_elapsed(
            LoginFailureTimingClass::FastMasked,
            300,
            Duration::from_millis(900),
        ),
        None,
    );
}

#[tokio::test]
async fn empty_local_login_inputs_use_masked_failure_delay() {
    let (app, _) = bootstrap();
    let started = std::time::Instant::now();

    let result = app.authenticate_credentials("", "s3cr3t!!").await;

    assert!(result.is_err());
    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "empty credential failure returned before the masked delay band"
    );
}

#[tokio::test]
async fn existing_short_password_remains_valid_after_minimum_is_raised() {
    let (app, admin) = bootstrap();
    let short_password = "short7!";
    let user = User {
        id: "existing-short-password-user".to_string(),
        username: "existing_short_password".to_string(),
        password_hash: Some(
            app.hash_password(short_password)
                .expect("hash short password"),
        ),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create short-password user");

    app.update_security_settings(
        &admin,
        UpdateSecuritySettings {
            form_login_enabled: false,
            password_min_length: 12,
            skip_login_for_local_ips: false,
            api_keys_restrict_to_system_settings_users: Some(false),
            mfa_require_config_step_up: false,
            mfa_require_password_login: false,
            totp_require_jellyfin_login: false,
            totp_require_emby_login: Some(false),
        },
    )
    .await
    .expect("raise password minimum");

    let authenticated = app
        .authenticate_credentials("existing_short_password", short_password)
        .await
        .expect("authenticate existing short password");
    assert_eq!(authenticated.id, user.id);
}

#[tokio::test]
async fn security_settings_read_legacy_totp_mfa_keys_when_new_keys_are_unset() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let settings_handle = settings.clone();
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            settings::keys::LEGACY_TOTP_REQUIRE_CONFIG_STEP_UP_KEY,
            "true",
        )
        .await;
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            settings::keys::LEGACY_TOTP_REQUIRE_PASSWORD_LOGIN_KEY,
            "true",
        )
        .await;
    let (app, _) = bootstrap_with_settings_repo_and_profiles(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );

    let loaded = app
        .security_settings()
        .await
        .expect("load security settings");

    assert!(loaded.mfa_require_config_step_up);
    assert!(loaded.mfa_require_password_login);
    assert_eq!(
        settings_handle
            .get_value(
                SETTINGS_SCOPE_SYSTEM,
                settings::keys::MFA_REQUIRE_CONFIG_STEP_UP_KEY,
            )
            .await
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        settings_handle
            .get_value(
                SETTINGS_SCOPE_SYSTEM,
                settings::keys::MFA_REQUIRE_PASSWORD_LOGIN_KEY,
            )
            .await
            .as_deref(),
        Some("true")
    );
}

#[tokio::test]
async fn emby_totp_requirement_round_trips_through_settings_values() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let settings_handle = settings.clone();
    let (app, admin) = bootstrap_with_settings_repo_and_profiles(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );

    app.update_security_settings(
        &admin,
        UpdateSecuritySettings {
            form_login_enabled: false,
            password_min_length: 8,
            skip_login_for_local_ips: false,
            api_keys_restrict_to_system_settings_users: Some(false),
            mfa_require_config_step_up: false,
            mfa_require_password_login: false,
            totp_require_jellyfin_login: false,
            totp_require_emby_login: Some(true),
        },
    )
    .await
    .expect("save Emby TOTP setting");

    assert_eq!(
        settings_handle
            .get_value(
                SETTINGS_SCOPE_SYSTEM,
                settings::keys::TOTP_REQUIRE_EMBY_LOGIN_KEY,
            )
            .await
            .as_deref(),
        Some("true")
    );
    assert!(
        app.security_settings()
            .await
            .expect("reload security settings")
            .totp_require_emby_login
    );
}

#[tokio::test]
async fn security_settings_do_not_overwrite_new_mfa_keys_with_legacy_values() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let settings_handle = settings.clone();
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            settings::keys::MFA_REQUIRE_CONFIG_STEP_UP_KEY,
            "false",
        )
        .await;
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            settings::keys::LEGACY_TOTP_REQUIRE_CONFIG_STEP_UP_KEY,
            "true",
        )
        .await;
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            settings::keys::MFA_REQUIRE_PASSWORD_LOGIN_KEY,
            "false",
        )
        .await;
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            settings::keys::LEGACY_TOTP_REQUIRE_PASSWORD_LOGIN_KEY,
            "true",
        )
        .await;
    let (app, _) = bootstrap_with_settings_repo_and_profiles(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );

    let loaded = app
        .security_settings()
        .await
        .expect("load security settings");

    assert!(!loaded.mfa_require_config_step_up);
    assert!(!loaded.mfa_require_password_login);
    assert_eq!(
        settings_handle
            .get_value(
                SETTINGS_SCOPE_SYSTEM,
                settings::keys::MFA_REQUIRE_CONFIG_STEP_UP_KEY,
            )
            .await
            .as_deref(),
        Some("false")
    );
    assert_eq!(
        settings_handle
            .get_value(
                SETTINGS_SCOPE_SYSTEM,
                settings::keys::MFA_REQUIRE_PASSWORD_LOGIN_KEY,
            )
            .await
            .as_deref(),
        Some("false")
    );
}

#[tokio::test]
async fn local_password_login_requires_exact_spacing() {
    let (app, _) = bootstrap();
    let password = "  exact-pass  ";
    let user = User {
        id: "exact-spacing-login-user".to_string(),
        username: "exact_spacing_login".to_string(),
        password_hash: Some(app.hash_password(password).expect("hash spaced password")),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create spaced-password user");

    let authenticated = app
        .authenticate_credentials("exact_spacing_login", password)
        .await
        .expect("exact password should authenticate");
    assert_eq!(authenticated.id, user.id);

    let trimmed = app
        .authenticate_credentials("exact_spacing_login", password.trim())
        .await;
    assert!(trimmed.is_err(), "trimmed password must be rejected");
}

#[tokio::test]
async fn change_own_password_requires_exact_current_password_spacing() {
    let (app, _) = bootstrap();
    let old_password = "  old-pass  ";
    let user = User {
        id: "exact-current-password-user".to_string(),
        username: "exact_current_password".to_string(),
        password_hash: Some(app.hash_password(old_password).expect("hash old password")),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    let user = app
        .services
        .identity
        .users
        .create(user)
        .await
        .expect("create exact-current-password user");

    let trimmed_current = app
        .change_own_password(
            &user,
            "new-pass-1".to_string(),
            old_password.trim().to_string(),
        )
        .await;
    assert!(
        trimmed_current.is_err(),
        "trimmed current password must be rejected"
    );

    let changed = app
        .change_own_password(&user, "new-pass-1".to_string(), old_password.to_string())
        .await
        .expect("exact current password should succeed");
    assert!(
        app.validate_password(
            "new-pass-1",
            changed.password_hash.as_deref().expect("new password hash")
        )
        .expect("new password should validate")
    );
}

// ── password edge cases ───────────────────────────────────────────────────

#[test]
fn hash_password_empty_returns_error() {
    let (app, _) = bootstrap();
    assert!(app.hash_password("").is_err());
}

#[test]
fn hash_password_preserves_password_spacing() {
    let (app, _) = bootstrap();
    let password = "  P@ssw0rd  ";
    let hash = app.hash_password(password).expect("hash password");

    assert!(
        app.validate_password(password, &hash)
            .expect("exact password should validate")
    );
    assert!(
        !app.validate_password(password.trim(), &hash)
            .expect("trimmed password should be rejected")
    );
}

#[test]
fn validate_password_v1_malformed_no_salt_separator() {
    let (app, _) = bootstrap();
    // Only "v1" prefix, no $ after it
    let bad_hash = "v1nope";
    assert!(app.validate_password_hash(bad_hash).is_err());
    let result = app.validate_password("anything", bad_hash);
    assert!(
        result.is_err(),
        "malformed v1 hash (no $) should return Err"
    );
}

#[test]
fn validate_password_v1_malformed_no_hash_component() {
    let (app, _) = bootstrap();
    // Has v1$salt but no third segment
    let bad_hash = "v1$somesalt";
    assert!(app.validate_password_hash(bad_hash).is_err());
    let result = app.validate_password("anything", bad_hash);
    assert!(
        result.is_err(),
        "malformed v1 hash (no hash segment) should return Err"
    );
}

#[test]
fn validate_password_unknown_version_returns_error() {
    let (app, _) = bootstrap();
    assert!(app.validate_password_hash("v99$somehash").is_err());
    let result = app.validate_password("pass", "v99$somehash");
    assert!(result.is_err(), "unknown hash version should return Err");
}

// ── JWT round-trip ────────────────────────────────────────────────────────

#[tokio::test]
async fn issue_and_authenticate_token_round_trips() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-1".to_string(),
        username: "jwt_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    let token = app.issue_access_token(&user).await.expect("issue token");
    let decoded = app
        .authenticate_token(&token)
        .await
        .expect("authenticate token");
    assert_eq!(decoded.id, user.id);
    assert_eq!(decoded.username, user.username);
}

#[tokio::test]
async fn token_signed_without_auth_session_version_authenticates() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-no-session-version".to_string(),
        username: "jwt_no_session_version".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    let claims = JwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() + 3600,
        iat: Utc::now().timestamp(),
        iss: app.auth.issuer.clone(),
        username: user.username.clone(),
        app_permissions: vec![],
        library_permissions: vec![],
        mfa_verified_until: None,
        mfa_step_up_verified_until: None,
        security_action_verified_until: None,
        actor_capabilities: vec![],
        oauth_client_id: None,
        oauth_grant_id: None,
        oauth_authorization_source: crate::types::OAuthAuthorizationSource::Authenticated,
        auth_scope: JwtSessionScope::Full,
        persist_session: false,
        auth_session_version: None,
        password_change_required_after_enrollment: false,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let token = jsonwebtoken::encode(&header, &claims, &key).expect("encode token");

    let decoded = app
        .authenticate_token(&token)
        .await
        .expect("token without auth session version should authenticate");
    assert_eq!(decoded.id, user.id);
}

#[tokio::test]
async fn issue_mfa_enrollment_token_sets_enrollment_scope() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-mfa-enroll".to_string(),
        username: "jwt_mfa_enroll".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();

    let token = app
        .issue_mfa_enrollment_token(&user, false, false, None)
        .await
        .expect("issue enrollment token");
    let (decoded, claims) = app
        .authenticate_token_with_claims(&token)
        .await
        .expect("authenticate enrollment token");

    assert_eq!(decoded.id, user.id);
    assert_eq!(claims.session_scope, JwtSessionScope::MfaEnrollment);
    assert_eq!(claims.mfa_verified_until, None);
    assert_eq!(claims.mfa_step_up_verified_until, None);
    assert!(!claims.persist_session);

    let persistent_token = app
        .issue_mfa_enrollment_token(&user, true, false, None)
        .await
        .expect("issue persistent enrollment token");
    let (_, persistent_claims) = app
        .authenticate_token_with_claims(&persistent_token)
        .await
        .expect("authenticate persistent enrollment token");
    assert!(persistent_claims.persist_session);
}

#[tokio::test]
async fn login_mfa_claim_does_not_imply_step_up_claim() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-login-mfa".to_string(),
        username: "jwt_login_mfa".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();

    let login_mfa_until = app.mfa_freshness_verified_until();
    let token = app
        .issue_access_token_with_mfa(&user, Some(login_mfa_until), None)
        .await
        .expect("issue login MFA token");
    let (_, claims) = app
        .authenticate_token_with_claims(&token)
        .await
        .expect("authenticate login MFA token");

    assert_eq!(claims.mfa_verified_until, Some(login_mfa_until.timestamp()));
    assert_eq!(claims.mfa_step_up_verified_until, None);

    let step_up_until = app.mfa_freshness_verified_until();
    let token = app
        .issue_access_token_with_mfa(&user, Some(login_mfa_until), Some(step_up_until))
        .await
        .expect("issue step-up MFA token");
    let (_, claims) = app
        .authenticate_token_with_claims(&token)
        .await
        .expect("authenticate step-up MFA token");

    assert_eq!(claims.mfa_verified_until, Some(login_mfa_until.timestamp()));
    assert_eq!(
        claims.mfa_step_up_verified_until,
        Some(step_up_until.timestamp())
    );
}

#[tokio::test]
async fn legacy_token_without_scope_claim_defaults_to_full_scope() {
    #[derive(serde::Serialize)]
    struct LegacyJwtClaims {
        sub: String,
        exp: i64,
        iat: i64,
        iss: String,
        username: String,
        #[serde(rename = "appPermissions")]
        app_permissions: Vec<String>,
        #[serde(rename = "libraryPermissions")]
        library_permissions: Vec<serde_json::Value>,
        #[serde(rename = "mfaVerifiedUntil")]
        mfa_verified_until: Option<i64>,
    }

    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-legacy-scope".to_string(),
        username: "jwt_legacy_scope".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    let claims = LegacyJwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() + 3600,
        iat: Utc::now().timestamp(),
        iss: app.auth.issuer.clone(),
        username: user.username.clone(),
        app_permissions: vec![],
        library_permissions: vec![],
        mfa_verified_until: None,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let token = jsonwebtoken::encode(&header, &claims, &key).expect("encode legacy token");

    let (decoded, token_claims) = app
        .authenticate_token_with_claims(&token)
        .await
        .expect("legacy token should authenticate");

    assert_eq!(decoded.id, user.id);
    assert_eq!(token_claims.session_scope, JwtSessionScope::Full);
}

#[tokio::test]
async fn permission_claims_survive_token_round_trip() {
    let (app, admin) = bootstrap();
    let user = create_user_with_permissions(
        &app,
        &admin,
        "permission_claims_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
            TestPermissionPreset::UserManagement,
        ],
    )
    .await
    .expect("create user");
    let token = app.issue_access_token(&user).await.expect("issue token");
    let decoded =
        jsonwebtoken::dangerous::insecure_decode::<JwtClaims>(&token).expect("token should decode");
    assert!(
        decoded
            .claims
            .app_permissions
            .contains(&"manageUsers".to_string())
    );
    assert!(
        decoded
            .claims
            .app_permissions
            .contains(&"managePermissions".to_string())
    );
    assert!(decoded.claims.library_permissions.iter().any(|grant| {
        grant.permissions.contains(&"view".to_string())
            && grant.permissions.contains(&"manageTitles".to_string())
    }));
}

#[tokio::test]
async fn oauth_access_token_omits_app_permissions_and_actor_capabilities() {
    let (app, admin) = bootstrap();
    let user = create_user_with_permissions(
        &app,
        &admin,
        "oauth_admin_claims",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
            TestPermissionPreset::UserManagement,
            TestPermissionPreset::ConfigManagement,
        ],
    )
    .await
    .expect("create user");

    let token = app
        .issue_oauth_access_token(&user, "generic-native", "grant-oauth-admin-claims")
        .await
        .expect("issue OAuth access token");
    let decoded =
        jsonwebtoken::dangerous::insecure_decode::<JwtClaims>(&token).expect("token should decode");

    assert!(decoded.claims.app_permissions.is_empty());
    assert!(decoded.claims.actor_capabilities.is_empty());
    assert_eq!(
        decoded.claims.oauth_client_id.as_deref(),
        Some("generic-native")
    );
    assert_eq!(
        decoded.claims.oauth_grant_id.as_deref(),
        Some("grant-oauth-admin-claims")
    );
    assert_eq!(
        decoded.claims.oauth_authorization_source,
        crate::OAuthAuthorizationSource::Authenticated
    );

    let (_, token_claims) = app
        .authenticate_token_with_claims(&token)
        .await
        .expect("authenticate OAuth token");
    assert!(token_claims.is_oauth_access_token());
    assert_eq!(
        token_claims.oauth_authorization_source,
        crate::OAuthAuthorizationSource::Authenticated
    );
    assert!(token_claims.actor_capabilities.is_empty());
}

#[tokio::test]
async fn authless_oauth_access_token_carries_authless_source_without_app_permissions() {
    let (app, admin) = bootstrap();
    let user = create_user_with_permissions(
        &app,
        &admin,
        "authless_oauth_claims",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
            TestPermissionPreset::UserManagement,
            TestPermissionPreset::ConfigManagement,
        ],
    )
    .await
    .expect("create user");

    let token = app
        .issue_oauth_access_token_with_source(
            &user,
            "generic-native",
            "grant-authless-claims",
            crate::OAuthAuthorizationSource::Authless,
        )
        .await
        .expect("issue authless OAuth access token");
    let decoded =
        jsonwebtoken::dangerous::insecure_decode::<JwtClaims>(&token).expect("token should decode");

    assert!(decoded.claims.app_permissions.is_empty());
    assert!(decoded.claims.actor_capabilities.is_empty());
    assert_eq!(
        decoded.claims.oauth_authorization_source,
        crate::OAuthAuthorizationSource::Authless
    );

    let (_, token_claims) = app
        .authenticate_token_with_claims(&token)
        .await
        .expect("authenticate authless OAuth token");
    assert!(token_claims.is_oauth_access_token());
    assert_eq!(
        token_claims.oauth_authorization_source,
        crate::OAuthAuthorizationSource::Authless
    );
    assert!(token_claims.actor_capabilities.is_empty());
}

#[tokio::test]
async fn oauth_redirect_validation_and_code_exchange_reject_fragments() {
    let (app, _) = bootstrap();
    let redirect_uri = "http://127.0.0.1:49152/callback";
    let fragment_redirect_uri = "http://127.0.0.1:49152/callback#token";
    let verifier = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~abcdef";

    app.validate_oauth_redirect_uri(OAUTH_GENERIC_NATIVE_CLIENT_ID, redirect_uri)
        .await
        .expect("fragment-free redirect should remain valid");
    match app
        .validate_oauth_redirect_uri(OAUTH_GENERIC_NATIVE_CLIENT_ID, fragment_redirect_uri)
        .await
        .expect_err("fragment-bearing redirect should be rejected")
    {
        AppError::Validation(message) => {
            assert_eq!(message, "redirect_uri must not contain a fragment");
        }
        other => panic!("expected redirect_uri validation error, got {other}"),
    }

    match app
        .exchange_oauth_authorization_code(
            OAUTH_GENERIC_NATIVE_CLIENT_ID,
            "scryer_oac_test.invalid",
            fragment_redirect_uri,
            verifier,
            true,
        )
        .await
        .expect_err("fragment-bearing token redirect should be rejected")
    {
        AppError::Validation(message) => {
            assert_eq!(message, "redirect_uri must not contain a fragment");
        }
        other => panic!("expected token redirect_uri validation error, got {other}"),
    }
}

#[tokio::test]
async fn oauth_token_with_app_permissions_is_rejected_during_authentication() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-oauth-app-permission-claim".to_string(),
        username: "oauth_app_permission_claim".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create user");
    app.ensure_jwt_signing_keys_loaded()
        .await
        .expect("seed signing key cache");

    let claims = JwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() + 3600,
        iat: Utc::now().timestamp(),
        iss: app.auth.issuer.clone(),
        username: user.username.clone(),
        app_permissions: vec!["manageSystemSettings".to_string()],
        library_permissions: vec![],
        mfa_verified_until: None,
        mfa_step_up_verified_until: None,
        security_action_verified_until: None,
        actor_capabilities: vec![],
        oauth_client_id: Some("generic-native".to_string()),
        oauth_grant_id: Some("grant-with-app-permission".to_string()),
        oauth_authorization_source: crate::types::OAuthAuthorizationSource::Authenticated,
        auth_scope: JwtSessionScope::Full,
        persist_session: false,
        auth_session_version: None,
        password_change_required_after_enrollment: false,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let token = jsonwebtoken::encode(&header, &claims, &key).expect("encode token");

    let error = app
        .authenticate_token_with_claims(&token)
        .await
        .expect_err("OAuth token with app permissions should be rejected");
    match error {
        AppError::Unauthorized(message) => {
            assert_eq!(message, "OAuth tokens cannot carry app permissions");
        }
        other => panic!("expected OAuth app-permission rejection, got {other}"),
    }
}

#[tokio::test]
async fn oauth_token_with_actor_capabilities_is_rejected_during_authentication() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-oauth-actor-capability-claim".to_string(),
        username: "oauth_actor_capability_claim".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create user");
    app.ensure_jwt_signing_keys_loaded()
        .await
        .expect("seed signing key cache");

    let claims = JwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() + 3600,
        iat: Utc::now().timestamp(),
        iss: app.auth.issuer.clone(),
        username: user.username.clone(),
        app_permissions: vec![],
        library_permissions: vec![],
        mfa_verified_until: None,
        mfa_step_up_verified_until: None,
        security_action_verified_until: None,
        actor_capabilities: vec!["manageOwnAccount".to_string()],
        oauth_client_id: Some("generic-native".to_string()),
        oauth_grant_id: Some("grant-with-actor-capability".to_string()),
        oauth_authorization_source: crate::types::OAuthAuthorizationSource::Authenticated,
        auth_scope: JwtSessionScope::Full,
        persist_session: false,
        auth_session_version: None,
        password_change_required_after_enrollment: false,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let token = jsonwebtoken::encode(&header, &claims, &key).expect("encode token");

    let error = app
        .authenticate_token_with_claims(&token)
        .await
        .expect_err("OAuth token with actor capabilities should be rejected");
    match error {
        AppError::Unauthorized(message) => {
            assert_eq!(message, "OAuth tokens cannot carry actor capabilities");
        }
        other => panic!("expected OAuth actor-capability rejection, got {other}"),
    }
}

#[tokio::test]
async fn release_candidate_token_resolves_password_without_exposing_it() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: Some("indexer-1".to_string()),
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
        source_password: Some(" release-password ".to_string()),
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-1",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");
    let payload = token.split('.').nth(1).expect("jwt payload segment");
    let payload_json = String::from_utf8(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .expect("jwt payload should decode"),
    )
    .expect("jwt payload should be utf-8");
    assert!(
        !payload_json.contains("release-password"),
        "candidate token payload must not contain archive password: {payload_json}"
    );
    let claims = jsonwebtoken::dangerous::insecure_decode::<ReleaseCandidateTokenClaims>(&token)
        .expect("candidate token should decode")
        .claims;
    assert_eq!(claims.indexer_id.as_deref(), Some("indexer-1"));
    assert!(claims.password_ref.is_some());
    let decoded = app
        .verify_release_candidate_token(
            &authenticated_user,
            "title-1",
            &SubmissionScope::Title,
            &token,
        )
        .await
        .expect("candidate token should verify");

    assert_eq!(decoded.indexer_id, selection.indexer_id);
    assert_eq!(decoded.source_hint, selection.source_hint);
    assert_eq!(decoded.source_kind, selection.source_kind);
    assert_eq!(decoded.source_title, selection.source_title);
    assert_eq!(decoded.source_password.as_deref(), Some("release-password"));
}

#[tokio::test]
async fn release_candidate_token_rejects_missing_password_ticket() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_missing_ticket_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
        source_password: Some("release-password".to_string()),
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-missing-ticket",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");
    app.runtime
        .acquisition
        .release_candidate_passwords
        .lock()
        .expect("ticket store")
        .clear();

    let error = app
        .verify_release_candidate_token(
            &authenticated_user,
            "title-missing-ticket",
            &SubmissionScope::Title,
            &token,
        )
        .await
        .expect_err("missing password ticket should reject token");
    assert!(
        error
            .to_string()
            .contains("release candidate expired; search again"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn release_candidate_token_drops_placeholder_password_flags() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_placeholder_password_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
        source_password: Some("protected".to_string()),
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-placeholder-password",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");
    let claims = jsonwebtoken::dangerous::insecure_decode::<ReleaseCandidateTokenClaims>(&token)
        .expect("candidate token should decode")
        .claims;
    assert_eq!(claims.password_ref, None);
    let decoded = app
        .verify_release_candidate_token(
            &authenticated_user,
            "title-placeholder-password",
            &SubmissionScope::Title,
            &token,
        )
        .await
        .expect("candidate token should verify");
    assert_eq!(decoded.source_password, None);
}

#[tokio::test]
async fn release_candidate_token_round_trips_episode_set_scope() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_episode_set_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/range-pack.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.S01E01-E03.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };
    let scope = SubmissionScope::EpisodeSet {
        episode_ids: vec![
            "episode-1".to_string(),
            "episode-2".to_string(),
            "episode-3".to_string(),
        ],
    };

    let token = app
        .issue_release_candidate_token(&authenticated_user, "title-1", &scope, &selection)
        .await
        .expect("candidate token should issue");
    let (decoded, signed_scope) = app
        .verify_release_candidate_token_for_signed_scope(&authenticated_user, "title-1", &token)
        .await
        .expect("candidate token should verify");

    assert_eq!(decoded.source_hint, selection.source_hint);
    assert_eq!(signed_scope, scope);
}

#[tokio::test]
async fn release_candidate_token_carries_size_and_seeder_count_to_redemption() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_seeders_user",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;
    // No indexer id, so admission accepts regardless; this is about the count
    // surviving the round trip rather than about the verdict.
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("magnet:?xt=urn:btih:abc".to_string()),
        source_kind: Some(DownloadSourceKind::MagnetUri),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
        size_bytes: Some(1_234_567_890),
        seeders: Some(42),
    };

    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-seeders",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");
    let (decoded, _) = app
        .verify_release_candidate_token_for_signed_scope(
            &authenticated_user,
            "title-seeders",
            &token,
        )
        .await
        .expect("candidate token should verify");

    assert_eq!(
        decoded.seeders,
        Some(42),
        "redemption must be able to re-judge admission from the token alone"
    );
    assert_eq!(decoded.size_bytes, Some(1_234_567_890));
    assert_eq!(
        decoded.info_hash_hint.as_deref(),
        Some("abcdef0123456789abcdef0123456789abcdef01")
    );
}

#[tokio::test]
async fn a_token_minted_before_this_feature_still_redeems() {
    // The legacy-token grace: claims without a `seeders` field deserialize as
    // unknown, and unknown is eligible. A candidate whose count was known to be
    // zero therefore slips through for the remainder of its short TTL. That is
    // a deliberate, bounded migration window — not the same thing as an indexer
    // that genuinely reports nothing.
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_legacy_token_user",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("magnet:?xt=urn:btih:def".to_string()),
        source_kind: Some(DownloadSourceKind::MagnetUri),
        source_title: Some("Example.Legacy.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-legacy",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");
    let (decoded, _) = app
        .verify_release_candidate_token_for_signed_scope(
            &authenticated_user,
            "title-legacy",
            &token,
        )
        .await
        .expect("a token without a seeder count must still redeem");

    assert_eq!(decoded.seeders, None);
    assert_eq!(decoded.info_hash_hint, None);
}

#[tokio::test]
async fn release_candidate_token_rejects_tampering() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_user_2",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-2",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");
    let tampered = format!("{token}x");

    assert!(
        app.verify_release_candidate_token(
            &authenticated_user,
            "title-2",
            &SubmissionScope::Title,
            &tampered,
        )
        .await
        .is_err(),
        "tampered token should be rejected"
    );
}

#[tokio::test]
async fn release_candidate_token_rejects_actor_title_and_scope_mismatch() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_user_3",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;
    let (_other_created, other_authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_user_4",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-3",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");

    assert!(
        app.verify_release_candidate_token(
            &other_authenticated_user,
            "title-3",
            &SubmissionScope::Title,
            &token,
        )
        .await
        .is_err(),
        "actor mismatch should be rejected"
    );
    assert!(
        app.verify_release_candidate_token(
            &authenticated_user,
            "other-title",
            &SubmissionScope::Title,
            &token,
        )
        .await
        .is_err(),
        "title mismatch should be rejected"
    );
    assert!(
        app.verify_release_candidate_token(
            &authenticated_user,
            "title-3",
            &SubmissionScope::Episode {
                episode_id: "episode-1".to_string(),
            },
            &token,
        )
        .await
        .is_err(),
        "scope mismatch should be rejected"
    );
}

#[tokio::test]
async fn release_candidate_token_is_invalidated_by_password_rotation() {
    let (app, admin) = bootstrap();
    let (created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "candidate_pw_rotate",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };
    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-password-rotate",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");

    app.set_user_password(&admin, &created.id, "after-pass".to_string())
        .await
        .expect("rotate password");

    let result = app
        .verify_release_candidate_token(
            &authenticated_user,
            "title-password-rotate",
            &SubmissionScope::Title,
            &token,
        )
        .await;
    assert!(
        result.is_err(),
        "candidate token should be rejected after password rotation"
    );
}

#[tokio::test]
async fn release_candidate_token_is_invalidated_by_permission_change() {
    let (app, admin) = bootstrap();
    let (created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "candidate_permission_rotate",
        "same-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };
    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-permission-rotate",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");

    let grants = test_library_grants_from_presets(&[
        TestPermissionPreset::CatalogView,
        TestPermissionPreset::TitleManagement,
    ]);
    app.set_user_library_permissions(&admin, &created.id, grants)
        .await
        .expect("update permissions");

    let result = app
        .verify_release_candidate_token(
            &authenticated_user,
            "title-permission-rotate",
            &SubmissionScope::Title,
            &token,
        )
        .await;
    assert!(
        result.is_err(),
        "candidate token should be rejected after permission change"
    );
}

#[tokio::test]
async fn backup_download_token_round_trips() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "backup_download_user",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    let ticket = app
        .issue_backup_download_token(&authenticated_user, "backup_20260515_abcd1234.tar.zst")
        .await
        .expect("backup download token should issue");

    app.verify_backup_download_token(
        &authenticated_user,
        "backup_20260515_abcd1234.tar.zst",
        &ticket.token,
    )
    .await
    .expect("backup download token should verify");
}

#[tokio::test]
async fn backup_download_token_rejects_tampering_and_filename_mismatch() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "backup_download_user_2",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    let ticket = app
        .issue_backup_download_token(&authenticated_user, "backup_20260515_abcd1234.tar.zst")
        .await
        .expect("backup download token should issue");

    assert!(
        app.verify_backup_download_token(
            &authenticated_user,
            "backup_20260515_different.tar.zst",
            &ticket.token,
        )
        .await
        .is_err(),
        "filename mismatch should be rejected"
    );

    let tampered = format!("{}x", ticket.token);
    assert!(
        app.verify_backup_download_token(
            &authenticated_user,
            "backup_20260515_abcd1234.tar.zst",
            &tampered,
        )
        .await
        .is_err(),
        "tampered token should be rejected"
    );
}

#[tokio::test]
async fn backup_download_token_rejects_wrong_kind_and_expired_claims() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "backup_download_user_3",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    let signing_key = app
        .backup_download_signing_key_for_actor(&authenticated_user)
        .await
        .expect("signing key should resolve");
    let now = Utc::now();
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);

    let wrong_kind = crate::types::BackupDownloadTokenClaims {
        sub: authenticated_user.id.clone(),
        exp: (now + chrono::Duration::minutes(5)).timestamp(),
        iat: now.timestamp(),
        iss: app.auth.issuer.clone(),
        kind: "wrong_backup_kind".to_string(),
        filename: "backup_20260515_abcd1234.tar.zst".to_string(),
    };
    let wrong_kind_token =
        jsonwebtoken::encode(&header, &wrong_kind, &key).expect("wrong kind token should encode");
    assert!(
        app.verify_backup_download_token(
            &authenticated_user,
            "backup_20260515_abcd1234.tar.zst",
            &wrong_kind_token,
        )
        .await
        .is_err(),
        "wrong kind token should be rejected"
    );

    let expired = crate::types::BackupDownloadTokenClaims {
        sub: authenticated_user.id.clone(),
        exp: (now - chrono::Duration::minutes(5)).timestamp(),
        iat: (now - chrono::Duration::minutes(10)).timestamp(),
        iss: app.auth.issuer.clone(),
        kind: "backup_download_v1".to_string(),
        filename: "backup_20260515_abcd1234.tar.zst".to_string(),
    };
    let expired_token =
        jsonwebtoken::encode(&header, &expired, &key).expect("expired token should encode");
    assert!(
        app.verify_backup_download_token(
            &authenticated_user,
            "backup_20260515_abcd1234.tar.zst",
            &expired_token,
        )
        .await
        .is_err(),
        "expired token should be rejected"
    );
}

#[tokio::test]
async fn backup_download_token_is_invalidated_by_permission_change() {
    let (app, admin) = bootstrap();
    let (created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "backup_download_permission_rotate",
        "same-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    let ticket = app
        .issue_backup_download_token(
            &authenticated_user,
            "backup_20260515_permission_rotate.tar.zst",
        )
        .await
        .expect("backup download token should issue");

    let grants = test_library_grants_from_presets(&[
        TestPermissionPreset::CatalogView,
        TestPermissionPreset::TitleManagement,
    ]);
    app.set_user_library_permissions(&admin, &created.id, grants)
        .await
        .expect("update permissions");

    let result = app
        .verify_backup_download_token(
            &authenticated_user,
            "backup_20260515_permission_rotate.tar.zst",
            &ticket.token,
        )
        .await;
    assert!(
        result.is_err(),
        "backup download token should be rejected after permission change"
    );
}

#[tokio::test]
async fn expired_token_returns_unauthorized() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-3".to_string(),
        username: "exp_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    // Encode a token with an exp 100 seconds in the past
    let claims = JwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() - 100,
        iat: Utc::now().timestamp() - 200,
        iss: app.auth.issuer.clone(),
        username: user.username.clone(),
        app_permissions: vec![],
        library_permissions: vec![],
        mfa_verified_until: None,
        mfa_step_up_verified_until: None,
        security_action_verified_until: None,
        actor_capabilities: vec![],
        oauth_client_id: None,
        oauth_grant_id: None,
        oauth_authorization_source: crate::types::OAuthAuthorizationSource::Authenticated,
        auth_scope: JwtSessionScope::Full,
        persist_session: false,
        auth_session_version: None,
        password_change_required_after_enrollment: false,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let expired_token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");
    let result = app.authenticate_token(&expired_token).await;
    assert!(result.is_err(), "expired token should be rejected");
}

#[tokio::test]
async fn wrong_issuer_token_returns_unauthorized() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-4".to_string(),
        username: "iss_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    let claims = JwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() + 3600,
        iat: Utc::now().timestamp(),
        iss: "wrong-issuer".to_string(),
        username: user.username.clone(),
        app_permissions: vec![],
        library_permissions: vec![],
        mfa_verified_until: None,
        mfa_step_up_verified_until: None,
        security_action_verified_until: None,
        actor_capabilities: vec![],
        oauth_client_id: None,
        oauth_grant_id: None,
        oauth_authorization_source: crate::types::OAuthAuthorizationSource::Authenticated,
        auth_scope: JwtSessionScope::Full,
        persist_session: false,
        auth_session_version: None,
        password_change_required_after_enrollment: false,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let bad_token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");
    let result = app.authenticate_token(&bad_token).await;
    assert!(
        result.is_err(),
        "token with wrong issuer should be rejected"
    );
}

#[tokio::test]
async fn authenticate_token_uses_cached_signing_key_and_loads_current_user() {
    let users = Arc::new(MockUserRepo::default());
    let (app, _) = bootstrap_with_user_repo(users.clone());
    let user = User {
        id: "user-jwt-cache-1".to_string(),
        username: "cache_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();

    let token = app.issue_access_token(&user).await.expect("issue token");
    app.authenticate_token(&token)
        .await
        .expect("authenticate token");
    app.authenticate_token(&token)
        .await
        .expect("authenticate token from warm cache");

    assert_eq!(users.get_by_id_call_count(), 3);
    assert_eq!(users.list_all_call_count(), 1);
}

#[tokio::test]
async fn passkey_registration_allows_external_user() {
    let users = Arc::new(MockUserRepo::default());
    let mut user = test_user_with_app_permissions("jellyfin_user", AppPermissionMask::NONE);
    user.account_kind = scryer_domain::UserAccountKind::ExternalAutoProvisioned;
    user.authorization.actor_capabilities = scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT;
    users.create(user.clone()).await.expect("create user");

    let (mut app, _) = bootstrap_with_user_repo(users);
    let origin = url::Url::parse("https://scryer.test").expect("valid WebAuthn origin");
    let webauthn = webauthn_rs::WebauthnBuilder::new("scryer.test", &origin)
        .expect("valid WebAuthn builder")
        .build()
        .expect("valid WebAuthn runtime");
    app.webauthn = services::RuntimeFeature::enabled(Arc::new(webauthn));

    let result = app.webauthn_register_start(&user, true).await;

    assert!(
        matches!(&result, Err(AppError::Repository(message)) if message == "not configured"),
        "the external user should pass eligibility checks before reaching the WebAuthn repository: {result:?}"
    );
}

#[tokio::test]
async fn passkey_registration_requires_own_account_capability() {
    let users = Arc::new(MockUserRepo::default());
    let user = test_user_with_app_permissions("passkey_unauthorized", AppPermissionMask::NONE);
    users.create(user.clone()).await.expect("create user");

    let (app, _) = bootstrap_with_user_repo(users);

    let result = app.webauthn_register_start(&user, true).await;

    assert!(matches!(result, Err(AppError::Unauthorized(_))));
}

#[tokio::test]
async fn passkey_management_requires_enabled_form_login() {
    let users = Arc::new(MockUserRepo::default());
    let user = User {
        id: "passkey-form-login-user".to_string(),
        username: "passkey_form_login".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    users.create(user.clone()).await.expect("create user");

    let (mut app, _) = bootstrap_with_user_repo(users);
    let origin = url::Url::parse("https://scryer.test").expect("valid WebAuthn origin");
    let webauthn = webauthn_rs::WebauthnBuilder::new("scryer.test", &origin)
        .expect("valid WebAuthn builder")
        .build()
        .expect("valid WebAuthn runtime");
    app.webauthn = services::RuntimeFeature::enabled(Arc::new(webauthn));

    fn assert_form_login_required<T>(result: AppResult<T>) {
        match result {
            Err(AppError::Validation(message)) => {
                assert_eq!(
                    message,
                    "passkey authentication is unavailable while form login is disabled"
                );
            }
            Err(error) => panic!("expected form-login validation error, got {error}"),
            Ok(_) => panic!("expected form-login validation error"),
        }
    }

    assert_form_login_required(app.webauthn_register_start(&user, false).await);
    assert_form_login_required(app.list_my_passkeys(&user, false).await);
    assert_form_login_required(
        app.delete_my_passkey(&user, "credential-id", false, None)
            .await,
    );
}

#[derive(Default)]
struct InMemoryWebauthnChallengeRepository {
    challenges: Mutex<HashMap<String, WebauthnChallengeRecord>>,
}

#[async_trait]
impl WebauthnRepository for InMemoryWebauthnChallengeRepository {
    async fn list_credentials_for_user(&self, _: &str) -> AppResult<Vec<WebauthnCredentialRecord>> {
        Ok(Vec::new())
    }

    async fn get_credential_by_id_for_user(
        &self,
        _: &str,
        _: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>> {
        Ok(None)
    }

    async fn get_credential_by_credential_id(
        &self,
        _: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>> {
        Ok(None)
    }

    async fn create_credential(
        &self,
        _: WebauthnCredentialRecord,
    ) -> AppResult<WebauthnCredentialRecord> {
        Err(AppError::Repository(
            "not needed for discoverable starts".into(),
        ))
    }

    async fn update_credential(
        &self,
        _: WebauthnCredentialRecord,
    ) -> AppResult<WebauthnCredentialRecord> {
        Err(AppError::Repository(
            "not needed for discoverable starts".into(),
        ))
    }

    async fn update_credential_if_current(
        &self,
        _: WebauthnCredentialRecord,
        _: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>> {
        Err(AppError::Repository(
            "not needed for discoverable starts".into(),
        ))
    }

    async fn delete_credential_for_user(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn create_challenge(
        &self,
        challenge: WebauthnChallengeRecord,
    ) -> AppResult<WebauthnChallengeRecord> {
        self.challenges
            .lock()
            .await
            .insert(challenge.id.clone(), challenge.clone());
        Ok(challenge)
    }

    async fn get_challenge(&self, id: &str) -> AppResult<Option<WebauthnChallengeRecord>> {
        Ok(self.challenges.lock().await.get(id).cloned())
    }

    async fn take_challenge(&self, id: &str) -> AppResult<Option<WebauthnChallengeRecord>> {
        Ok(self.challenges.lock().await.remove(id))
    }

    async fn delete_challenge(&self, id: &str) -> AppResult<()> {
        self.challenges.lock().await.remove(id);
        Ok(())
    }

    async fn delete_expired_challenges(&self, _: &str) -> AppResult<u64> {
        Ok(0)
    }
}

#[tokio::test]
async fn discoverable_passkey_start_does_not_enumerate_disabled_or_unknown_users() {
    let users = Arc::new(MockUserRepo::default());
    let mut disabled_user = User::with_password_hash("disabled_passkey", TEST_PASSWORD_HASH);
    disabled_user.set_login_status(scryer_domain::UserLoginStatus::Disabled);
    users
        .create(disabled_user.clone())
        .await
        .expect("create disabled user");
    let enabled_user = User::with_password_hash("enabled_passkey", TEST_PASSWORD_HASH);
    users
        .create(enabled_user.clone())
        .await
        .expect("create enabled user");

    let challenges = Arc::new(InMemoryWebauthnChallengeRepository::default());
    let (app, _) = bootstrap_with_user_repo(users);
    let app = app.with_test_overrides(|services| services.with_webauthn_store(challenges));
    let origin = url::Url::parse("https://scryer.test").expect("valid WebAuthn origin");
    let webauthn = webauthn_rs::WebauthnBuilder::new("scryer.test", &origin)
        .expect("valid WebAuthn builder")
        .build()
        .expect("valid WebAuthn runtime");
    let mut app = app;
    app.webauthn = services::RuntimeFeature::enabled(Arc::new(webauthn));

    for username in [
        disabled_user.username.as_str(),
        "unknown_passkey_user",
        enabled_user.username.as_str(),
    ] {
        let challenge = app
            .webauthn_authenticate_start(Some(username), true)
            .await
            .expect("discoverable authentication start should not enumerate users");
        assert!(!challenge.challenge_id.is_empty());
    }
}

#[tokio::test]
async fn password_change_invalidates_existing_token_immediately() {
    let (app, admin) = bootstrap();
    let created = create_user_with_permissions(
        &app,
        &admin,
        "pw_rotate",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");
    let token = app.issue_access_token(&created).await.expect("issue token");

    app.set_user_password(&admin, &created.id, "after-pass".to_string())
        .await
        .expect("rotate password");

    let result = app.authenticate_token(&token).await;
    assert!(
        result.is_err(),
        "old token should be rejected after password change"
    );
}

#[tokio::test]
async fn verified_old_password_cannot_continue_after_password_epoch_changes() {
    let (app, admin) = bootstrap();
    let created = create_user_with_permissions(
        &app,
        &admin,
        "password_epoch_race",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");
    let verified = app
        .authenticate_local_credentials("password_epoch_race", "before-pass")
        .await
        .expect("verify old password");

    app.set_user_password(&admin, &created.id, "after-pass".to_string())
        .await
        .expect("rotate password epoch");

    let old_login = app
        .login_verification_requirement(
            &verified.user,
            LoginVerificationMethod::LocalPassword,
            false,
            false,
            None,
            Some(&verified.auth_session_version),
        )
        .await;
    assert!(
        matches!(old_login, Err(AppError::Unauthorized(_))),
        "the paused old-password request must not issue a token or challenge"
    );
    assert!(
        app.authenticate_local_credentials("password_epoch_race", "after-pass")
            .await
            .is_ok(),
        "the replacement password should authenticate normally"
    );
}

#[tokio::test]
async fn permission_change_invalidates_existing_token_and_relogin_works() {
    let (app, admin) = bootstrap();
    let created = create_user_with_permissions(
        &app,
        &admin,
        "permission_rotate",
        "same-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");
    let old_token = app.issue_access_token(&created).await.expect("issue token");

    let grants = test_library_grants_from_presets(&[
        TestPermissionPreset::CatalogView,
        TestPermissionPreset::TitleManagement,
    ]);
    let updated = app
        .set_user_library_permissions(&admin, &created.id, grants)
        .await
        .expect("update permissions");

    let old_result = app.authenticate_token(&old_token).await;
    assert!(
        old_result.is_err(),
        "old token should be rejected after permission change"
    );

    let relogged = app
        .authenticate_credentials("permission_rotate", "same-pass")
        .await
        .expect("re-login after permission change");
    let new_token = app
        .issue_access_token(&relogged)
        .await
        .expect("issue refreshed token");
    let decoded = app
        .authenticate_token(&new_token)
        .await
        .expect("authenticate refreshed token");

    assert_eq!(decoded.id, updated.id);
    let authorization = app
        .load_user_authorization(&decoded)
        .await
        .expect("load authorization");
    assert!(
        authorization.has_any_library_permission(scryer_domain::LibraryPermission::ManageTitles)
    );
}

#[tokio::test]
async fn deleting_user_invalidates_existing_token_immediately() {
    let (app, admin) = bootstrap();
    let created = create_user_with_permissions(
        &app,
        &admin,
        "gone_user",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");
    let token = app.issue_access_token(&created).await.expect("issue token");

    app.delete_user(&admin, &created.id)
        .await
        .expect("delete user");

    let result = app.authenticate_token(&token).await;
    assert!(result.is_err(), "deleted user token should be rejected");
}

#[test]
fn jwt_key_derivation_is_stable_across_permission_order() {
    let (app, _) = bootstrap();
    let key_a = test_derive_jwt_key(
        &app.auth.jwt_signing_salt,
        TEST_PASSWORD_HASH,
        &[
            TestPermissionPreset::TitleManagement,
            TestPermissionPreset::CatalogView,
        ],
    );
    let key_b = test_derive_jwt_key(
        &app.auth.jwt_signing_salt,
        TEST_PASSWORD_HASH,
        &[
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    );

    assert_eq!(key_a, key_b);
}

#[tokio::test]
async fn token_permission_claims_do_not_override_database_authorization() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-malformed".to_string(),
        username: "jwt_claims".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    app.ensure_jwt_signing_keys_loaded()
        .await
        .expect("seed signing key cache");

    let claims = JwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() + 3600,
        iat: Utc::now().timestamp(),
        iss: app.auth.issuer.clone(),
        username: user.username.clone(),
        app_permissions: vec!["manageSystemSettings".to_string()],
        library_permissions: vec![],
        mfa_verified_until: None,
        mfa_step_up_verified_until: None,
        security_action_verified_until: None,
        actor_capabilities: vec![],
        oauth_client_id: None,
        oauth_grant_id: None,
        oauth_authorization_source: crate::types::OAuthAuthorizationSource::Authenticated,
        auth_scope: JwtSessionScope::Full,
        persist_session: false,
        auth_session_version: None,
        password_change_required_after_enrollment: false,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");

    let authenticated = app
        .authenticate_token(&token)
        .await
        .expect("token identity should authenticate from DB permissions");
    assert_eq!(authenticated.id, user.id);
}
