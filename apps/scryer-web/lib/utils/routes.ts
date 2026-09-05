import type {
  ContentSettingsSection,
  SettingsSection,
  SystemSection,
  ViewId,
} from "@/components/root/types";
import { isMediaView } from "../facets/registry.ts";

export function isMediaSettingsSection(section: ContentSettingsSection): boolean {
  return (
    section === "library" ||
    section === "general" ||
    section === "quality" ||
    section === "renaming" ||
    section === "routing"
  );
}

export function isProtectedSettingsRoute(
  view: ViewId,
  settingsSection: SettingsSection,
  contentSettingsSection: ContentSettingsSection,
): boolean {
  if (view === "settings") {
    return settingsSection !== "profile";
  }

  return isMediaView(view) && isMediaSettingsSection(contentSettingsSection);
}

export function isManageConfigMediaSection(section: ContentSettingsSection): boolean {
  return isMediaSettingsSection(section);
}

export function canAccessMediaSettingsSection(
  section: ContentSettingsSection,
  canManageConfig: boolean,
  canManageLibrarySettings: boolean,
  canResolveImports = false,
): boolean {
  if (section === "import") {
    return canResolveImports;
  }

  if (!isManageConfigMediaSection(section)) {
    return true;
  }

  if (section === "library") {
    return canManageConfig || canManageLibrarySettings;
  }

  return canManageConfig;
}

export function canAccessSettingsSection(
  section: SettingsSection,
  canManageUserAccounts: boolean,
  canManageUserAccess: boolean,
  canManageSystemSettings: boolean,
  canManageCatalogSettings: boolean,
): boolean {
  switch (section) {
    case "profile":
      return true;
    case "security":
      return canManageUserAccounts;
    case "users":
      return canManageUserAccess;
    case "general":
    case "backups":
    case "mediaServers":
    case "indexers":
    case "downloadClients":
    case "proxies":
    case "acquisition":
    case "plugins":
    case "notifications":
      return canManageSystemSettings;
    case "qualityProfiles":
    case "delayProfiles":
    case "titleTags":
    case "rules":
    case "maintenanceRules":
    case "post-processing":
    case "subtitles":
      return canManageCatalogSettings;
    default:
      return false;
  }
}

export function canAccessRecycleBinPage(
  canManageSystemSettings: boolean,
  canManageTitle: boolean,
): boolean {
  return canManageSystemSettings || canManageTitle;
}

export function canAccessSystemSection(
  section: SystemSection,
  canManageSystemSettings: boolean,
  canManageTitle: boolean,
): boolean {
  return section === "recycleBin"
    ? canAccessRecycleBinPage(canManageSystemSettings, canManageTitle)
    : canManageSystemSettings;
}

/**
 * The dashboard is the operator's landing page, so it needs the same
 * entitlement as the system section rather than mere catalog access.
 *
 * Three surfaces gate on this — the sidebar's Overview group, the command
 * palette entry, and the route guard in the shell — and they must agree: a
 * hidden nav entry beside a reachable route is a leak, not a tidy menu.
 */
export function canAccessDashboard(canManageSystemSettings: boolean): boolean {
  return canManageSystemSettings;
}

export function defaultSettingsSection(
  canManageSystemSettings: boolean,
  canManageCatalogSettings: boolean,
  canManageUserAccounts: boolean,
  canManageUserAccess: boolean,
): SettingsSection {
  if (canManageSystemSettings) {
    return "general";
  }

  if (canManageCatalogSettings) {
    return "qualityProfiles";
  }

  if (canManageUserAccounts) {
    return "security";
  }

  if (canManageUserAccess) {
    return "users";
  }

  return "profile";
}

export type AccessibleRoute = {
  view: ViewId;
  settingsSection?: SettingsSection;
  contentSettingsSection?: ContentSettingsSection;
};

/**
 * The best route a user can actually reach, in preference order.
 *
 * This is both where `/` sends a user — the path alone cannot decide, because
 * the answer depends on permissions — and where the shell bounces anyone who
 * navigates directly to a route they may not open.
 */
export function defaultAccessibleRoute(
  canViewCatalog: boolean,
  canRequestMedia: boolean,
  canResolveImports: boolean,
  canManageUserAccounts: boolean,
  canManageUserAccess: boolean,
  canManageSystemSettings: boolean,
  canManageCatalogSettings: boolean,
  canManageLibrarySettings: boolean,
): AccessibleRoute {
  const canManageConfig = canManageSystemSettings || canManageCatalogSettings;

  // The dashboard is the operator home, so it outranks the catalog for anyone
  // who can reach it. This is also what `/` resolves to for those users.
  if (canManageSystemSettings) {
    return {
      view: "dashboard",
    };
  }

  if (canViewCatalog) {
    return {
      view: "movies",
      contentSettingsSection: "overview",
    };
  }

  if (canRequestMedia) {
    return {
      view: "requests",
    };
  }

  if (canResolveImports) {
    return {
      view: "movies",
      contentSettingsSection: "import",
    };
  }

  if (canManageLibrarySettings && !canManageConfig) {
    return {
      view: "movies",
      contentSettingsSection: "library",
    };
  }

  return {
    view: "settings",
    settingsSection: defaultSettingsSection(
      canManageSystemSettings,
      canManageCatalogSettings,
      canManageUserAccounts,
      canManageUserAccess,
    ),
  };
}
