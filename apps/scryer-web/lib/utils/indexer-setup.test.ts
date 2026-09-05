import assert from "node:assert/strict";
import test from "node:test";

import type { ConfigFieldDef } from "../types/indexers.ts";
import { applyIndexerConfigOption } from "./indexer-setup.ts";

const fields: ConfigFieldDef[] = [
  {
    key: "profile_id",
    label: "Known provider",
    fieldType: "SELECT",
    required: false,
    defaultValue: null,
    valueSource: "USER",
    role: null,
    hostBinding: null,
    options: [
      {
        value: "preset",
        label: "Preset",
        configOverrides: [
          { key: "base_url", value: "https://api.example.test" },
          { key: "api_path", value: "/api" },
          { key: "unknown", value: "ignored" },
        ],
      },
    ],
    helpText: null,
    visibleWhen: null,
    requiredWhen: null,
    advanced: false,
  },
  {
    key: "base_url",
    label: "Base URL",
    fieldType: "STRING",
    required: false,
    defaultValue: null,
    valueSource: "USER",
    role: "CONNECTION_URL",
    hostBinding: null,
    options: [],
    helpText: null,
    visibleWhen: null,
    requiredWhen: null,
    advanced: false,
  },
  {
    key: "api_path",
    label: "API Path",
    fieldType: "STRING",
    required: false,
    defaultValue: "/api",
    valueSource: "USER",
    role: null,
    hostBinding: null,
    options: [],
    helpText: null,
    visibleWhen: null,
    requiredWhen: null,
    advanced: false,
  },
];

test("applies declared preset overrides when an option is selected", () => {
  assert.deepEqual(
    applyIndexerConfigOption(fields, {}, "profile_id", "preset"),
    {
      profile_id: "preset",
      base_url: "https://api.example.test",
      api_path: "/api",
    },
  );
});

test("preserves later explicit edits", () => {
  const preset = applyIndexerConfigOption(fields, {}, "profile_id", "preset");
  const edited = applyIndexerConfigOption(
    fields,
    preset,
    "base_url",
    "https://custom.example.test",
  );

  assert.equal(edited.profile_id, "preset");
  assert.equal(edited.base_url, "https://custom.example.test");
  assert.equal(edited.api_path, "/api");
});
