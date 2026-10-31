use super::*;

impl AppUseCase {
    pub async fn discover_plex_media_servers(
        &self,
        actor: &User,
        plex_auth_token: &str,
    ) -> AppResult<Vec<PlexServerDiscovery>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .external_identity_verifier
            .discover_plex_servers(plex_auth_token)
            .await
    }

    pub(super) async fn resolve_plex_server_selection(
        &self,
        provider: &MediaServerProvider,
        existing_machine_id: Option<&str>,
        requested_machine_id: Option<String>,
        plex_auth_token: Option<&str>,
        plex_server_id: Option<&str>,
    ) -> AppResult<ResolvedPlexServerSelection> {
        if *provider != MediaServerProvider::Plex {
            return Ok(ResolvedPlexServerSelection::default());
        }

        let token = plex_auth_token
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let selected_server_id = plex_server_id
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let Some(token) = token {
            let servers = self
                .services
                .integrations
                .external_identity_verifier
                .discover_plex_servers(token)
                .await?;
            if servers.is_empty() {
                return Err(AppError::validation_refused(crate::error_reasons::media_servers::PLEX_DID_NOT_RETURN_ANY_ACCESSIBLE_SERVERS_ACCOUNT, "Plex did not return any accessible servers for that account"));
            }
            let selected = if let Some(selected_server_id) = selected_server_id {
                servers
                    .iter()
                    .find(|server| server.id == selected_server_id)
                    .ok_or_else(|| {
                        AppError::validation_refused(crate::error_reasons::media_servers::SELECTED_PLEX_SERVER_NOT_RETURNED_PLEX_DISCOVERY, "The selected Plex server was not returned by Plex discovery")
                    })?
            } else if servers.len() == 1 {
                &servers[0]
            } else {
                return Err(AppError::validation_refused(crate::error_reasons::media_servers::SELECT_PLEX_SERVER_BEFORE_SAVING_CONNECTION, "Select a Plex server before saving this connection"));
            };
            return Ok(ResolvedPlexServerSelection {
                machine_id: Some(selected.id.clone()),
            });
        }

        if selected_server_id.is_some() {
            return Err(AppError::validation_refused(
                crate::error_reasons::media_servers::SIGN_PLEX_BEFORE_SELECTING_PLEX_SERVER,
                "Sign in with Plex before selecting a Plex server",
            ));
        }

        Ok(ResolvedPlexServerSelection {
            machine_id: normalize_optional_string(requested_machine_id)
                .or_else(|| existing_machine_id.map(ToString::to_string)),
        })
    }
}
