import type { AuthUser } from "@/lib/hooks/use-auth";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  hasAnyLibraryPermission,
  hasAnyPrivilegedLibraryAccess,
  hasAppPermission,
} from "@/lib/utils/permissions";
import { canAccessListsPage } from "@/lib/utils/routes";

export function usePermissions(authenticatedUser: AuthUser) {
  const canViewCatalog = hasAnyLibraryPermission(
    authenticatedUser,
    LIBRARY_PERMISSIONS.view,
  );
  const canManageTitle = hasAnyLibraryPermission(
    authenticatedUser,
    LIBRARY_PERMISSIONS.manageTitles,
  );
  const canRequestMedia = hasAnyLibraryPermission(
    authenticatedUser,
    LIBRARY_PERMISSIONS.request,
  );
  const canResolveImports = hasAnyLibraryPermission(
    authenticatedUser,
    LIBRARY_PERMISSIONS.resolveImports,
  );
  const canAccessActivity = canResolveImports || canManageTitle;
  // Download activity, acquisition history and on-disk locations: only for
  // actors with privileged access to at least one library.
  const canViewLibraryInternals =
    hasAnyPrivilegedLibraryAccess(authenticatedUser);
  const canManageUserAccounts = hasAppPermission(
    authenticatedUser,
    APP_PERMISSIONS.manageUsers,
  );
  const canManagePermissions = hasAppPermission(
    authenticatedUser,
    APP_PERMISSIONS.managePermissions,
  );
  const canManageSystemSettings = hasAppPermission(
    authenticatedUser,
    APP_PERMISSIONS.manageSystemSettings,
  );
  const canManageCatalogSettings = hasAppPermission(
    authenticatedUser,
    APP_PERMISSIONS.manageCatalogSettings,
  );
  const canManageLists = hasAppPermission(
    authenticatedUser,
    APP_PERMISSIONS.manageLists,
  );
  const canAccessLists = canAccessListsPage(
    canViewCatalog || canRequestMedia || canManageTitle,
    canManageLists || canManageSystemSettings,
  );
  const canManageUsers = canManageUserAccounts || canManagePermissions;
  const canManageConfig = canManageSystemSettings || canManageCatalogSettings;
  const canManageLibrarySettings =
    canManageConfig ||
    hasAnyLibraryPermission(
      authenticatedUser,
      LIBRARY_PERMISSIONS.manageLibrary,
    );
  return {
    canViewCatalog,
    canManageTitle,
    canRequestMedia,
    canResolveImports,
    canAccessActivity,
    canViewLibraryInternals,
    canManageUserAccounts,
    canManagePermissions,
    canManageSystemSettings,
    canManageCatalogSettings,
    canManageLists,
    canAccessLists,
    canManageUsers,
    canManageConfig,
    canManageLibrarySettings,
  };
}
