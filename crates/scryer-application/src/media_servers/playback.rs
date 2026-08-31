use std::collections::{HashMap, HashSet};

use scryer_domain::{
    ExternalAccountStatus, MediaServerConnection, MediaServerPlaybackEntityKind,
    MediaServerProvider, User,
};

use super::*;

const PLEX_WEB_APP_URL: &str = "https://app.plex.tv/desktop";

/// An authorized, provider-native link to a catalog item on a linked media server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaServerPlaybackLink {
    pub connection_id: String,
    pub display_name: String,
    pub provider: MediaServerProvider,
    pub href: String,
}

impl AppUseCase {
    /// List playback links for an exact, previously verified provider item.
    ///
    /// This deliberately has no provider search fallback: a link is returned only
    /// when the current user actively linked the exact configured connection and
    /// a catalog scan persisted a matching opaque provider item ID.
    pub async fn media_server_playback_links(
        &self,
        actor: &User,
        entity_kind: MediaServerPlaybackEntityKind,
        entity_id: &str,
    ) -> AppResult<Vec<MediaServerPlaybackLink>> {
        match entity_kind {
            MediaServerPlaybackEntityKind::Title => {
                self.get_title(actor, entity_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("title {entity_id}")))?;
            }
            MediaServerPlaybackEntityKind::Episode => {
                self.get_episode(actor, entity_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("episode {entity_id}")))?;
            }
        }

        let key = (entity_kind, entity_id.to_string());
        let mut links_by_entity = self
            .media_server_playback_links_for_authorized_entities(actor, std::slice::from_ref(&key))
            .await?;
        Ok(links_by_entity.remove(&key).unwrap_or_default())
    }

    /// Resolve links for entities that were already authorized by the calling use case.
    pub async fn media_server_playback_links_for_authorized_entities(
        &self,
        actor: &User,
        entities: &[(MediaServerPlaybackEntityKind, String)],
    ) -> AppResult<HashMap<(MediaServerPlaybackEntityKind, String), Vec<MediaServerPlaybackLink>>>
    {
        let entities = entities
            .iter()
            .filter(|(_, entity_id)| !entity_id.trim().is_empty())
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if entities.is_empty() {
            return Ok(HashMap::new());
        }

        let mappings = self
            .services
            .integrations
            .media_server_connections
            .list_playback_items_for_entities(&entities)
            .await?;
        if mappings.is_empty() {
            return Ok(HashMap::new());
        }

        let active_account_connections = self
            .services
            .identity
            .external_accounts
            .list_by_user_id(&actor.id)
            .await?
            .into_iter()
            .filter(|account| account.status == ExternalAccountStatus::Active)
            .map(|account| (account.connection_id, account.provider))
            .collect::<HashSet<_>>();
        if active_account_connections.is_empty() {
            return Ok(HashMap::new());
        }

        let connections = self
            .services
            .integrations
            .media_server_connections
            .list(None)
            .await?
            .into_iter()
            .map(|connection| (connection.id.clone(), connection))
            .collect::<HashMap<_, _>>();

        let mut links_by_entity =
            HashMap::<(MediaServerPlaybackEntityKind, String), Vec<MediaServerPlaybackLink>>::new();
        for mapping in mappings {
            let Some(link) = (|| {
                let connection = connections.get(&mapping.connection_id)?;
                if !is_eligible_playback_connection(connection, &active_account_connections) {
                    return None;
                }
                let href = playback_href(connection, &mapping.provider_item_id)?;
                Some(MediaServerPlaybackLink {
                    connection_id: connection.id.clone(),
                    display_name: connection.display_name.clone(),
                    provider: connection.provider.clone(),
                    href,
                })
            })() else {
                continue;
            };
            links_by_entity
                .entry((mapping.entity_kind, mapping.entity_id))
                .or_default()
                .push(link);
        }
        for links in links_by_entity.values_mut() {
            links.sort_by(|left, right| {
                left.display_name
                    .cmp(&right.display_name)
                    .then_with(|| left.connection_id.cmp(&right.connection_id))
            });
        }
        Ok(links_by_entity)
    }
}

fn is_eligible_playback_connection(
    connection: &MediaServerConnection,
    active_accounts: &HashSet<(String, scryer_domain::ExternalAccountProvider)>,
) -> bool {
    connection.enabled
        && connection
            .provider
            .external_account_provider()
            .is_some_and(|provider| active_accounts.contains(&(connection.id.clone(), provider)))
}

fn playback_href(connection: &MediaServerConnection, provider_item_id: &str) -> Option<String> {
    let item_id = provider_item_id.trim();
    if item_id.is_empty() {
        return None;
    }
    match &connection.provider {
        MediaServerProvider::Jellyfin => {
            let base_url = connection.external_url.as_deref()?.trim_end_matches('/');
            Some(format!(
                "{base_url}/web/index.html#!/details?id={}&context=home",
                percent_encode(item_id)
            ))
        }
        MediaServerProvider::Emby => {
            let base_url = connection.external_url.as_deref()?.trim_end_matches('/');
            Some(format!(
                "{base_url}/web/index.html#!/item?id={}&context=home",
                percent_encode(item_id)
            ))
        }
        MediaServerProvider::Plex => {
            let machine_id = connection.machine_id.as_deref()?.trim();
            if machine_id.is_empty() {
                return None;
            }
            let key = format!("/library/metadata/{item_id}");
            Some(format!(
                "{PLEX_WEB_APP_URL}/#!/server/{}/details?key={}",
                percent_encode(machine_id),
                percent_encode(&key)
            ))
        }
    }
}

fn percent_encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use scryer_domain::{
        AppPermissionMask, MediaServerConnection, MediaServerDefaultLibraryGrant,
        MediaServerPathMapping,
    };

    use super::*;

    fn connection(provider: MediaServerProvider) -> MediaServerConnection {
        MediaServerConnection {
            id: "connection-1".into(),
            provider,
            display_name: "Home".into(),
            base_url: "http://api.example.test".into(),
            external_url: Some("https://watch.example.test".into()),
            enabled: true,
            login_enabled: true,
            linking_enabled: true,
            auto_add_enabled: false,
            default_app_permissions: AppPermissionMask::NONE,
            default_library_grants: Vec::<MediaServerDefaultLibraryGrant>::new(),
            machine_id: Some("machine-1".into()),
            api_key: None,
            emby_server_id: None,
            emby_connect_enabled: false,
            path_mappings: Vec::<MediaServerPathMapping>::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn builds_jellyfin_and_emby_item_urls_from_external_url() {
        assert_eq!(
            playback_href(&connection(MediaServerProvider::Jellyfin), "item-1"),
            Some(
                "https://watch.example.test/web/index.html#!/details?id=item-1&context=home".into()
            )
        );
        assert_eq!(
            playback_href(&connection(MediaServerProvider::Emby), "item-1"),
            Some("https://watch.example.test/web/index.html#!/item?id=item-1&context=home".into())
        );
    }

    #[test]
    fn builds_plex_web_app_url_with_opaque_rating_key() {
        assert_eq!(
            playback_href(&connection(MediaServerProvider::Plex), "123"),
            Some("https://app.plex.tv/desktop/#!/server/machine-1/details?key=%2Flibrary%2Fmetadata%2F123".into())
        );
    }

    #[test]
    fn jellyfin_and_emby_require_external_url() {
        let mut connection = connection(MediaServerProvider::Jellyfin);
        connection.external_url = None;
        assert_eq!(playback_href(&connection, "item-1"), None);
    }

    #[test]
    fn playback_requires_an_active_account_for_the_exact_enabled_connection() {
        let mut connection = connection(MediaServerProvider::Jellyfin);
        let active_accounts = HashSet::from([(
            connection.id.clone(),
            scryer_domain::ExternalAccountProvider::Jellyfin,
        )]);

        assert!(is_eligible_playback_connection(
            &connection,
            &active_accounts
        ));
        connection.enabled = false;
        assert!(!is_eligible_playback_connection(
            &connection,
            &active_accounts
        ));
        connection.enabled = true;
        connection.id = "another-connection".into();
        assert!(!is_eligible_playback_connection(
            &connection,
            &active_accounts
        ));
    }

    #[test]
    fn playback_urls_never_include_connection_credentials() {
        let mut connection = connection(MediaServerProvider::Jellyfin);
        connection.api_key = Some("super-secret-token".into());

        let href = playback_href(&connection, "item-1").expect("playback URL");

        assert!(!href.contains("super-secret-token"));
    }
}
