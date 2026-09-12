use std::collections::HashMap;

use scryer_domain::{
    ActorCapability, ActorCapabilityMask, AppPermission, AppPermissionMask, LibraryPermission,
    LibraryPermissionMask, MediaFacet, User, UserAuthorization,
};

use crate::{AppError, AppResult, AppUseCase};

fn permission_allows_app_library_override(permission: LibraryPermission) -> bool {
    !matches!(
        permission,
        LibraryPermission::Request
            | LibraryPermission::AutoApproveRequests
            | LibraryPermission::ManageTitles
    )
}

/// The one rule for library access.
///
/// Every listing, filter, and per-library check in the application resolves
/// through this function, so an actor who can see a library on one surface
/// can act on it on every other. An explicit grant row wins; otherwise the
/// actor's default-library mask applies (full for permission administrators,
/// see [`AppUseCase::load_user_authorization`]); otherwise catalog
/// administrators pass for every permission that is not request- or
/// title-management-scoped.
pub(crate) fn effective_library_permission(
    authorization: &UserAuthorization,
    library_id: &str,
    permission: LibraryPermission,
) -> bool {
    authorization.has_library_permission(library_id, permission)
        || (authorization
            .app
            .contains(AppPermissionMask::MANAGE_CATALOG_SETTINGS)
            && permission_allows_app_library_override(permission))
}

fn default_library_ids(facet: Option<MediaFacet>) -> Vec<String> {
    match facet {
        Some(facet) => vec![scryer_domain::default_library_id_for_facet(&facet)],
        None => [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
            .into_iter()
            .map(|facet| scryer_domain::default_library_id_for_facet(&facet))
            .collect(),
    }
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
                .has_any_library_permission(actor, LibraryPermission::ManageLibrary)
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
        if self
            .has_library_permission(actor, library_id, permission)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ))
        }
    }

    pub async fn has_library_permission(
        &self,
        actor: &User,
        library_id: &str,
        permission: LibraryPermission,
    ) -> AppResult<bool> {
        let authorization = self.authorization_for_actor(actor).await?;
        Ok(effective_library_permission(
            &authorization,
            library_id,
            permission,
        ))
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

    /// The libraries the actor holds `permission` on, in catalog order.
    pub async fn list_libraries_for_permission(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        permission: LibraryPermission,
    ) -> AppResult<Vec<scryer_domain::Library>> {
        let libraries = self.services.catalog.libraries.list(facet).await?;
        let authorization = self.authorization_for_actor(actor).await?;
        Ok(libraries
            .into_iter()
            .filter(|library| effective_library_permission(&authorization, &library.id, permission))
            .collect())
    }

    /// Ids of the libraries the actor holds `permission` on. Before any
    /// library row exists the built-in default library ids stand in, so
    /// bootstrap-time callers still resolve a scope.
    pub async fn authorized_library_ids(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        permission: LibraryPermission,
    ) -> AppResult<Vec<String>> {
        let libraries = self.services.catalog.libraries.list(facet.clone()).await?;
        let authorization = self.authorization_for_actor(actor).await?;
        let candidates = if libraries.is_empty() {
            default_library_ids(facet)
        } else {
            libraries.into_iter().map(|library| library.id).collect()
        };
        Ok(candidates
            .into_iter()
            .filter(|library_id| {
                effective_library_permission(&authorization, library_id, permission)
            })
            .collect())
    }
}
