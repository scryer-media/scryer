import type { AuthUser } from "@/lib/hooks/use-auth";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  hasAnyLibraryPermission,
  hasAppPermission,
} from "@/lib/utils/permissions";
import { canAccessListsPage } from "@/lib/utils/routes";
import { useExperimentalFeaturesEnabled } from "@/lib/context/instance-features-context";

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
  // Lists are experimental: without the instance opt-in the permission grants
  // nothing, so every lists surface reads this one value.
  const experimentalFeaturesEnabled = useExperimentalFeaturesEnabled();
  const canManageLists =
    experimentalFeaturesEnabled &&
    hasAppPermission(authenticatedUser, APP_PERMISSIONS.manageLists);
  const canAccessLists = canAccessListsPage(
    canViewCatalog,
    canManageLists,
    experimentalFeaturesEnabled,
  );
  const canManageUsers = canManageUserAccounts || canManagePermissions;
  const canManageConfig = canManageSystemSettings || canManageCatalogSettings;
  const canManageLibrarySettings =
    canManageConfig ||
    hasAnyLibraryPermission(authenticatedUser, LIBRARY_PERMISSIONS.manageLibrary);
  return {
    canViewCatalog,
    canManageTitle,
    canRequestMedia,
    canResolveImports,
    canAccessActivity,
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
