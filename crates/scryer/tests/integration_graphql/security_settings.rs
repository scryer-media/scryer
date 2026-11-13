use super::*;

#[tokio::test]
async fn graphql_typed_security_settings_defaults() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let body = schema_exec(
        &ctx,
        r#"
        query SecuritySettings {
          securitySettings {
            formLoginEnabled
            passwordMinLength
            skipLoginForLocalIps
            mfaRequirePasswordLogin
            totpRequireEmbyLogin
            effectiveFormLoginEnabled
            envOverrideActive
            envOverrideDescription
          }
        }
        "#,
        Some(admin),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(body["data"]["securitySettings"]["formLoginEnabled"], false);
    assert_eq!(body["data"]["securitySettings"]["passwordMinLength"], 8);
    assert_eq!(
        body["data"]["securitySettings"]["skipLoginForLocalIps"],
        false
    );
    assert_eq!(
        body["data"]["securitySettings"]["mfaRequirePasswordLogin"],
        false
    );
    assert_eq!(
        body["data"]["securitySettings"]["totpRequireEmbyLogin"],
        false
    );
    assert_eq!(
        body["data"]["securitySettings"]["effectiveFormLoginEnabled"],
        false
    );
    assert_eq!(body["data"]["securitySettings"]["envOverrideActive"], false);
    assert!(body["data"]["securitySettings"]["envOverrideDescription"].is_null());
}

#[tokio::test]
async fn graphql_oauth_client_registration_lifecycle_enforces_admin_access_and_revocation() {
    const REDIRECT_URI: &str = "https://client.example.test/oauth/callback";
    const NEW_REDIRECT_URI: &str = "https://client.example.test/oauth/new-callback";
    const CODE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CODE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let created = schema_exec(
        &ctx,
        r#"
        mutation CreateOAuthClientRegistration {
          createOauthClientRegistration(input: {
            displayName: "Example desktop client"
            redirectUris: ["https://client.example.test/oauth/callback"]
          }) {
            clientId
            displayName
            redirectUris
            enabled
            source
            kind
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&created);
    let client = &created["data"]["createOauthClientRegistration"];
    let client_id = client["clientId"]
        .as_str()
        .expect("generated OAuth client ID")
        .to_string();
    assert!(client_id.starts_with("oauth-client-"));
    assert_eq!(client["displayName"], "Example desktop client");
    assert_eq!(client["redirectUris"], json!([REDIRECT_URI]));
    assert_eq!(client["enabled"], true);
    assert_eq!(client["source"], "CUSTOM");
    // An omitted kind registers a generic application, never the Jellyfin plugin.
    assert_eq!(client["kind"], "CUSTOM");

    // The plugin identifies itself at registration; nothing downstream infers it from the callback.
    let plugin = schema_exec(
        &ctx,
        r#"
        mutation CreateJellyfinPluginOAuthClientRegistration {
          createOauthClientRegistration(input: {
            displayName: "Jellyfin Scryer plugin"
            redirectUris: ["https://jellyfin.example.test/Scryer/Auth/Callback"]
            kind: JELLYFIN_PLUGIN
          }) {
            clientId
            kind
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&plugin);
    let plugin_client_id = plugin["data"]["createOauthClientRegistration"]["clientId"]
        .as_str()
        .expect("generated plugin OAuth client ID")
        .to_string();
    assert_eq!(
        plugin["data"]["createOauthClientRegistration"]["kind"],
        "JELLYFIN_PLUGIN"
    );

    let listed = schema_exec(
        &ctx,
        r#"
        query OAuthClientRegistrations {
          oauthClientRegistrations { clientId source kind }
        }
        "#,
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&listed);
    // The stored kind survives the round trip through the registration table.
    assert!(
        listed["data"]["oauthClientRegistrations"]
            .as_array()
            .expect("OAuth client registration list")
            .iter()
            .any(|client| client["clientId"] == plugin_client_id.as_str()
                && client["kind"] == "JELLYFIN_PLUGIN")
    );
    let auth_session_version = ctx
        .app
        .current_actor_auth_session_version(&admin)
        .await
        .expect("load admin session version");
    assert!(
        listed["data"]["oauthClientRegistrations"]
            .as_array()
            .expect("OAuth client registration list")
            .iter()
            .any(|client| client["clientId"] == "generic-native" && client["source"] == "MANAGED")
    );

    let lookup = schema_exec(
        &ctx,
        &format!(
            r#"
            query OAuthAuthorizationClient {{
              oauthAuthorizationClient(
                clientId: "{client_id}"
                redirectUri: "{REDIRECT_URI}"
                scope: "library jellyfin-link"
              ) {{
                clientId
                displayName
                scope
              }}
            }}
            "#
        ),
        None,
    )
    .await;
    assert_no_errors(&lookup);
    assert_eq!(
        lookup["data"]["oauthAuthorizationClient"]["clientId"],
        client_id
    );
    assert_eq!(
        lookup["data"]["oauthAuthorizationClient"]["displayName"],
        "Example desktop client"
    );
    assert_eq!(
        lookup["data"]["oauthAuthorizationClient"]["scope"],
        "library"
    );

    let ordinary_user = ctx
        .app
        .create_user(
            &admin,
            "oauth_client_registration_denied".to_string(),
            "ordinary-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create ordinary user");
    let denied = schema_exec(
        &ctx,
        r#"
        mutation CreateOAuthClientRegistration {
          createOauthClientRegistration(input: {
            displayName: "Denied"
            redirectUris: ["https://denied.example.test/callback"]
          }) { clientId }
        }
        "#,
        Some(ordinary_user),
    )
    .await;
    assert_graphql_field_denied(&denied, "createOauthClientRegistration");

    let oauth_admin = ctx
        .app
        .create_user(
            &admin,
            "oauth_client_registration_oauth_admin".to_string(),
            "oauth-pass1".to_string(),
            AppPermissionMask::from_permissions([
                scryer_domain::AppPermission::ManageSystemSettings,
            ]),
            vec![],
        )
        .await
        .expect("create OAuth settings admin");
    let oauth_token = ctx
        .app
        .issue_oauth_access_token(&oauth_admin, "generic-native", "oauth-client-registration")
        .await
        .expect("issue OAuth settings token");
    let oauth_denied = gql_with_token(
        &ctx,
        r#"
        mutation CreateOAuthClientRegistration {
          createOauthClientRegistration(input: {
            displayName: "Denied OAuth"
            redirectUris: ["https://oauth-denied.example.test/callback"]
          }) { clientId }
        }
        "#,
        json!({}),
        &oauth_token,
    )
    .await;
    assert_graphql_field_denied(&oauth_denied, "createOauthClientRegistration");

    assert!(
        ctx.app
            .create_oauth_authorization_code(
                &admin,
                &client_id,
                "https://client.example.test/oauth/other-callback",
                scryer_application::OAUTH_LIBRARY_SCOPE,
                CODE_CHALLENGE,
                "S256",
                scryer_application::OAuthAuthorizationSource::Authenticated,
                auth_session_version.as_deref(),
            )
            .await
            .is_err()
    );

    let issued = ctx
        .app
        .create_oauth_authorization_code(
            &admin,
            &client_id,
            REDIRECT_URI,
            scryer_application::OAUTH_LIBRARY_SCOPE,
            CODE_CHALLENGE,
            "S256",
            scryer_application::OAuthAuthorizationSource::Authenticated,
            auth_session_version.as_deref(),
        )
        .await
        .expect("create custom client authorization code");
    let tokens = ctx
        .app
        .exchange_oauth_authorization_code(
            &client_id,
            &issued.code,
            REDIRECT_URI,
            CODE_VERIFIER,
            true,
        )
        .await
        .expect("exchange custom client authorization code");
    let grant_id = ctx
        .app
        .list_oauth_connected_apps(&admin)
        .await
        .expect("list custom client grants")
        .into_iter()
        .find(|grant| grant.client_id == client_id)
        .expect("custom client refresh grant")
        .grant_id;
    ctx.app
        .validate_oauth_access_token(&client_id, &grant_id)
        .await
        .expect("custom client grant remains active");
    let refreshed = ctx
        .app
        .refresh_oauth_token(&client_id, &tokens.refresh_token, true)
        .await
        .expect("refresh custom client OAuth token");
    let (_, refreshed_claims) = ctx
        .app
        .authenticate_token_with_claims(&refreshed.access_token)
        .await
        .expect("authenticate refreshed OAuth access token");
    assert_eq!(
        refreshed_claims.oauth_client_id.as_deref(),
        Some(client_id.as_str())
    );
    assert_eq!(
        refreshed_claims.oauth_grant_id.as_deref(),
        Some(grant_id.as_str())
    );
    ctx.app
        .validate_oauth_access_token(&client_id, &grant_id)
        .await
        .expect("refreshed OAuth access token is active");

    let redirect_changed = schema_exec(
        &ctx,
        &format!(
            r#"
            mutation ChangeOAuthClientRedirectUris {{
              updateOauthClientRegistration(
                clientId: "{client_id}"
                input: {{
                  displayName: "Example desktop client"
                  redirectUris: ["{NEW_REDIRECT_URI}"]
                  enabled: true
                }}
              ) {{ enabled redirectUris }}
            }}
            "#
        ),
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&redirect_changed);
    assert_eq!(
        redirect_changed["data"]["updateOauthClientRegistration"]["redirectUris"],
        json!([NEW_REDIRECT_URI]),
        "the committed registration must expose only the replacement allowlist"
    );
    assert!(
        ctx.app
            .validate_oauth_access_token(&client_id, &grant_id)
            .await
            .is_err(),
        "changing a client's redirect allowlist must revoke its active grants"
    );
    assert!(
        ctx.app
            .refresh_oauth_token(&client_id, &refreshed.refresh_token, true)
            .await
            .is_err(),
        "a redirect allowlist change must invalidate the refresh-token family"
    );
    assert!(
        ctx.app
            .create_oauth_authorization_code(
                &admin,
                &client_id,
                REDIRECT_URI,
                scryer_application::OAUTH_LIBRARY_SCOPE,
                CODE_CHALLENGE,
                "S256",
                scryer_application::OAuthAuthorizationSource::Authenticated,
                auth_session_version.as_deref(),
            )
            .await
            .is_err(),
        "the replaced redirect URI must be rejected after the old grant family is revoked"
    );
    let replacement_issued = ctx
        .app
        .create_oauth_authorization_code(
            &admin,
            &client_id,
            NEW_REDIRECT_URI,
            scryer_application::OAUTH_LIBRARY_SCOPE,
            CODE_CHALLENGE,
            "S256",
            scryer_application::OAuthAuthorizationSource::Authenticated,
            auth_session_version.as_deref(),
        )
        .await
        .expect("the replacement redirect URI should issue a new grant after the update commits");
    let replacement_tokens = ctx
        .app
        .exchange_oauth_authorization_code(
            &client_id,
            &replacement_issued.code,
            NEW_REDIRECT_URI,
            CODE_VERIFIER,
            true,
        )
        .await
        .expect("the replacement redirect URI should exchange after the update commits");
    let (_, replacement_claims) = ctx
        .app
        .authenticate_token_with_claims(&replacement_tokens.access_token)
        .await
        .expect("replacement access token should authenticate");
    let replacement_grant_id = replacement_claims
        .oauth_grant_id
        .expect("replacement OAuth token should carry its grant");
    ctx.app
        .validate_oauth_access_token(&client_id, &replacement_grant_id)
        .await
        .expect("replacement grant must be active after redirect-update revocation");

    let disabled = schema_exec(
        &ctx,
        &format!(
            r#"
            mutation DisableOAuthClientRegistration {{
              updateOauthClientRegistration(
                clientId: "{client_id}"
                input: {{
                  displayName: "Example desktop client"
                  redirectUris: ["{REDIRECT_URI}"]
                  enabled: false
                }}
              ) {{ enabled }}
            }}
            "#
        ),
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&disabled);
    assert_eq!(
        disabled["data"]["updateOauthClientRegistration"]["enabled"],
        false
    );
    assert!(
        ctx.app
            .validate_oauth_access_token(&client_id, &replacement_grant_id)
            .await
            .is_err()
    );
    assert!(
        ctx.app
            .refresh_oauth_token(&client_id, &replacement_tokens.refresh_token, true)
            .await
            .is_err()
    );

    let deleted = schema_exec(
        &ctx,
        &format!(
            r#"
            mutation DeleteOAuthClientRegistration {{
              deleteOauthClientRegistration(clientId: "{client_id}") {{
                clientId
                deleted
              }}
            }}
            "#
        ),
        Some(admin),
    )
    .await;
    assert_no_errors(&deleted);
    assert_eq!(
        deleted["data"]["deleteOauthClientRegistration"]["clientId"],
        client_id
    );
    assert_eq!(
        deleted["data"]["deleteOauthClientRegistration"]["deleted"],
        true
    );
    assert!(
        ctx.app
            .oauth_client_info(&client_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn graphql_security_settings_omitted_emby_requirement_preserves_saved_value() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let enable = schema_exec(
        &ctx,
        r#"
        mutation EnableEmbyTotp {
          updateSecuritySettings(input: {
            formLoginEnabled: false
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
            totpRequireEmbyLogin: true
          }) {
            totpRequireEmbyLogin
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&enable);
    assert_eq!(
        enable["data"]["updateSecuritySettings"]["totpRequireEmbyLogin"],
        true
    );

    let omitted = schema_exec(
        &ctx,
        r#"
        mutation PreserveEmbyTotp {
          updateSecuritySettings(input: {
            formLoginEnabled: false
            passwordMinLength: 9
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
          }) {
            passwordMinLength
            totpRequireEmbyLogin
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&omitted);
    assert_eq!(
        omitted["data"]["updateSecuritySettings"]["passwordMinLength"],
        9
    );
    assert_eq!(
        omitted["data"]["updateSecuritySettings"]["totpRequireEmbyLogin"],
        true
    );

    let disable = schema_exec(
        &ctx,
        r#"
        mutation DisableEmbyTotp {
          updateSecuritySettings(input: {
            formLoginEnabled: false
            passwordMinLength: 9
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
            totpRequireEmbyLogin: false
          }) {
            totpRequireEmbyLogin
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert_no_errors(&disable);
    assert_eq!(
        disable["data"]["updateSecuritySettings"]["totpRequireEmbyLogin"],
        false
    );
}

#[tokio::test]
async fn graphql_auth_runtime_suppresses_mfa_requirements_when_login_is_disabled() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: false
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: true
            totpRequireJellyfinLogin: true
            totpRequireEmbyLogin: true
          }) {
            formLoginEnabled
            mfaRequireConfigStepUp
            mfaRequirePasswordLogin
            totpRequireJellyfinLogin
            totpRequireEmbyLogin
            effectiveFormLoginEnabled
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;

    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateSecuritySettings"]["totpRequireJellyfinLogin"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["totpRequireEmbyLogin"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["effectiveFormLoginEnabled"],
        false
    );
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "auth.mfa.require_config_step_up",
            None,
            "true",
            "test",
            None,
        )
        .await
        .unwrap();

    let runtime = schema_exec(
        &ctx,
        r#"
        query AuthRuntimeState {
          authRuntimeState {
            effectiveFormLoginEnabled
            mfaRequirePasswordLogin
            mfaRequireConfigStepUp
            totpRequireJellyfinLogin
            totpRequireEmbyLogin
          }
        }
        "#,
        Some(admin),
    )
    .await;

    assert_no_errors(&runtime);
    assert_eq!(
        runtime["data"]["authRuntimeState"]["effectiveFormLoginEnabled"],
        false
    );
    assert_eq!(
        runtime["data"]["authRuntimeState"]["totpRequireJellyfinLogin"],
        false
    );
    assert_eq!(
        runtime["data"]["authRuntimeState"]["totpRequireEmbyLogin"],
        false
    );
    assert_eq!(
        runtime["data"]["authRuntimeState"]["mfaRequirePasswordLogin"],
        false
    );
    assert_eq!(
        runtime["data"]["authRuntimeState"]["mfaRequireConfigStepUp"],
        false
    );
}

#[tokio::test]
async fn graphql_typed_security_settings_round_trip_updates_runtime() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let admin = ctx
        .app
        .set_initial_own_password(&admin, "admin-pass1".to_string())
        .await
        .expect("set initial default admin password");

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: true
            passwordMinLength: 12
            skipLoginForLocalIps: true
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
            totpRequireEmbyLogin: true
          }) {
            formLoginEnabled
            passwordMinLength
            skipLoginForLocalIps
            totpRequireEmbyLogin
            effectiveFormLoginEnabled
            envOverrideActive
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;

    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateSecuritySettings"]["formLoginEnabled"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["passwordMinLength"],
        12
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["skipLoginForLocalIps"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["totpRequireEmbyLogin"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["effectiveFormLoginEnabled"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["envOverrideActive"],
        false
    );

    let auth_runtime = schema_exec(
        &ctx,
        r#"
        query AuthRuntimeState {
          authRuntimeState {
            effectiveFormLoginEnabled
            skipLoginForLocalIps
          }
        }
        "#,
        None,
    )
    .await;
    assert_no_errors(&auth_runtime);
    assert_eq!(
        auth_runtime["data"]["authRuntimeState"]["effectiveFormLoginEnabled"],
        true
    );
    assert_eq!(
        auth_runtime["data"]["authRuntimeState"]["skipLoginForLocalIps"],
        true
    );

    let me_with_local_bypass = gql(&ctx, "{ me { username } }", json!({})).await;
    assert_no_errors(&me_with_local_bypass);
    assert_eq!(
        me_with_local_bypass["data"]["me"]["username"],
        admin.username
    );

    let read = schema_exec(
        &ctx,
        r#"
        query SecuritySettings {
          securitySettings {
            formLoginEnabled
            passwordMinLength
            totpRequireEmbyLogin
            effectiveFormLoginEnabled
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["securitySettings"]["formLoginEnabled"], true);
    assert_eq!(read["data"]["securitySettings"]["passwordMinLength"], 12);
    assert_eq!(
        read["data"]["securitySettings"]["totpRequireEmbyLogin"],
        true
    );
    assert_eq!(
        read["data"]["securitySettings"]["effectiveFormLoginEnabled"],
        true
    );
}

#[tokio::test]
async fn graphql_security_settings_form_login_enable_revokes_authless_oauth_grants() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let admin = ctx
        .app
        .set_initial_own_password(&admin, "admin-pass1".to_string())
        .await
        .expect("set initial default admin password");

    create_oauth_refresh_grant_for_security_test(
        &ctx,
        &admin,
        scryer_application::OAuthAuthorizationSource::Authless,
        "authless",
    )
    .await;
    create_oauth_refresh_grant_for_security_test(
        &ctx,
        &admin,
        scryer_application::OAuthAuthorizationSource::Authenticated,
        "authenticated",
    )
    .await;
    let before = ctx
        .app
        .list_oauth_connected_apps(&admin)
        .await
        .expect("list connected apps before enabling form login");
    assert_eq!(before.len(), 2);

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: true
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
            totpRequireEmbyLogin: false
          }) {
            formLoginEnabled
            effectiveFormLoginEnabled
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;

    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateSecuritySettings"]["effectiveFormLoginEnabled"],
        true
    );
    let after = ctx
        .app
        .list_oauth_connected_apps(&admin)
        .await
        .expect("list connected apps after enabling form login");
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].client_id,
        scryer_application::OAUTH_GENERIC_NATIVE_CLIENT_ID
    );
}

#[tokio::test]
async fn graphql_security_session_duration_round_trip_and_validation() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let original_token = ctx.app.issue_access_token(&admin).await.unwrap();
    assert_eq!(ctx.app.token_lifetime().await.unwrap(), 3 * 86_400);
    for (field, expected, valid) in [
        ("sessionDurationDays: 14", 14, true),
        ("", 14, true),
        ("sessionDurationDays: 0", 14, false),
        ("sessionDurationDays: 366", 14, false),
        ("sessionDurationDays: 1", 1, true),
        ("sessionDurationDays: 365", 365, true),
    ] {
        let mutation = format!(
            r#"mutation {{ updateSecuritySettings(input: {{
            formLoginEnabled: false passwordMinLength: 8 skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false mfaRequirePasswordLogin: false
            mfaRequireJellyfinLogin: false {field}
        }}) {{ sessionDurationDays }} }}"#
        );
        let result = schema_exec(&ctx, &mutation, Some(admin.clone())).await;
        if valid {
            assert_no_errors(&result);
            assert_eq!(
                result["data"]["updateSecuritySettings"]["sessionDurationDays"],
                expected
            );
        } else {
            assert!(
                result["errors"][0]["message"]
                    .as_str()
                    .unwrap()
                    .contains("session duration")
            );
        }
        let saved = schema_exec(
            &ctx,
            "{ securitySettings { sessionDurationDays } }",
            Some(admin.clone()),
        )
        .await;
        assert_no_errors(&saved);
        assert_eq!(
            saved["data"]["securitySettings"]["sessionDurationDays"],
            expected
        );
        for persistent in [false, true] {
            let token = ctx
                .app
                .issue_access_token_with_mfa_and_persistence(&admin, None, None, persistent)
                .await
                .unwrap();
            ctx.app
                .authenticate_token(&token)
                .await
                .expect("new session is valid");
        }
        assert_eq!(ctx.app.token_lifetime().await.unwrap(), expected * 86_400);
        ctx.app
            .authenticate_token(&original_token)
            .await
            .expect("existing session remains valid");
    }
    let viewer = ctx
        .app
        .create_user(
            &admin,
            "session-viewer".into(),
            "test-password-123".into(),
            Default::default(),
            vec![],
        )
        .await
        .unwrap();
    let denied = schema_exec(
        &ctx,
        r#"mutation { updateSecuritySettings(input: {
        sessionDurationDays: 2 formLoginEnabled: false passwordMinLength: 8
        skipLoginForLocalIps: false mfaRequireConfigStepUp: false
        mfaRequirePasswordLogin: false mfaRequireJellyfinLogin: false
    }) { sessionDurationDays } }"#,
        Some(viewer),
    )
    .await;
    assert!(
        denied["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
    );
    assert_eq!(ctx.app.token_lifetime().await.unwrap(), 365 * 86_400);
}

#[tokio::test]
async fn graphql_typed_security_settings_reject_short_password_minimum() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: false
            passwordMinLength: 7
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
            totpRequireEmbyLogin: false
          }) {
            formLoginEnabled
          }
        }
        "#,
        Some(admin),
    )
    .await;

    let errors = update["errors"].as_array().expect("graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(
        message.contains("password minimum length must be at least 8"),
        "expected minimum-length validation error: {update}"
    );
}

#[tokio::test]
async fn graphql_typed_security_settings_rejects_enable_without_usable_admin_login() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: true
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
            totpRequireEmbyLogin: false
          }) {
            formLoginEnabled
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;

    let errors = update["errors"].as_array().expect("graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(
        message
            .contains("configure an enabled full administrator login before enabling form login"),
        "expected usable administrator validation error: {update}"
    );

    let read = schema_exec(
        &ctx,
        r#"
        query SecuritySettings {
          securitySettings {
            formLoginEnabled
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["securitySettings"]["formLoginEnabled"], false);
}

#[tokio::test]
async fn graphql_delay_profiles_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let upsert = gql(
        &ctx,
        r#"
        mutation UpsertDelayProfile($input: DelayProfileInput!) {
          upsertDelayProfile(input: $input) {
            id
            name
            usenetDelayMinutes
            torrentDelayMinutes
            preferredProtocol
            minAgeMinutes
            bypassScoreThreshold
            appliesToFacets
            tags
            priority
            enabled
          }
        }
        "#,
        json!({
          "input": {
            "id": "balanced-delay",
            "name": "Balanced Delay",
            "usenetDelayMinutes": 30,
            "torrentDelayMinutes": 90,
            "preferredProtocol": "USENET",
            "minAgeMinutes": 15,
            "bypassScoreThreshold": 320,
            "appliesToFacets": ["MOVIE", "SERIES"],
            "tags": ["4k", "hdr"],
            "priority": 5,
            "enabled": true
          }
        }),
    )
    .await;
    assert_no_errors(&upsert);
    assert_eq!(upsert["data"]["upsertDelayProfile"]["id"], "balanced-delay");
    assert_eq!(
        upsert["data"]["upsertDelayProfile"]["appliesToFacets"][1],
        "SERIES"
    );

    let read = gql(
        &ctx,
        r#"
        query DelayProfiles {
          delayProfiles {
            id
            name
            usenetDelayMinutes
            torrentDelayMinutes
            preferredProtocol
            minAgeMinutes
            bypassScoreThreshold
            appliesToFacets
            tags
            priority
            enabled
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    let profile = &read["data"]["delayProfiles"][0];
    assert_eq!(profile["id"], "balanced-delay");
    assert_eq!(profile["name"], "Balanced Delay");
    assert_eq!(profile["usenetDelayMinutes"], 30);
    assert_eq!(profile["torrentDelayMinutes"], 90);
    assert_eq!(profile["preferredProtocol"], "USENET");
    assert_eq!(profile["minAgeMinutes"], 15);
    assert_eq!(profile["bypassScoreThreshold"], 320);
    assert_eq!(profile["appliesToFacets"][0], "MOVIE");
    assert_eq!(profile["appliesToFacets"][1], "SERIES");
    assert_eq!(profile["tags"][0], "4k");
    assert_eq!(profile["priority"], 5);
    assert_eq!(profile["enabled"], true);

    let delete = gql(
        &ctx,
        r#"
        mutation DeleteDelayProfile($id: ID!) {
          deleteDelayProfile(id: $id) {
            id
          }
        }
        "#,
        json!({
          "id": "balanced-delay"
        }),
    )
    .await;
    assert_no_errors(&delete);
    assert_eq!(delete["data"]["deleteDelayProfile"]["id"], "balanced-delay");
}

const UPSERT_SCOPED_DELAY_PROFILE: &str = r#"
    mutation UpsertDelayProfile($input: DelayProfileInput!) {
      upsertDelayProfile(input: $input) { id libraryIds enabled }
    }
"#;

const READ_SCOPED_DELAY_PROFILES: &str = r#"
    query DelayProfiles { delayProfiles { id libraryIds enabled } }
"#;

fn scoped_delay_profile_input(library_ids: Option<Vec<&str>>) -> Value {
    let mut input = json!({
        "id": "scoped-delay",
        "name": "Scoped Delay",
        "usenetDelayMinutes": 45,
        "torrentDelayMinutes": 45,
        "preferredProtocol": "USENET",
        "minAgeMinutes": 0,
        "appliesToFacets": ["MOVIE"],
        "tags": [],
        "priority": 1,
        "enabled": true
    });
    if let Some(library_ids) = library_ids {
        input["libraryIds"] = json!(library_ids);
    }
    json!({ "input": input })
}

#[tokio::test]
async fn graphql_delay_profile_library_scope_round_trips_and_follows_library_delete() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let library = gql(
        &ctx,
        r#"mutation($input: CreateLibraryInput!) {
            createLibrary(input: $input) { id }
        }"#,
        json!({
            "input": {
                "facet": "MOVIE",
                "name": "Delay Scope Movies",
                "roots": [{ "path": "/delay-scope/movies", "isDefault": true }]
            }
        }),
    )
    .await;
    assert_no_errors(&library);
    let library_id = library["data"]["createLibrary"]["id"]
        .as_str()
        .expect("library id")
        .to_string();

    let unknown = gql(
        &ctx,
        UPSERT_SCOPED_DELAY_PROFILE,
        scoped_delay_profile_input(Some(vec!["missing-library"])),
    )
    .await;
    assert!(
        unknown["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()),
        "an unknown library id should be refused: {unknown}"
    );

    let scoped = gql(
        &ctx,
        UPSERT_SCOPED_DELAY_PROFILE,
        scoped_delay_profile_input(Some(vec![library_id.as_str()])),
    )
    .await;
    assert_no_errors(&scoped);
    assert_eq!(
        scoped["data"]["upsertDelayProfile"]["libraryIds"],
        json!([library_id])
    );

    // Leaving libraryIds out keeps the libraries the profile already has.
    let kept = gql(
        &ctx,
        UPSERT_SCOPED_DELAY_PROFILE,
        scoped_delay_profile_input(None),
    )
    .await;
    assert_no_errors(&kept);
    assert_eq!(
        kept["data"]["upsertDelayProfile"]["libraryIds"],
        json!([library_id])
    );

    let deleted = gql(
        &ctx,
        r#"mutation($id: ID!) { deleteLibrary(id: $id) { id jobRun { id } } }"#,
        json!({ "id": library_id }),
    )
    .await;
    assert_no_errors(&deleted);
    // The delete runs as a job; the profile changes when it finishes.
    let run_id = deleted["data"]["deleteLibrary"]["jobRun"]["id"]
        .as_str()
        .expect("library deletion job run")
        .to_string();
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    crate::common::wait_until("library deletion job completes", || {
        let admin = &admin;
        let run_id = run_id.clone();
        let ctx = &ctx;
        async move {
            ctx.app
                .list_recent_job_runs(admin, 20)
                .await
                .expect("list job runs")
                .into_iter()
                .any(|run| run.id == run_id && run.completed_at.is_some())
        }
    })
    .await;

    let read = gql(&ctx, READ_SCOPED_DELAY_PROFILES, json!({})).await;
    assert_no_errors(&read);
    let profile = &read["data"]["delayProfiles"][0];
    assert_eq!(profile["id"], "scoped-delay");
    assert_eq!(profile["libraryIds"], json!([]));
    assert_eq!(
        profile["enabled"], false,
        "a profile that loses its last library is disabled, not widened to every library"
    );
}

async fn create_oauth_refresh_grant_for_security_test(
    ctx: &TestContext,
    user: &User,
    authorization_source: scryer_application::OAuthAuthorizationSource,
    state: &str,
) {
    const REDIRECT_URI: &str = "http://127.0.0.1:49152/callback";
    const CODE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CODE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    let auth_session_version = ctx
        .app
        .current_actor_auth_session_version(user)
        .await
        .unwrap_or_else(|err| panic!("load OAuth authorization session version ({state}): {err}"));
    let issued = ctx
        .app
        .create_oauth_authorization_code(
            user,
            scryer_application::OAUTH_GENERIC_NATIVE_CLIENT_ID,
            REDIRECT_URI,
            scryer_application::OAUTH_LIBRARY_SCOPE,
            CODE_CHALLENGE,
            "S256",
            authorization_source,
            auth_session_version.as_deref(),
        )
        .await
        .unwrap_or_else(|err| panic!("create OAuth authorization code ({state}): {err}"));
    ctx.app
        .exchange_oauth_authorization_code(
            scryer_application::OAUTH_GENERIC_NATIVE_CLIENT_ID,
            &issued.code,
            REDIRECT_URI,
            CODE_VERIFIER,
            true,
        )
        .await
        .unwrap_or_else(|err| panic!("exchange OAuth authorization code ({state}): {err}"));
}
