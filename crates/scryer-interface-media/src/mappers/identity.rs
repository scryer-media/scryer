use super::*;

pub fn from_user(user: User) -> UserPayload {
    from_user_with_auth_factor_status(user, scryer_application::UserAuthFactorStatus::default())
}

pub fn from_user_with_auth_factor_status(
    user: User,
    auth_factor_status: scryer_application::UserAuthFactorStatus,
) -> UserPayload {
    let login_status = user.login_status();
    let User {
        id,
        username,
        password_hash,
        password_change_required,
        account_kind,
        authorization,
        ..
    } = user;

    let app_permissions = authorization
        .app
        .to_permissions()
        .into_iter()
        .map(AppPermissionValue::from_domain)
        .collect();
    let mut library_permissions = authorization
        .libraries
        .into_iter()
        .map(
            |(library_id, permissions)| UserLibraryPermissionGrantPayload {
                library_id: library_id.into(),
                permissions: permissions
                    .with_request_shadowing()
                    .to_permissions()
                    .into_iter()
                    .map(LibraryPermissionValue::from_domain)
                    .collect(),
            },
        )
        .collect::<Vec<_>>();
    library_permissions
        .sort_by(|left, right| left.library_id.as_str().cmp(right.library_id.as_str()));

    UserPayload {
        id: id.into(),
        is_default_admin: username.eq_ignore_ascii_case("admin"),
        username,
        login_enabled: login_status.is_enabled(),
        has_password: password_hash.is_some(),
        password_change_required,
        has_mfa: auth_factor_status.has_mfa,
        has_passkey: auth_factor_status.has_passkey,
        account_kind: UserAccountKindValue::from_domain(account_kind),
        app_permissions,
        library_permissions,
    }
}

pub fn from_linked_account(account: scryer_domain::UserExternalAccount) -> LinkedAccountPayload {
    LinkedAccountPayload {
        id: account.id.into(),
        user_id: account.user_id.into(),
        provider: ExternalAccountProviderValue::from_domain(account.provider),
        connection_id: account.connection_id.into(),
        external_user_id: account.external_user_id,
        username: account.username,
        display_name: account.display_name,
        avatar_url: account.avatar_url,
        status: ExternalAccountStatusValue::from_domain(account.status),
        verified_at: account.verified_at,
        last_login_at: account.last_login_at,
        created_at: account.created_at,
        updated_at: account.updated_at,
    }
}

pub fn from_media_server_connection(
    connection: scryer_domain::MediaServerConnection,
) -> MediaServerConnectionPayload {
    let api_key_present = connection.api_key_present();
    let machine_id_present = connection.machine_id.is_some();
    let emby_server_id_present = connection
        .emby_server_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    MediaServerConnectionPayload {
        id: connection.id.into(),
        provider: MediaServerProviderValue::from_domain(connection.provider),
        display_name: connection.display_name,
        base_url: connection.base_url,
        external_url: connection.external_url,
        enabled: connection.enabled,
        login_enabled: connection.login_enabled,
        linking_enabled: connection.linking_enabled,
        auto_add_enabled: connection.auto_add_enabled,
        default_app_permissions: connection
            .default_app_permissions
            .to_permissions()
            .into_iter()
            .map(AppPermissionValue::from_domain)
            .collect(),
        default_library_grants: connection
            .default_library_grants
            .into_iter()
            .map(|grant| MediaServerDefaultLibraryGrantPayload {
                library_id: grant.library_id.into(),
                permissions: grant
                    .permissions
                    .with_request_shadowing()
                    .to_permissions()
                    .into_iter()
                    .map(LibraryPermissionValue::from_domain)
                    .collect(),
            })
            .collect(),
        machine_id_present,
        api_key_present,
        emby_server_id_present,
        emby_connect_enabled: connection.emby_connect_enabled,
        path_mappings: connection
            .path_mappings
            .into_iter()
            .map(|mapping| MediaServerPathMappingPayload {
                source_path: mapping.source_path,
                destination_path: mapping.destination_path,
            })
            .collect(),
        created_at: connection.created_at,
        updated_at: connection.updated_at,
    }
}

pub fn from_jellyfin_server_user(
    user: scryer_application::JellyfinServerUser,
) -> JellyfinServerUserPayload {
    JellyfinServerUserPayload {
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        avatar_url: user.avatar_url,
    }
}

pub fn from_media_server_user(user: scryer_application::MediaServerUser) -> MediaServerUserPayload {
    MediaServerUserPayload {
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        avatar_url: user.avatar_url,
    }
}

pub fn from_media_server_user_group(
    group: scryer_application::MediaServerUserGroup,
) -> MediaServerUserGroupPayload {
    MediaServerUserGroupPayload {
        connection_id: group.connection_id.into(),
        connection_name: group.connection_name,
        provider: ExternalAccountProviderValue::from_domain(group.provider),
        status: match group.status {
            scryer_application::MediaServerUserGroupStatus::Ready => {
                MediaServerUserGroupStatusValue::Ready
            }
            scryer_application::MediaServerUserGroupStatus::MissingCredentials => {
                MediaServerUserGroupStatusValue::MissingCredentials
            }
            scryer_application::MediaServerUserGroupStatus::Error => {
                MediaServerUserGroupStatusValue::Error
            }
        },
        error_message: group.error_message,
        users: group
            .users
            .into_iter()
            .map(from_media_server_user)
            .collect(),
    }
}

pub fn from_plex_server_discovery(
    server: scryer_application::PlexServerDiscovery,
) -> PlexServerDiscoveryPayload {
    PlexServerDiscoveryPayload {
        id: server.id,
        name: server.name,
    }
}

pub fn from_activity_event(event: ActivityEvent) -> ActivityEventPayload {
    ActivityEventPayload {
        id: event.id.into(),
        kind: ActivityKindValue::from_application(event.kind),
        severity: ActivitySeverityValue::from_application(event.severity),
        channels: event
            .channels
            .into_iter()
            .map(ActivityChannelValue::from_application)
            .collect(),
        actor_kind: event.actor_kind.into(),
        actor_user_id: event.actor_user_id.map(Into::into),
        actor_display_name: event.actor_display_name,
        title_id: event.title_id.map(Into::into),
        facet: event.facet.as_deref().and_then(MediaFacetValue::parse),
        episode_ids: event.episode_ids.into_iter().map(Into::into).collect(),
        message: event.message,
        occurred_at: event.occurred_at,
    }
}
