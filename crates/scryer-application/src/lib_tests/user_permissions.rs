use super::*;

#[tokio::test]
async fn update_user_library_permissions_changes_grants() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let grants = test_library_grants_from_presets(&[
        TestPermissionPreset::CatalogView,
        TestPermissionPreset::TitleManagement,
    ]);
    let updated = app
        .set_user_library_permissions(&user, &created.id, grants)
        .await
        .expect("update permissions");

    let authorization = app
        .load_user_authorization(&updated)
        .await
        .expect("load authorization");
    assert!(
        authorization.has_any_library_permission(scryer_domain::LibraryPermission::ManageTitles)
    );
}

#[tokio::test]
async fn update_user_password_is_hashed() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "password-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let first_updated = app
        .set_user_password(&user, &created.id, "after-pass".to_string())
        .await
        .expect("update password");
    let first_auth_session_version = app
        .services
        .identity
        .users
        .auth_session_version(&created.id)
        .await
        .expect("load first authentication epoch")
        .expect("first authentication epoch");

    let second_updated = app
        .set_user_password(&user, &created.id, "final-pass".to_string())
        .await
        .expect("replace password unconditionally");
    let second_auth_session_version = app
        .services
        .identity
        .users
        .auth_session_version(&created.id)
        .await
        .expect("load second authentication epoch")
        .expect("second authentication epoch");

    assert!(first_updated.password_hash.is_some());
    assert_ne!(
        first_updated.password_hash, created.password_hash,
        "password hash should change when password is updated"
    );
    assert_ne!(second_updated.password_hash, Some("final-pass".to_string()));
    assert_ne!(first_auth_session_version, second_auth_session_version);
}

#[tokio::test]
async fn create_user_rejects_password_shorter_than_minimum() {
    let (app, user) = bootstrap();

    let result = create_user_with_permissions(
        &app,
        &user,
        "short-password-user",
        "1234567",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn set_user_password_rejects_password_shorter_than_minimum() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "password-reset-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let result = app
        .set_user_password(&user, &created.id, "1234567".to_string())
        .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn self_password_change_is_hashed() {
    let (app, admin) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &admin,
        "self-password-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let updated = app
        .change_own_password(
            &created,
            "after-pass".to_string(),
            "before-pass".to_string(),
        )
        .await
        .expect("update own password");

    assert!(updated.password_hash.is_some());
    assert_ne!(
        updated.password_hash, created.password_hash,
        "password hash should change when password is updated"
    );
    assert_ne!(updated.password_hash, Some("after-pass".to_string()));
}

#[tokio::test]
async fn self_password_change_rejects_password_shorter_than_minimum() {
    let (app, admin) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &admin,
        "self-short-password-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let result = app
        .change_own_password(&created, "1234567".to_string(), "before-pass".to_string())
        .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn stale_self_password_change_cannot_overwrite_a_newer_change() {
    let users = Arc::new(MockUserRepo::default());
    let (app, admin) = bootstrap_with_user_repo(users.clone());
    let app = Arc::new(app);
    let created = create_user_with_permissions(
        &app,
        &admin,
        "self-password-race-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    users.pause_next_own_password_update().await;
    let stale_app = app.clone();
    let stale_actor = created.clone();
    let stale_change = tokio::spawn(async move {
        stale_app
            .change_own_password(
                &stale_actor,
                "stale-password".to_string(),
                "before-pass".to_string(),
            )
            .await
    });
    timeout(Duration::from_secs(5), users.wait_for_own_password_update())
        .await
        .expect("stale password change should reach the conditional write");

    app.change_own_password(
        &created,
        "winning-password".to_string(),
        "before-pass".to_string(),
    )
    .await
    .expect("newer password change should win");
    users.resume_own_password_update();

    let stale_result = timeout(Duration::from_secs(5), stale_change)
        .await
        .expect("stale password change should finish")
        .expect("stale password task should not panic");
    assert!(matches!(
        stale_result,
        Err(AppError::ReauthenticationRequired(_))
    ));

    let stored = app
        .services
        .identity
        .users
        .get_by_id(&created.id)
        .await
        .expect("load winning password")
        .expect("user should remain present");
    let hash = stored
        .password_hash
        .as_deref()
        .expect("stored password hash");
    assert!(app.validate_password("winning-password", hash).unwrap());
    assert!(!app.validate_password("stale-password", hash).unwrap());
}

#[tokio::test]
async fn concurrent_initial_password_claims_allow_exactly_one_winner() {
    let users = Arc::new(MockUserRepo::default());
    let (app, _) = bootstrap_with_user_repo(users.clone());
    let app = Arc::new(app);
    let mut user =
        test_user_with_app_permissions("initial-password-race-user", AppPermissionMask::NONE);
    user.authorization.actor_capabilities = scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT;
    let user = app
        .services
        .identity
        .users
        .create(user)
        .await
        .expect("create passwordless user");

    users.pause_next_own_password_update().await;
    let first_app = app.clone();
    let first_actor = user.clone();
    let first_claim = tokio::spawn(async move {
        first_app
            .set_initial_own_password(&first_actor, "first-password".to_string())
            .await
    });
    timeout(Duration::from_secs(5), users.wait_for_own_password_update())
        .await
        .expect("first claim should reach the conditional write");

    app.set_initial_own_password(&user, "winning-password".to_string())
        .await
        .expect("second claim should win");
    users.resume_own_password_update();

    let first_result = timeout(Duration::from_secs(5), first_claim)
        .await
        .expect("first claim should finish")
        .expect("first claim task should not panic");
    assert!(matches!(
        first_result,
        Err(AppError::ReauthenticationRequired(_))
    ));

    let stored = app
        .services
        .identity
        .users
        .get_by_id(&user.id)
        .await
        .expect("load winning password")
        .expect("user should remain present");
    let hash = stored
        .password_hash
        .as_deref()
        .expect("stored password hash");
    assert!(app.validate_password("winning-password", hash).unwrap());
    assert!(!app.validate_password("first-password", hash).unwrap());
}

#[tokio::test]
async fn set_initial_own_password_rejects_password_shorter_than_minimum() {
    let (app, _) = bootstrap();
    let mut user =
        test_user_with_app_permissions("initial-short-password-user", AppPermissionMask::NONE);
    user.authorization.actor_capabilities = scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT;
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create passwordless user");

    let result = app
        .set_initial_own_password(&user, "1234567".to_string())
        .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn set_initial_own_password_requires_own_account_capability() {
    let (app, _) = bootstrap();
    let user =
        test_user_with_app_permissions("initial-password-unauthorized", AppPermissionMask::NONE);
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create passwordless user");

    let result = app
        .set_initial_own_password(&user, "valid-password".to_string())
        .await;

    assert!(matches!(result, Err(AppError::Unauthorized(_))));
}

#[tokio::test]
async fn delete_other_user_removes_user() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "removable",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    app.delete_user(&user, &created.id)
        .await
        .expect("delete user");

    let users = app.list_users(&user).await.expect("list users");
    assert!(!users.iter().any(|entry| entry.id == created.id));
}

#[tokio::test]
async fn delete_user_rejects_removing_last_full_administrator() {
    let (app, actor) = bootstrap();
    let bootstrap_admin = app
        .find_or_create_default_user()
        .await
        .expect("create bootstrap admin");

    let result = app.delete_user(&actor, &bootstrap_admin.id).await;

    assert!(matches!(
        result,
        Err(AppError::Validation(message))
            if message == "cannot delete the default admin; disable its login instead"
    ));
}

#[tokio::test]
async fn delete_user_rejects_removing_bootstrap_admin_after_replacement_exists() {
    let (app, actor) = bootstrap();
    let bootstrap_admin = app
        .find_or_create_default_user()
        .await
        .expect("create bootstrap admin");
    app.create_user(
        &actor,
        "replacement-admin".to_string(),
        "password123".to_string(),
        scryer_domain::UserAuthorization::full_admin().app,
        vec![],
    )
    .await
    .expect("create replacement full admin");

    let result = app.delete_user(&actor, &bootstrap_admin.id).await;

    assert!(matches!(
        result,
        Err(AppError::Validation(message))
            if message == "cannot delete the default admin; disable its login instead"
    ));
    assert!(app.find_default_user().await.unwrap().is_some());
}

#[tokio::test]
async fn disabling_user_revokes_login_and_reenable_preserves_credentials() {
    let (app, actor) = bootstrap();
    let user = create_user_with_permissions(
        &app,
        &actor,
        "suspended-user",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");
    let password_hash = user.password_hash.clone();
    let old_token = app.issue_access_token(&user).await.expect("issue token");

    let disabled = app
        .set_user_login_enabled(&actor, &user.id, false, false)
        .await
        .expect("disable login");
    assert_eq!(
        disabled.login_status(),
        scryer_domain::UserLoginStatus::Disabled
    );
    assert_eq!(disabled.password_hash, password_hash);
    assert!(
        app.services
            .identity
            .users
            .auth_session_version(&user.id)
            .await
            .expect("load session version")
            .is_some()
    );
    assert!(
        app.authenticate_credentials("suspended-user", "password123")
            .await
            .is_err()
    );
    assert!(app.authenticate_token(&old_token).await.is_err());
    assert!(
        app.issue_oauth_access_token_with_source(
            &disabled,
            "authless-client",
            "authless-grant",
            OAuthAuthorizationSource::Authless,
        )
        .await
        .is_err()
    );

    let enabled = app
        .set_user_login_enabled(&actor, &user.id, true, false)
        .await
        .expect("enable login");
    assert_eq!(
        enabled.login_status(),
        scryer_domain::UserLoginStatus::Enabled
    );
    assert_eq!(enabled.password_hash, password_hash);
    assert!(
        app.authenticate_credentials("suspended-user", "password123")
            .await
            .is_ok()
    );
    assert!(app.authenticate_token(&old_token).await.is_err());
}

#[tokio::test]
async fn login_status_changes_reject_self_and_recovery_admin() {
    let (app, actor) = bootstrap();
    let stored_admin = app
        .find_or_create_default_user()
        .await
        .expect("create stored admin");
    let self_result = app
        .set_user_login_enabled(&stored_admin, &stored_admin.id, false, false)
        .await;
    assert!(matches!(self_result, Err(AppError::Validation(_))));

    let recovery = app
        .recover_reserved_admin_access("recovery-password")
        .await
        .expect("create recovery admin");
    let recovery_result = app
        .set_user_login_enabled(&actor, &recovery.id, false, false)
        .await;
    assert!(matches!(
        recovery_result,
        Err(AppError::Validation(message))
            if message == "recovery-admin login is managed by the environment"
    ));
}

#[tokio::test]
async fn form_login_prevents_disabling_last_usable_full_admin() {
    let (app, actor) = bootstrap();
    let full_admin = app
        .create_user(
            &actor,
            "only-login-admin".to_string(),
            "password123".to_string(),
            scryer_domain::UserAuthorization::full_admin().app,
            vec![],
        )
        .await
        .expect("create full admin");
    app.update_security_settings(
        &actor,
        UpdateSecuritySettings {
            form_login_enabled: true,
            password_min_length: 8,
            skip_login_for_local_ips: false,
            api_keys_restrict_to_system_settings_users: Some(false),
            mfa_require_config_step_up: false,
            mfa_require_password_login: false,
            totp_require_jellyfin_login: false,
            totp_require_emby_login: Some(false),
        },
    )
    .await
    .expect("enable form login");

    let result = app
        .set_user_login_enabled(&actor, &full_admin.id, false, true)
        .await;
    assert!(matches!(
        result,
        Err(AppError::Validation(message))
            if message == "cannot disable the last usable full administrator"
    ));
}

#[tokio::test]
async fn form_login_transition_requires_usable_admin_and_repairs_default_identity() {
    let (app, actor) = bootstrap();
    let settings = |form_login_enabled| UpdateSecuritySettings {
        form_login_enabled,
        password_min_length: 8,
        skip_login_for_local_ips: false,
        api_keys_restrict_to_system_settings_users: Some(false),
        mfa_require_config_step_up: false,
        mfa_require_password_login: false,
        totp_require_jellyfin_login: false,
        totp_require_emby_login: Some(false),
    };

    assert!(
        app.update_security_settings(&actor, settings(true))
            .await
            .is_err()
    );
    app.create_user(
        &actor,
        "replacement-login-admin".to_string(),
        "password123".to_string(),
        scryer_domain::UserAuthorization::full_admin().app,
        vec![],
    )
    .await
    .expect("create replacement admin");
    app.update_security_settings(&actor, settings(true))
        .await
        .expect("enable form login");
    assert!(app.find_default_user().await.unwrap().is_none());

    app.update_security_settings(&actor, settings(false))
        .await
        .expect("disable form login");
    let default_admin = app
        .find_default_user()
        .await
        .expect("load default admin")
        .expect("default admin repaired");
    let auth_session_version = scryer_domain::Id::new().0;
    let default_admin = app
        .services
        .identity
        .users
        .update_password_and_invalidate_sessions(
            &default_admin.id,
            app.hash_password("admin").expect("hash bootstrap password"),
            false,
            &auth_session_version,
        )
        .await
        .expect("seed bootstrap password");
    app.set_user_login_enabled(&actor, &default_admin.id, false, false)
        .await
        .expect("disable default admin login");

    app.update_security_settings(&actor, settings(true))
        .await
        .expect("disabled bootstrap admin should not block form login");
    assert_eq!(
        app.find_default_user()
            .await
            .expect("load default admin")
            .expect("default admin exists")
            .login_status(),
        scryer_domain::UserLoginStatus::Disabled
    );
}

#[tokio::test]
async fn disabled_default_admin_remains_available_only_for_authless_oauth() {
    let (app, actor) = bootstrap();
    let admin = app
        .find_or_create_default_user()
        .await
        .expect("create default admin");
    app.set_user_login_enabled(&actor, &admin.id, false, false)
        .await
        .expect("disable default admin login");

    let repaired = app
        .find_or_create_default_user()
        .await
        .expect("load default admin");
    assert_eq!(
        repaired.login_status(),
        scryer_domain::UserLoginStatus::Disabled
    );
    let repaired = app
        .attach_user_authorization(repaired)
        .await
        .expect("attach default admin permissions");
    assert!(
        repaired
            .authorization
            .app
            .contains(scryer_domain::UserAuthorization::full_admin().app)
    );
    assert!(app.issue_access_token(&repaired).await.is_err());

    let token = app
        .issue_oauth_access_token_with_source(
            &repaired,
            "authless-client",
            "authless-grant",
            OAuthAuthorizationSource::Authless,
        )
        .await
        .expect("issue authless OAuth token");
    let authenticated = app
        .authenticate_token(&token)
        .await
        .expect("authenticate authless OAuth token");
    assert_eq!(authenticated.id, repaired.id);
}

#[tokio::test]
async fn update_security_settings_preserves_api_key_policy_when_omitted() {
    let (app, admin) = bootstrap();
    app.update_security_settings(
        &admin,
        UpdateSecuritySettings {
            form_login_enabled: false,
            password_min_length: 8,
            skip_login_for_local_ips: false,
            api_keys_restrict_to_system_settings_users: Some(true),
            mfa_require_config_step_up: false,
            mfa_require_password_login: false,
            totp_require_jellyfin_login: false,
            totp_require_emby_login: Some(false),
        },
    )
    .await
    .expect("enable API-key restriction");

    let users_manager = app
        .create_user(
            &admin,
            "api-key-policy-user-manager".into(),
            "password123".into(),
            scryer_domain::AppPermissionMask::MANAGE_USERS,
            Vec::new(),
        )
        .await
        .expect("create ManageUsers actor");
    let users_manager = app
        .attach_user_authorization(users_manager)
        .await
        .expect("attach ManageUsers authorization");

    let updated = app
        .update_security_settings(
            &users_manager,
            UpdateSecuritySettings {
                form_login_enabled: false,
                password_min_length: 8,
                skip_login_for_local_ips: false,
                api_keys_restrict_to_system_settings_users: None,
                mfa_require_config_step_up: false,
                mfa_require_password_login: false,
                totp_require_jellyfin_login: false,
                totp_require_emby_login: Some(false),
            },
        )
        .await
        .expect("save unrelated security settings");

    assert!(updated.api_keys_restrict_to_system_settings_users);
    assert!(
        app.security_settings()
            .await
            .expect("load security settings")
            .api_keys_restrict_to_system_settings_users
    );
}
