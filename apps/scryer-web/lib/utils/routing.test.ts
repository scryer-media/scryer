import assert from "node:assert/strict";
import test from "node:test";

import {
  buildIndexerSettingsPath,
  buildOverviewDetailPath,
  indexerSettingsTabFromPath,
  resolveAppRoute,
  type ParsedAppRoute,
} from "./routing.ts";
import { isMediaSettingsSection } from "./routes.ts";

function canonical(path: string): ParsedAppRoute {
  const resolution = resolveAppRoute(path);
  assert.equal(resolution.kind, "canonical", path);
  if (resolution.kind !== "canonical") {
    throw new Error(`Expected canonical route for ${path}`);
  }
  return resolution.route;
}

function redirects(from: string, to: string): void {
  assert.deepEqual(resolveAppRoute(from), { kind: "redirect", to });
}

test("facet settings sections that consume media settings trigger loading", () => {
  for (const section of ["library", "general", "quality", "renaming", "routing"] as const) {
    assert.equal(isMediaSettingsSection(section), true, section);
  }

  for (const section of ["overview", "import"] as const) {
    assert.equal(isMediaSettingsSection(section), false, section);
  }
});

test("canonical route families resolve to typed application state", () => {
  for (const path of [
    "/dashboard",
    "/movies",
    "/series",
    "/anime",
    "/discovery",
    "/requests",
    "/activity",
    "/activity/import",
    "/activity/history",
    "/calendar",
    "/automation/wanted/items",
    "/automation/wanted/cutoff-unmet",
    "/automation/wanted/pending",
    "/automation/acquisition",
    "/automation/rules",
    "/automation/subtitles",
    "/automation/post-processing",
    "/integrations/indexers",
    "/integrations/download-clients",
    "/integrations/proxies",
    "/integrations/media-servers",
    "/integrations/notifications",
    "/settings/profile",
    "/settings/general",
    "/settings/quality-profiles",
    "/settings/delay-profiles",
    "/settings/plugins",
    "/system",
    "/system/jobs",
    "/system/recycle-bin",
    "/system/users",
    "/system/security",
    "/system/backup",
    "/logs",
    "/logs/audit",
  ]) {
    assert.equal(canonical(path).canonicalPath, path);
  }
});

test("media detail and settings routes reject ambiguous or extra segments", () => {
  assert.equal(canonical("/movies/sample-title").overviewTitleSlug, "sample-title");
  assert.equal(
    canonical("/series/library-a/sample-title").overviewLibrarySlug,
    "library-a",
  );
  assert.equal(
    canonical("/anime/settings/renaming").contentSettingsSection,
    "renaming",
  );
  assert.deepEqual(resolveAppRoute("/movies/settings/unknown"), {
    kind: "not-found",
  });
  assert.deepEqual(resolveAppRoute("/series/library-a/sample-title/extra"), {
    kind: "not-found",
  });
});

test("reserved title slugs use library-qualified paths", () => {
  const path = buildOverviewDetailPath("movies", "movies", "settings");
  assert.equal(path, "/movies/movies/settings");
  assert.equal(canonical(path).overviewTitleSlug, "settings");
});

test("0.16 route aliases redirect to canonical 0.17 paths", () => {
  for (const [from, to] of [
    ["/movies/overview", "/movies"],
    ["/series/settings", "/series/settings/library"],
    ["/series/media", "/series/settings/library"],
    ["/anime/requests", "/requests"],
    ["/wanted", "/automation/wanted/items"],
    ["/wanted/wanted-items", "/automation/wanted/items"],
    ["/wanted/wanted", "/automation/wanted/items"],
    ["/wanted/cutoff-unmet", "/automation/wanted/cutoff-unmet"],
    ["/wanted/cutoff", "/automation/wanted/cutoff-unmet"],
    ["/automation/wanted/history", "/activity/history"],
    ["/wanted/history", "/activity/history"],
    ["/history", "/activity/history"],
    ["/settings/acquisition", "/automation/acquisition"],
    ["/settings/rules", "/automation/rules"],
    ["/settings/subtitles", "/automation/subtitles"],
    ["/settings/post-processing", "/automation/post-processing"],
    ["/settings/post-procesing", "/automation/post-processing"],
    ["/settings/indexers", "/integrations/indexers"],
    ["/settings/proxies", "/integrations/proxies"],
    // Proxies were a pane of the Indexers page until they became a section of
    // their own; both spellings of that pane are links people already have.
    ["/integrations/indexers/proxies", "/integrations/proxies"],
    ["/integrations/indexers/indexer-proxies", "/integrations/proxies"],
    ["/integrations/indexer-proxies", "/integrations/proxies"],
    ["/settings/indexer-proxies", "/integrations/proxies"],
    ["/settings/download-clients", "/integrations/download-clients"],
    ["/settings/downloadClients", "/integrations/download-clients"],
    ["/settings/media-servers", "/integrations/media-servers"],
    ["/settings/mediaServers", "/integrations/media-servers"],
    ["/settings/notifications", "/integrations/notifications"],
    ["/settings/users", "/system/users"],
    ["/settings/security", "/system/security"],
    ["/settings/qualityProfiles", "/settings/quality-profiles"],
    ["/settings/delayProfiles", "/settings/delay-profiles"],
    ["/settings/backup", "/system/backup"],
    ["/settings/backups", "/system/backup"],
    ["/settings/recycle-bin", "/system/recycle-bin"],
    ["/settings/recycleBin", "/system/recycle-bin"],
    ["/automation/post-procesing", "/automation/post-processing"],
    ["/system/overview", "/system"],
    ["/system/backups", "/system/backup"],
    ["/system/recycleBin", "/system/recycle-bin"],
    ["/system/logs", "/logs"],
    ["/system/audit", "/logs/audit"],
    ["/logs/logs", "/logs"],
    ["/logs/service", "/logs"],
    ["/logs/service-logs", "/logs"],
    ["/logs/audit-logs", "/logs/audit"],
  ] as const) {
    redirects(from, to);
  }
});

test("redirects preserve query parameters and hashes", () => {
  assert.deepEqual(resolveAppRoute(
    "/settings/recycleBin",
    "?library=library-a&id=title-id",
    "#items",
  ), {
    kind: "redirect",
    to: "/system/recycle-bin?library=library-a&id=title-id#items",
  });
  for (const path of [
    "/automation/wanted/history",
    "/wanted/history",
    "/history",
  ]) {
    assert.deepEqual(resolveAppRoute(
      path,
      "?library=library-a&id=title-id",
      "#items",
    ), {
      kind: "redirect",
      to: "/activity/history?library=library-a&id=title-id#items",
    });
  }
});

test("legacy id-based media routes remain canonical until title lookup replaces them", () => {
  const resolution = resolveAppRoute("/movies", "?id=title-id&episodeId=episode-id");
  assert.equal(resolution.kind, "canonical");
});

test("unknown roots and invalid sections do not fall back to another page", () => {
  assert.deepEqual(resolveAppRoute("/unknown"), { kind: "not-found" });
  assert.deepEqual(resolveAppRoute("/system/unknown"), { kind: "not-found" });
  assert.deepEqual(resolveAppRoute("/automation/unknown"), { kind: "not-found" });
});

test("the root path defers to the shell instead of resolving by path", () => {
  // `/` depends on the signed-in user's permissions, which parsing cannot see;
  // `lib/utils/routes.test.ts` covers where each user class actually lands.
  assert.deepEqual(resolveAppRoute("/"), { kind: "landing" });
  assert.deepEqual(resolveAppRoute(""), { kind: "landing" });
  assert.deepEqual(resolveAppRoute(null), { kind: "landing" });
  // The query string survives because the shell, not the parser, navigates.
  assert.deepEqual(resolveAppRoute("/", "?lang=fra"), { kind: "landing" });
});

test("the dashboard route is canonical and takes no subpaths", () => {
  assert.equal(canonical("/dashboard").view, "dashboard");
  assert.equal(canonical("/dashboard").canonicalPath, "/dashboard");
  assert.deepEqual(resolveAppRoute("/dashboard/x"), { kind: "not-found" });
  assert.deepEqual(resolveAppRoute("/dashboard/storage/roots"), {
    kind: "not-found",
  });
});

test("the indexers page carries its panes as a third path segment", () => {
  for (const path of [
    "/integrations/indexers",
    "/integrations/indexers/seeding-profiles",
  ]) {
    const route = canonical(path);
    assert.equal(route.canonicalPath, path, path);
    assert.equal(route.settingsSection, "indexers", path);
  }
  assert.deepEqual(resolveAppRoute("/integrations/indexers/nope"), {
    kind: "not-found",
  });
  // Panes belong to indexers alone; other integrations stay two-segment.
  assert.deepEqual(resolveAppRoute("/integrations/notifications/proxies"), {
    kind: "not-found",
  });
});

test("seeding profiles are no longer a settings section of their own", () => {
  assert.deepEqual(resolveAppRoute("/settings/seeding-profiles"), {
    kind: "not-found",
  });
});

test("indexer pane paths round-trip through the tab helpers", () => {
  for (const tab of ["indexers", "seedingProfiles"] as const) {
    assert.equal(
      indexerSettingsTabFromPath(buildIndexerSettingsPath(tab)),
      tab,
      tab,
    );
  }
  // Anything that is not a known pane segment falls back to the default pane.
  assert.equal(indexerSettingsTabFromPath("/integrations/indexers"), "indexers");
  assert.equal(
    indexerSettingsTabFromPath("/integrations/indexers/unknown"),
    "indexers",
  );
  assert.equal(indexerSettingsTabFromPath("/settings/profile"), "indexers");
  // Proxies left the page, so its old segment is no longer a pane.
  assert.equal(
    indexerSettingsTabFromPath("/integrations/indexers/proxies"),
    "indexers",
  );
});
