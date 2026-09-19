use super::*;

#[tokio::test]
async fn graphql_auth_runtime_state_is_public() {
    let ctx = TestContext::new().await;

    let body = schema_exec(
        &ctx,
        r#"
        query AuthRuntimeState {
          authRuntimeState {
            effectiveFormLoginEnabled
            skipLoginForLocalIps
            passkeyEnabled
            defaultPersistSession
            mfaRequireConfigStepUp
          }
        }
        "#,
        None,
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(
        body["data"]["authRuntimeState"]["effectiveFormLoginEnabled"],
        false
    );
    assert_eq!(
        body["data"]["authRuntimeState"]["skipLoginForLocalIps"],
        false
    );
    assert_eq!(body["data"]["authRuntimeState"]["passkeyEnabled"], false);
    assert_eq!(
        body["data"]["authRuntimeState"]["defaultPersistSession"], false,
        "schema execution without HTTP provenance must fail closed"
    );
    assert_eq!(
        body["data"]["authRuntimeState"]["mfaRequireConfigStepUp"],
        false
    );
}

#[tokio::test]
async fn graphql_auth_runtime_state_exposes_config_step_up_without_manage_users() {
    let ctx = TestContext::new().await;
    let (_admin, _token, _totp_code) =
        enable_form_login_with_config_step_up(&ctx, "admin", "admin-pass1").await;
    let settings_actor = User {
        id: Id::new().0,
        username: "catalog-settings-manager".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::from_permissions([
                scryer_domain::AppPermission::ManageCatalogSettings,
            ]),
            libraries: HashMap::new(),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    };

    let body = schema_exec(
        &ctx,
        r#"
        query AuthRuntimeState {
          authRuntimeState {
            effectiveFormLoginEnabled
            mfaRequireConfigStepUp
          }
        }
        "#,
        Some(settings_actor),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(
        body["data"]["authRuntimeState"]["effectiveFormLoginEnabled"],
        true
    );
    assert_eq!(
        body["data"]["authRuntimeState"]["mfaRequireConfigStepUp"],
        true
    );
}

#[tokio::test]
async fn graphql_passkey_register_start_requires_authentication() {
    let ctx = TestContext::new().await;

    let body = schema_exec(
        &ctx,
        r#"
        mutation PasskeyRegisterStart {
          webauthnRegisterStart {
            challengeId
          }
        }
        "#,
        None,
    )
    .await;

    let (_message, code) = first_graphql_error_message_and_code(&body);
    assert_eq!(code, "AUTHENTICATION_REQUIRED");
}

#[tokio::test]
async fn graphql_passkey_authenticate_start_is_public() {
    let ctx = TestContext::new().await;

    // Pause the clock: work takes no virtual time, while a login-failure delay
    // is a tokio sleep that advances virtual time by the whole delay.
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    let body = schema_exec(
        &ctx,
        r#"
        mutation PasskeyAuthenticateStart {
          webauthnAuthenticateStart {
            challengeId
          }
        }
        "#,
        None,
    )
    .await;
    let elapsed = started.elapsed();

    let (message, code) = first_graphql_error_message_and_code(&body);
    assert_eq!(
        message,
        "Sign-in failed. Check your sign-in details and try again."
    );
    assert_eq!(code, "LOGIN_FAILED");
    assert!(
        elapsed.is_zero(),
        "form-login-disabled passkey auth should not use login timing fuzz, slept {elapsed:?}"
    );

    let started = tokio::time::Instant::now();
    let complete_body = schema_exec(
        &ctx,
        r#"
        mutation PasskeyAuthenticateComplete {
          webauthnAuthenticateComplete(input: { challengeId: "missing", responseJson: "{}" }) {
            token
          }
        }
        "#,
        None,
    )
    .await;
    let elapsed = started.elapsed();
    let (message, code) = first_graphql_error_message_and_code(&complete_body);
    assert_eq!(
        message,
        "Sign-in failed. Check your sign-in details and try again."
    );
    assert_eq!(code, "LOGIN_FAILED");
    assert!(
        elapsed.is_zero(),
        "form-login-disabled passkey completion should not use login timing fuzz, slept {elapsed:?}"
    );
}

#[tokio::test]
async fn graphql_passkey_management_requires_form_login_when_disabled() {
    let mut ctx = TestContext::new().await;
    let origin = url::Url::parse("https://scryer.test").expect("valid WebAuthn origin");
    let webauthn = webauthn_rs::WebauthnBuilder::new("scryer.test", &origin)
        .expect("valid WebAuthn builder")
        .build()
        .expect("valid WebAuthn runtime");
    ctx.app.webauthn = scryer_application::RuntimeFeature::enabled(std::sync::Arc::new(webauthn));
    ctx.schema = scryer_interface::context::build_schema(ctx.app.clone(), ctx.auth_runtime.clone());

    let admin = ctx
        .app
        .attach_user_authorization(
            ctx.app
                .find_or_create_default_user()
                .await
                .expect("default user should exist"),
        )
        .await
        .expect("default user authorization");

    let list_body = schema_exec(
        &ctx,
        r#"
        query MyPasskeys {
          myPasskeys {
            id
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;
    let errors = list_body["errors"].as_array().expect("graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("graphql error message");
    assert_eq!(
        message,
        "validation: passkey authentication is unavailable while form login is disabled"
    );
}

#[tokio::test]
async fn graphql_my_passkeys_requires_authentication() {
    let ctx = TestContext::new().await;

    let body = schema_exec(
        &ctx,
        r#"
        query MyPasskeys {
          myPasskeys {
            id
          }
        }
        "#,
        None,
    )
    .await;

    let (_message, code) = first_graphql_error_message_and_code(&body);
    assert_eq!(code, "AUTHENTICATION_REQUIRED");
}
