import assert from "node:assert/strict";
import test from "node:test";

import {
  buildDiscoverySectionPath,
  buildIndexerSettingsPath,
  buildListsPath,
  discoverySectionIdFromPath,
  buildOverviewDetailPath,
  buildRulesPath,
  indexerSettingsTabFromPath,
  INDEXER_SETTINGS_TABS,
  listsSectionFromPath,
  lifecycleRulesSectionFromPath,
  resolveAppRoute,
  rulesSectionFromPath,
  BETA_RULES_SECTIONS,
  RULES_SECTIONS,
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
  for (const section of [
    "library",
    "general",
    "quality",
    "renaming",
    "routing",
  ] as const) {
    assert.equal(isMediaSettingsSection(section), true, section);
  }

  for (const section of ["overview", "import"] as const) {
    assert.equal(isMediaSettingsSection(section), false, section);
  }
});

test("canonical route families resolve to typed application state", () => {
  for (const path of [
    "/api-explorer",
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
    "/lists",
    "/lists/exclusions",
    "/automation/wanted/items",
    "/automation/wanted/cutoff-unmet",
    "/automation/wanted/pending",
    "/settings/acquisition",
    "/automation/rules/scoring",
    "/automation/rules/lifecycle",
    "/automation/rules/lifecycle/candidates",
    "/automation/rules/lifecycle/history",
    "/automation/rules/request",
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
    "/settings/scripts",
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
  assert.equal(
    canonical("/movies/sample-title").overviewTitleSlug,
    "sample-title",
  );
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
    ["/automation/acquisition", "/settings/acquisition"],
    ["/settings/rules", "/automation/rules/scoring"],
    ["/settings/request-rules", "/automation/rules/request"],
    ["/settings/requestRules", "/automation/rules/request"],
    // Rules names the page, not a pane, so it lands on the first one.
    ["/automation/rules", "/automation/rules/scoring"],
    ["/automation/rules/scoring-rules", "/automation/rules/scoring"],
    ["/automation/rules/lifecycle/rules", "/automation/rules/lifecycle"],
    ["/automation/rules/request-rules", "/automation/rules/request"],
    // Folded panes redirect: exclusions to the candidates, gates to the list.
    [
      "/automation/rules/lifecycle/exclusions",
      "/automation/rules/lifecycle/candidates",
    ],
    ["/automation/rules/lifecycle/gates", "/automation/rules/lifecycle"],
    [
      "/automation/rules/lifecycle/runs",
      "/automation/rules/lifecycle/history",
    ],
    // Lifecycle rules were called maintenance rules; every path that named
    // them by the old name redirects to the lifecycle pane it meant.
    ["/settings/maintenance-rules", "/automation/rules/lifecycle"],
    ["/settings/maintenanceRules", "/automation/rules/lifecycle"],
    ["/automation/maintenance-rules", "/automation/rules/lifecycle"],
    ["/automation/maintenanceRules", "/automation/rules/lifecycle"],
    ["/automation/rules/maintenance-rules", "/automation/rules/lifecycle"],
    ["/automation/rules/maintenance", "/automation/rules/lifecycle"],
    [
      "/automation/rules/maintenance/candidates",
      "/automation/rules/lifecycle/candidates",
    ],
    [
      "/automation/rules/maintenance/history",
      "/automation/rules/lifecycle/history",
    ],
    [
      "/automation/rules/maintenance/runs",
      "/automation/rules/lifecycle/history",
    ],
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
  assert.deepEqual(
    resolveAppRoute(
      "/settings/recycleBin",
      "?library=library-a&id=title-id",
      "#items",
    ),
    {
      kind: "redirect",
      to: "/system/recycle-bin?library=library-a&id=title-id#items",
    },
  );
  for (const path of [
    "/automation/wanted/history",
    "/wanted/history",
    "/history",
  ]) {
    assert.deepEqual(
      resolveAppRoute(path, "?library=library-a&id=title-id", "#items"),
      {
        kind: "redirect",
        to: "/activity/history?library=library-a&id=title-id#items",
      },
    );
  }
});

test("legacy id-based media routes remain canonical until title lookup replaces them", () => {
  const resolution = resolveAppRoute(
    "/movies",
    "?id=title-id&episodeId=episode-id",
  );
  assert.equal(resolution.kind, "canonical");
});

test("unknown roots and invalid sections do not fall back to another page", () => {
  assert.deepEqual(resolveAppRoute("/unknown"), { kind: "not-found" });
  assert.deepEqual(resolveAppRoute("/system/unknown"), { kind: "not-found" });
  assert.deepEqual(resolveAppRoute("/automation/unknown"), {
    kind: "not-found",
  });
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
    "/integrations/indexers/search",
    "/integrations/indexers/clients",
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
  for (const tab of [
    "indexers",
    "search",
    "clients",
    "seedingProfiles",
  ] as const) {
    assert.equal(
      indexerSettingsTabFromPath(buildIndexerSettingsPath(tab)),
      tab,
      tab,
    );
  }
  // Anything that is not a known pane segment falls back to the default pane.
  assert.equal(
    indexerSettingsTabFromPath("/integrations/indexers"),
    "indexers",
  );
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

test("the rules page carries all three kinds of rule as panes", () => {
  assert.equal(canonical("/automation/rules/scoring").settingsSection, "rules");
  assert.equal(
    canonical("/automation/rules/request").settingsSection,
    "requestRules",
  );
  for (const path of [
    "/automation/rules/lifecycle",
    "/automation/rules/lifecycle/candidates",
    "/automation/rules/lifecycle/history",
  ]) {
    const route = canonical(path);
    assert.equal(route.canonicalPath, path, path);
    assert.equal(route.settingsSection, "lifecycleRules", path);
  }
  assert.deepEqual(resolveAppRoute("/automation/rules/nope"), {
    kind: "not-found",
  });
  assert.deepEqual(resolveAppRoute("/automation/rules/lifecycle/nope"), {
    kind: "not-found",
  });
  // Scoring and request rules have no second level of panes, and lifecycle
  // stops at one.
  assert.deepEqual(resolveAppRoute("/automation/rules/scoring/candidates"), {
    kind: "not-found",
  });
  assert.deepEqual(resolveAppRoute("/automation/rules/request/decisions"), {
    kind: "not-found",
  });
  assert.deepEqual(
    resolveAppRoute("/automation/rules/lifecycle/history/extra"),
    {
      kind: "not-found",
    },
  );
});

test("rules pane paths round-trip through the section helpers", () => {
  for (const section of ["scoring", "lifecycle", "request"] as const) {
    assert.equal(
      rulesSectionFromPath(buildRulesPath(section)),
      section,
      section,
    );
  }
  for (const section of ["rules", "candidates", "history"] as const) {
    const path = buildRulesPath("lifecycle", section);
    assert.equal(lifecycleRulesSectionFromPath(path), section, section);
    assert.equal(rulesSectionFromPath(path), "lifecycle", section);
  }
  // Anything that is not a known pane segment falls back to the default pane.
  assert.equal(rulesSectionFromPath("/automation/rules"), "scoring");
  assert.equal(rulesSectionFromPath("/automation/rules/unknown"), "scoring");
  assert.equal(rulesSectionFromPath("/settings/profile"), "scoring");
  assert.equal(
    lifecycleRulesSectionFromPath("/automation/rules/lifecycle"),
    "rules",
  );
  assert.equal(
    lifecycleRulesSectionFromPath("/automation/rules/scoring"),
    "rules",
  );
  assert.equal(lifecycleRulesSectionFromPath("/settings/profile"), "rules");
});

test("indexer settings always offer the search and client panes", () => {
  assert.deepEqual(INDEXER_SETTINGS_TABS, [
    "indexers",
    "search",
    "clients",
    "seedingProfiles",
  ]);
  assert.equal(buildIndexerSettingsPath("indexers"), "/integrations/indexers");
});

test("every rule kind is offered, with lifecycle and request badged beta", () => {
  assert.deepEqual(RULES_SECTIONS, ["scoring", "lifecycle", "request"]);
  assert.deepEqual([...BETA_RULES_SECTIONS].sort(), ["lifecycle", "request"]);
  assert.equal(BETA_RULES_SECTIONS.has("scoring"), false);
  assert.equal(buildRulesPath("scoring"), "/automation/rules/scoring");
  assert.equal(buildRulesPath("request"), "/automation/rules/request");
});

test("the Lists page resolves its panes and redirects the default pane's alias", () => {
  assert.equal(canonical("/lists").view, "lists");
  assert.equal(canonical("/lists/exclusions").view, "lists");
  assert.equal(canonical("/lists/personal").view, "lists");
  assert.equal(canonical("/settings/lists/provider-apps").view, "lists");
  redirects("/lists/provider-apps", "/settings/lists/provider-apps");
  redirects("/lists/public", "/lists");
  assert.deepEqual(resolveAppRoute("/lists/unknown"), { kind: "not-found" });
  assert.deepEqual(resolveAppRoute("/lists/exclusions/extra"), {
    kind: "not-found",
  });
  for (const section of [
    "public",
    "personal",
    "exclusions",
    "providerApps",
  ] as const) {
    assert.equal(
      listsSectionFromPath(buildListsPath(section)),
      section,
      section,
    );
  }
  assert.equal(listsSectionFromPath("/lists/unknown"), "public");
});

test("discovery section grids round-trip their section id", () => {
  for (const sectionId of [
    "for_you",
    "because_you_like_tag_coming-of-age",
    "browse_genre_science-fiction",
    "public:trending now/€",
  ]) {
    const path = buildDiscoverySectionPath(sectionId);
    assert.equal(discoverySectionIdFromPath(path), sectionId, path);
    const route = canonical(path);
    assert.equal(route.view, "discovery");
    assert.equal(route.canonicalPath, path);
  }
  assert.equal(discoverySectionIdFromPath("/discovery"), null);
  assert.equal(discoverySectionIdFromPath("/discovery/section"), null);
  assert.equal(discoverySectionIdFromPath("/discovery/section/a/b"), null);
  assert.equal(discoverySectionIdFromPath("/discovery/section/%E0%A4%A"), null);
  assert.equal(
    discoverySectionIdFromPath("/discovery/section/top%5Frated"),
    "top_rated",
  );
  assert.equal(
    discoverySectionIdFromPath("/discovery/section/label%2520key"),
    "label%20key",
  );
  assert.equal(
    discoverySectionIdFromPath("/discovery/section/public%3Atrending%2Fnow"),
    "public:trending/now",
  );
  redirects("/Discovery/Section/for_you", "/discovery/section/for_you");
  for (const path of [
    "/discovery/section",
    "/discovery/other/for_you",
    "/discovery/section/for_you/extra",
  ]) {
    assert.deepEqual(resolveAppRoute(path), { kind: "not-found" }, path);
  }
});
