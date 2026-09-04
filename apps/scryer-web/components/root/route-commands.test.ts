import assert from "node:assert/strict";
import test from "node:test";

import type { AuthUser } from "@/lib/hooks/use-auth";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
} from "../../lib/utils/permissions.ts";
import { buildRouteCommands } from "./route-commands.ts";

function user(overrides: Partial<AuthUser> = {}): AuthUser {
  return {
    id: "user-id",
    username: "user",
    appPermissions: [],
    libraryPermissions: [],
    ...overrides,
  };
}

const t = (key: string) => key;

test("recycle-bin command is visible to either backend-authorized role", () => {
  const systemUser = user({
    appPermissions: [APP_PERMISSIONS.manageSystemSettings],
  });
  const titleManager = user({
    libraryPermissions: [{
      libraryId: "library-id",
      permissions: [LIBRARY_PERMISSIONS.manageTitles],
    }],
  });
  const ordinaryUser = user();

  for (const authorizedUser of [systemUser, titleManager]) {
    assert.ok(buildRouteCommands({
      t,
      user: authorizedUser,
      onNavigate: () => {},
    }).some((command) => command.id === "system-recycle-bin"));
  }
  assert.equal(buildRouteCommands({
    t,
    user: ordinaryUser,
    onNavigate: () => {},
  }).some((command) => command.id === "system-recycle-bin"), false);
});

test("recycle-bin command targets the canonical system section", () => {
  const calls: unknown[][] = [];
  const command = buildRouteCommands({
    t,
    user: user({
      libraryPermissions: [{
        libraryId: "library-id",
        permissions: [LIBRARY_PERMISSIONS.manageTitles],
      }],
    }),
    onNavigate: (...args) => calls.push(args),
  }).find((candidate) => candidate.id === "system-recycle-bin");

  assert.ok(command);
  command.onSelect();
  assert.equal(calls[0]?.[0], "system");
  assert.equal(calls[0]?.[3], "recycleBin");
});

test("dashboard command is limited to system-settings managers", () => {
  const systemUser = user({
    appPermissions: [APP_PERMISSIONS.manageSystemSettings],
  });
  const titleManager = user({
    libraryPermissions: [{
      libraryId: "library-id",
      permissions: [LIBRARY_PERMISSIONS.manageTitles],
    }],
  });

  assert.ok(buildRouteCommands({
    t,
    user: systemUser,
    onNavigate: () => {},
  }).some((command) => command.id === "dashboard"));

  for (const unauthorizedUser of [titleManager, user()]) {
    assert.equal(buildRouteCommands({
      t,
      user: unauthorizedUser,
      onNavigate: () => {},
    }).some((command) => command.id === "dashboard"), false);
  }
});

test("dashboard command navigates to the dashboard view", () => {
  const calls: unknown[][] = [];
  const command = buildRouteCommands({
    t,
    user: user({ appPermissions: [APP_PERMISSIONS.manageSystemSettings] }),
    onNavigate: (...args) => calls.push(args),
  }).find((candidate) => candidate.id === "dashboard");

  assert.ok(command);
  assert.equal(command.groupLabel, "nav.group.overview");
  command.onSelect();
  assert.equal(calls[0]?.[0], "dashboard");
});

test("post-processing is grouped with Automation", () => {
  const command = buildRouteCommands({
    t,
    user: user({
      appPermissions: [APP_PERMISSIONS.manageCatalogSettings],
    }),
    onNavigate: () => {},
  }).find((candidate) => candidate.id === "settings-post-processing");

  assert.equal(command?.groupLabel, "nav.group.automation");
});

test("maintenance-rules command appears only when experimental features are on", () => {
  const catalogManager = user({
    appPermissions: [APP_PERMISSIONS.manageCatalogSettings],
  });

  assert.ok(
    buildRouteCommands({
      t,
      user: catalogManager,
      experimentalFeaturesEnabled: true,
      onNavigate: () => {},
    }).some((command) => command.id === "settings-maintenance-rules"),
  );
  assert.equal(
    buildRouteCommands({
      t,
      user: catalogManager,
      experimentalFeaturesEnabled: false,
      onNavigate: () => {},
    }).some((command) => command.id === "settings-maintenance-rules"),
    false,
  );
  // A caller that has not read the switch yet must not surface the page.
  assert.equal(
    buildRouteCommands({
      t,
      user: catalogManager,
      onNavigate: () => {},
    }).some((command) => command.id === "settings-maintenance-rules"),
    false,
  );
});

test("hiding maintenance rules leaves the rest of the catalog settings commands", () => {
  const commands = buildRouteCommands({
    t,
    user: user({ appPermissions: [APP_PERMISSIONS.manageCatalogSettings] }),
    experimentalFeaturesEnabled: false,
    onNavigate: () => {},
  });

  for (const id of [
    "settings-quality-profiles",
    "settings-rules",
    "settings-post-processing",
  ]) {
    assert.ok(
      commands.some((command) => command.id === id),
      `expected ${id} to survive the maintenance-rules gate`,
    );
  }
});
