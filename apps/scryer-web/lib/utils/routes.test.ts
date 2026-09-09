import assert from "node:assert/strict";
import test from "node:test";

import {
  canAccessApiExplorer,
  canAccessSettingsSection,
  canAccessDashboard,
  canAccessRecycleBinPage,
  canAccessSystemSection,
  defaultAccessibleRoute,
} from "./routes.ts";

test("API navigation and direct access require both the opt-in and system permission", () => {
  for (const enabled of [false, undefined]) {
    assert.equal(canAccessApiExplorer(true, enabled), false);
  }
  assert.equal(canAccessApiExplorer(false, true), false);
  assert.equal(canAccessApiExplorer(true, true), true);
  assert.equal(canAccessSettingsSection("general", true, true, false, true), false);
  assert.equal(canAccessSettingsSection("general", false, false, true, false), true);
});

/**
 * `defaultAccessibleRoute` is what `/` resolves to once the signed-in user is
 * known, so these cases are the two user classes landing on the root path.
 */
function landingFor({
  canViewCatalog = false,
  canRequestMedia = false,
  canResolveImports = false,
  canManageUserAccounts = false,
  canManageUserAccess = false,
  canManageSystemSettings = false,
  canManageCatalogSettings = false,
  canManageLibrarySettings = false,
} = {}) {
  return defaultAccessibleRoute(
    canViewCatalog,
    canRequestMedia,
    canResolveImports,
    canManageUserAccounts,
    canManageUserAccess,
    canManageSystemSettings,
    canManageCatalogSettings,
    canManageLibrarySettings,
  );
}

test("system-settings managers land on the dashboard", () => {
  assert.deepEqual(landingFor({ canManageSystemSettings: true }), {
    view: "dashboard",
  });
  // The dashboard outranks the catalog even when both are reachable.
  assert.deepEqual(
    landingFor({ canManageSystemSettings: true, canViewCatalog: true }),
    { view: "dashboard" },
  );
});

test("everyone else lands on the route they can actually open", () => {
  assert.deepEqual(landingFor({ canViewCatalog: true }), {
    view: "movies",
    contentSettingsSection: "overview",
  });
  // A catalog manager without system settings is not an admin here.
  assert.deepEqual(
    landingFor({ canViewCatalog: true, canManageCatalogSettings: true }),
    { view: "movies", contentSettingsSection: "overview" },
  );
  assert.deepEqual(landingFor({ canRequestMedia: true }), { view: "requests" });
  assert.deepEqual(landingFor({ canResolveImports: true }), {
    view: "movies",
    contentSettingsSection: "import",
  });
  assert.deepEqual(landingFor(), {
    view: "settings",
    settingsSection: "profile",
  });
});

test("recycle-bin access follows the broader page-access permission", () => {
  assert.equal(canAccessSystemSection("recycleBin", false, false), false);
  assert.equal(canAccessSystemSection("recycleBin", false, true), true);
  assert.equal(canAccessSystemSection("recycleBin", true, false), true);
  assert.equal(canAccessSystemSection("recycleBin", true, true), true);
  assert.equal(canAccessRecycleBinPage(false, false), false);
  assert.equal(canAccessRecycleBinPage(false, true), true);
  assert.equal(canAccessRecycleBinPage(true, false), true);
});

test("other system sections still require system settings permission", () => {
  assert.equal(canAccessSystemSection("overview", false, true), false);
  assert.equal(canAccessSystemSection("jobs", false, true), false);
  assert.equal(canAccessSystemSection("overview", true, false), true);
  assert.equal(canAccessSystemSection("jobs", true, false), true);
});

test("the dashboard needs system-settings management, not catalog access", () => {
  assert.equal(canAccessDashboard(true), true);
  assert.equal(canAccessDashboard(false), false);
});

test("a non-admin's landing route never resolves to the dashboard", () => {
  // The nav entry and the route guard read the same predicate, so a user who
  // cannot open /dashboard is never sent there either.
  const landing = landingFor({ canViewCatalog: true, canRequestMedia: true });
  assert.notEqual(landing.view, "dashboard");
  assert.equal(canAccessDashboard(false), false);
});
