use std::time::Instant;

use async_graphql::{Context, ID, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::{
    LoginFailureTimingClass, LoginVerificationMethod, LoginVerificationRequirement,
};
use scryer_domain::AppPermission;

use scryer_interface_core::{
    actor_from_ctx, app_from_ctx, auth_runtime_from_ctx, default_persist_session_from_ctx,
    login_verification_required_gql_error, persist_session_or_default,
    require_config_app_permission, to_gql_error, to_login_gql_error_after_timing,
};
use scryer_interface_media::mappers::{from_linked_account, from_user_with_auth_factor_status};
use scryer_interface_media::types::*;

#[derive(Default)]
pub struct UserMutations;

fn emby_connection_mode(value: EmbyConnectionModeValue) -> scryer_application::EmbyConnectionMode {
    match value {
        EmbyConnectionModeValue::Local => scryer_application::EmbyConnectionMode::Local,
        EmbyConnectionModeValue::Connect => scryer_application::EmbyConnectionMode::Connect,
    }
}

async fn user_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
) -> GqlResult<UserPayload> {
    let auth_factor_status = app
        .user_auth_factor_status(&user.id)
        .await
        .map_err(to_gql_error)?;
    Ok(from_user_with_auth_factor_status(user, auth_factor_status))
}

async fn login_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
    mfa_verified_until: Option<chrono::DateTime<Utc>>,
    mfa_step_up_verified_until: Option<chrono::DateTime<Utc>>,
    persist_session: bool,
    expected_auth_session_version: Option<&Option<String>>,
    password_change_required: bool,
) -> GqlResult<LoginPayload> {
    let user = app
        .load_user_for_auth_payload(&user)
        .await
        .map_err(to_gql_error)?;
    let token = if password_change_required {
        app.issue_password_change_required_token(
            &user,
            mfa_verified_until,
            persist_session,
            expected_auth_session_version,
        )
        .await
    } else {
        match expected_auth_session_version {
            Some(expected_auth_session_version) => {
                app.issue_access_token_with_mfa_and_persistence_at_auth_session_version(
                    &user,
                    mfa_verified_until,
                    mfa_step_up_verified_until,
                    persist_session,
                    expected_auth_session_version,
                )
                .await
            }
            None => {
                app.issue_access_token_with_mfa_and_persistence(
                    &user,
                    mfa_verified_until,
                    mfa_step_up_verified_until,
                    persist_session,
                )
                .await
            }
        }
    }
    .map_err(to_gql_error)?;
    let expires_at = Utc::now()
        + chrono::Duration::seconds(if password_change_required {
            app.mfa_enrollment_token_lifetime()
        } else {
            app.token_lifetime()
        });
    Ok(LoginPayload {
        token,
        user: user_payload_from_user(app, user).await?,
        expires_at,
        mfa_verified_until,
        security_action_verified_until: (!password_change_required)
            .then(|| app.security_action_verified_until()),
        mfa_enrollment_required: false,
        password_change_required,
        persist_session,
    })
}

async fn login_mfa_enrollment_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
    persist_session: bool,
    password_change_required_after_enrollment: bool,
    expected_auth_session_version: &Option<String>,
) -> GqlResult<LoginPayload> {
    let user = app
        .load_user_for_auth_payload(&user)
        .await
        .map_err(to_gql_error)?;
    let token = app
        .issue_mfa_enrollment_token(
            &user,
            persist_session,
            password_change_required_after_enrollment,
            Some(expected_auth_session_version),
        )
        .await
        .map_err(to_gql_error)?;
    let expires_at = Utc::now() + chrono::Duration::seconds(app.mfa_enrollment_token_lifetime());
    Ok(LoginPayload {
        token,
        user: user_payload_from_user(app, user).await?,
        expires_at,
        mfa_verified_until: None,
        security_action_verified_until: None,
        mfa_enrollment_required: true,
        password_change_required: false,
        persist_session,
    })
}

#[Object]
impl UserMutations {
    /// Creates a user with normalized app and library permissions after checking the manage-users permission.
    async fn create_user(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Username, password, and initial app and library permissions for the new user."
        )]
        input: CreateUserInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageUsers).await?;
        let app_permissions = scryer_domain::AppPermissionMask::from_permissions(
            input
                .app_permissions
                .into_iter()
                .map(|permission| permission.into_domain()),
        );
        let library_grants = input
            .library_permissions
            .into_iter()
            .map(|grant| {
                let permissions = scryer_domain::LibraryPermissionMask::from_permissions(
                    grant
                        .permissions
                        .into_iter()
                        .map(|permission| permission.into_domain()),
                )
                .normalized_for_storage();
                scryer_domain::LibraryGrant {
                    user_id: String::new(),
                    library_id: String::from(grant.library_id),
                    permissions,
                }
            })
            .collect();
        let user = app
            .create_user(
                &actor,
                input.username,
                input.password,
                app_permissions,
                library_grants,
            )
            .await
            .map_err(to_gql_error)?;
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        user_payload_from_user(&app, user).await
    }

    /// Sets another user's password or changes the authenticated actor's password after validating the target and credentials.
    async fn set_user_password(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Target user ID, new password, and optional current password for a self-service change."
        )]
        input: SetUserPasswordInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let user_id = String::from(input.user_id);
        let actor = if user_id != actor.id {
            require_config_app_permission(ctx, AppPermission::ManageUsers).await?
        } else {
            actor
        };
        let user = if user_id == actor.id {
            if let Some(current_password) = input.current_password {
                app.change_own_password(&actor, input.password, current_password)
                    .await
                    .map_err(to_gql_error)?
            } else {
                app.set_initial_own_password(&actor, input.password)
                    .await
                    .map_err(to_gql_error)?
            }
        } else {
            let user = app
                .set_user_password(&actor, &user_id, input.password)
                .await
                .map_err(to_gql_error)?;
            auth_runtime_from_ctx(ctx).invalidate_connections();
            user
        };
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        user_payload_from_user(&app, user).await
    }

    /// Replaces a user's app permissions after checking the manage-permissions permission.
    async fn set_user_app_permissions(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Target user ID and complete app-permission set; an empty set removes app permissions."
        )]
        input: SetUserAppPermissionsInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManagePermissions).await?;
        let user_id = String::from(input.user_id);
        let permissions = scryer_domain::AppPermissionMask::from_permissions(
            input
                .permissions
                .into_iter()
                .map(|permission| permission.into_domain()),
        );
        let user = app
            .set_user_app_permissions(&actor, &user_id, permissions)
            .await
            .map_err(to_gql_error)?;
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        user_payload_from_user(&app, user).await
    }

    /// Replaces a user's library grants after checking the manage-permissions permission.
    async fn set_user_library_permissions(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Target user ID and complete library-grant set; an empty set removes library grants."
        )]
        input: SetUserLibraryPermissionsInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManagePermissions).await?;
        let user_id = String::from(input.user_id);
        let grants = input
            .grants
            .into_iter()
            .map(|grant| {
                let permissions = scryer_domain::LibraryPermissionMask::from_permissions(
                    grant
                        .permissions
                        .into_iter()
                        .map(|permission| permission.into_domain()),
                )
                .normalized_for_storage();
                scryer_domain::LibraryGrant {
                    user_id: user_id.clone(),
                    library_id: String::from(grant.library_id),
                    permissions,
                }
            })
            .collect();
        let user = app
            .set_user_library_permissions(&actor, &user_id, grants)
            .await
            .map_err(to_gql_error)?;
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        user_payload_from_user(&app, user).await
    }

    /// Enables or disables login for the user identified in the input and invalidates active connections.
    async fn set_user_login_enabled(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Target user ID and desired login-enabled state.")]
        input: SetUserLoginEnabledInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let actor = require_config_app_permission(ctx, AppPermission::ManageUsers).await?;
        let user = app
            .set_user_login_enabled(
                &actor,
                input.user_id.as_str(),
                input.enabled,
                auth_runtime.snapshot().effective_form_login_enabled,
            )
            .await
            .map_err(to_gql_error)?;
        auth_runtime.invalidate_connections();
        user_payload_from_user(&app, user).await
    }

    /// Deletes the user identified by `id` after checking the manage-users permission.
    async fn delete_user(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the user to delete.")] id: ID,
    ) -> GqlResult<DeleteUserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageUsers).await?;
        let id_string = id.to_string();
        app.delete_user(&actor, &id_string)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteUserPayload { id })
    }

    /// Resets all authentication factors for the user identified by `id` after checking the manage-users permission.
    async fn reset_user_mfa(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the user whose authentication factors should be reset.")] id: ID,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageUsers).await?;
        let id = id.to_string();
        let user = app
            .reset_user_mfa(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        auth_runtime_from_ctx(ctx).invalidate_connections();
        let user = app
            .attach_user_authorization(user)
            .await
            .map_err(to_gql_error)?;
        user_payload_from_user(&app, user).await
    }

    /// Creates an external-account invite for a target user and connection without returning provider credentials.
    async fn create_external_account_invite(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Target user, media-server connection, provider, and provider account identifiers for the invite."
        )]
        input: CreateExternalAccountInviteInput,
    ) -> GqlResult<LinkedAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageUsers).await?;
        let user_id = input.user_id.to_string();
        let connection_id = input.connection_id.to_string();
        app.create_external_account_invite(
            &actor,
            &user_id,
            input.provider.into_domain(),
            connection_id,
            input.provider_user_identifier,
            input.provider_user_id,
        )
        .await
        .map(from_linked_account)
        .map_err(to_gql_error)
    }

    /// Links the authenticated actor to Plex using a connection ID and token.
    async fn link_plex_account(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Plex connection ID and authentication token used for linking.")]
        input: LinkPlexAccountInput,
    ) -> GqlResult<LinkedAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.link_plex_account(
            &actor,
            input.connection_id.to_string(),
            input.plex_auth_token,
        )
        .await
        .map(from_linked_account)
        .map_err(to_gql_error)
    }

    /// Links the authenticated actor to Jellyfin using a connection ID and credentials.
    async fn link_jellyfin_account(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Jellyfin connection ID, username, and password used for linking.")]
        input: LinkJellyfinAccountInput,
    ) -> GqlResult<LinkedAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.link_jellyfin_account(
            &actor,
            input.connection_id.to_string(),
            input.username,
            input.password,
        )
        .await
        .map(from_linked_account)
        .map_err(to_gql_error)
    }

    /// Links the authenticated actor to Emby using the selected connection mode and credentials.
    async fn link_emby_account(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Emby connection ID, local or Connect mode, and credentials used for linking."
        )]
        input: LinkEmbyAccountInput,
    ) -> GqlResult<LinkedAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.link_emby_account(
            &actor,
            input.connection_id.to_string(),
            emby_connection_mode(input.mode),
            input.username,
            input.password,
        )
        .await
        .map(from_linked_account)
        .map_err(to_gql_error)
    }

    /// Unlinks the external account identified by `linked_account_id`.
    async fn unlink_external_account(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the linked external account to remove.")] linked_account_id: ID,
    ) -> GqlResult<UnlinkExternalAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let linked_account_id_string = linked_account_id.to_string();
        app.unlink_external_account(&actor, &linked_account_id_string)
            .await
            .map_err(to_gql_error)?;
        Ok(UnlinkExternalAccountPayload { linked_account_id })
    }

    /// Authenticates through Plex and issues a session token.
    async fn login_with_plex(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Plex connection ID, authentication token, and optional session-persistence preference."
        )]
        input: LoginWithPlexInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let started_at = Instant::now();
        let persist_session = persist_session_or_default(
            input.persist_session,
            default_persist_session_from_ctx(ctx),
        );
        let user = match app
            .federated_login_with_plex(input.connection_id.to_string(), input.plex_auth_token)
            .await
        {
            Ok(user) => user,
            Err(err) => {
                return Err(to_login_gql_error_after_timing(
                    "plex",
                    LoginFailureTimingClass::FastMasked,
                    started_at,
                    err,
                )
                .await);
            }
        };
        login_payload_from_user(&app, user, None, None, persist_session, None, false).await
    }

    /// Authenticates through Jellyfin, applies configured TOTP requirements, and issues a session or MFA-enrollment token.
    async fn login_with_jellyfin(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Jellyfin connection ID, credentials, optional TOTP code, and optional session-persistence preference."
        )]
        input: LoginWithJellyfinInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let started_at = Instant::now();
        let persist_session = persist_session_or_default(
            input.persist_session,
            default_persist_session_from_ctx(ctx),
        );
        let user = match app
            .federated_login_with_jellyfin(
                input.connection_id.to_string(),
                input.username,
                input.password,
            )
            .await
        {
            Ok(user) => user,
            Err(err) => {
                return Err(to_login_gql_error_after_timing(
                    "jellyfin",
                    LoginFailureTimingClass::FastMasked,
                    started_at,
                    err,
                )
                .await);
            }
        };
        let effective_login_enabled = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
        let jellyfin_mfa_required = effective_login_enabled
            && app
                .security_settings()
                .await
                .map_err(to_gql_error)?
                .totp_require_jellyfin_login;
        match app
            .login_verification_requirement(
                &user,
                LoginVerificationMethod::Jellyfin,
                jellyfin_mfa_required,
                persist_session,
                input.totp_code.as_deref(),
            )
            .await
            .map_err(to_gql_error)?
        {
            LoginVerificationRequirement::Satisfied(satisfied) => {
                let expected_auth_session_version = satisfied
                    .mfa_verified_until
                    .is_some()
                    .then_some(&satisfied.auth_session_version);
                login_payload_from_user(
                    &app,
                    user,
                    satisfied.mfa_verified_until,
                    None,
                    persist_session,
                    expected_auth_session_version,
                    false,
                )
                .await
            }
            LoginVerificationRequirement::EnrollmentRequired {
                auth_session_version,
            } => {
                login_mfa_enrollment_payload_from_user(
                    &app,
                    user,
                    persist_session,
                    false,
                    &auth_session_version,
                )
                .await
            }
            LoginVerificationRequirement::Challenge(challenge) => {
                Err(login_verification_required_gql_error(
                    &challenge.id,
                    &challenge.expires_at,
                    challenge.allow_passkey,
                    challenge.allow_totp,
                ))
            }
        }
    }

    /// Authenticates through Emby, applies configured TOTP requirements, and issues a session or MFA-enrollment token.
    async fn login_with_emby(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Emby connection ID, mode, credentials, optional TOTP code, and optional session-persistence preference."
        )]
        input: LoginWithEmbyInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let started_at = Instant::now();
        let persist_session = persist_session_or_default(
            input.persist_session,
            default_persist_session_from_ctx(ctx),
        );
        let user = match app
            .federated_login_with_emby(
                input.connection_id.to_string(),
                emby_connection_mode(input.mode),
                input.username,
                input.password,
            )
            .await
        {
            Ok(user) => user,
            Err(err) => {
                return Err(to_login_gql_error_after_timing(
                    "emby",
                    LoginFailureTimingClass::FastMasked,
                    started_at,
                    err,
                )
                .await);
            }
        };
        let emby_mfa_required = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled
            && app
                .security_settings()
                .await
                .map_err(to_gql_error)?
                .totp_require_emby_login;
        match app
            .login_verification_requirement(
                &user,
                LoginVerificationMethod::Emby,
                emby_mfa_required,
                persist_session,
                input.totp_code.as_deref(),
            )
            .await
            .map_err(to_gql_error)?
        {
            LoginVerificationRequirement::Satisfied(satisfied) => {
                let expected_auth_session_version = satisfied
                    .mfa_verified_until
                    .is_some()
                    .then_some(&satisfied.auth_session_version);
                login_payload_from_user(
                    &app,
                    user,
                    satisfied.mfa_verified_until,
                    None,
                    persist_session,
                    expected_auth_session_version,
                    false,
                )
                .await
            }
            LoginVerificationRequirement::EnrollmentRequired {
                auth_session_version,
            } => {
                login_mfa_enrollment_payload_from_user(
                    &app,
                    user,
                    persist_session,
                    false,
                    &auth_session_version,
                )
                .await
            }
            LoginVerificationRequirement::Challenge(challenge) => {
                Err(login_verification_required_gql_error(
                    &challenge.id,
                    &challenge.expires_at,
                    challenge.allow_passkey,
                    challenge.allow_totp,
                ))
            }
        }
    }
}
