import assert from "node:assert/strict";
import test from "node:test";

import type { RegistryPluginRecord } from "../../components/views/settings/settings-plugins-section.tsx";
import type { TrackedRulePackRecord } from "../../components/views/settings/tracked-rule-packs-section.tsx";
import {
  defaultRulePackTemplateIds,
  otherSetupPlugins,
  resolveSetupPluginRecommendations,
  rulePackEnabled,
  rulePackSettingsInput,
} from "./setup-recommendations.ts";

function plugin(id: string, overrides: Partial<RegistryPluginRecord> = {}): RegistryPluginRecord {
  return {
    id,
    name: id,
    description: "",
    version: "1.0.0",
    pluginType: "notification",
    providerType: id,
    author: "scryer",
    official: true,
    builtin: false,
    isInstalled: false,
    isEnabled: false,
    installedVersion: null,
    updateAvailable: false,
    installInProgress: false,
    ...overrides,
  };
}

function pack(overrides: Partial<TrackedRulePackRecord> = {}): TrackedRulePackRecord {
  return {
    packId: "pack",
    name: "Pack",
    version: "1.0.0",
    digest: "sha256:0",
    revision: 7,
    availableVersion: null,
    autoUpdate: true,
    autoUpdateAvailable: false,
    lastError: null,
    lastUpdated: null,
    members: [
      { templateId: "a", ruleSetId: "ra", removed: false, enabled: true, priority: 5, name: null, description: null, appliedFacets: [] },
      { templateId: "b", ruleSetId: "rb", removed: false, enabled: false, priority: null, name: null, description: null, appliedFacets: [] },
      { templateId: "gone", ruleSetId: "rg", removed: true, enabled: true, priority: 1, name: null, description: null, appliedFacets: [] },
    ],
    ...overrides,
  };
}

test("recommendations keep their order and only the plugins the registry offers", () => {
  const resolved = resolveSetupPluginRecommendations([
    plugin("plex"),
    plugin("jellyfin"),
    plugin("qbittorrent"),
    plugin("opensubtitles", { official: false }),
  ]);
  assert.deepEqual(
    resolved.map((entry) => [entry.key, entry.plugins.map((p) => p.id)]),
    [
      ["qbittorrent", ["qbittorrent"]],
      ["media-server", ["jellyfin", "plex"]],
    ],
  );
});

test("the full list hides built-ins and anything already recommended", () => {
  const others = otherSetupPlugins([
    plugin("newznab", { builtin: true }),
    plugin("discord"),
    plugin("emby"),
    plugin("community", { official: false }),
  ]);
  assert.deepEqual(others.map((p) => p.id), ["discord"]);
});

test("a pack's switch follows whether any live rule is on", () => {
  assert.equal(rulePackEnabled(undefined), false);
  assert.equal(rulePackEnabled(pack()), true);
  assert.equal(
    rulePackEnabled(
      pack({
        members: pack().members.map((member) => ({ ...member, enabled: member.removed })),
      }),
    ),
    false,
  );
});

test("turning a pack on uses its defaults, or every rule when it marks none", () => {
  assert.deepEqual(
    defaultRulePackTemplateIds([
      { id: "core", defaultEnabled: true },
      { id: "locale", defaultEnabled: false },
    ]),
    ["core"],
  );
  assert.deepEqual(defaultRulePackTemplateIds([{ id: "only", defaultEnabled: false }]), ["only"]);
});

test("settings keep priorities and auto-update and skip removed rules", () => {
  assert.deepEqual(rulePackSettingsInput(pack(), ["b", "gone"]), {
    packId: "pack",
    enabledTemplateIds: ["b"],
    priorities: [{ templateId: "a", priority: 5 }],
    autoUpdate: true,
    expectedRevision: 7,
  });
});
