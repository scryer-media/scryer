use super::*;

impl AppUseCase {
    pub async fn list_jellyfin_server_users(
        &self,
        actor: &User,
        connection_id: &str,
        search: Option<&str>,
    ) -> AppResult<Vec<JellyfinServerUser>> {
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
        if connection.provider != MediaServerProvider::Jellyfin {
            return Err(AppError::validation_refused(crate::error_reasons::media_servers::JELLYFIN_USER_LISTING_REQUIRES_JELLYFIN_CONNECTION, "Jellyfin user listing requires a Jellyfin connection"));
        }
        let api_key = connection.api_key.as_deref().ok_or_else(|| {
            AppError::validation_refused(crate::error_reasons::media_servers::JELLYFIN_USER_LISTING_REQUIRES_SAVED_API_KEY, "Jellyfin user listing requires a saved API key; save an API key to load Jellyfin users"
                    )
        })?;
        self.services
            .integrations
            .external_identity_verifier
            .list_jellyfin_users(&connection.base_url, api_key, search)
            .await
    }

    pub(super) async fn jellyfin_api_key_from_credentials_or_input(
        &self,
        connection: &MediaServerConnection,
        admin_username: Option<&str>,
        admin_password: Option<&str>,
        api_key: Option<String>,
        api_key_supplied: bool,
    ) -> AppResult<Option<String>> {
        if connection.provider != MediaServerProvider::Jellyfin {
            return Ok(api_key);
        }
        let admin_username = admin_username
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let admin_password = admin_password
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if api_key_supplied && (admin_username.is_some() || admin_password.is_some()) {
            return Err(AppError::validation_refused(crate::error_reasons::media_servers::CHOOSE_EITHER_JELLYFIN_API_KEY_JELLYFIN_ADMIN_LOGIN_CREDENTIALS_NOT_BOTH, "choose either a Jellyfin API key or Jellyfin admin login credentials, not both"
                    ));
        }
        match (admin_username, admin_password) {
            (Some(username), Some(password)) => {
                let generated = self
                    .services
                    .integrations
                    .external_identity_verifier
                    .exchange_jellyfin_admin_api_key(
                        &connection.id,
                        &connection.base_url,
                        username,
                        password,
                    )
                    .await?;
                Ok(Some(generated))
            }
            (None, None) => Ok(api_key),
            _ => Err(AppError::validation_refused(
                crate::error_reasons::media_servers::BOTH_JELLYFIN_ADMIN_USERNAME_PASSWORD_REQUIRED,
                "both Jellyfin admin username and password are required",
            )),
        }
    }
}
