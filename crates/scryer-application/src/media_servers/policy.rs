use super::*;

pub(super) fn media_server_update_requires_manage_permissions(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
    provider: &MediaServerProvider,
) -> bool {
    let effective_app_permissions = if provider.supports_external_auth() {
        patch
            .default_app_permissions
            .unwrap_or(existing.default_app_permissions)
    } else {
        AppPermissionMask::NONE
    };
    let effective_library_grants = if provider.supports_external_auth() {
        normalize_default_library_grants(
            patch
                .default_library_grants
                .clone()
                .unwrap_or_else(|| existing.default_library_grants.clone()),
        )
    } else {
        Vec::new()
    };

    if media_server_patch_changes_default_grants_to_non_empty(
        existing,
        patch,
        effective_app_permissions,
        &effective_library_grants,
    ) {
        return true;
    }

    if !media_server_default_grants_are_non_empty(
        effective_app_permissions,
        &effective_library_grants,
    ) {
        return false;
    }

    media_server_patch_activates_external_auth_surface(existing, patch, provider)
        || media_server_patch_changes_external_auth_identity(existing, patch, provider)
}

pub(super) fn media_server_patch_changes_default_grants_to_non_empty(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
    effective_app_permissions: AppPermissionMask,
    effective_library_grants: &[MediaServerDefaultLibraryGrant],
) -> bool {
    if patch.default_app_permissions.is_none() && patch.default_library_grants.is_none() {
        return false;
    }
    if !media_server_default_grants_are_non_empty(
        effective_app_permissions,
        effective_library_grants,
    ) {
        return false;
    }

    let existing_app_permissions = if existing.provider.supports_external_auth() {
        existing.default_app_permissions
    } else {
        AppPermissionMask::NONE
    };
    let existing_library_grants = if existing.provider.supports_external_auth() {
        normalize_default_library_grants(existing.default_library_grants.clone())
    } else {
        Vec::new()
    };

    effective_app_permissions != existing_app_permissions
        || !media_server_default_library_grants_equal(
            effective_library_grants,
            &existing_library_grants,
        )
}

pub(super) fn media_server_patch_activates_external_auth_surface(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
    provider: &MediaServerProvider,
) -> bool {
    let resulting_enabled = patch.enabled.unwrap_or(existing.enabled);
    let resulting_login_enabled = patch.login_enabled.unwrap_or(existing.login_enabled);
    let resulting_linking_enabled = patch.linking_enabled.unwrap_or(existing.linking_enabled);
    let resulting_auto_add_enabled = patch.auto_add_enabled.unwrap_or(existing.auto_add_enabled);
    let existing_surface = media_server_external_auth_surface_usable(
        &existing.provider,
        existing.enabled,
        existing.login_enabled,
        existing.linking_enabled,
        existing.auto_add_enabled,
    );
    let resulting_surface = media_server_external_auth_surface_usable(
        provider,
        resulting_enabled,
        resulting_login_enabled,
        resulting_linking_enabled,
        resulting_auto_add_enabled,
    );

    if !resulting_surface {
        return false;
    }
    if !existing_surface {
        return true;
    }

    patch
        .enabled
        .is_some_and(|enabled| enabled && !existing.enabled)
        || patch
            .login_enabled
            .is_some_and(|enabled| enabled && !existing.login_enabled)
        || patch
            .linking_enabled
            .is_some_and(|enabled| enabled && !existing.linking_enabled)
        || patch
            .auto_add_enabled
            .is_some_and(|enabled| enabled && !existing.auto_add_enabled)
}

pub(super) fn media_server_patch_changes_external_auth_identity(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
    provider: &MediaServerProvider,
) -> bool {
    if !media_server_external_auth_surface_usable(
        provider,
        patch.enabled.unwrap_or(existing.enabled),
        patch.login_enabled.unwrap_or(existing.login_enabled),
        patch.linking_enabled.unwrap_or(existing.linking_enabled),
        patch.auto_add_enabled.unwrap_or(existing.auto_add_enabled),
    ) {
        return false;
    }
    if patch
        .provider
        .as_ref()
        .is_some_and(|provider| provider != &existing.provider)
    {
        return true;
    }
    if patch.base_url.as_ref().is_some_and(|base_url| {
        media_server_base_url_changed(provider, &existing.base_url, base_url)
    }) {
        return true;
    }

    match provider {
        MediaServerProvider::Plex => media_server_plex_identity_changed(existing, patch),
        MediaServerProvider::Jellyfin => media_server_jellyfin_identity_changed(existing, patch),
        MediaServerProvider::Emby => media_server_emby_identity_changed(existing, patch),
    }
}

pub(super) fn media_server_external_auth_surface_usable(
    provider: &MediaServerProvider,
    enabled: bool,
    login_enabled: bool,
    linking_enabled: bool,
    auto_add_enabled: bool,
) -> bool {
    provider.supports_external_auth()
        && enabled
        && (login_enabled || linking_enabled || auto_add_enabled)
}

pub(super) fn media_server_default_grants_are_non_empty(
    app_permissions: AppPermissionMask,
    library_grants: &[MediaServerDefaultLibraryGrant],
) -> bool {
    !app_permissions.is_empty()
        || library_grants
            .iter()
            .any(|grant| !grant.permissions.is_empty())
}

pub(super) fn media_server_default_library_grants_equal(
    left: &[MediaServerDefaultLibraryGrant],
    right: &[MediaServerDefaultLibraryGrant],
) -> bool {
    let mut left = media_server_default_library_grant_entries(left);
    let mut right = media_server_default_library_grant_entries(right);
    left.sort_by(|a, b| a.0.cmp(&b.0));
    right.sort_by(|a, b| a.0.cmp(&b.0));
    left == right
}

pub(super) fn media_server_default_library_grant_entries(
    grants: &[MediaServerDefaultLibraryGrant],
) -> Vec<(String, scryer_domain::LibraryPermissionMask)> {
    grants
        .iter()
        .filter(|grant| !grant.permissions.is_empty())
        .map(|grant| (grant.library_id.clone(), grant.permissions))
        .collect()
}

pub(super) fn media_server_base_url_changed(
    provider: &MediaServerProvider,
    existing_base_url: &str,
    base_url: &str,
) -> bool {
    match normalize_media_server_base_url(provider, base_url.to_string()) {
        Ok(normalized) => normalized != existing_base_url,
        Err(_) => true,
    }
}

pub(super) fn media_server_plex_identity_changed(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
) -> bool {
    (patch.clear_machine_id && existing.machine_id.is_some())
        || (patch.clear_api_key && existing.api_key.is_some())
        || patch.machine_id.as_ref().is_some_and(|machine_id| {
            normalize_optional_string(Some(machine_id.clone())) != existing.machine_id
        })
        || patch.api_key.as_ref().is_some_and(|api_key| {
            normalize_optional_string(Some(api_key.clone())) != existing.api_key
        })
        || option_has_non_empty_text(patch.plex_auth_token.as_deref())
        || option_has_non_empty_text(patch.plex_server_id.as_deref())
}

pub(super) fn media_server_jellyfin_identity_changed(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
) -> bool {
    (patch.clear_api_key && existing.api_key.is_some())
        || patch.api_key.as_ref().is_some_and(|api_key| {
            normalize_optional_string(Some(api_key.clone())) != existing.api_key
        })
        || option_has_non_empty_text(patch.admin_username.as_deref())
        || option_has_non_empty_text(patch.admin_password.as_deref())
}

pub(super) fn media_server_emby_identity_changed(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
) -> bool {
    (patch.clear_api_key && existing.api_key.is_some())
        || patch.api_key.as_ref().is_some_and(|api_key| {
            normalize_optional_string(Some(api_key.clone())) != existing.api_key
        })
        || option_has_non_empty_text(patch.admin_username.as_deref())
        || option_has_non_empty_secret(patch.admin_password.as_deref())
        || patch.emby_connection_mode.is_some()
        || patch
            .emby_connect_enabled
            .is_some_and(|enabled| enabled != existing.emby_connect_enabled)
        || option_has_non_empty_text(patch.emby_connect_username_or_email.as_deref())
        || option_has_non_empty_secret(patch.emby_connect_password.as_deref())
        || patch
            .emby_connect_server_id
            .as_ref()
            .is_some_and(|server_id| {
                normalize_optional_string(Some(server_id.clone())) != existing.emby_server_id
            })
}

pub(super) fn option_has_non_empty_text(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|value| !value.is_empty())
}

pub(super) fn option_has_non_empty_secret(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

pub(super) fn default_media_server_display_name(provider: &MediaServerProvider) -> &'static str {
    match provider {
        MediaServerProvider::Jellyfin => "Jellyfin",
        MediaServerProvider::Plex => "Plex",
        MediaServerProvider::Emby => "Emby",
    }
}

pub(super) fn normalize_media_server_base_url(
    provider: &MediaServerProvider,
    value: String,
) -> AppResult<String> {
    let value = value.trim();
    let value = if value.is_empty() && *provider == MediaServerProvider::Plex {
        "https://plex.tv"
    } else {
        value
    };
    if value.is_empty() {
        return Err(AppError::Validation(
            "media server base URL is required".into(),
        ));
    }
    let parsed = url::Url::parse(value)
        .map_err(|_| AppError::Validation("media server base URL is invalid".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::Validation(
            "media server base URL must be an HTTP or HTTPS URL".into(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(AppError::Validation(
            "media server base URL must not include a query or fragment".into(),
        ));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

pub(super) fn normalize_media_server_external_url(
    value: Option<String>,
) -> AppResult<Option<String>> {
    let Some(value) = normalize_optional_string(value) else {
        return Ok(None);
    };
    let parsed = url::Url::parse(&value)
        .map_err(|_| AppError::Validation("media server external URL is invalid".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::Validation(
            "media server external URL must be an HTTP or HTTPS URL".into(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::Validation(
            "media server external URL must not include credentials".into(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(AppError::Validation(
            "media server external URL must not include a query or fragment".into(),
        ));
    }
    Ok(Some(parsed.as_str().trim_end_matches('/').to_string()))
}

pub(super) fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn normalize_path_mappings(
    values: Vec<MediaServerPathMapping>,
) -> AppResult<Vec<MediaServerPathMapping>> {
    let mut normalized = Vec::new();
    for (index, mapping) in values.into_iter().enumerate() {
        let source_path = mapping.source_path.trim().to_string();
        let destination_path = mapping.destination_path.trim().to_string();
        if source_path.is_empty() || destination_path.is_empty() {
            continue;
        }
        if normalized
            .iter()
            .any(|existing: &MediaServerPathMapping| existing.source_path == source_path)
        {
            return Err(AppError::Validation(
                "media server path mappings must have unique source paths".into(),
            ));
        }
        normalized.push(MediaServerPathMapping {
            source_path,
            destination_path,
            sort_order: index as i64,
        });
    }
    Ok(normalized)
}

pub(super) fn normalize_default_library_grants(
    values: Vec<MediaServerDefaultLibraryGrant>,
) -> Vec<MediaServerDefaultLibraryGrant> {
    let mut normalized = Vec::new();
    for grant in values {
        let library_id = grant.library_id.trim().to_string();
        if library_id.is_empty() {
            continue;
        }
        if let Some(existing) =
            normalized
                .iter_mut()
                .find(|existing: &&mut MediaServerDefaultLibraryGrant| {
                    existing.library_id == library_id
                })
        {
            existing.permissions = grant.permissions.normalized_for_storage();
        } else {
            normalized.push(MediaServerDefaultLibraryGrant {
                library_id,
                permissions: grant.permissions.normalized_for_storage(),
            });
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_url_rejects_embedded_credentials() {
        let result = normalize_media_server_external_url(Some(
            "https://admin:secret@media.example.test".into(),
        ));

        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn external_url_preserves_reverse_proxy_paths() {
        assert_eq!(
            normalize_media_server_external_url(Some(
                "https://media.example.test/jellyfin/".into(),
            ))
            .expect("external URL should be valid"),
            Some("https://media.example.test/jellyfin".into())
        );
    }
}
