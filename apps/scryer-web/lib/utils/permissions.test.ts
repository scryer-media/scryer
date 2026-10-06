import test from "node:test";
import assert from "node:assert/strict";

import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  canManageLibrarySubtitles,
  hasAnyLibraryPermission,
  hasAppPermission,
  hasLibraryPermission,
  libraryPermissionShadowSource,
  libraryPermissionsWithRequestShadowing,
  normalizeJwtPermissionClaims,
  normalizeLibraryPermissionsForStorage,
  withHiddenPermissionsKept,
} from "./permissions.ts";

test("normalizeJwtPermissionClaims restores camelCase JWT permissions", () => {
  const user = normalizeJwtPermissionClaims(
    [
      "manageUsers",
      "MANAGE_PERMISSIONS",
      "manageSystemSettings",
      "manageCatalogSettings",
      "manageUsers",
      "futurePermission",
    ],
    [
      {
        libraryId: " library-primary ",
        permissions: ["view", "MANAGE_TITLES", "futureLibraryPermission"],
      },
      {
        libraryId: "library-primary",
        permissions: ["resolveImports", "manageLibrary"],
      },
      {
        libraryId: "library-secondary",
        permissions: ["request", "autoApproveRequests"],
      },
    ],
  );

  assert.deepEqual(user, {
    appPermissions: [
      APP_PERMISSIONS.manageUsers,
      APP_PERMISSIONS.managePermissions,
      APP_PERMISSIONS.manageSystemSettings,
      APP_PERMISSIONS.manageCatalogSettings,
    ],
    libraryPermissions: [
      {
        libraryId: "library-primary",
        permissions: [
          LIBRARY_PERMISSIONS.view,
          LIBRARY_PERMISSIONS.manageTitles,
          LIBRARY_PERMISSIONS.resolveImports,
          LIBRARY_PERMISSIONS.manageLibrary,
        ],
      },
      {
        libraryId: "library-secondary",
        permissions: [
          LIBRARY_PERMISSIONS.request,
          LIBRARY_PERMISSIONS.autoApproveRequests,
        ],
      },
    ],
  });
  assert.equal(hasAppPermission(user, APP_PERMISSIONS.manageUsers), true);
  assert.equal(
    hasLibraryPermission(
      user,
      "library-primary",
      LIBRARY_PERMISSIONS.manageTitles,
    ),
    true,
  );
});

test("normalizeJwtPermissionClaims discards malformed and unknown claims", () => {
  const user = normalizeJwtPermissionClaims(
    [null, "", "not-a-permission"],
    [
      null,
      { libraryId: "", permissions: ["view"] },
      { libraryId: 42, permissions: ["manageTitles"] },
      { libraryId: "library-primary", permissions: [null, "unknown"] },
    ],
  );

  assert.deepEqual(user, {
    appPermissions: [],
    libraryPermissions: [{ libraryId: "library-primary", permissions: [] }],
  });
});

test("administrator fallback yields to explicit library grants", () => {
  const admin = normalizeJwtPermissionClaims(
    ["managePermissions"],
    [{ libraryId: "library-primary", permissions: ["view"] }],
  );

  assert.equal(
    hasLibraryPermission(admin, "library-created-later", LIBRARY_PERMISSIONS.manageTitles),
    true,
  );
  assert.equal(
    hasLibraryPermission(admin, "library-primary", LIBRARY_PERMISSIONS.manageLibrary),
    false,
  );
  assert.equal(hasAnyLibraryPermission(admin, LIBRARY_PERMISSIONS.resolveImports), true);
  assert.equal(hasAnyLibraryPermission(admin, LIBRARY_PERMISSIONS.request), false);
  assert.equal(hasLibraryPermission(admin, null, LIBRARY_PERMISSIONS.view), false);
  assert.equal(hasLibraryPermission(null, "library-primary", LIBRARY_PERMISSIONS.view), false);
});

test("administrator request grants remain strictly requestable", () => {
  const admin = normalizeJwtPermissionClaims(
    ["managePermissions"],
    [{ libraryId: "library-requestable", permissions: ["request"] }],
  );

  assert.equal(
    hasLibraryPermission(admin, "library-created-later", LIBRARY_PERMISSIONS.request),
    false,
  );
  assert.equal(
    hasLibraryPermission(admin, "library-requestable", LIBRARY_PERMISSIONS.request),
    true,
  );
  assert.equal(hasAnyLibraryPermission(admin, LIBRARY_PERMISSIONS.request), true);
});

test("non-administrators only hold explicitly granted library permissions", () => {
  const user = normalizeJwtPermissionClaims(
    ["manageCatalogSettings"],
    [{ libraryId: "library-primary", permissions: ["view"] }],
  );

  assert.equal(
    hasLibraryPermission(user, "library-created-later", LIBRARY_PERMISSIONS.view),
    false,
  );
  assert.equal(
    hasLibraryPermission(user, "library-primary", LIBRARY_PERMISSIONS.manageTitles),
    false,
  );
  assert.equal(hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageTitles), false);
});

// Discovery and search offer the add-to-catalog flow on Manage Titles and the
// request dialog on Request. An Auto-Approve holder must land on the request
// dialog so their approval is a recorded request, not a bare title add.
test("an Auto-Approve Requests grant routes to requesting, not adding", () => {
  const user = normalizeJwtPermissionClaims(
    [],
    [
      {
        libraryId: "library-primary",
        // The stored form: Auto-Approve strips the Request bit it implies.
        permissions: normalizeLibraryPermissionsForStorage([
          LIBRARY_PERMISSIONS.view,
          LIBRARY_PERMISSIONS.request,
          LIBRARY_PERMISSIONS.autoApproveRequests,
        ]),
      },
    ],
  );

  assert.equal(hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.request), true);
  assert.equal(
    hasLibraryPermission(user, "library-primary", LIBRARY_PERMISSIONS.autoApproveRequests),
    true,
  );
  assert.equal(hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageTitles), false);
});

test("Manage Titles shadows the request pair and Manage Subtitles", () => {
  const expanded = libraryPermissionsWithRequestShadowing([
    LIBRARY_PERMISSIONS.view,
    LIBRARY_PERMISSIONS.manageTitles,
  ]);

  assert.equal(expanded.includes(LIBRARY_PERMISSIONS.request), true);
  assert.equal(expanded.includes(LIBRARY_PERMISSIONS.autoApproveRequests), true);
  assert.equal(expanded.includes(LIBRARY_PERMISSIONS.manageSubtitles), true);

  // Auto-Approve Requests shadows only Request; it says nothing about subtitles.
  const autoApprove = libraryPermissionsWithRequestShadowing([
    LIBRARY_PERMISSIONS.autoApproveRequests,
  ]);
  assert.equal(autoApprove.includes(LIBRARY_PERMISSIONS.request), true);
  assert.equal(autoApprove.includes(LIBRARY_PERMISSIONS.manageSubtitles), false);

  // An explicit Manage Subtitles grant stands on its own and widens nothing.
  const explicitOnly = libraryPermissionsWithRequestShadowing([
    LIBRARY_PERMISSIONS.view,
    LIBRARY_PERMISSIONS.manageSubtitles,
  ]);
  assert.deepEqual(explicitOnly.sort(), [
    LIBRARY_PERMISSIONS.manageSubtitles,
    LIBRARY_PERMISSIONS.view,
  ].sort());
});

test("storage normalization strips every shadowed permission", () => {
  assert.deepEqual(
    normalizeLibraryPermissionsForStorage([
      LIBRARY_PERMISSIONS.view,
      LIBRARY_PERMISSIONS.manageTitles,
      LIBRARY_PERMISSIONS.request,
      LIBRARY_PERMISSIONS.autoApproveRequests,
      LIBRARY_PERMISSIONS.manageSubtitles,
    ]).sort(),
    [LIBRARY_PERMISSIONS.manageTitles, LIBRARY_PERMISSIONS.view].sort(),
  );

  // Without Manage Titles the subtitle grant is stored as itself.
  assert.deepEqual(
    normalizeLibraryPermissionsForStorage([
      LIBRARY_PERMISSIONS.view,
      LIBRARY_PERMISSIONS.manageSubtitles,
    ]).sort(),
    [LIBRARY_PERMISSIONS.manageSubtitles, LIBRARY_PERMISSIONS.view].sort(),
  );

  assert.deepEqual(
    normalizeLibraryPermissionsForStorage([
      LIBRARY_PERMISSIONS.autoApproveRequests,
      LIBRARY_PERMISSIONS.request,
    ]),
    [LIBRARY_PERMISSIONS.autoApproveRequests],
  );
});

test("the shadow tooltip names Manage Titles for Manage Subtitles", () => {
  assert.equal(
    libraryPermissionShadowSource(
      [LIBRARY_PERMISSIONS.manageTitles],
      LIBRARY_PERMISSIONS.manageSubtitles,
    ),
    "Manage Titles",
  );
  assert.equal(
    libraryPermissionShadowSource(
      [LIBRARY_PERMISSIONS.manageTitles],
      LIBRARY_PERMISSIONS.request,
    ),
    "Manage Titles",
  );
  assert.equal(
    libraryPermissionShadowSource(
      [LIBRARY_PERMISSIONS.autoApproveRequests],
      LIBRARY_PERMISSIONS.request,
    ),
    "Auto-Approve Requests",
  );
  // An explicitly ticked box is not shadowed by anything.
  assert.equal(
    libraryPermissionShadowSource(
      [LIBRARY_PERMISSIONS.manageSubtitles],
      LIBRARY_PERMISSIONS.manageSubtitles,
    ),
    null,
  );
  assert.equal(
    libraryPermissionShadowSource(
      [LIBRARY_PERMISSIONS.autoApproveRequests],
      LIBRARY_PERMISSIONS.manageSubtitles,
    ),
    null,
  );
});

test("a Manage Titles holder passes the Manage Subtitles gate", () => {
  const titleManager = normalizeJwtPermissionClaims(
    [],
    [{ libraryId: "library-primary", permissions: ["view", "manageTitles"] }],
  );
  const subtitleManager = normalizeJwtPermissionClaims(
    [],
    [{ libraryId: "library-primary", permissions: ["view", "manageSubtitles"] }],
  );
  const viewer = normalizeJwtPermissionClaims(
    [],
    [{ libraryId: "library-primary", permissions: ["view"] }],
  );

  for (const user of [titleManager, subtitleManager]) {
    assert.equal(
      hasLibraryPermission(user, "library-primary", LIBRARY_PERMISSIONS.manageSubtitles),
      true,
    );
    assert.equal(hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageSubtitles), true);
    // The grant does not cross libraries.
    assert.equal(
      hasLibraryPermission(user, "library-secondary", LIBRARY_PERMISSIONS.manageSubtitles),
      false,
    );
  }

  assert.equal(
    hasLibraryPermission(viewer, "library-primary", LIBRARY_PERMISSIONS.manageSubtitles),
    false,
  );
  assert.equal(hasAnyLibraryPermission(viewer, LIBRARY_PERMISSIONS.manageSubtitles), false);
  // Manage Subtitles is narrow: it never widens into title management.
  assert.equal(
    hasLibraryPermission(subtitleManager, "library-primary", LIBRARY_PERMISSIONS.manageTitles),
    false,
  );

  // The administrator fallback covers libraries without an explicit grant.
  const admin = normalizeJwtPermissionClaims(["managePermissions"], []);
  assert.equal(
    hasLibraryPermission(admin, "library-created-later", LIBRARY_PERMISSIONS.manageSubtitles),
    true,
  );
});

test("the catalog-settings override reaches subtitles in every library", () => {
  const catalogAdmin = normalizeJwtPermissionClaims(["manageCatalogSettings"], []);
  const catalogAdminWithViewGrant = normalizeJwtPermissionClaims(
    ["manageCatalogSettings"],
    [{ libraryId: "library-primary", permissions: ["view"] }],
  );
  const permissionsAdmin = normalizeJwtPermissionClaims(["managePermissions"], []);
  const viewer = normalizeJwtPermissionClaims(
    [],
    [{ libraryId: "library-primary", permissions: ["view"] }],
  );

  // Mirrors `effective_library_permission`: the catalog-settings app
  // permission overrides the library grant, whether it is absent or View-only.
  assert.equal(canManageLibrarySubtitles(catalogAdmin, "library-primary"), true);
  assert.equal(canManageLibrarySubtitles(catalogAdminWithViewGrant, "library-primary"), true);
  assert.equal(canManageLibrarySubtitles(permissionsAdmin, "library-primary"), true);
  assert.equal(canManageLibrarySubtitles(viewer, "library-primary"), false);
  // No library, no grant to check, and no override to fall back on.
  assert.equal(canManageLibrarySubtitles(viewer, null), false);

  // `hasLibraryPermission` itself stays narrow: it knows nothing of the
  // catalog-settings override.
  assert.equal(
    hasLibraryPermission(
      catalogAdminWithViewGrant,
      "library-primary",
      LIBRARY_PERMISSIONS.manageSubtitles,
    ),
    false,
  );
});

test("editing visible app permissions keeps a grant the picker hides", () => {
  const hidden = [APP_PERMISSIONS.manageLists];
  const current = [APP_PERMISSIONS.manageUsers, APP_PERMISSIONS.manageLists];

  // Adding a visible permission keeps the hidden one.
  assert.deepEqual(
    withHiddenPermissionsKept(
      [APP_PERMISSIONS.manageUsers, APP_PERMISSIONS.managePermissions],
      current,
      hidden,
    ),
    [APP_PERMISSIONS.manageUsers, APP_PERMISSIONS.managePermissions, APP_PERMISSIONS.manageLists],
  );
  // Clearing every visible permission keeps it too.
  assert.deepEqual(withHiddenPermissionsKept([], current, hidden), [APP_PERMISSIONS.manageLists]);
  // Nothing hidden is granted that was not granted before.
  assert.deepEqual(
    withHiddenPermissionsKept([APP_PERMISSIONS.manageUsers], [APP_PERMISSIONS.manageUsers], hidden),
    [APP_PERMISSIONS.manageUsers],
  );
  // With nothing hidden, the edit is taken as made.
  assert.deepEqual(withHiddenPermissionsKept([], current, []), []);
});
