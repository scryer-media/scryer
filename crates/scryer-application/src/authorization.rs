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

/// The one rule for privileged library access: where a library's media lives
/// on disk, how it was acquired, and its history.
///
/// Holding `View` on a library lets an actor browse its catalog. Paths,
/// release provenance, scoring, queue, wanted and history data are reserved
/// for actors who can manage the library's titles or the library itself, and
/// for system or catalog administrators. Full administrators hold
/// `ManageTitles` through their default-library mask, so the library check
/// covers them.
pub(crate) fn privileged_library_access(
    authorization: &UserAuthorization,
    library_id: &str,
) -> bool {
    effective_library_permission(authorization, library_id, LibraryPermission::ManageTitles)
        || effective_library_permission(authorization, library_id, LibraryPermission::ManageLibrary)
        || authorization.has_any_app_permission(AppPermissionMask::from_permissions([
            AppPermission::ManageSystemSettings,
            AppPermission::ManageCatalogSettings,
        ]))
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

    pub(crate) async fn authorization_for_actor(
        &self,
        actor: &User,
    ) -> AppResult<UserAuthorization> {
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
            Err(AppError::unauthorized_reason(
                crate::error_reasons::authorization::ACTION_PERMISSION_DENIED,
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
            Err(AppError::unauthorized_reason(
                crate::error_reasons::authorization::ACTION_PERMISSION_DENIED,
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
            Err(AppError::unauthorized_reason(
                crate::error_reasons::authorization::ACTION_PERMISSION_DENIED,
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
            Err(AppError::unauthorized_reason(
                crate::error_reasons::authorization::LIBRARY_ACCESS_DENIED,
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

    /// The library ids a permission check resolves against, before any actor is
    /// considered. Before any library row exists the built-in default library
    /// ids stand in, so bootstrap-time callers still resolve a scope.
    ///
    /// Held apart from [`Self::authorized_library_ids`] so a caller that
    /// filters many actors against the same catalog — the navigation badge
    /// counts — can read it once instead of per actor.
    pub(crate) async fn permission_candidate_library_ids(
        &self,
        facet: Option<MediaFacet>,
    ) -> AppResult<Vec<String>> {
        let libraries = self.services.catalog.libraries.list(facet.clone()).await?;
        Ok(if libraries.is_empty() {
            default_library_ids(facet)
        } else {
            libraries.into_iter().map(|library| library.id).collect()
        })
    }

    /// Whether the actor holds privileged access to `library_id`; see
    /// [`privileged_library_access`].
    pub async fn has_privileged_library_access(
        &self,
        actor: &User,
        library_id: &str,
    ) -> AppResult<bool> {
        let authorization = self.authorization_for_actor(actor).await?;
        Ok(privileged_library_access(&authorization, library_id))
    }

    /// Ids of the libraries the actor holds privileged access to; see
    /// [`privileged_library_access`]. Batch callers read this once and filter
    /// or redact per library.
    pub async fn privileged_library_ids(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
    ) -> AppResult<Vec<String>> {
        let candidates = self.permission_candidate_library_ids(facet).await?;
        let authorization = self.authorization_for_actor(actor).await?;
        Ok(candidates
            .into_iter()
            .filter(|library_id| privileged_library_access(&authorization, library_id))
            .collect())
    }

    /// Refuses with `LIBRARY_ACCESS_DENIED` unless the actor holds privileged
    /// access to `library_id`.
    pub async fn require_privileged_library_access(
        &self,
        actor: &User,
        library_id: &str,
    ) -> AppResult<()> {
        if self
            .has_privileged_library_access(actor, library_id)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::unauthorized_reason(
                crate::error_reasons::authorization::LIBRARY_ACCESS_DENIED,
                "You do not have access to this library".to_string(),
            ))
        }
    }

    /// [`Self::privileged_library_ids`] for a root listing: refuses with
    /// `LIBRARY_ACCESS_DENIED` when the actor holds privileged access to no
    /// library at all, so a viewer is told the surface is closed rather than
    /// handed an empty page.
    pub async fn require_privileged_library_ids(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
    ) -> AppResult<Vec<String>> {
        let library_ids = self.privileged_library_ids(actor, facet).await?;
        if library_ids.is_empty() {
            Err(AppError::unauthorized_reason(
                crate::error_reasons::authorization::LIBRARY_ACCESS_DENIED,
                "You do not have access to this library".to_string(),
            ))
        } else {
            Ok(library_ids)
        }
    }

    /// Media files of one library as the actor may see them: unchanged with
    /// privileged access, otherwise each one
    /// [redacted](crate::TitleMediaFile::redacted_for_viewer).
    pub(crate) async fn redact_media_files_for_library(
        &self,
        actor: &User,
        library_id: &str,
        files: Vec<crate::TitleMediaFile>,
    ) -> AppResult<Vec<crate::TitleMediaFile>> {
        if self
            .has_privileged_library_access(actor, library_id)
            .await?
        {
            Ok(files)
        } else {
            Ok(files
                .into_iter()
                .map(crate::TitleMediaFile::redacted_for_viewer)
                .collect())
        }
    }

    /// Ids of the libraries the actor holds `permission` on.
    pub async fn authorized_library_ids(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        permission: LibraryPermission,
    ) -> AppResult<Vec<String>> {
        Ok(self
            .library_permission_context(actor, facet)
            .await?
            .library_ids(permission))
    }

    /// The libraries and the actor's authorization, read once so a caller
    /// that needs several permissions filters them without reading either
    /// again.
    pub(crate) async fn library_permission_context(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
    ) -> AppResult<LibraryPermissionContext> {
        let libraries = self.services.catalog.libraries.list(facet.clone()).await?;
        let candidate_library_ids = if libraries.is_empty() {
            default_library_ids(facet)
        } else {
            libraries.iter().map(|library| library.id.clone()).collect()
        };
        Ok(LibraryPermissionContext {
            libraries,
            candidate_library_ids,
            authorization: self.authorization_for_actor(actor).await?,
        })
    }
}

/// What [`AppUseCase::library_permission_context`] read.
pub(crate) struct LibraryPermissionContext {
    pub(crate) libraries: Vec<scryer_domain::Library>,
    candidate_library_ids: Vec<String>,
    authorization: UserAuthorization,
}

impl LibraryPermissionContext {
    /// [`AppUseCase::authorized_library_ids`] for `permission`.
    pub(crate) fn library_ids(&self, permission: LibraryPermission) -> Vec<String> {
        self.candidate_library_ids
            .iter()
            .filter(|library_id| {
                effective_library_permission(&self.authorization, library_id, permission)
            })
            .cloned()
            .collect()
    }
}
