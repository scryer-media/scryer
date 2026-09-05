import assert from "node:assert/strict";
import test from "node:test";

import type { ConfigFieldDef } from "../types/indexers.ts";
import {
  fieldConditionHolds,
  isConfigFieldRequired,
  isConfigFieldVisible,
  resolveConfigFieldsForValues,
  splitAdvancedConfigFields,
} from "./provider-config-fields.ts";
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

test("each operator matches the host's semantics", () => {
  const values = { definition: "custom", cookie: "  ", profile_id: "nzbgeek" };
  const holds = (op: string, key: string, vals: string[]) =>
    fieldConditionHolds(
      { key, op: op as never, values: vals },
      values,
    );

  assert.equal(holds("EQ", "definition", ["custom"]), true);
  assert.equal(holds("EQ", "definition", ["byo"]), false);
  assert.equal(holds("NE", "definition", ["byo"]), true);
  assert.equal(holds("IN", "profile_id", ["nzbgeek", "drunkenslug"]), true);
  assert.equal(holds("IN", "profile_id", ["drunkenslug"]), false);
  assert.equal(holds("NOT_IN", "profile_id", ["drunkenslug"]), true);
  // whitespace-only reads as empty, matching the host's trim
  assert.equal(holds("NON_EMPTY", "cookie", []), false);
  assert.equal(holds("NON_EMPTY", "definition", []), true);
  // a field that was never filled in reads the same as one cleared by hand
  assert.equal(holds("NON_EMPTY", "never_set", []), false);
  assert.equal(holds("EQ", "never_set", [""]), true);
});

test("a hidden field is never required, whatever it declared", () => {
  const yaml = field({
    key: "definition_yaml",
    required: true,
    visibleWhen: { key: "definition", op: "EQ", values: ["custom"] },
  });

  assert.equal(isConfigFieldVisible(yaml, { definition: "custom" }), true);
  assert.equal(isConfigFieldRequired(yaml, { definition: "custom" }), true);
  assert.equal(isConfigFieldVisible(yaml, { definition: "aether" }), false);
  assert.equal(isConfigFieldRequired(yaml, { definition: "aether" }), false);
});

test("requiredWhen raises a field that declared required: false", () => {
  const apiKey = field({
    key: "api_key",
    required: false,
    requiredWhen: { key: "profile_id", op: "IN", values: ["nzbgeek"] },
  });

  assert.equal(isConfigFieldRequired(apiKey, { profile_id: "nzbgeek" }), true);
  assert.equal(isConfigFieldRequired(apiKey, { profile_id: "custom" }), false);
});

test("resolving carries effective requiredness onto the field the form renders", () => {
  const fields = [
    field({ key: "profile_id" }),
    field({
      key: "api_key",
      required: false,
      requiredWhen: { key: "profile_id", op: "EQ", values: ["nzbgeek"] },
    }),
    field({
      key: "definition_yaml",
      visibleWhen: { key: "profile_id", op: "EQ", values: ["custom"] },
    }),
  ];

  const forKnown = resolveConfigFieldsForValues(fields, {
    profile_id: "nzbgeek",
  });
  assert.deepEqual(
    forKnown.map((entry) => entry.key),
    ["profile_id", "api_key"],
  );
  assert.equal(forKnown[1].required, true);

  const forCustom = resolveConfigFieldsForValues(fields, {
    profile_id: "custom",
  });
  assert.deepEqual(
    forCustom.map((entry) => entry.key),
    ["profile_id", "api_key", "definition_yaml"],
  );
  assert.equal(forCustom[1].required, false);
});
