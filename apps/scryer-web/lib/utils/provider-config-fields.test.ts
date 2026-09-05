import assert from "node:assert/strict";
import test from "node:test";

import type { ConfigFieldDef } from "../types/indexers.ts";
import { splitAdvancedConfigFields } from "./provider-config-fields.ts";
import { visibleIndexerConfigFields } from "../types/indexers.ts";

function field(overrides: Partial<ConfigFieldDef> & Pick<ConfigFieldDef, "key">): ConfigFieldDef {
  return {
    key: overrides.key,
    label: overrides.label ?? overrides.key,
    fieldType: overrides.fieldType ?? "STRING",
    required: overrides.required ?? false,
    defaultValue: overrides.defaultValue ?? null,
    valueSource: overrides.valueSource ?? "USER",
    role: overrides.role ?? null,
    hostBinding: overrides.hostBinding ?? null,
    options: overrides.options ?? [],
    helpText: overrides.helpText ?? null,
    visibleWhen: overrides.visibleWhen ?? null,
    requiredWhen: overrides.requiredWhen ?? null,
    advanced: overrides.advanced ?? false,
  };
}

test("advanced fields separate from standard ones, keeping declared order", () => {
  const { standard, advanced } = splitAdvancedConfigFields([
    field({ key: "base_url" }),
    field({ key: "cookie", advanced: true }),
    field({ key: "username" }),
    field({ key: "captcha", advanced: true }),
  ]);

  assert.deepEqual(
    standard.map((entry) => entry.key),
    ["base_url", "username"],
  );
  assert.deepEqual(
    advanced.map((entry) => entry.key),
    ["cookie", "captcha"],
  );
});

test("a provider declaring nothing advanced keeps every field up front", () => {
  const fields = [field({ key: "base_url" }), field({ key: "api_key" })];
  const { standard, advanced } = splitAdvancedConfigFields(fields);

  assert.equal(standard.length, 2);
  assert.deepEqual(advanced, []);
});

test("host-bound values never reach the form", () => {
  const visible = visibleIndexerConfigFields([
    field({ key: "base_url" }),
    field({
      key: "api_key",
      valueSource: "HOST_BINDING",
      hostBinding: "smg.opensubtitles_api_key",
    }),
  ]);

  assert.deepEqual(
    visible.map((entry) => entry.key),
    ["base_url"],
  );
});
