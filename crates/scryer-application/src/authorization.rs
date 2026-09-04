use std::collections::HashMap;

use scryer_domain::{
    ActorCapability, ActorCapabilityMask, AppPermission, AppPermissionMask, LibraryPermission,
    LibraryPermissionMask, MediaFacet, User, UserAuthorization,
};

use crate::{AppError, AppResult, AppUseCase};

fn library_permission_matches(
    permissions: LibraryPermissionMask,
    permission: LibraryPermission,
) -> bool {
    match permission {
        LibraryPermission::Request => permissions.is_strictly_requestable(),
        LibraryPermission::AutoApproveRequests => permissions.can_auto_approve_requests(),
        _ => permissions.contains(LibraryPermissionMask::from_permission(permission)),
    }
}

fn permission_allows_app_library_override(permission: LibraryPermission) -> bool {
    !matches!(
        permission,
        LibraryPermission::Request
            | LibraryPermission::AutoApproveRequests
            | LibraryPermission::ManageTitles
    )
}

/// Catalog administrators can read and administer every library without an
/// explicit per-library grant row. Grant rows are only seeded for the libraries
/// that exist when an admin account is provisioned, so a library created later
/// has none; the single-library check must reach the same verdict as the
/// library listings or a title that shows up in the catalog cannot be opened.
fn app_override_allows_library_permission(
    authorization: &UserAuthorization,
    permission: LibraryPermission,
) -> bool {
    authorization
        .app
        .contains(AppPermissionMask::MANAGE_CATALOG_SETTINGS)
        && permission_allows_app_library_override(permission)
}

impl AppUseCase {
    pub async fn load_user_authorization(&self, actor: &User) -> AppResult<UserAuthorization> {
        let app = self
            .services
            .catalog
            .libraries
            .app_permission_mask_for_user(&actor.id)
            .await?;

        let grants = self
            .services
            .catalog
            .libraries
            .permission_masks_for_user(&actor.id)
            .await?;
        let mut libraries = HashMap::with_capacity(grants.len());
        for grant in grants {
            libraries.insert(grant.library_id, grant.permissions);
        }

        // An administrator who can grant library permissions holds every
        // library permission on every library, including libraries created
        // after the account was provisioned and therefore missing a grant
        // row. Explicit rows still win where one exists.
        let default_library = if app.contains(AppPermissionMask::MANAGE_PERMISSIONS) {
            UserAuthorization::full_admin().default_library
        } else {
            LibraryPermissionMask::NONE
        };

        Ok(UserAuthorization {
            app,
            libraries,
            default_library,
            actor_capabilities: ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: actor.login_status(),
            loaded: true,
        })
    }

    pub async fn attach_user_authorization(&self, mut actor: User) -> AppResult<User> {
        let login_status = actor.login_status();
        actor.authorization = self.load_user_authorization(&actor).await?;
        actor.set_login_status(login_status);
        Ok(actor)
    }

    async fn authorization_for_actor(&self, actor: &User) -> AppResult<UserAuthorization> {
        if actor.authorization.loaded {
            Ok(actor.authorization.clone())
        } else {
            self.load_user_authorization(actor).await
        }
    }

    pub async fn require_app_permission(
        &self,
        actor: &User,
        permission: AppPermission,
    ) -> AppResult<()> {
        let authorization = self.authorization_for_actor(actor).await?;
        if authorization
            .app
            .contains(AppPermissionMask::from_permission(permission))
        {
            Ok(())
        } else {
            Err(AppError::Unauthorized(
                "You do not have permission to perform this action".to_string(),
            ))
        }
    }

    pub async fn require_actor_capability(
        &self,
        actor: &User,
        capability: ActorCapability,
    ) -> AppResult<()> {
        let authorization = self.authorization_for_actor(actor).await?;
        if authorization
            .actor_capabilities
            .contains(ActorCapabilityMask::from_capability(capability))
        {
            Ok(())
        } else {
            Err(AppError::Unauthorized(
                "You do not have permission to perform this action".to_string(),
            ))
        }
    }

    pub async fn has_app_permission(
        &self,
        actor: &User,
        permission: AppPermission,
    ) -> AppResult<bool> {
        Ok(self
            .authorization_for_actor(actor)
            .await?
            .has_app_permission(permission))
    }

    pub async fn has_any_app_permission(
        &self,
        actor: &User,
        permissions: AppPermissionMask,
    ) -> AppResult<bool> {
        Ok(self
            .authorization_for_actor(actor)
            .await?
            .has_any_app_permission(permissions))
    }

    pub async fn require_library_settings_read_permission(&self, actor: &User) -> AppResult<()> {
        let app_permissions = AppPermissionMask::from_permissions([
            AppPermission::ManageSystemSettings,
            AppPermission::ManageCatalogSettings,
        ]);
        if self.has_any_app_permission(actor, app_permissions).await?
            || self
                .has_any_granted_library_permission(actor, LibraryPermission::ManageLibrary)
                .await?
        {
            Ok(())
        } else {
            Err(AppError::Unauthorized(
                "You do not have permission to perform this action".to_string(),
            ))
        }
    }

    pub async fn require_library_permission(
        &self,
        actor: &User,
        library_id: &str,
        permission: LibraryPermission,
    ) -> AppResult<()> {
        let authorization = self.authorization_for_actor(actor).await?;
        let permissions = authorization.library_permissions(library_id);
        if library_permission_matches(permissions, permission)
            || app_override_allows_library_permission(&authorization, permission)
        {
            Ok(())
        } else {
            Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ))
        }
    }

    pub async fn require_granted_library_permission(
        &self,
        actor: &User,
        library_id: &str,
        permission: LibraryPermission,
    ) -> AppResult<()> {
        if self
            .has_granted_library_permission(actor, library_id, permission)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ))
        }
    }

    pub async fn has_granted_library_permission(
        &self,
        actor: &User,
        library_id: &str,
        permission: LibraryPermission,
    ) -> AppResult<bool> {
        let authorization = self.authorization_for_actor(actor).await?;
        if library_permission_matches(authorization.library_permissions(library_id), permission) {
            return Ok(true);
        }

        if !library_permission_matches(authorization.default_library, permission) {
            return Ok(false);
        }

        Ok([MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
            .into_iter()
            .any(|facet| scryer_domain::default_library_id_for_facet(&facet) == library_id))
    }

    pub async fn has_any_granted_library_permission(
        &self,
        actor: &User,
        permission: LibraryPermission,
    ) -> AppResult<bool> {
        Ok(!self
            .granted_library_ids_for_permission(actor, None, permission)
            .await?
            .is_empty())
    }

    pub async fn granted_library_ids_for_permission(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        permission: LibraryPermission,
    ) -> AppResult<Vec<String>> {
        let libraries = self.services.catalog.libraries.list(facet.clone()).await?;
        let authorization = self.authorization_for_actor(actor).await?;
        if libraries.is_empty()
            && library_permission_matches(authorization.default_library, permission)
        {
            return Ok(match facet {
                Some(facet) => vec![scryer_domain::default_library_id_for_facet(&facet)],
                None => [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
                    .into_iter()
                    .map(|facet| scryer_domain::default_library_id_for_facet(&facet))
                    .collect(),
            });
        }

        Ok(libraries
            .into_iter()
            .filter(|library| {
                library_permission_matches(
                    authorization.library_permissions(&library.id),
                    permission,
                )
            })
            .map(|library| library.id)
            .collect())
    }

    pub async fn has_any_library_permission(
        &self,
        actor: &User,
        permission: LibraryPermission,
    ) -> AppResult<bool> {
        Ok(!self
            .authorized_library_ids(actor, None, permission)
            .await?
            .is_empty())
    }

    pub async fn authorized_library_ids(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        permission: LibraryPermission,
    ) -> AppResult<Vec<String>> {
        let libraries = self.services.catalog.libraries.list(facet.clone()).await?;
        let authorization = self.authorization_for_actor(actor).await?;
        if libraries.is_empty()
            && library_permission_matches(authorization.default_library, permission)
        {
            return Ok(match facet {
                Some(facet) => vec![scryer_domain::default_library_id_for_facet(&facet)],
                None => [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
                    .into_iter()
                    .map(|facet| scryer_domain::default_library_id_for_facet(&facet))
                    .collect(),
            });
        }
        if app_override_allows_library_permission(&authorization, permission) {
            return Ok(libraries.into_iter().map(|library| library.id).collect());
        }
        Ok(libraries
            .into_iter()
            .filter(|library| {
                library_permission_matches(
                    authorization.library_permissions(&library.id),
                    permission,
                )
            })
            .map(|library| library.id)
            .collect())
    }

    pub(crate) async fn library_ids_with_library_permission(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        permission: LibraryPermission,
    ) -> AppResult<Vec<String>> {
        let libraries = self.services.catalog.libraries.list(facet.clone()).await?;
        let authorization = self.authorization_for_actor(actor).await?;
        if libraries.is_empty()
            && library_permission_matches(authorization.default_library, permission)
        {
            return Ok(match facet {
                Some(facet) => vec![scryer_domain::default_library_id_for_facet(&facet)],
                None => [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
                    .into_iter()
                    .map(|facet| scryer_domain::default_library_id_for_facet(&facet))
                    .collect(),
            });
        }
        Ok(libraries
            .into_iter()
            .filter(|library| {
                library_permission_matches(
                    authorization.library_permissions(&library.id),
                    permission,
                )
            })
            .map(|library| library.id)
            .collect())
    }

    pub async fn list_libraries_for_permission(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        permission: LibraryPermission,
    ) -> AppResult<Vec<scryer_domain::Library>> {
        let libraries = self.services.catalog.libraries.list(facet).await?;
        let authorization = self.authorization_for_actor(actor).await?;
        if permission_allows_app_library_override(permission)
            && (authorization
                .app
                .contains(AppPermissionMask::MANAGE_CATALOG_SETTINGS)
                || authorization
                    .app
                    .contains(AppPermissionMask::MANAGE_PERMISSIONS))
        {
            return Ok(libraries);
        }
        Ok(libraries
            .into_iter()
            .filter(|library| {
                library_permission_matches(
                    authorization.library_permissions(&library.id),
                    permission,
                )
            })
            .collect())
    }
}
