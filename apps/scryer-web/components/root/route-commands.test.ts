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
    libraryPermissions: [
      {
        libraryId: "library-id",
        permissions: [LIBRARY_PERMISSIONS.manageTitles],
      },
    ],
  });
  const ordinaryUser = user();

  for (const authorizedUser of [systemUser, titleManager]) {
    assert.ok(
      buildRouteCommands({
        t,
        user: authorizedUser,
        onNavigate: () => {},
      }).some((command) => command.id === "system-recycle-bin"),
    );
  }
  assert.equal(
    buildRouteCommands({
      t,
      user: ordinaryUser,
      onNavigate: () => {},
    }).some((command) => command.id === "system-recycle-bin"),
    false,
  );
});

test("recycle-bin command targets the canonical system section", () => {
  const calls: unknown[][] = [];
  const command = buildRouteCommands({
    t,
    user: user({
      libraryPermissions: [
        {
          libraryId: "library-id",
          permissions: [LIBRARY_PERMISSIONS.manageTitles],
        },
      ],
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
    libraryPermissions: [
      {
        libraryId: "library-id",
        permissions: [LIBRARY_PERMISSIONS.manageTitles],
      },
    ],
  });

  assert.ok(
    buildRouteCommands({
      t,
      user: systemUser,
      onNavigate: () => {},
    }).some((command) => command.id === "dashboard"),
  );

  for (const unauthorizedUser of [titleManager, user()]) {
    assert.equal(
      buildRouteCommands({
        t,
        user: unauthorizedUser,
        onNavigate: () => {},
      }).some((command) => command.id === "dashboard"),
      false,
    );
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

test("scripts is grouped with Settings for system-settings managers only", () => {
  const find = (appPermissions: AuthUser["appPermissions"]) =>
    buildRouteCommands({
      t,
      user: user({ appPermissions }),
      onNavigate: () => {},
    }).find((candidate) => candidate.id === "settings-scripts");

  assert.equal(
    find([APP_PERMISSIONS.manageSystemSettings])?.groupLabel,
    "nav.settings",
  );
  assert.equal(find([APP_PERMISSIONS.manageCatalogSettings]), undefined);
});

test("lifecycle-rules command is offered to catalog managers without the experimental switch", () => {
  const catalogManager = user({
    appPermissions: [APP_PERMISSIONS.manageCatalogSettings],
  });

  assert.ok(
    buildRouteCommands({
      t,
      user: catalogManager,
      onNavigate: () => {},
    }).some((command) => command.id === "settings-lifecycle-rules"),
  );
  // The catalog-settings permission still gates it.
  assert.equal(
    buildRouteCommands({
      t,
      user: user(),
      onNavigate: () => {},
    }).some((command) => command.id === "settings-lifecycle-rules"),
    false,
  );
});

test("lists command reaches catalog viewers and list managers", () => {
  const viewer = user({
    libraryPermissions: [
      {
        libraryId: "library-id",
        permissions: [LIBRARY_PERMISSIONS.view],
      },
    ],
  });
  const listManager = user({ appPermissions: [APP_PERMISSIONS.manageLists] });

  for (const authorizedUser of [viewer, listManager]) {
    const commands = buildRouteCommands({
      t,
      user: authorizedUser,
      onNavigate: () => {},
    });
    const command = commands.find((candidate) => candidate.id === "lists");
    assert.ok(command);
    assert.equal(command.groupLabel, "nav.group.automation");
  }
  assert.equal(
    buildRouteCommands({
      t,
      user: user(),
      onNavigate: () => {},
    }).some((command) => command.id === "lists"),
    false,
  );
  assert.equal(
    buildRouteCommands({ t, user: listManager, onNavigate: () => {} }).some(
      (command) => command.id === "lists" || command.id === "lists-exclusions",
    ),
    true,
  );
});

test("lists command is hidden from a viewer whose lists are not allowed", () => {
  const viewer = user({
    libraryPermissions: [
      {
        libraryId: "library-id",
        permissions: [LIBRARY_PERMISSIONS.view],
      },
    ],
  });
  const listManager = user({ appPermissions: [APP_PERMISSIONS.manageLists] });
  const hasLists = (authorizedUser: typeof viewer) =>
    buildRouteCommands({
      t,
      user: authorizedUser,
      personalListsRefused: true,
      onNavigate: () => {},
    }).some((command) => command.id === "lists");

  assert.equal(hasLists(viewer), false);
  assert.equal(hasLists(listManager), true);
});

test("list exclusions command is limited to list managers and opens its pane", () => {
  const viewer = user({
    libraryPermissions: [
      {
        libraryId: "library-id",
        permissions: [LIBRARY_PERMISSIONS.view],
      },
    ],
  });
  assert.equal(
    buildRouteCommands({
      t,
      user: viewer,
      onNavigate: () => {},
    }).some((command) => command.id === "lists-exclusions"),
    false,
  );

  const paths: string[] = [];
  const command = buildRouteCommands({
    t,
    user: user({ appPermissions: [APP_PERMISSIONS.manageLists] }),
    onNavigate: () => {},
    onNavigatePath: (path) => paths.push(path),
  }).find((candidate) => candidate.id === "lists-exclusions");
  assert.ok(command);
  command.onSelect();
  assert.deepEqual(paths, ["/lists/exclusions"]);
});
