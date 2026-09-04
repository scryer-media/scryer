export const APP_PERMISSIONS = {
  manageUsers: "MANAGE_USERS",
  managePermissions: "MANAGE_PERMISSIONS",
  manageSystemSettings: "MANAGE_SYSTEM_SETTINGS",
  manageCatalogSettings: "MANAGE_CATALOG_SETTINGS",
} as const;

export const LIBRARY_PERMISSIONS = {
  view: "VIEW",
  manageTitles: "MANAGE_TITLES",
  resolveImports: "RESOLVE_IMPORTS",
  manageLibrary: "MANAGE_LIBRARY",
  request: "REQUEST",
  autoApproveRequests: "AUTO_APPROVE_REQUESTS",
} as const;

/**
 * Stable DOM-id tokens for permission values. Test hooks keep the historical
 * camelCase tokens (the map keys) even though wire values are SCREAMING_SNAKE.
 */
export const PERMISSION_ID_TOKENS: Record<string, string> = Object.fromEntries(
  [...Object.entries(APP_PERMISSIONS), ...Object.entries(LIBRARY_PERMISSIONS)].map(
    ([key, value]) => [value, key],
  ),
);

export function permissionIdToken(value: string): string {
  return PERMISSION_ID_TOKENS[value] ?? value.toLowerCase();
}

export type AppPermission = (typeof APP_PERMISSIONS)[keyof typeof APP_PERMISSIONS];
export type LibraryPermission = (typeof LIBRARY_PERMISSIONS)[keyof typeof LIBRARY_PERMISSIONS];

export type LibraryPermissionGrant = {
  libraryId: string;
  permissions: LibraryPermission[];
};

export type PermissionUser = {
  appPermissions: AppPermission[];
  libraryPermissions: LibraryPermissionGrant[];
};

type RawLibraryPermissionGrant = {
  libraryId?: unknown;
  permissions?: readonly unknown[] | null;
};

const APP_PERMISSION_VALUES = new Set<string>(Object.values(APP_PERMISSIONS));
const LIBRARY_PERMISSION_VALUES = new Set<string>(Object.values(LIBRARY_PERMISSIONS));

function normalizePermissionClaim(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }

  const normalized = value
    .trim()
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .toUpperCase();
  return normalized || null;
}

function normalizePermissionList<T extends string>(
  values: readonly unknown[] | null | undefined,
  knownValues: ReadonlySet<string>,
): T[] {
  const permissions = new Set<T>();
  for (const value of values ?? []) {
    const normalized = normalizePermissionClaim(value);
    if (normalized && knownValues.has(normalized)) {
      permissions.add(normalized as T);
    }
  }
  return Array.from(permissions);
}

export function normalizeJwtPermissionClaims(
  appPermissions: readonly unknown[] | null | undefined,
  libraryPermissions:
    | ReadonlyArray<RawLibraryPermissionGrant | null>
    | null
    | undefined,
): PermissionUser {
  const grantsByLibrary = new Map<string, Set<LibraryPermission>>();

  for (const grant of libraryPermissions ?? []) {
    if (!grant || typeof grant.libraryId !== "string") {
      continue;
    }

    const libraryId = grant.libraryId.trim();
    if (!libraryId) {
      continue;
    }

    const permissions = normalizePermissionList<LibraryPermission>(
      grant.permissions,
      LIBRARY_PERMISSION_VALUES,
    );
    const combined = grantsByLibrary.get(libraryId) ?? new Set<LibraryPermission>();
    for (const permission of permissions) {
      combined.add(permission);
    }
    grantsByLibrary.set(libraryId, combined);
  }

  return {
    appPermissions: normalizePermissionList<AppPermission>(
      appPermissions,
      APP_PERMISSION_VALUES,
    ),
    libraryPermissions: Array.from(grantsByLibrary, ([libraryId, permissions]) => ({
      libraryId,
      permissions: Array.from(permissions),
    })),
  };
}

export function authorizationCacheSignature(
  user: PermissionUser | null | undefined,
): string {
  const appPermissions = Array.from(new Set(user?.appPermissions ?? []))
    .sort()
    .join(",");
  const libraryPermissions = (user?.libraryPermissions ?? [])
    .map((grant) => {
      const libraryId = grant.libraryId.trim();
      const permissions = Array.from(new Set(grant.permissions)).sort().join(",");
      return `${libraryId}:${permissions}`;
    })
    .sort()
    .join("|");
  return `app=${appPermissions};libraries=${libraryPermissions}`;
}

export function hasAppPermission(user: PermissionUser | null | undefined, permission: AppPermission): boolean {
  return user?.appPermissions.includes(permission) === true;
}

export function hasAnyAppPermission(
  user: PermissionUser | null | undefined,
  permissions: AppPermission[],
): boolean {
  return permissions.some((permission) => hasAppPermission(user, permission));
}

/**
 * Administrators (holders of MANAGE_PERMISSIONS) hold every library permission
 * on every library, including libraries created after their explicit grants
 * were seeded. Mirrors the backend's `load_user_authorization` fallback.
 */
function isLibraryAdministrator(user: PermissionUser | null | undefined): boolean {
  return hasAppPermission(user, APP_PERMISSIONS.managePermissions);
}

export function hasLibraryPermission(
  user: PermissionUser | null | undefined,
  libraryId: string | null | undefined,
  permission: LibraryPermission,
): boolean {
  if (!user || !libraryId) {
    return false;
  }
  if (isLibraryAdministrator(user)) {
    return true;
  }
  return user.libraryPermissions.some((grant) => {
    if (grant.libraryId !== libraryId) {
      return false;
    }
    return libraryPermissionMatches(grant.permissions, permission);
  });
}

export function hasAnyLibraryPermission(
  user: PermissionUser | null | undefined,
  permission: LibraryPermission,
): boolean {
  if (isLibraryAdministrator(user)) {
    return true;
  }
  return user?.libraryPermissions.some((grant) =>
    libraryPermissionMatches(grant.permissions, permission),
  ) === true;
}

export function libraryPermissionsWithRequestShadowing(values: string[]): string[] {
  const next = new Set(values);
  if (next.has(LIBRARY_PERMISSIONS.manageTitles)) {
    next.add(LIBRARY_PERMISSIONS.autoApproveRequests);
    next.add(LIBRARY_PERMISSIONS.request);
  } else if (next.has(LIBRARY_PERMISSIONS.autoApproveRequests)) {
    next.add(LIBRARY_PERMISSIONS.request);
  }
  return Array.from(next);
}

export function normalizeLibraryPermissionsForStorage(values: string[]): string[] {
  const next = new Set(values);
  if (next.has(LIBRARY_PERMISSIONS.manageTitles)) {
    next.delete(LIBRARY_PERMISSIONS.autoApproveRequests);
    next.delete(LIBRARY_PERMISSIONS.request);
  } else if (next.has(LIBRARY_PERMISSIONS.autoApproveRequests)) {
    next.delete(LIBRARY_PERMISSIONS.request);
  }
  return Array.from(next);
}

export function libraryPermissionShadowSource(
  explicitValues: string[],
  permission: string,
): string | null {
  const explicit = new Set(explicitValues);
  if (
    (permission === LIBRARY_PERMISSIONS.request ||
      permission === LIBRARY_PERMISSIONS.autoApproveRequests) &&
    explicit.has(LIBRARY_PERMISSIONS.manageTitles)
  ) {
    return "Manage Titles";
  }
  if (
    permission === LIBRARY_PERMISSIONS.request &&
    explicit.has(LIBRARY_PERMISSIONS.autoApproveRequests)
  ) {
    return "Auto-Approve Requests";
  }
  return null;
}

function libraryPermissionMatches(
  values: LibraryPermission[],
  permission: LibraryPermission,
): boolean {
  const explicit = new Set<string>(values);
  switch (permission) {
    case LIBRARY_PERMISSIONS.request:
      return (
        !explicit.has(LIBRARY_PERMISSIONS.manageTitles) &&
        libraryPermissionsWithRequestShadowing(values).includes(permission)
      );
    case LIBRARY_PERMISSIONS.autoApproveRequests:
      return (
        !explicit.has(LIBRARY_PERMISSIONS.manageTitles) &&
        libraryPermissionsWithRequestShadowing(values).includes(permission)
      );
    default:
      return explicit.has(permission);
  }
}
