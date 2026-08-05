use super::*;
use scryer_application::{JobKey, JobRunStatus};
use tokio::time::{Duration, timeout};

async fn wait_for_interactive_job(
    ctx: &TestContext,
    actor: &User,
    job_key: JobKey,
    run_id: &str,
) -> scryer_application::JobRun {
    timeout(Duration::from_secs(5), async {
        loop {
            let run = ctx
                .app
                .list_job_runs(actor, job_key, 10)
                .await
                .expect("list interactive job runs")
                .into_iter()
                .find(|run| run.id == run_id);
            if let Some(run) = run
                && run.status.is_terminal()
            {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("interactive job should complete")
}

#[tokio::test]
async fn graphql_me_query() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ me { id username } }", json!({})).await;
    assert_no_errors(&body);
    // auth-disabled mode creates an "admin" user
    assert_eq!(body["data"]["me"]["username"], "admin");
}

#[tokio::test]
async fn graphql_authless_mode_uses_disabled_default_admin() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    UserRepository::update_login_status_and_rotate_session(
        &ctx.users,
        &admin.id,
        scryer_domain::UserLoginStatus::Disabled,
        &Id::new().0,
    )
    .await
    .expect("disable default admin login");

    let body = gql(&ctx, "{ me { id username } }", json!({})).await;

    assert_no_errors(&body);
    assert_eq!(body["data"]["me"]["id"], admin.id);
    assert_eq!(body["data"]["me"]["username"], "admin");
}

#[tokio::test]
async fn recovery_admin_token_resolves_while_form_login_enabled_and_resets_other_user_password() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let admin = ctx
        .app
        .set_initial_own_password(&admin, "admin-recovery-old-pass1".to_string())
        .await
        .expect("set initial admin password");
    ctx.app.set_recovery_admin_login_enabled(true);
    let recovery_admin = ctx
        .app
        .recover_reserved_admin_access("recovery-admin-pass1")
        .await
        .expect("create recovery admin");
    let snapshot = ctx.auth_runtime.apply_saved_security_settings(true, false);
    assert!(snapshot.effective_form_login_enabled);
    assert!(!snapshot.skip_login_for_local_ips);

    let unauthenticated_me = gql(&ctx, "{ me { username } }", json!({})).await;
    assert_eq!(
        unauthenticated_me["data"]["me"],
        Value::Null,
        "recovery form-login mode should not resolve an unauthenticated actor: {unauthenticated_me}"
    );

    let login = gql(
        &ctx,
        r#"
        mutation RecoveryAdminLogin($username: String!, $password: String!) {
          login(input: { username: $username, password: $password }) {
            token
            user { username }
          }
        }
        "#,
        json!({
            "username": "recovery-admin",
            "password": "recovery-admin-pass1",
        }),
    )
    .await;
    assert_no_errors(&login);
    assert_eq!(login["data"]["login"]["user"]["username"], "recovery-admin");
    let recovery_token = login["data"]["login"]["token"]
        .as_str()
        .expect("recovery admin login token");

    let me = gql_with_token(
        &ctx,
        r#"query { me { id username appPermissions } }"#,
        json!({}),
        recovery_token,
    )
    .await;
    assert_no_errors(&me);
    assert_eq!(me["data"]["me"]["id"], recovery_admin.id);
    assert_eq!(me["data"]["me"]["username"], "recovery-admin");
    assert!(
        me["data"]["me"]["appPermissions"]
            .as_array()
            .is_some_and(|permissions| permissions.contains(&json!("MANAGE_USERS"))),
        "recovery admin should resolve with ManageUsers: {me}"
    );

    let self_without_current_password = gql_with_token(
        &ctx,
        r#"
        mutation($input: SetUserPasswordInput!) {
          setUserPassword(input: $input) { id username }
        }
        "#,
        json!({
            "input": {
                "userId": recovery_admin.id,
                "password": "recovery-admin-pass2",
            }
        }),
        recovery_token,
    )
    .await;
    assert!(
        self_without_current_password
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty()),
        "self password reset without current password should fail: {self_without_current_password}"
    );

    let reset_admin = gql_with_token(
        &ctx,
        r#"
        mutation($input: SetUserPasswordInput!) {
          setUserPassword(input: $input) { id username hasPassword }
        }
        "#,
        json!({
            "input": {
                "userId": admin.id,
                "password": "admin-recovery-new-pass1",
            }
        }),
        recovery_token,
    )
    .await;
    assert_no_errors(&reset_admin);
    assert_eq!(reset_admin["data"]["setUserPassword"]["username"], "admin");
    ctx.app
        .authenticate_credentials("admin", "admin-recovery-new-pass1")
        .await
        .expect("admin password was reset by recovery admin");

    let ordinary = ctx
        .app
        .create_user(
            &admin,
            "recovery_reset_denied".to_string(),
            "ordinary-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create ordinary user");
    let ordinary_token = ctx
        .app
        .issue_access_token(&ordinary)
        .await
        .expect("issue ordinary user token");
    let denied = gql_with_token(
        &ctx,
        r#"
        mutation($input: SetUserPasswordInput!) {
          setUserPassword(input: $input) { id username }
        }
        "#,
        json!({
            "input": {
                "userId": admin.id,
                "password": "admin-recovery-denied-pass1",
            }
        }),
        &ordinary_token,
    )
    .await;
    assert!(
        denied
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty()),
        "ordinary user should not reset another user's password: {denied}"
    );
}

#[tokio::test]
async fn graphql_enrollment_scoped_token_cannot_access_normal_apis() {
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
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
          }) {
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

    let token = ctx
        .app
        .issue_mfa_enrollment_token(&admin)
        .await
        .expect("issue enrollment token");

    let me = gql_with_token(&ctx, "{ me { id username } }", json!({}), &token).await;
    let errors = me["errors"].as_array().expect("expected GraphQL errors");
    assert!(
        !errors.is_empty(),
        "expected me query to reject enrollment scope: {me}"
    );
    assert_eq!(
        errors[0]["extensions"]["code"], "MFA_ENROLLMENT_REQUIRED",
        "unexpected enrollment-scope me rejection shape: {me}"
    );

    let enrollment_start = gql_with_token(
        &ctx,
        r#"mutation { totpEnrollmentStart { challengeId otpauthUrl } }"#,
        json!({}),
        &token,
    )
    .await;
    assert_no_errors(&enrollment_start);
    assert!(
        enrollment_start["data"]["totpEnrollmentStart"]["challengeId"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "enrollment-scoped token should be allowed to start TOTP enrollment: {enrollment_start}"
    );

    let create = gql_with_token(
        &ctx,
        r#"mutation($input: CreateUserInput!) {
            createUser(input: $input) { id username }
        }"#,
        json!({ "input": { "username": "enrollment_blocked", "password": "testpass123", "appPermissions": [], "libraryPermissions": [] } }),
        &token,
    )
    .await;
    let errors = create["errors"]
        .as_array()
        .expect("expected GraphQL errors");
    assert!(
        !errors.is_empty(),
        "expected normal API access to be rejected: {create}"
    );
    assert_eq!(
        errors[0]["extensions"]["code"], "MFA_ENROLLMENT_REQUIRED",
        "unexpected enrollment-scope rejection shape: {create}"
    );

    let step_up = gql_with_token(
        &ctx,
        r#"mutation { mfaVerifyStepUp(input: { code: "123456" }) { token } }"#,
        json!({}),
        &token,
    )
    .await;
    let errors = step_up["errors"]
        .as_array()
        .expect("expected GraphQL errors");
    assert!(
        !errors.is_empty(),
        "expected step-up to reject enrollment scope: {step_up}"
    );
    assert_eq!(
        errors[0]["extensions"]["code"], "MFA_ENROLLMENT_REQUIRED",
        "unexpected enrollment step-up rejection shape: {step_up}"
    );
}

#[tokio::test]
async fn graphql_oauth_admin_token_cannot_use_app_permission_surfaces() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let target = ctx
        .app
        .create_user(
            &admin,
            "oauth_permission_target".to_string(),
            "target-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create permission target");
    let oauth_admin = ctx
        .app
        .create_user(
            &admin,
            "oauth_app_admin".to_string(),
            "oauth-pass1".to_string(),
            AppPermissionMask::from_permissions([
                scryer_domain::AppPermission::ManageUsers,
                scryer_domain::AppPermission::ManagePermissions,
                scryer_domain::AppPermission::ManageSystemSettings,
                scryer_domain::AppPermission::ManageCatalogSettings,
            ]),
            vec![],
        )
        .await
        .expect("create OAuth admin");
    let token = ctx
        .app
        .issue_oauth_access_token(&oauth_admin, "generic-native", "graphql-oauth-admin-deny")
        .await
        .expect("issue OAuth token");

    let cases = vec![
        (
            "ManageSystemSettings",
            "createBackup",
            r#"mutation { createBackup(input: { password: "oauth-denied-backup-pass" }) { filename } }"#,
            json!({}),
        ),
        (
            "ManageCatalogSettings",
            "createRuleSet",
            r#"mutation($input: CreateRuleSetInput!) { createRuleSet(input: $input) { id } }"#,
            json!({
                "input": {
                    "name": "OAuth denied rule",
                    "description": "oauth should not manage catalog settings",
                    "regoSource": "package scryer\nallow := true",
                    "appliedFacets": ["movie"]
                }
            }),
        ),
        (
            "ManageUsers",
            "createUser",
            r#"mutation($input: CreateUserInput!) { createUser(input: $input) { id } }"#,
            json!({
                "input": {
                    "username": "oauth_blocked_new_user",
                    "password": "blocked-pass1",
                    "appPermissions": [],
                    "libraryPermissions": []
                }
            }),
        ),
        (
            "ManagePermissions",
            "setUserAppPermissions",
            r#"mutation($input: SetUserAppPermissionsInput!) { setUserAppPermissions(input: $input) { id } }"#,
            json!({
                "input": {
                    "userId": target.id,
                    "permissions": []
                }
            }),
        ),
    ];

    for (permission, field_key, query, variables) in cases {
        let body = gql_with_token(&ctx, query, variables, &token).await;
        assert_graphql_field_denied(&body, field_key);
        assert!(
            body["errors"][0]["message"]
                .as_str()
                .is_some_and(|message| message.contains("permission")),
            "OAuth admin token should fail {permission} through permission checks: {body}"
        );
    }
}

#[tokio::test]
async fn graphql_me_reports_effective_oauth_permissions() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let user = ctx
        .app
        .create_user(
            &admin,
            "oauth_me_permissions".to_string(),
            "oauth-pass1".to_string(),
            AppPermissionMask::from_permissions([scryer_domain::AppPermission::ManageUsers]),
            vec![],
        )
        .await
        .expect("create OAuth user with stored app permissions");
    let oauth_token = ctx
        .app
        .issue_oauth_access_token(&user, "generic-native", "graphql-oauth-me")
        .await
        .expect("issue OAuth token");
    let session_token = ctx
        .app
        .issue_access_token(&user)
        .await
        .expect("issue session token");

    let oauth_me = gql_with_token(
        &ctx,
        r#"query { me { username appPermissions } }"#,
        json!({}),
        &oauth_token,
    )
    .await;
    assert_no_errors(&oauth_me);
    assert_eq!(oauth_me["data"]["me"]["username"], "oauth_me_permissions");
    assert_eq!(oauth_me["data"]["me"]["appPermissions"], json!([]));

    let session_me = gql_with_token(
        &ctx,
        r#"query { me { username appPermissions } }"#,
        json!({}),
        &session_token,
    )
    .await;
    assert_no_errors(&session_me);
    assert_eq!(session_me["data"]["me"]["username"], "oauth_me_permissions");
    assert_eq!(
        session_me["data"]["me"]["appPermissions"],
        json!(["MANAGE_USERS"])
    );
}

#[tokio::test]
async fn graphql_authless_oauth_token_is_anonymous_while_auth_disabled() {
    let ctx = TestContext::new().await;
    ctx.auth_runtime.apply_saved_security_settings(false, false);
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let user = UserRepository::update_login_status_and_rotate_session(
        &ctx.users,
        &user.id,
        scryer_domain::UserLoginStatus::Disabled,
        &Id::new().0,
    )
    .await
    .expect("disable default admin login");
    let oauth_token = ctx
        .app
        .issue_oauth_access_token_with_source(
            &user,
            "generic-native",
            "graphql-authless-oauth-anonymous",
            scryer_application::OAuthAuthorizationSource::Authless,
        )
        .await
        .expect("issue authless OAuth token");
    let (_token_user, token_claims) = ctx
        .app
        .authenticate_token_with_claims(&oauth_token)
        .await
        .expect("authless OAuth token should authenticate");
    assert_eq!(
        token_claims.oauth_authorization_source,
        scryer_application::OAuthAuthorizationSource::Authless
    );

    let body = gql_with_token(
        &ctx,
        r#"query { me { username appPermissions } }"#,
        json!({}),
        &oauth_token,
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(body["data"]["me"]["username"], "Anonymous");
    assert_eq!(body["data"]["me"]["appPermissions"], json!([]));
}

#[tokio::test]
async fn graphql_authless_oauth_token_is_rejected_when_auth_enabled() {
    let ctx = TestContext::new().await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let oauth_token = ctx
        .app
        .issue_oauth_access_token_with_source(
            &user,
            "generic-native",
            "graphql-authless-oauth-rejected",
            scryer_application::OAuthAuthorizationSource::Authless,
        )
        .await
        .expect("issue authless OAuth token");
    let (_token_user, token_claims) = ctx
        .app
        .authenticate_token_with_claims(&oauth_token)
        .await
        .expect("authless OAuth token should authenticate");
    assert_eq!(
        token_claims.oauth_authorization_source,
        scryer_application::OAuthAuthorizationSource::Authless
    );
    ctx.auth_runtime.apply_saved_security_settings(true, false);

    let response = ctx
        .http_client()
        .post(ctx.graphql_url())
        .bearer_auth(oauth_token)
        .json(&json!({ "query": "query { titles { items { id } } }", "variables": {} }))
        .send()
        .await
        .expect("request should complete");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.expect("response body should be JSON");
    let (_message, code) = first_graphql_error_message_and_code(&body);
    assert_eq!(code, "AUTHENTICATION_REQUIRED");
}

#[tokio::test]
async fn graphql_authenticated_oauth_token_still_works_when_auth_enabled() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let user = ctx
        .app
        .create_user(
            &admin,
            "authenticated_oauth_enabled".to_string(),
            "oauth-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create OAuth user");
    let oauth_token = ctx
        .app
        .issue_oauth_access_token(
            &user,
            "generic-native",
            "graphql-authenticated-oauth-enabled",
        )
        .await
        .expect("issue authenticated OAuth token");
    ctx.auth_runtime.apply_saved_security_settings(true, false);

    let body = gql_with_token(
        &ctx,
        r#"query { me { username appPermissions } }"#,
        json!({}),
        &oauth_token,
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(
        body["data"]["me"]["username"],
        "authenticated_oauth_enabled"
    );
    assert_eq!(body["data"]["me"]["appPermissions"], json!([]));
}

#[tokio::test]
async fn graphql_oauth_token_cannot_use_own_account_surfaces() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let user = ctx
        .app
        .create_user(
            &admin,
            "oauth_own_account_user".to_string(),
            "oauth-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create OAuth user");
    let oauth_token = ctx
        .app
        .issue_oauth_access_token(&user, "generic-native", "graphql-oauth-own-account-deny")
        .await
        .expect("issue OAuth token");
    let session_token = ctx
        .app
        .issue_access_token(&user)
        .await
        .expect("issue full session token");

    let denied_cases = vec![
        (
            "myOauthApps",
            r#"query { myOauthApps { grantId clientName lastUsedAt } }"#,
            json!({}),
        ),
        (
            "revokeMyOauthApp",
            r#"mutation { revokeMyOauthApp(grantId: "missing-grant") { revoked } }"#,
            json!({}),
        ),
        (
            "myPasskeys",
            r#"query { myPasskeys { id friendlyName } }"#,
            json!({}),
        ),
        (
            "webauthnRegisterStart",
            r#"mutation { webauthnRegisterStart { challengeId } }"#,
            json!({}),
        ),
        (
            "myTotp",
            r#"query { myTotp { enabled recoveryCodesRemaining } }"#,
            json!({}),
        ),
        (
            "totpEnrollmentStart",
            r#"mutation { totpEnrollmentStart { challengeId otpauthUrl } }"#,
            json!({}),
        ),
    ];

    for (field_key, query, variables) in denied_cases {
        let body = gql_with_token(&ctx, query, variables, &oauth_token).await;
        assert_graphql_field_denied(&body, field_key);
    }

    let full_session = gql_with_token(
        &ctx,
        r#"query {
          myOauthApps { grantId }
          myTotp { enabled }
        }"#,
        json!({}),
        &session_token,
    )
    .await;
    assert_no_errors(&full_session);
    assert!(full_session["data"]["myOauthApps"].is_array());
    assert!(full_session["data"]["myTotp"]["enabled"].is_boolean());
}

#[tokio::test]
async fn graphql_local_bypass_session_satisfies_config_step_up_without_totp() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app.find_or_create_default_user().await.unwrap();
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "auth.form_login_enabled",
            None,
            "true",
            "test",
            None,
        )
        .await
        .unwrap();
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "auth.skip_login_for_local_ips",
            None,
            "true",
            "test",
            None,
        )
        .await
        .unwrap();
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
    ctx.auth_runtime.apply_saved_security_settings(true, true);

    set_folder_template(&ctx, "MOVIE", "{title} ({year})").await;
}

#[tokio::test]
async fn graphql_totp_enrollment_code_cannot_immediately_step_up() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let enrollment = ctx
        .app
        .totp_enrollment_start(&admin)
        .await
        .expect("start TOTP enrollment");
    let code = test_totp_code(&enrollment.secret_base32);
    ctx.app
        .totp_enrollment_complete(&admin, &enrollment.challenge_id, &code)
        .await
        .expect("complete TOTP enrollment");

    let error = ctx
        .app
        .mfa_verify_step_up(&admin, &code)
        .await
        .expect_err("enrollment code should not be accepted for immediate step-up");
    assert!(
        error.to_string().contains("invalid TOTP code"),
        "unexpected replay rejection: {error}"
    );

    let next_code = test_totp_code_for_step_offset(&enrollment.secret_base32, 1);
    ctx.app
        .mfa_verify_step_up(&admin, &next_code)
        .await
        .expect("later TOTP step should still verify");
}

#[tokio::test]
async fn graphql_settings_mutations_require_config_step_up() {
    let ctx = TestContext::new().await;
    let (admin, token, _totp_code) =
        enable_form_login_with_config_step_up(&ctx, "admin", "admin-pass1").await;
    let target = ctx
        .app
        .create_user(
            &admin,
            "step_up_target".to_string(),
            "target-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create target user");
    let library_root = ctx
        .app_data_dir
        .path()
        .join("step-up-library-root")
        .to_string_lossy()
        .to_string();

    let cases = vec![
        (
            "createRuleSet",
            "createRuleSet",
            r#"mutation($input: CreateRuleSetInput!) { createRuleSet(input: $input) { id } }"#,
            json!({
                "input": {
                    "name": "Step-up rule",
                    "description": "requires step-up",
                    "regoSource": "package scryer\nallow := true",
                    "appliedFacets": ["movie"]
                }
            }),
        ),
        (
            "validateRuleSet",
            "validateRuleSet",
            r#"mutation($input: ValidateRuleSetInput!) { validateRuleSet(input: $input) { valid } }"#,
            json!({ "input": { "regoSource": "package scryer\nallow := true" } }),
        ),
        (
            "createIndexerConfig",
            "createIndexerConfig",
            r#"mutation($input: CreateIndexerConfigInput!) { createIndexerConfig(input: $input) { id } }"#,
            json!({
                "input": {
                    "name": "Step-up indexer",
                    "providerType": "newznab",
                    "config": []
                }
            }),
        ),
        (
            "createUser",
            "createUser",
            r#"mutation($input: CreateUserInput!) { createUser(input: $input) { id } }"#,
            json!({
                "input": {
                    "username": "blocked_new_user",
                    "password": "blocked-pass1",
                    "appPermissions": [],
                    "libraryPermissions": []
                }
            }),
        ),
        (
            "setUserPassword for another user",
            "setUserPassword",
            r#"mutation($input: SetUserPasswordInput!) { setUserPassword(input: $input) { id } }"#,
            json!({
                "input": {
                    "userId": target.id,
                    "password": "target-pass2"
                }
            }),
        ),
        (
            "createBackup",
            "createBackup",
            r#"mutation { createBackup(input: { password: "step-up-backup-pass" }) { filename } }"#,
            json!({}),
        ),
        (
            "acknowledgeAutoBackupDisabledMissingKeyNotice",
            "acknowledgeAutoBackupDisabledMissingKeyNotice",
            r#"mutation { acknowledgeAutoBackupDisabledMissingKeyNotice { enabled autoBackupDisabledMissingKeyNotice } }"#,
            json!({}),
        ),
        (
            "completeSetup",
            "completeSetup",
            r#"mutation { completeSetup { completed } }"#,
            json!({}),
        ),
        (
            "beginInstallPlugin",
            "beginInstallPlugin",
            r#"mutation($pluginId: ID!) { beginInstallPlugin(pluginId: $pluginId) { pluginId } }"#,
            json!({ "pluginId": "missing-plugin" }),
        ),
        (
            "createNotificationChannel",
            "createNotificationChannel",
            r#"mutation($input: CreateNotificationChannelInput!) { createNotificationChannel(input: $input) { id } }"#,
            json!({
                "input": {
                    "name": "Step-up notification",
                    "channelType": "webhook",
                    "config": []
                }
            }),
        ),
        (
            "executeExternalImport",
            "executeExternalImport",
            r#"mutation($input: ExecuteExternalImportInput!) { executeExternalImport(input: $input) { mediaPathsSaved } }"#,
            json!({
                "input": {
                    "prowlarr": null,
                    "sourceWarmupSessionIds": [],
                    "selectedDownloadClientDedupKeys": [],
                    "selectedIndexerDedupKeys": [],
                    "downloadClientApiKeyOverrides": [],
                    "downloadClientPasswordOverrides": [],
                    "indexerApiKeyOverrides": []
                }
            }),
        ),
        (
            "saveExternalImportSetupSecretDraft",
            "saveExternalImportSetupSecretDraft",
            r#"mutation($input: SaveExternalImportSetupSecretDraftInput!) { saveExternalImportSetupSecretDraft(input: $input) { updatedAt } }"#,
            json!({
                "input": {
                    "instanceApiKeys": [
                        {
                            "instanceId": "step-up-sonarr",
                            "kind": "SONARR",
                            "apiKey": "step-up-secret"
                        }
                    ],
                    "downloadClientApiKeyOverrides": [],
                    "downloadClientPasswordOverrides": [],
                    "indexerApiKeyOverrides": []
                }
            }),
        ),
        (
            "clearExternalImportSetupSecretDraft",
            "clearExternalImportSetupSecretDraft",
            r#"mutation { clearExternalImportSetupSecretDraft { cleared } }"#,
            json!({}),
        ),
        (
            "createPostProcessingScript",
            "createPostProcessingScript",
            r#"mutation($input: CreatePostProcessingScriptInput!) { createPostProcessingScript(input: $input) { id } }"#,
            json!({
                "input": {
                    "name": "Step-up post-processing script",
                    "scriptType": "inline",
                    "scriptContent": "true",
                    "inlineShellAcknowledged": true,
                    "appliedFacets": ["movie"]
                }
            }),
        ),
        (
            "createLibrary",
            "createLibrary",
            r#"mutation($input: CreateLibraryInput!) { createLibrary(input: $input) { id } }"#,
            json!({
                "input": {
                    "facet": "MOVIE",
                    "name": "Step-up Library",
                    "roots": [{ "path": library_root, "isDefault": true }]
                }
            }),
        ),
    ];

    for (name, field_key, query, variables) in cases {
        let body = gql_with_token(&ctx, query, variables, &token).await;
        assert!(
            body.get("errors").is_some(),
            "expected {name} to require MFA step-up: {body}"
        );
        assert_mfa_step_up_required(&body);
        assert!(
            body["data"].is_null() || body["data"][field_key].is_null(),
            "blocked mutation should not return data for {name}: {body}"
        );
    }
}

#[tokio::test]
async fn graphql_post_processing_inline_shell_requires_acknowledgement() {
    let ctx = TestContext::new().await;
    let create = r#"mutation($input: CreatePostProcessingScriptInput!) {
        createPostProcessingScript(input: $input) { id scriptType enabled }
    }"#;

    let missing_type = gql(
        &ctx,
        create,
        json!({
            "input": {
                "name": "Missing type",
                "scriptContent": "true"
            }
        }),
    )
    .await;
    assert!(
        missing_type.get("errors").is_some(),
        "missing scriptType should fail: {missing_type}"
    );

    let invalid_type = gql(
        &ctx,
        create,
        json!({
            "input": {
                "name": "Invalid type",
                "scriptType": "bogus",
                "scriptContent": "true"
            }
        }),
    )
    .await;
    assert!(
        invalid_type.get("errors").is_some(),
        "invalid scriptType should fail: {invalid_type}"
    );

    let bare_file_path = gql(
        &ctx,
        create,
        json!({
            "input": {
                "name": "Bare file path",
                "scriptType": "file",
                "scriptContent": "true"
            }
        }),
    )
    .await;
    assert!(
        bare_file_path.get("errors").is_some(),
        "bare file script path should fail: {bare_file_path}"
    );

    let relative_file_path = gql(
        &ctx,
        create,
        json!({
            "input": {
                "name": "Relative file path",
                "scriptType": "file",
                "scriptContent": "./post-process.sh"
            }
        }),
    )
    .await;
    assert!(
        relative_file_path.get("errors").is_some(),
        "relative file script path should fail: {relative_file_path}"
    );

    let inline_without_ack = gql(
        &ctx,
        create,
        json!({
            "input": {
                "name": "Inline without acknowledgement",
                "scriptType": "inline",
                "scriptContent": "true"
            }
        }),
    )
    .await;
    assert!(
        inline_without_ack.get("errors").is_some(),
        "inline create should require acknowledgement: {inline_without_ack}"
    );

    let file_without_ack = gql(
        &ctx,
        create,
        json!({
            "input": {
                "name": "File without acknowledgement",
                "scriptType": "file",
                "scriptContent": "/bin/true",
                "appliedFacets": ["movie"]
            }
        }),
    )
    .await;
    assert_no_errors(&file_without_ack);
    assert_eq!(
        file_without_ack["data"]["createPostProcessingScript"]["scriptType"],
        "file"
    );
    let file_id = file_without_ack["data"]["createPostProcessingScript"]["id"]
        .as_str()
        .expect("file script id");

    let update_file_to_relative = gql(
        &ctx,
        r#"mutation($input: UpdatePostProcessingScriptInput!) {
            updatePostProcessingScript(input: $input) { id scriptContent }
        }"#,
        json!({
            "input": {
                "id": file_id,
                "scriptContent": "relative-post-process.sh"
            }
        }),
    )
    .await;
    assert!(
        update_file_to_relative.get("errors").is_some(),
        "file script update to a relative path should fail: {update_file_to_relative}"
    );

    let inline_with_ack = gql(
        &ctx,
        create,
        json!({
            "input": {
                "name": "Inline with acknowledgement",
                "scriptType": "inline",
                "scriptContent": "true",
                "inlineShellAcknowledged": true,
                "appliedFacets": ["movie"]
            }
        }),
    )
    .await;
    assert_no_errors(&inline_with_ack);
    let inline_id = inline_with_ack["data"]["createPostProcessingScript"]["id"]
        .as_str()
        .expect("inline script id");

    let update_inline_without_ack = gql(
        &ctx,
        r#"mutation($input: UpdatePostProcessingScriptInput!) {
            updatePostProcessingScript(input: $input) { id scriptContent }
        }"#,
        json!({
            "input": {
                "id": inline_id,
                "scriptContent": "echo changed"
            }
        }),
    )
    .await;
    assert!(
        update_inline_without_ack.get("errors").is_some(),
        "inline content update should require acknowledgement: {update_inline_without_ack}"
    );

    let update_inline_with_ack = gql(
        &ctx,
        r#"mutation($input: UpdatePostProcessingScriptInput!) {
            updatePostProcessingScript(input: $input) { id scriptContent }
        }"#,
        json!({
            "input": {
                "id": inline_id,
                "scriptContent": "echo changed",
                "inlineShellAcknowledged": true
            }
        }),
    )
    .await;
    assert_no_errors(&update_inline_with_ack);

    let toggle = r#"mutation($id: ID!, $inlineShellAcknowledged: Boolean) {
        togglePostProcessingScript(id: $id, inlineShellAcknowledged: $inlineShellAcknowledged) {
            id
            enabled
        }
    }"#;

    let disable_inline = gql(&ctx, toggle, json!({ "id": inline_id })).await;
    assert_no_errors(&disable_inline);
    assert_eq!(
        disable_inline["data"]["togglePostProcessingScript"]["enabled"],
        false
    );

    let enable_inline_without_ack = gql(&ctx, toggle, json!({ "id": inline_id })).await;
    assert!(
        enable_inline_without_ack.get("errors").is_some(),
        "inline enable should require acknowledgement: {enable_inline_without_ack}"
    );

    let enable_inline_with_ack = gql(
        &ctx,
        toggle,
        json!({
            "id": inline_id,
            "inlineShellAcknowledged": true
        }),
    )
    .await;
    assert_no_errors(&enable_inline_with_ack);
    assert_eq!(
        enable_inline_with_ack["data"]["togglePostProcessingScript"]["enabled"],
        true
    );
}

#[tokio::test]
async fn graphql_config_step_up_token_satisfies_protected_settings_mutation() {
    let ctx = TestContext::new().await;
    let (_admin, token, totp_code) =
        enable_form_login_with_config_step_up(&ctx, "admin", "admin-pass1").await;
    let step_up = gql_with_token(
        &ctx,
        r#"mutation($code: String!) { mfaVerifyStepUp(input: { code: $code }) { token } }"#,
        json!({ "code": totp_code }),
        &token,
    )
    .await;
    assert_no_errors(&step_up);
    let step_up_token = step_up["data"]["mfaVerifyStepUp"]["token"]
        .as_str()
        .expect("step-up token");

    let body = gql_with_token(
        &ctx,
        r#"mutation($input: CreateUserInput!) { createUser(input: $input) { id username } }"#,
        json!({
            "input": {
                "username": "stepped_up_user",
                "password": "stepped-pass1",
                "appPermissions": [],
                "libraryPermissions": []
            }
        }),
        step_up_token,
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["createUser"]["username"], "stepped_up_user");

    let external_import = gql_with_token(
        &ctx,
        r#"mutation($input: ExecuteExternalImportInput!) { executeExternalImport(input: $input) { mediaPathsSaved errors } }"#,
        json!({
            "input": {
                "prowlarr": null,
                "sourceWarmupSessionIds": [],
                "selectedDownloadClientDedupKeys": [],
                "selectedIndexerDedupKeys": [],
                "downloadClientApiKeyOverrides": [],
                "downloadClientPasswordOverrides": [],
                "indexerApiKeyOverrides": []
            }
        }),
        step_up_token,
    )
    .await;
    assert_no_errors(&external_import);
    assert!(
        external_import["data"]["executeExternalImport"]["errors"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "stepped-up external import should reach normal execution: {external_import}"
    );
}

#[tokio::test]
async fn graphql_set_own_password_does_not_require_config_step_up() {
    let ctx = TestContext::new().await;
    let (admin, token, _totp_code) =
        enable_form_login_with_config_step_up(&ctx, "admin", "admin-pass1").await;

    let body = gql_with_token(
        &ctx,
        r#"mutation($input: SetUserPasswordInput!) { setUserPassword(input: $input) { id username } }"#,
        json!({
            "input": {
                "userId": admin.id,
                "password": "admin-pass2",
                "currentPassword": "admin-pass1"
            }
        }),
        &token,
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["setUserPassword"]["username"], "admin");
}

#[tokio::test]
async fn graphql_users_query() {
    let ctx = TestContext::new().await;
    // Trigger default admin user creation first
    gql(&ctx, "{ me { id } }", json!({})).await;

    let body = gql(&ctx, "{ users { id username } }", json!({})).await;
    assert_no_errors(&body);
    let users = body["data"]["users"].as_array().unwrap();
    assert!(!users.is_empty(), "should have at least one user");
}

#[tokio::test]
async fn graphql_create_user() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let body = gql(
        &ctx,
        r#"mutation($input: CreateUserInput!) {
            createUser(input: $input) { id username }
        }"#,
        json!({ "input": { "username": "testuser", "password": "testpass123", "appPermissions": [], "libraryPermissions": [] } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["createUser"]["username"], "testuser");
}

#[tokio::test]
async fn graphql_create_user_rejects_short_password() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let body = gql(
        &ctx,
        r#"mutation($input: CreateUserInput!) {
            createUser(input: $input) { id username }
        }"#,
        json!({ "input": { "username": "shortpass", "password": "1234567", "appPermissions": [], "libraryPermissions": [] } }),
    )
    .await;

    let errors = body["errors"].as_array().expect("graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(
        message.contains("password must be at least 8 characters"),
        "expected short-password validation error: {body}"
    );
}

#[tokio::test]
async fn graphql_users_query_exposes_auth_factor_status_with_manage_users() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let with_factors = ctx
        .app
        .create_user(
            &admin,
            "factor_status".to_string(),
            "s3cr3t!!".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create factor status user");
    let without_factors = ctx
        .app
        .create_user(
            &admin,
            "factor_status_empty".to_string(),
            "s3cr3t!!".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create user without factors");

    enroll_totp_for_test(&ctx, &with_factors).await;
    seed_test_passkey(&ctx, &with_factors.id, "factor-status-credential").await;

    let body = schema_exec(
        &ctx,
        "{ users { id username loginEnabled isDefaultAdmin hasMfa hasPasskey } }",
        Some(manage_users_actor("user-manager")),
    )
    .await;
    assert_no_errors(&body);
    let users = body["data"]["users"].as_array().expect("users");
    let row_with_factors = users
        .iter()
        .find(|row| row["id"].as_str() == Some(with_factors.id.as_str()))
        .expect("user with factors in users query");
    assert_eq!(row_with_factors["hasMfa"], true);
    assert_eq!(row_with_factors["hasPasskey"], true);
    assert_eq!(row_with_factors["loginEnabled"], true);
    assert_eq!(row_with_factors["isDefaultAdmin"], false);

    let row_without_factors = users
        .iter()
        .find(|row| row["id"].as_str() == Some(without_factors.id.as_str()))
        .expect("user without factors in users query");
    assert_eq!(row_without_factors["hasMfa"], false);
    assert_eq!(row_without_factors["hasPasskey"], false);
}

#[tokio::test]
async fn graphql_set_user_login_enabled_updates_status_and_auth_epoch() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let target = ctx
        .app
        .create_user(
            &admin,
            "graphql-suspended-user".to_string(),
            "s3cr3t!!".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create target user");
    let previous_epoch = ctx.auth_runtime.snapshot().epoch;

    let body = schema_exec(
        &ctx,
        &format!(
            r#"mutation {{ setUserLoginEnabled(input: {{ userId: "{}", enabled: false }}) {{ id loginEnabled isDefaultAdmin hasPassword }} }}"#,
            target.id
        ),
        Some(manage_users_actor("login-status-manager")),
    )
    .await;
    assert_no_errors(&body);
    let payload = &body["data"]["setUserLoginEnabled"];
    assert_eq!(payload["id"], target.id);
    assert_eq!(payload["loginEnabled"], false);
    assert_eq!(payload["isDefaultAdmin"], false);
    assert_eq!(payload["hasPassword"], true);
    assert_eq!(ctx.auth_runtime.snapshot().epoch, previous_epoch + 1);
}

#[tokio::test]
async fn graphql_reset_user_mfa_clears_totp_state_and_preserves_passkeys() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let target = ctx
        .app
        .create_user(
            &admin,
            "reset_mfa_target".to_string(),
            "s3cr3t!!".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create reset target");
    enroll_totp_for_test(&ctx, &target).await;
    seed_test_passkey(&ctx, &target.id, "reset-mfa-passkey").await;

    let old_token = ctx
        .app
        .issue_access_token(&target)
        .await
        .expect("issue token before reset");
    let now = Utc::now();
    let now_string = now.to_rfc3339();
    let pending_challenge = TotpEnrollmentChallengeRecord {
        id: Id::new().0,
        user_id: target.id.clone(),
        secret_base32: "JBSWY3DPEHPK3PXP".to_string(),
        algorithm: "SHA1".to_string(),
        digits: 6,
        period_seconds: 30,
        created_at: now_string.clone(),
        expires_at: (now + chrono::Duration::minutes(10)).to_rfc3339(),
    };
    let totp_store = TotpStore::new(ctx.db.datastore(), ctx.db.encryption_key_state());
    totp_store
        .create_enrollment_challenge(pending_challenge.clone())
        .await
        .expect("seed pending TOTP enrollment challenge");
    totp_store
        .record_failed_attempt(TotpFailedAttemptRecord {
            id: Id::new().0,
            user_id: target.id.clone(),
            attempted_at: now_string,
        })
        .await
        .expect("seed failed TOTP attempt");

    let reset = schema_exec(
        &ctx,
        &format!(
            r#"
            mutation {{
              resetUserMfa(id: "{}") {{
                id
                username
                hasMfa
                hasPasskey
              }}
            }}
            "#,
            target.id
        ),
        Some(manage_users_actor("mfa-reset-manager")),
    )
    .await;
    assert_no_errors(&reset);
    let reset_user = &reset["data"]["resetUserMfa"];
    assert_eq!(reset_user["id"], target.id);
    assert_eq!(reset_user["hasMfa"], false);
    assert_eq!(reset_user["hasPasskey"], true);

    assert!(
        totp_store
            .get_credential_for_user(&target.id)
            .await
            .expect("load TOTP credential")
            .is_none(),
        "TOTP credential should be removed"
    );
    assert!(
        totp_store
            .list_recovery_codes_for_user(&target.id)
            .await
            .expect("list recovery codes")
            .is_empty(),
        "recovery codes should be removed"
    );
    let failed_attempts = totp_store
        .count_failed_attempts_since(
            &target.id,
            &(Utc::now() - chrono::Duration::hours(1)).to_rfc3339(),
        )
        .await
        .expect("count failed attempts");
    assert_eq!(failed_attempts, 0);
    assert!(
        totp_store
            .get_enrollment_challenge(&pending_challenge.id, &target.id)
            .await
            .expect("load pending enrollment challenge")
            .is_none(),
        "pending enrollment challenges should be removed"
    );

    let passkeys = WebauthnStore::new(ctx.db.datastore())
        .list_credentials_for_user(&target.id)
        .await
        .expect("list passkeys");
    assert_eq!(passkeys.len(), 1, "passkeys should be preserved");
    assert!(
        ctx.app.authenticate_token(&old_token).await.is_err(),
        "tokens issued before MFA reset should be invalidated"
    );
    let new_token = ctx
        .app
        .issue_access_token(&target)
        .await
        .expect("issue token after reset");
    ctx.app
        .authenticate_token(&new_token)
        .await
        .expect("token issued after MFA reset should authenticate");
}

#[tokio::test]
async fn graphql_reset_user_mfa_requires_manage_users_and_rejects_self() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let target = ctx
        .app
        .create_user(
            &admin,
            "reset_mfa_authz".to_string(),
            "s3cr3t!!".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create reset authz target");
    let mutation = format!(
        r#"
        mutation {{
          resetUserMfa(id: "{}") {{
            id
          }}
        }}
        "#,
        target.id
    );

    let denied = schema_exec(
        &ctx,
        &mutation,
        Some(User {
            id: Id::new().0,
            username: "not-a-manager".to_string(),
            password_hash: None,
            account_kind: Default::default(),
            authorization: UserAuthorization {
                app: AppPermissionMask::NONE,
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
                login_status: Default::default(),
                loaded: true,
            },
        }),
    )
    .await;
    assert!(
        denied
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty()),
        "reset should require Manage Users: {denied}"
    );

    let mut self_actor = target.clone();
    self_actor.authorization = UserAuthorization {
        app: AppPermissionMask::from_permissions([scryer_domain::AppPermission::ManageUsers]),
        libraries: HashMap::new(),
        default_library: LibraryPermissionMask::NONE,
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        login_status: Default::default(),
        loaded: true,
    };
    let self_reset = schema_exec(&ctx, &mutation, Some(self_actor)).await;
    let errors = self_reset["errors"]
        .as_array()
        .expect("self reset should return errors");
    assert!(
        errors[0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("cannot reset your own MFA")),
        "expected self-reset rejection: {self_reset}"
    );
}

#[tokio::test]
async fn graphql_external_account_invites_expose_last_login() {
    let ctx = TestContext::new().await;
    let user = gql(
        &ctx,
        r#"mutation($input: CreateUserInput!) {
            createUser(input: $input) { id username }
        }"#,
        json!({ "input": { "username": "invitee", "password": "testpass123", "appPermissions": [], "libraryPermissions": [] } }),
    )
    .await;
    assert_no_errors(&user);
    let user_id = user["data"]["createUser"]["id"]
        .as_str()
        .expect("created user id");

    let now = Utc::now();
    let media_servers =
        MediaServerConnectionStore::new(ctx.db.datastore(), ctx.db.encryption_key_state());
    MediaServerConnectionRepository::create(
        &media_servers,
        MediaServerConnection {
            id: "jellyfin-main".to_string(),
            provider: MediaServerProvider::Jellyfin,
            display_name: "Main Jellyfin".to_string(),
            base_url: ctx.smg_server.uri(),
            enabled: true,
            login_enabled: true,
            linking_enabled: false,
            auto_add_enabled: false,
            default_app_permissions: AppPermissionMask::NONE,
            default_library_grants: Vec::new(),
            machine_id: None,
            api_key: Some("jellyfin-api-key".to_string()),
            path_mappings: Vec::new(),
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .expect("seed Jellyfin media server connection");

    Mock::given(method("GET"))
        .and(path("/Users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "Id": "jellyfin-user-id",
            "Name": "jellyfin-user"
        }])))
        .mount(&ctx.smg_server)
        .await;

    let invite = gql(
        &ctx,
        r#"mutation($input: CreateExternalAccountInviteInput!) {
            createExternalAccountInvite(input: $input) {
                id
                userId
                provider
                connectionId
                username
                status
                lastLoginAt
            }
        }"#,
        json!({
            "input": {
                "userId": user_id,
                "provider": "JELLYFIN",
                "connectionId": "jellyfin-main",
                "providerUserIdentifier": "jellyfin-user",
                "providerUserId": "jellyfin-user-id"
            }
        }),
    )
    .await;
    assert_no_errors(&invite);
    assert_eq!(
        invite["data"]["createExternalAccountInvite"]["lastLoginAt"],
        Value::Null
    );

    let invites = gql(
        &ctx,
        r#"query {
            externalAccountInvites {
                userId
                provider
                connectionId
                username
                status
                lastLoginAt
            }
        }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&invites);
    let rows = invites["data"]["externalAccountInvites"]
        .as_array()
        .expect("invite rows");
    let row = rows
        .iter()
        .find(|row| row["userId"].as_str() == Some(user_id))
        .expect("created invite row");
    assert_eq!(row["provider"], "JELLYFIN");
    assert_eq!(row["status"], "PENDING_CLAIM");
    assert_eq!(row["lastLoginAt"], Value::Null);

    let viewer = User {
        id: "viewer".to_string(),
        username: "viewer".to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::NONE,
            libraries: HashMap::new(),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    };
    let denied = schema_exec(
        &ctx,
        "query { externalAccountInvites { id } }",
        Some(viewer),
    )
    .await;
    assert!(
        denied
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty()),
        "expected authorization error: {denied}"
    );
}

/// The login mutation is available without a pre-existing session.
/// After providing valid credentials, the server returns a non-empty JWT.
///
/// Note: the migration-seeded "admin" user has a NULL password_hash (it is
/// intended for dev-mode auto-login, not credential-based login). We
/// therefore create a fresh user with an explicit password to exercise the
/// full login path.
#[tokio::test]
async fn login_with_valid_credentials_returns_token() {
    let ctx = TestContext::new().await;

    // Need an actor to create the test user; admin carries the required masks.
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .create_user(
            &admin,
            "logintest".to_string(),
            "s3cr3t!!".to_string(),
            scryer_domain::AppPermissionMask::from_permissions([
                scryer_domain::AppPermission::ManageUsers,
            ]),
            vec![],
        )
        .await
        .unwrap();

    let body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "logintest", password: "s3cr3t!!" }) { token expiresAt user { username appPermissions } } }"#,
        None,
    )
    .await;

    assert!(
        body["errors"].is_null(),
        "login should not return errors: {body}"
    );
    let token = body["data"]["login"]["token"].as_str().unwrap();
    assert!(!token.is_empty(), "JWT token should not be empty");
    assert_eq!(body["data"]["login"]["user"]["username"], "logintest");
    assert_eq!(
        body["data"]["login"]["user"]["appPermissions"],
        json!(["MANAGE_USERS"])
    );
}

#[tokio::test]
async fn me_reports_password_status_for_token_authenticated_user() {
    let ctx = TestContext::new().await;

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .create_user(
            &admin,
            "metest".to_string(),
            "s3cr3t!!".to_string(),
            scryer_domain::AppPermissionMask::NONE,
            vec![],
        )
        .await
        .unwrap();

    let login_body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "metest", password: "s3cr3t!!" }) { token } }"#,
        None,
    )
    .await;
    assert!(
        login_body["errors"].is_null(),
        "login should succeed: {login_body}"
    );
    let token = login_body["data"]["login"]["token"]
        .as_str()
        .expect("login token should be a string");
    let (token_user, _) = ctx
        .app
        .authenticate_token_with_claims(token)
        .await
        .expect("token should authenticate");
    assert!(
        token_user.password_hash.is_none(),
        "request context user should not carry password hashes"
    );

    let me_body = schema_exec(
        &ctx,
        r#"{ me { username hasPassword accountKind } }"#,
        Some(token_user.clone()),
    )
    .await;
    assert!(me_body["errors"].is_null(), "me should succeed: {me_body}");
    assert_eq!(me_body["data"]["me"]["username"], "metest");
    assert_eq!(me_body["data"]["me"]["hasPassword"], true);
    assert_eq!(me_body["data"]["me"]["accountKind"], "LOCAL");

    let refreshed_token = ctx
        .app
        .issue_access_token(&token_user)
        .await
        .expect("redacted context user should be able to refresh a token");
    ctx.app
        .authenticate_token(&refreshed_token)
        .await
        .expect("refreshed token should authenticate");
}

/// Providing the wrong password must produce a GraphQL error — never a token.
#[tokio::test]
async fn login_with_wrong_password_returns_error() {
    let ctx = TestContext::new().await;

    // Create a user with a known password so we can test wrong-password rejection.
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .create_user(
            &admin,
            "wrongpasstest".to_string(),
            "correct_horse".to_string(),
            scryer_domain::AppPermissionMask::NONE,
            vec![],
        )
        .await
        .unwrap();

    let body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "wrongpasstest", password: "wrong_password" }) { token } }"#,
        None,
    )
    .await;

    assert!(
        !body["errors"].is_null()
            && body["errors"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false),
        "wrong password should return a GraphQL error: {body}"
    );
    // Verify the error is the masked bad-credentials response, not a server error.
    let error_msg = body["errors"][0]["message"].as_str().unwrap_or("");
    assert_eq!(
        error_msg,
        "Sign-in failed. Check your sign-in details and try again."
    );
}

#[tokio::test]
async fn delete_media_file_honors_custom_library_permissions_after_library_refactor() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let now = Utc::now();
    let custom_library_id = Id::new().0;

    scryer_application::LibraryRepository::create(
        &ctx.libraries,
        Library {
            id: custom_library_id.clone(),
            facet: MediaFacet::Movie,
            name: "Scoped Movies".to_string(),
            slug: "scoped-movies".to_string(),
            is_default: false,
            roots: Vec::new(),
            created_at: now,
            updated_at: now,
        },
        vec![LibraryRootDraft {
            path: media_root.path().to_string_lossy().to_string(),
            is_default: true,
        }],
    )
    .await
    .expect("create custom library");

    let title = Title {
        id: Id::new().0,
        name: "Scoped Delete Movie".to_string(),
        library_id: custom_library_id.clone(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: "998877".to_string(),
        }],
        root_folder_id: scryer_domain::root_folder_id_for_path(
            media_root.path().to_string_lossy().as_ref(),
        ),
        created_by: None,
        created_at: now,
        year: Some(2024),
        overview: Some("delete path coverage".to_string()),
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: Some("Scoped Delete Movie".to_string()),
        catalog_sort_key: String::new(),
        slug: Some("scoped-delete-movie".to_string()),
        imdb_id: Some("tt9988776".to_string()),
        runtime_minutes: Some(90),
        popularity: None,
        content_status: Some("released".to_string()),
        language: Some("eng".to_string()),
        first_aired: Some("2024-01-01".to_string()),
        network: None,
        studio: Some("Scoped Studio".to_string()),
        country: Some("usa".to_string()),
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: Some("eng".to_string()),
        metadata_fetched_at: Some(now),
        min_availability: None,
        digital_release_date: Some("2024-01-01".to_string()),
        folder_path: None,
    };
    let title = ctx.titles.create(title).await.expect("create scoped title");

    let file_path = media_root.path().join("Scoped.Delete.Movie.2024.1080p.mkv");
    std::fs::write(&file_path, b"scoped-delete").expect("write media file");

    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4_096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(file_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: now,
        })
        .await
        .expect("create matching movie collection");

    let actor = User {
        id: Id::new().0,
        username: "scoped-delete-user".to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: scryer_domain::AppPermissionMask::NONE,
            libraries: HashMap::from([(
                custom_library_id.clone(),
                LibraryPermissionMask::from_permission(LibraryPermission::ManageTitles),
            )]),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    };
    UserRepository::create(&ctx.users, actor.clone())
        .await
        .expect("create scoped-delete actor");

    let preview_body = schema_exec(
        &ctx,
        &format!(
            r#"
            query {{
              deleteMediaFilePreview(fileId: "{file_id}") {{
                fingerprint
                requiresTypedConfirmation
              }}
            }}
            "#
        ),
        Some(actor.clone()),
    )
    .await;
    assert_no_errors(&preview_body);
    let preview = &preview_body["data"]["deleteMediaFilePreview"];
    assert_eq!(preview["requiresTypedConfirmation"], json!(false));
    let fingerprint = preview["fingerprint"]
        .as_str()
        .expect("preview fingerprint should be present");

    let stale_delete_body = schema_exec(
        &ctx,
        &format!(
            r#"
            mutation {{
              deleteMediaFile(input: {{
                fileId: "{file_id}",
                deleteFromDisk: true,
                previewFingerprint: "stale"
              }}) {{
                id
              }}
            }}
            "#
        ),
        Some(actor.clone()),
    )
    .await;
    assert!(
        stale_delete_body.get("errors").is_some(),
        "stale previews must be rejected before a deletion job is queued: {stale_delete_body}"
    );
    assert!(
        file_path.exists(),
        "stale previews must not delete the file"
    );

    let delete_body = schema_exec(
        &ctx,
        &format!(
            r#"
            mutation {{
              deleteMediaFile(input: {{
                fileId: "{file_id}",
                deleteFromDisk: true,
                previewFingerprint: "{fingerprint}"
              }}) {{
                id
                jobRun {{ id jobKey status }}
              }}
            }}
            "#
        ),
        Some(actor.clone()),
    )
    .await;
    assert_no_errors(&delete_body);
    assert_eq!(delete_body["data"]["deleteMediaFile"]["id"], file_id);
    assert_eq!(
        delete_body["data"]["deleteMediaFile"]["jobRun"]["jobKey"],
        "MEDIA_FILE_DELETION"
    );
    let delete_run_id = delete_body["data"]["deleteMediaFile"]["jobRun"]["id"]
        .as_str()
        .expect("delete job id");
    assert_eq!(
        wait_for_interactive_job(&ctx, &actor, JobKey::MediaFileDeletion, delete_run_id)
            .await
            .status,
        JobRunStatus::Completed
    );

    assert!(
        !file_path.exists(),
        "delete should remove the on-disk media file"
    );
    assert!(
        ctx.media_files
            .get_media_file_by_id(&file_id)
            .await
            .expect("lookup deleted media file")
            .is_none(),
        "delete should remove the media file row"
    );
    assert!(
        ctx.shows
            .list_collections_for_title(&title.id)
            .await
            .expect("list remaining collections")
            .into_iter()
            .all(|entry| entry.id != collection.id),
        "delete should remove the matching movie collection row"
    );

    let catalog_only_path = media_root
        .path()
        .join("Scoped.Delete.Movie.2024.Catalog.Only.mkv");
    std::fs::write(&catalog_only_path, b"catalog-only").expect("write catalog-only media file");
    let catalog_only_file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: catalog_only_path.to_string_lossy().to_string(),
            size_bytes: 4_096,
            quality_label: Some("720p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert catalog-only media file");

    let catalog_only_preview_body = schema_exec(
        &ctx,
        &format!(
            r#"
            query {{
              deleteMediaFilePreview(fileId: "{catalog_only_file_id}") {{
                fingerprint
              }}
            }}
            "#
        ),
        Some(actor.clone()),
    )
    .await;
    assert_no_errors(&catalog_only_preview_body);
    let catalog_only_fingerprint =
        catalog_only_preview_body["data"]["deleteMediaFilePreview"]["fingerprint"]
            .as_str()
            .expect("catalog-only preview fingerprint should be present");

    let catalog_only_delete_body = schema_exec(
        &ctx,
        &format!(
            r#"
            mutation {{
              deleteMediaFile(input: {{
                fileId: "{catalog_only_file_id}",
                previewFingerprint: "{catalog_only_fingerprint}"
              }}) {{
                id
                jobRun {{ id jobKey status }}
              }}
            }}
            "#
        ),
        Some(actor.clone()),
    )
    .await;
    assert_no_errors(&catalog_only_delete_body);
    assert_eq!(
        catalog_only_delete_body["data"]["deleteMediaFile"]["id"],
        catalog_only_file_id
    );
    let catalog_only_delete_run_id = catalog_only_delete_body["data"]["deleteMediaFile"]["jobRun"]
        ["id"]
        .as_str()
        .expect("catalog-only delete job id");
    assert_eq!(
        wait_for_interactive_job(
            &ctx,
            &actor,
            JobKey::MediaFileDeletion,
            catalog_only_delete_run_id,
        )
        .await
        .status,
        JobRunStatus::Completed
    );
    assert!(
        catalog_only_path.exists(),
        "omitting deleteFromDisk should remove only the catalog row"
    );
    assert!(
        ctx.media_files
            .get_media_file_by_id(&catalog_only_file_id)
            .await
            .expect("lookup catalog-only deleted media file")
            .is_none(),
        "catalog-only delete should remove the media file row"
    );
}

/// Most queries require a user in the request context.  Executing one via the
/// schema directly (without injecting a User) must return an authentication
/// error rather than leaking data.
#[tokio::test]
async fn unauthenticated_request_returns_error() {
    let ctx = TestContext::new().await;

    // `titles` calls actor_from_ctx — must fail without a user in context.
    let body = schema_exec(&ctx, "{ titles { items { id } } }", None).await;

    let errors = body["errors"].as_array().expect("should have errors");
    assert!(
        !errors.is_empty(),
        "unauthenticated request should return errors"
    );
    let codes: Vec<&str> = errors
        .iter()
        .filter_map(|e| e["extensions"]["code"].as_str())
        .collect();
    assert!(
        codes.contains(&"AUTHENTICATION_REQUIRED"),
        "error code should require authentication: {codes:?}"
    );
}

/// After obtaining a JWT via the login mutation, the caller can authenticate
/// that token to retrieve the User and use it on a protected query.
#[tokio::test]
async fn authenticated_request_with_valid_token_succeeds() {
    let ctx = TestContext::new().await;

    // Create a user with an explicit password and ViewCatalog so the
    // protected `titles` query can succeed.
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let view_grant = scryer_domain::LibraryGrant {
        user_id: String::new(),
        library_id: scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie),
        permissions: scryer_domain::LibraryPermissionMask::from_permission(
            scryer_domain::LibraryPermission::View,
        ),
    };
    ctx.app
        .create_user(
            &admin,
            "authtest".to_string(),
            "s3cr3t!!".to_string(),
            scryer_domain::AppPermissionMask::NONE,
            vec![view_grant],
        )
        .await
        .unwrap();

    // Step 1: log in and capture the token.
    let login_body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "authtest", password: "s3cr3t!!" }) { token } }"#,
        None,
    )
    .await;
    assert!(
        login_body["errors"].is_null(),
        "login should succeed: {login_body}"
    );
    let token = login_body["data"]["login"]["token"]
        .as_str()
        .expect("token should be a string")
        .to_string();

    // Step 2: validate the token to recover the User.
    let user = ctx
        .app
        .authenticate_token(&token)
        .await
        .expect("token should be valid");

    // Step 3: execute a protected query with the authenticated user attached.
    let body = schema_exec(&ctx, "{ titles { items { id } } }", Some(user)).await;
    assert!(
        body["errors"].is_null(),
        "authenticated query should not error: {body}"
    );
    assert!(body["data"]["titles"]["items"].is_array());
}

#[tokio::test]
async fn token_is_revoked_after_permission_change_until_relogin() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let library_id = scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie);

    let create_body = schema_exec(
        &ctx,
        &format!(
            r#"mutation {{
            createUser(input: {{
                username: "entrevoketest",
                password: "s3cr3t!!",
                appPermissions: [],
                libraryPermissions: [{{ libraryId: "{library_id}", permissions: [VIEW] }}]
            }}) {{
                id
                username
            }}
        }}"#
        ),
        Some(admin.clone()),
    )
    .await;
    assert!(
        create_body["errors"].is_null(),
        "createUser should succeed: {create_body}"
    );
    let user_id = create_body["data"]["createUser"]["id"]
        .as_str()
        .expect("created user id")
        .to_string();

    let login_before = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "entrevoketest", password: "s3cr3t!!" }) { token } }"#,
        None,
    )
    .await;
    assert!(
        login_before["errors"].is_null(),
        "initial login should succeed: {login_before}"
    );
    let old_token = login_before["data"]["login"]["token"]
        .as_str()
        .expect("token should be a string")
        .to_string();

    let update_body = schema_exec(
        &ctx,
        &format!(
            r#"mutation {{
                setUserLibraryPermissions(input: {{
                    userId: "{user_id}",
                    grants: [{{ libraryId: "{library_id}", permissions: [VIEW, REQUEST, AUTO_APPROVE_REQUESTS, MANAGE_TITLES] }}]
                }}) {{
                    id
                    libraryPermissions {{ libraryId permissions }}
                }}
            }}"#
        ),
        Some(admin),
    )
    .await;
    assert!(
        update_body["errors"].is_null(),
        "setUserLibraryPermissions should succeed: {update_body}"
    );
    let permissions =
        update_body["data"]["setUserLibraryPermissions"]["libraryPermissions"][0]["permissions"]
            .as_array()
            .expect("permissions should be an array")
            .iter()
            .map(|value| value.as_str().expect("permission string"))
            .collect::<Vec<_>>();
    assert!(permissions.contains(&"VIEW"));
    assert!(permissions.contains(&"MANAGE_TITLES"));
    assert!(permissions.contains(&"REQUEST"));
    assert!(permissions.contains(&"AUTO_APPROVE_REQUESTS"));

    let old_result = ctx.app.authenticate_token(&old_token).await;
    assert!(
        old_result.is_err(),
        "token issued before permission change should be rejected"
    );

    let login_after = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "entrevoketest", password: "s3cr3t!!" }) { token } }"#,
        None,
    )
    .await;
    assert!(
        login_after["errors"].is_null(),
        "re-login should succeed after permission change: {login_after}"
    );
    let new_token = login_after["data"]["login"]["token"]
        .as_str()
        .expect("refreshed token should be a string")
        .to_string();

    let decoded = ctx
        .app
        .authenticate_token(&new_token)
        .await
        .expect("refreshed token should authenticate");
    let authorization = ctx
        .app
        .load_user_authorization(&decoded)
        .await
        .expect("load authorization");
    assert!(
        authorization
            .has_library_permission(&library_id, scryer_domain::LibraryPermission::ManageTitles,)
    );
    assert!(
        !authorization
            .has_library_permission(&library_id, scryer_domain::LibraryPermission::Request)
    );
}

/// A token issued for a different issuer (or an arbitrary tampered token)
/// must be rejected by `authenticate_token` — not by a GraphQL error but as
/// a hard application-level failure.
#[tokio::test]
async fn tampered_token_is_rejected_by_authenticate_token() {
    let ctx = TestContext::new().await;

    // Craft a syntactically valid-looking but unsigned JWT (three base64 parts).
    let fake_token = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJoYWNrZXIifQ.invalidsig";

    let result = ctx.app.authenticate_token(fake_token).await;
    assert!(
        result.is_err(),
        "tampered/unsigned token must not be accepted"
    );
}

/// Creating a user with `createUser` and then logging in as that user must
/// succeed end-to-end — confirming that the password is stored and validated
/// consistently.
#[tokio::test]
async fn newly_created_user_can_login() {
    let ctx = TestContext::new().await;

    // The admin user must exist before we can create another user
    // (createUser requires user and permission management access).
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    // Create a new user as admin.
    let create_body = schema_exec(
        &ctx,
        r#"mutation { createUser(input: { username: "newuser", password: "s3cr3t!!", appPermissions: [], libraryPermissions: [] }) { id username } }"#,
        Some(admin),
    )
    .await;
    assert!(
        create_body["errors"].is_null(),
        "createUser should succeed: {create_body}"
    );
    assert_eq!(create_body["data"]["createUser"]["username"], "newuser");

    // Log in as the newly created user.
    let login_body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "newuser", password: "s3cr3t!!" }) { token user { username } } }"#,
        None,
    )
    .await;
    assert!(
        login_body["errors"].is_null(),
        "new user login should succeed: {login_body}"
    );
    let token = login_body["data"]["login"]["token"].as_str().unwrap();
    assert!(!token.is_empty());
    assert_eq!(login_body["data"]["login"]["user"]["username"], "newuser");
}

#[tokio::test]
async fn graphql_local_password_login_masks_account_disclosure() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let create_body = schema_exec(
        &ctx,
        r#"mutation { createUser(input: { username: "maskedlocal", password: "s3cr3t!!", appPermissions: [], libraryPermissions: [] }) { id username } }"#,
        Some(admin),
    )
    .await;
    assert_no_errors(&create_body);

    ctx.users
        .create(User {
            id: "masked-no-password-user".to_string(),
            username: "maskednopass".to_string(),
            password_hash: None,
            account_kind: Default::default(),
            authorization: Default::default(),
        })
        .await
        .expect("create passwordless user");

    async fn failed_login_shape(
        ctx: &TestContext,
        username: &str,
        password: &str,
    ) -> (String, String) {
        let body = schema_exec(
            ctx,
            &format!(
                r#"
                mutation {{
                  login(input: {{ username: "{username}", password: "{password}" }}) {{
                    token
                  }}
                }}
                "#
            ),
            None,
        )
        .await;
        let serialized = body.to_string().to_lowercase();
        for leaked in [
            "not invited",
            "not found",
            "disabled",
            "credentials unavailable",
        ] {
            assert!(
                !serialized.contains(leaked),
                "login response leaked {leaked}: {body}"
            );
        }
        first_graphql_error_message_and_code(&body)
    }

    let unknown = failed_login_shape(&ctx, "maskedmissing", "s3cr3t!!").await;
    let wrong_password = failed_login_shape(&ctx, "maskedlocal", "wrongpass").await;
    let no_password = failed_login_shape(&ctx, "maskednopass", "s3cr3t!!").await;
    let empty_username = failed_login_shape(&ctx, "", "s3cr3t!!").await;
    let empty_password = failed_login_shape(&ctx, "maskedlocal", "").await;

    assert_eq!(
        unknown.0,
        "Sign-in failed. Check your sign-in details and try again."
    );
    assert_eq!(unknown.1, "LOGIN_FAILED");
    assert_eq!(wrong_password, unknown);
    assert_eq!(no_password, unknown);
    assert_eq!(empty_username, unknown);
    assert_eq!(empty_password, unknown);
}

#[tokio::test]
async fn graphql_local_password_login_requires_mfa_enrollment_when_enabled() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let admin = ctx
        .app
        .set_initial_own_password(&admin, "admin-pass1".to_string())
        .await
        .expect("set initial default admin password");

    let create_body = schema_exec(
        &ctx,
        r#"mutation { createUser(input: { username: "localmfa", password: "s3cr3t!!", appPermissions: [], libraryPermissions: [] }) { id username } }"#,
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&create_body);

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: true
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: true
            totpRequireJellyfinLogin: false
          }) {
            effectiveFormLoginEnabled
            mfaRequirePasswordLogin
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateSecuritySettings"]["effectiveFormLoginEnabled"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["mfaRequirePasswordLogin"],
        true
    );

    let login_body = schema_exec(
        &ctx,
        r#"
        mutation {
          login(input: { username: "localmfa", password: "s3cr3t!!" }) {
            token
            mfaEnrollmentRequired
            mfaVerifiedUntil
            user { username }
          }
        }
        "#,
        None,
    )
    .await;
    assert_no_errors(&login_body);
    let payload = &login_body["data"]["login"];
    assert_eq!(payload["mfaEnrollmentRequired"], true);
    assert!(payload["mfaVerifiedUntil"].is_null());
    assert_eq!(payload["user"]["username"], "localmfa");

    let token = payload["token"].as_str().expect("enrollment token");
    let (_user, claims) = ctx
        .app
        .authenticate_token_with_claims(token)
        .await
        .expect("authenticate enrollment token");
    assert_eq!(claims.session_scope, JwtSessionScope::MfaEnrollment);

    let enrollment_start = gql_with_token(
        &ctx,
        r#"mutation { totpEnrollmentStart { challengeId secretBase32 } }"#,
        json!({}),
        token,
    )
    .await;
    assert_no_errors(&enrollment_start);
    let challenge_id = enrollment_start["data"]["totpEnrollmentStart"]["challengeId"]
        .as_str()
        .expect("challenge id");
    let secret_base32 = enrollment_start["data"]["totpEnrollmentStart"]["secretBase32"]
        .as_str()
        .expect("secret");
    let code = test_totp_code(secret_base32);

    let complete = gql_with_token(
        &ctx,
        r#"
        mutation CompleteLoginMfaEnrollment($input: TotpEnrollmentCompleteInput!) {
          completeLoginMfaEnrollment(input: $input) {
            recoveryCodes
            login {
              token
              mfaEnrollmentRequired
              mfaVerifiedUntil
              user { username }
            }
          }
        }
        "#,
        json!({
            "input": {
                "challengeId": challenge_id,
                "code": code
            }
        }),
        token,
    )
    .await;
    assert_no_errors(&complete);
    let complete_payload = &complete["data"]["completeLoginMfaEnrollment"];
    assert!(
        complete_payload["recoveryCodes"]
            .as_array()
            .is_some_and(|codes| !codes.is_empty()),
        "login MFA enrollment should return recovery codes: {complete}"
    );
    let login_payload = &complete_payload["login"];
    assert_eq!(login_payload["mfaEnrollmentRequired"], false);
    assert!(login_payload["mfaVerifiedUntil"].as_str().is_some());
    assert_eq!(login_payload["user"]["username"], "localmfa");
    let full_token = login_payload["token"].as_str().expect("full token");
    let (_user, full_claims) = ctx
        .app
        .authenticate_token_with_claims(full_token)
        .await
        .expect("authenticate full token");
    assert_eq!(full_claims.session_scope, JwtSessionScope::Full);
    assert!(full_claims.mfa_verified_until.is_some());
}

#[tokio::test]
async fn graphql_jellyfin_login_requires_mfa_enrollment_when_enabled() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let admin = ctx
        .app
        .set_initial_own_password(&admin, "admin-pass1".to_string())
        .await
        .expect("set initial default admin password");

    let now = Utc::now();
    let media_servers =
        MediaServerConnectionStore::new(ctx.db.datastore(), ctx.db.encryption_key_state());
    MediaServerConnectionRepository::create(
        &media_servers,
        MediaServerConnection {
            id: "jellyfin-main".to_string(),
            provider: MediaServerProvider::Jellyfin,
            display_name: "Main Jellyfin".to_string(),
            base_url: ctx.smg_server.uri(),
            enabled: true,
            login_enabled: true,
            linking_enabled: false,
            auto_add_enabled: true,
            default_app_permissions: AppPermissionMask::NONE,
            default_library_grants: Vec::new(),
            machine_id: None,
            api_key: Some("jellyfin-api-key".to_string()),
            path_mappings: Vec::new(),
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .expect("seed Jellyfin media server connection");

    Mock::given(method("POST"))
        .and(path("/Users/AuthenticateByName"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "User": {
                "Id": "jellyfin-mfa-user-id",
                "Name": "jellyfin-mfa"
            }
        })))
        .mount(&ctx.smg_server)
        .await;

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
            totpRequireJellyfinLogin: true
          }) {
            effectiveFormLoginEnabled
            totpRequireJellyfinLogin
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateSecuritySettings"]["effectiveFormLoginEnabled"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["totpRequireJellyfinLogin"],
        true
    );

    let login_body = gql(
        &ctx,
        r#"
        mutation LoginWithJellyfin($connectionId: ID!, $username: String!, $password: String!) {
          loginWithJellyfin(input: {
            connectionId: $connectionId
            username: $username
            password: $password
          }) {
            token
            mfaEnrollmentRequired
            mfaVerifiedUntil
            user { username }
          }
        }
        "#,
        json!({
            "connectionId": "jellyfin-main",
            "username": "jellyfin-mfa",
            "password": "jellyfin-pass1",
        }),
    )
    .await;
    assert_no_errors(&login_body);
    let payload = &login_body["data"]["loginWithJellyfin"];
    assert_eq!(payload["mfaEnrollmentRequired"], true);
    assert!(payload["mfaVerifiedUntil"].is_null());
    assert_eq!(payload["user"]["username"], "jellyfin-mfa");

    let token = payload["token"].as_str().expect("enrollment token");
    let (_user, claims) = ctx
        .app
        .authenticate_token_with_claims(token)
        .await
        .expect("authenticate enrollment token");
    assert_eq!(claims.session_scope, JwtSessionScope::MfaEnrollment);

    let enrollment_start = gql_with_token(
        &ctx,
        r#"mutation { totpEnrollmentStart { challengeId secretBase32 } }"#,
        json!({}),
        token,
    )
    .await;
    assert_no_errors(&enrollment_start);
    let challenge_id = enrollment_start["data"]["totpEnrollmentStart"]["challengeId"]
        .as_str()
        .expect("challenge id");
    let secret_base32 = enrollment_start["data"]["totpEnrollmentStart"]["secretBase32"]
        .as_str()
        .expect("secret");
    let code = test_totp_code(secret_base32);

    let complete = gql_with_token(
        &ctx,
        r#"
        mutation CompleteLoginMfaEnrollment($input: TotpEnrollmentCompleteInput!) {
          completeLoginMfaEnrollment(input: $input) {
            recoveryCodes
            login {
              token
              mfaEnrollmentRequired
              mfaVerifiedUntil
              user { username }
            }
          }
        }
        "#,
        json!({
            "input": {
                "challengeId": challenge_id,
                "code": code
            }
        }),
        token,
    )
    .await;
    assert_no_errors(&complete);
    let complete_payload = &complete["data"]["completeLoginMfaEnrollment"];
    assert!(
        complete_payload["recoveryCodes"]
            .as_array()
            .is_some_and(|codes| !codes.is_empty()),
        "Jellyfin login MFA enrollment should return recovery codes: {complete}"
    );
    let login_payload = &complete_payload["login"];
    assert_eq!(login_payload["mfaEnrollmentRequired"], false);
    assert!(login_payload["mfaVerifiedUntil"].as_str().is_some());
    assert_eq!(login_payload["user"]["username"], "jellyfin-mfa");
    let full_token = login_payload["token"].as_str().expect("full token");
    let (_user, full_claims) = ctx
        .app
        .authenticate_token_with_claims(full_token)
        .await
        .expect("authenticate full token");
    assert_eq!(full_claims.session_scope, JwtSessionScope::Full);
    assert!(full_claims.mfa_verified_until.is_some());
}

#[tokio::test]
async fn graphql_jellyfin_pending_invite_for_existing_user_starts_mfa_enrollment() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let admin = ctx
        .app
        .set_initial_own_password(&admin, "admin-pass1".to_string())
        .await
        .expect("set initial default admin password");

    let user = schema_exec(
        &ctx,
        r#"mutation { createUser(input: { username: "jellyfin-invite-mfa", password: "testpass123", appPermissions: [], libraryPermissions: [] }) { id username } }"#,
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&user);
    let user_id = user["data"]["createUser"]["id"]
        .as_str()
        .expect("created user id");

    let now = Utc::now();
    let media_servers =
        MediaServerConnectionStore::new(ctx.db.datastore(), ctx.db.encryption_key_state());
    MediaServerConnectionRepository::create(
        &media_servers,
        MediaServerConnection {
            id: "jellyfin-invite-main".to_string(),
            provider: MediaServerProvider::Jellyfin,
            display_name: "Jellyfin Invite MFA".to_string(),
            base_url: ctx.smg_server.uri(),
            enabled: true,
            login_enabled: true,
            linking_enabled: false,
            auto_add_enabled: false,
            default_app_permissions: AppPermissionMask::NONE,
            default_library_grants: Vec::new(),
            machine_id: None,
            api_key: Some("jellyfin-api-key".to_string()),
            path_mappings: Vec::new(),
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .expect("seed Jellyfin media server connection");

    Mock::given(method("GET"))
        .and(path("/Users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "Id": "jellyfin-invite-user-id",
            "Name": "jellyfin-invite-user"
        }])))
        .mount(&ctx.smg_server)
        .await;

    let invite = schema_exec(
        &ctx,
        &format!(
            r#"mutation {{
              createExternalAccountInvite(input: {{
                userId: "{user_id}"
                provider: JELLYFIN
                connectionId: "jellyfin-invite-main"
                providerUserIdentifier: "jellyfin-invite-user"
                providerUserId: "jellyfin-invite-user-id"
              }}) {{ status }}
            }}"#,
        ),
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&invite);
    assert_eq!(
        invite["data"]["createExternalAccountInvite"]["status"],
        "PENDING_CLAIM"
    );

    Mock::given(method("POST"))
        .and(path("/Users/AuthenticateByName"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "User": {
                "Id": "jellyfin-invite-user-id",
                "Name": "jellyfin-invite-user"
            }
        })))
        .mount(&ctx.smg_server)
        .await;

    let security = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: true
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: true
          }) {
            totpRequireJellyfinLogin
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert_no_errors(&security);

    let login = gql(
        &ctx,
        r#"
        mutation LoginWithJellyfin($connectionId: ID!, $username: String!, $password: String!) {
          loginWithJellyfin(input: {
            connectionId: $connectionId
            username: $username
            password: $password
          }) {
            token
            mfaEnrollmentRequired
            user { username }
          }
        }
        "#,
        json!({
            "connectionId": "jellyfin-invite-main",
            "username": "jellyfin-invite-user",
            "password": "jellyfin-pass1",
        }),
    )
    .await;
    assert_no_errors(&login);
    let payload = &login["data"]["loginWithJellyfin"];
    assert_eq!(payload["mfaEnrollmentRequired"], true);
    assert_eq!(payload["user"]["username"], "jellyfin-invite-mfa");

    let token = payload["token"].as_str().expect("enrollment token");
    let (_user, claims) = ctx
        .app
        .authenticate_token_with_claims(token)
        .await
        .expect("authenticate enrollment token");
    assert_eq!(claims.session_scope, JwtSessionScope::MfaEnrollment);

    let enrollment_start = gql_with_token(
        &ctx,
        r#"mutation { totpEnrollmentStart { challengeId secretBase32 } }"#,
        json!({}),
        token,
    )
    .await;
    assert_no_errors(&enrollment_start);
    assert!(
        enrollment_start["data"]["totpEnrollmentStart"]["challengeId"]
            .as_str()
            .is_some(),
        "pending Jellyfin invite should start MFA enrollment: {enrollment_start}"
    );
}

#[tokio::test]
async fn graphql_local_password_login_with_existing_totp_requires_and_accepts_code() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let admin = ctx
        .app
        .set_initial_own_password(&admin, "admin-pass1".to_string())
        .await
        .expect("set initial default admin password");

    let create_body = schema_exec(
        &ctx,
        r#"mutation { createUser(input: { username: "localmfa_totp", password: "s3cr3t!!", appPermissions: [], libraryPermissions: [] }) { id username } }"#,
        Some(admin.clone()),
    )
    .await;
    assert_no_errors(&create_body);

    let user = ctx
        .app
        .authenticate_credentials("localmfa_totp", "s3cr3t!!")
        .await
        .expect("authenticate local user");
    let enrollment = ctx
        .app
        .totp_enrollment_start(&user)
        .await
        .expect("start TOTP enrollment");
    let enrollment_code = test_totp_code(&enrollment.secret_base32);
    ctx.app
        .totp_enrollment_complete(&user, &enrollment.challenge_id, &enrollment_code)
        .await
        .expect("complete TOTP enrollment");

    let totp_store = TotpStore::new(ctx.db.datastore(), ctx.db.encryption_key_state());
    let mut credential = totp_store
        .get_credential_for_user(&user.id)
        .await
        .expect("load TOTP credential")
        .expect("TOTP credential");
    credential.last_accepted_step = None;
    totp_store
        .upsert_credential(credential)
        .await
        .expect("reset accepted TOTP step");

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: true
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: true
            totpRequireJellyfinLogin: false
          }) {
            effectiveFormLoginEnabled
            mfaRequirePasswordLogin
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert_no_errors(&update);

    let missing_code = schema_exec(
        &ctx,
        r#"
        mutation {
          login(input: { username: "localmfa_totp", password: "s3cr3t!!" }) {
            token
          }
        }
        "#,
        None,
    )
    .await;
    let errors = missing_code["errors"]
        .as_array()
        .expect("expected missing-code GraphQL errors");
    assert!(
        !errors.is_empty(),
        "expected local password login to require TOTP: {missing_code}"
    );
    assert_eq!(
        errors[0]["extensions"]["code"], "MFA_STEP_UP_REQUIRED",
        "unexpected missing-code rejection shape: {missing_code}"
    );

    let invalid_code = schema_exec(
        &ctx,
        r#"
        mutation {
          login(input: { username: "localmfa_totp", password: "s3cr3t!!", totpCode: "abc123" }) {
            token
          }
        }
        "#,
        None,
    )
    .await;
    let errors = invalid_code["errors"]
        .as_array()
        .expect("expected invalid-code GraphQL errors");
    assert!(
        !errors.is_empty(),
        "expected invalid TOTP code to be rejected: {invalid_code}"
    );
    assert_eq!(
        errors[0]["extensions"]["code"], "TOTP_INVALID_CODE",
        "unexpected invalid-code rejection shape: {invalid_code}"
    );

    let valid_code = test_totp_code(&enrollment.secret_base32);
    let valid_login = schema_exec(
        &ctx,
        &format!(
            r#"
            mutation {{
              login(input: {{ username: "localmfa_totp", password: "s3cr3t!!", totpCode: "{valid_code}" }}) {{
                token
                mfaEnrollmentRequired
                mfaVerifiedUntil
                user {{ username }}
              }}
            }}
            "#
        ),
        None,
    )
    .await;
    assert_no_errors(&valid_login);
    let payload = &valid_login["data"]["login"];
    assert_eq!(payload["mfaEnrollmentRequired"], false);
    assert!(payload["mfaVerifiedUntil"].as_str().is_some());
    assert_eq!(payload["user"]["username"], "localmfa_totp");
    let token = payload["token"].as_str().expect("full token");
    let (_user, claims) = ctx
        .app
        .authenticate_token_with_claims(token)
        .await
        .expect("authenticate full token");
    assert_eq!(claims.session_scope, JwtSessionScope::Full);
    assert!(claims.mfa_verified_until.is_some());
}
