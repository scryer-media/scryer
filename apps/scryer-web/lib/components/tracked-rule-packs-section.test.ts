import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import type React from "react";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

const WEB_ROOT = fileURLToPath(new URL("../..", import.meta.url));

const member = {
  templateId: "rule",
  ruleSetId: "rule-set",
  removed: false,
  enabled: true,
  priority: 0,
  name: "Managed rule",
  description: "",
  appliedFacets: [],
};

function pack(customizable?: boolean | null) {
  return {
    packId: "pack",
    name: "Pack",
    version: "1.0.0",
    digest: "digest",
    revision: 1,
    availableVersion: null,
    autoUpdate: false,
    autoUpdateAvailable: false,
    lastError: null,
    lastUpdated: null,
    customizable,
    members: [member],
  };
}

test("non-customizable packs hide Copy as custom while omitted metadata preserves it", async () => {
  const server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });
  try {
    const [module, uiSettingsModule] = await Promise.all([
      server.ssrLoadModule("/components/views/settings/tracked-rule-packs-section.tsx"),
      server.ssrLoadModule("/lib/context/ui-settings-context.tsx"),
    ]);
    const Component = module.TrackedRulePacksSection as ComponentType<Record<string, unknown>>;
    const UiSettingsContext = uiSettingsModule.UiSettingsContext as React.Context<unknown>;
    const uiSettingsValue = {
      uiSettings: uiSettingsModule.DEFAULT_UI_SETTINGS,
      uiSettingsLoading: false,
      uiSettingsLoaded: true,
      uiSettingsLoadError: null,
      setUiSettings: () => {},
      refreshUiSettings: async () => {},
    };
    const props = {
      packs: [pack(false)],
      canManage: true,
      mutatingPackId: null,
      mutatingRuleSetId: null,
      onPreviewUpdate: async () => null,
      onApplyUpdate: async () => {},
      onSetAutoUpdate: async () => true,
      onUninstall: async () => {},
      onToggleMember: async () => true,
      onCopyMember: () => {},
      defaultExpandedPackIds: ["pack"],
    };
    const render = () =>
      renderToStaticMarkup(
        createElement(
          UiSettingsContext.Provider,
          { value: uiSettingsValue },
          createElement(Component, props),
        ),
      );

    assert.match(render(), /Managed by the pack author/);
    assert.doesNotMatch(render(), /settings-tracked-rule-pack-copy-rule/);

    props.packs = [pack(undefined)];
    assert.match(render(), /settings-tracked-rule-pack-copy-rule/);
  } finally {
    await server.close();
  }
});
