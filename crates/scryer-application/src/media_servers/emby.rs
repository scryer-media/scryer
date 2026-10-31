use super::*;

impl AppUseCase {
    pub async fn discover_emby_connect_media_servers(
        &self,
        actor: &User,
        username_or_email: &str,
        password: &str,
    ) -> AppResult<Vec<EmbyConnectServer>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .external_identity_verifier
            .discover_emby_connect_servers(username_or_email, password)
            .await
    }

    pub async fn test_emby_connect(
        &self,
        actor: &User,
        connection_id: &str,
        username_or_email: &str,
        password: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(connection_id.trim())
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::media_servers::CONNECTION_NOT_FOUND,
                    format!("media server connection {connection_id}"),
                )
            })?;
        if connection.provider != MediaServerProvider::Emby {
            return Err(AppError::validation_refused(
                crate::error_reasons::media_servers::EMBY_CONNECT_TEST_REQUIRES_EMBY_CONNECTION,
                "Emby Connect test requires an Emby connection",
            ));
        }
        if !connection.emby_connect_enabled {
            return Err(AppError::validation_refused(
                crate::error_reasons::media_servers::EMBY_CONNECT_SIGN_DISABLED_CONNECTION,
                "Emby Connect sign-in is disabled for this connection",
            ));
        }
        let server_id = connection.emby_server_id.as_deref().ok_or_else(|| {
            AppError::validation_refused(
                crate::error_reasons::media_servers::EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY,
                "Emby connection has no verified server identity",
            )
        })?;
        let verification = self
            .services
            .integrations
            .external_identity_verifier
            .test_emby_connect_identity(
                &connection.id,
                &connection.base_url,
                server_id,
                username_or_email,
                password,
            )
            .await?;
        if verification.resolved_api_base_url != connection.base_url
            && let Err(error) = self
                .services
                .integrations
                .media_server_connections
                .compare_and_set_emby_base_url(
                    &connection.id,
                    &connection.base_url,
                    server_id,
                    &verification.resolved_api_base_url,
                )
                .await
        {
            tracing::warn!(
                connection_id = connection.id,
                operation = "emby_connect_address_refresh",
                error_class = %error,
                "Emby Connect address refresh failed after successful connection test"
            );
        }
        Ok(())
    }

    pub async fn list_emby_server_users(
        &self,
        actor: &User,
        connection_id: &str,
        search: Option<&str>,
    ) -> AppResult<Vec<EmbyServerUser>> {
        self.require_app_permission(actor, AppPermission::ManageUsers)
            .await?;
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(connection_id.trim())
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::media_servers::CONNECTION_NOT_FOUND,
                    format!("media server connection {connection_id}"),
                )
            })?;
        if connection.provider != MediaServerProvider::Emby {
            return Err(AppError::validation_refused(
                crate::error_reasons::media_servers::EMBY_USER_LISTING_REQUIRES_EMBY_CONNECTION,
                "Emby user listing requires an Emby connection",
            ));
        }
        let api_key = connection.api_key.as_deref().ok_or_else(|| {
            AppError::validation_refused(crate::error_reasons::media_servers::EMBY_USER_LISTING_REQUIRES_SAVED_INTEGRATION_API_KEY, "Emby user listing requires a saved integration API key")
        })?;
        self.services
            .integrations
            .external_identity_verifier
            .list_emby_users(&connection.id, &connection.base_url, api_key, search)
            .await
    }

    pub async fn fetch_emby_server_user_avatar(
        &self,
        actor: &User,
        connection_id: &str,
        user_id: &str,
        image_tag: &str,
    ) -> AppResult<Option<EmbyAvatar>> {
        self.require_app_permission(actor, AppPermission::ManageUsers)
            .await?;
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(connection_id.trim())
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::media_servers::CONNECTION_NOT_FOUND,
                    format!("media server connection {connection_id}"),
                )
            })?;
        if connection.provider != MediaServerProvider::Emby {
            return Err(AppError::not_found_reason(
                crate::error_reasons::media_servers::EMBY_AVATAR_NOT_FOUND,
                "Emby avatar was not found",
            ));
        }
        let api_key = connection.api_key.as_deref().ok_or_else(|| {
            AppError::not_found_reason(
                crate::error_reasons::media_servers::EMBY_AVATAR_NOT_FOUND,
                "Emby avatar was not found",
            )
        })?;
        self.services
            .integrations
            .external_identity_verifier
            .fetch_emby_user_avatar(
                &connection.id,
                &connection.base_url,
                api_key,
                user_id,
                image_tag,
            )
            .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Emby setup accepts three mutually exclusive credential modes"
    )]
    pub(super) async fn resolve_emby_credentials_for_setup(
        &self,
        connection_id: &str,
        base_url: &str,
        mode: Option<EmbyConnectionMode>,
        local_setup_method: Option<EmbyLocalSetupMethod>,
        connect_enabled: Option<bool>,
        api_key: Option<&str>,
        admin_username: Option<&str>,
        admin_password: Option<&str>,
        connect_username_or_email: Option<&str>,
        connect_password: Option<&str>,
        connect_server_id: Option<&str>,
    ) -> AppResult<ResolvedEmbyCredentials> {
        fn present(value: Option<&str>) -> Option<&str> {
            value.map(str::trim).filter(|value| !value.is_empty())
        }
        fn present_secret(value: Option<&str>) -> Option<&str> {
            value.filter(|value| !value.is_empty())
        }
        let api_key = present(api_key);
        let admin_username = present(admin_username);
        let admin_password = present_secret(admin_password);
        let connect_username_or_email = present(connect_username_or_email);
        let connect_password = present_secret(connect_password);
        let connect_server_id = present(connect_server_id);
        let mode = mode.unwrap_or(EmbyConnectionMode::Local);

        let mut exchange = match mode {
            EmbyConnectionMode::Local => {
                if connect_username_or_email.is_some()
                    || connect_password.is_some()
                    || connect_server_id.is_some()
                {
                    return Err(AppError::validation_refused(crate::error_reasons::media_servers::LOCAL_EMBY_SETUP_DOES_NOT_ACCEPT_EMBY_CONNECT_CREDENTIALS, "local Emby setup does not accept Emby Connect credentials"));
                }
                let method = local_setup_method.unwrap_or_else(|| {
                    if api_key.is_some() {
                        EmbyLocalSetupMethod::ApiKey
                    } else {
                        EmbyLocalSetupMethod::AdminCredentials
                    }
                });
                match method {
                    EmbyLocalSetupMethod::ApiKey => {
                        if admin_username.is_some() || admin_password.is_some() {
                            return Err(AppError::validation_refused(crate::error_reasons::media_servers::CHOOSE_EITHER_EMBY_API_KEY_LOCAL_ADMINISTRATOR_CREDENTIALS_NOT_BOTH, "choose either an Emby API key or local administrator credentials, not both"
                                    ));
                        }
                        let api_key = api_key.ok_or_else(|| {
                            AppError::validation_refused(
                                crate::error_reasons::media_servers::EMBY_API_KEY_REQUIRED,
                                "Emby API key is required",
                            )
                        })?;
                        let identity = self
                            .services
                            .integrations
                            .external_identity_verifier
                            .test_emby_api_key(connection_id, base_url, api_key, None)
                            .await?;
                        EmbyApiKeyExchange {
                            api_key: api_key.to_string(),
                            server_identity: identity,
                            created_new_key: false,
                            cleanup: None,
                        }
                    }
                    EmbyLocalSetupMethod::AdminCredentials => {
                        if api_key.is_some() {
                            return Err(AppError::validation_refused(crate::error_reasons::media_servers::CHOOSE_EITHER_EMBY_API_KEY_LOCAL_ADMINISTRATOR_CREDENTIALS_NOT_BOTH, "choose either an Emby API key or local administrator credentials, not both"
                                    ));
                        }
                        let (Some(username), Some(password)) = (admin_username, admin_password)
                        else {
                            return Err(AppError::validation_refused(crate::error_reasons::media_servers::BOTH_EMBY_ADMINISTRATOR_USERNAME_PASSWORD_REQUIRED, "both Emby administrator username and password are required"));
                        };
                        self.services
                            .integrations
                            .external_identity_verifier
                            .exchange_emby_local_admin_api_key(
                                connection_id,
                                base_url,
                                username,
                                password,
                            )
                            .await?
                    }
                }
            }
            EmbyConnectionMode::Connect => {
                if api_key.is_some() || admin_username.is_some() || admin_password.is_some() {
                    return Err(AppError::validation_refused(crate::error_reasons::media_servers::EMBY_CONNECT_LOCAL_CREDENTIALS_FORBIDDEN, "Emby Connect setup does not accept a pasted API key or local administrator credentials"
                            ));
                }
                let username = connect_username_or_email.ok_or_else(|| {
                    AppError::validation_refused(
                        crate::error_reasons::media_servers::EMBY_CONNECT_USERNAME_EMAIL_REQUIRED,
                        "Emby Connect username or email is required",
                    )
                })?;
                let password = connect_password.ok_or_else(|| {
                    AppError::validation_refused(
                        crate::error_reasons::media_servers::EMBY_CONNECT_PASSWORD_REQUIRED,
                        "Emby Connect password is required",
                    )
                })?;
                let server_id = connect_server_id.ok_or_else(|| {
                    AppError::validation_refused(
                        crate::error_reasons::media_servers::SELECT_EMBY_CONNECT_SERVER,
                        "select an Emby Connect server",
                    )
                })?;
                self.services
                    .integrations
                    .external_identity_verifier
                    .exchange_emby_connect_admin_api_key(
                        connection_id,
                        base_url,
                        server_id,
                        username,
                        password,
                    )
                    .await?
            }
        };
        if let Some(expected) = connect_server_id
            && exchange.server_identity.server_id != expected
        {
            if let Some(cleanup) = exchange.cleanup.take() {
                self.services
                    .integrations
                    .external_identity_verifier
                    .finish_emby_api_key_exchange(connection_id, cleanup, true)
                    .await;
            }
            return Err(AppError::validation_refused(crate::error_reasons::media_servers::SELECTED_EMBY_CONNECT_SERVER_IDENTITY_DOES_NOT_MATCH_REACHABLE_SERVER, "selected Emby Connect server identity does not match the reachable server"));
        }
        Ok(ResolvedEmbyCredentials {
            base_url: exchange.server_identity.api_base_url,
            api_key: exchange.api_key,
            server_id: exchange.server_identity.server_id,
            connect_enabled: connect_enabled.unwrap_or(mode == EmbyConnectionMode::Connect),
            cleanup: exchange.cleanup,
        })
    }
}
