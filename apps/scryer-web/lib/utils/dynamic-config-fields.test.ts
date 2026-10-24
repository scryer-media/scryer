import assert from "node:assert/strict";
import test from "node:test";

import type { ConfigFieldDef } from "../types/indexers.ts";
import {
  createRequestEpoch,
  dynamicTriggerSignature,
  dynamicTriggerValues,
  mergeDynamicConfigFields,
  omitHiddenConfigValues,
  reconcileDynamicConfigValues,
  type DynamicConfigFields,
} from "./dynamic-config-fields.ts";

function field(
  overrides: Partial<ConfigFieldDef> & Pick<ConfigFieldDef, "key">,
): ConfigFieldDef {
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

const staticFields = [
  field({ key: "definition", required: true, defaultValue: "alpha" }),
  field({ key: "username", required: true }),
];

const alphaAnswer: DynamicConfigFields = {
  fields: [
    field({ key: "cookie", fieldType: "PASSWORD", required: true }),
    field({ key: "sort", fieldType: "SELECT", defaultValue: "new" }),
  ],
  hiddenFieldKeys: ["username"],
};

test("merge hides named static fields and appends dynamic ones", () => {
  const merged = mergeDynamicConfigFields(staticFields, alphaAnswer);
  assert.deepEqual(
    merged.map((entry) => entry.key),
    ["definition", "cookie", "sort"],
  );
  assert.equal(mergeDynamicConfigFields(staticFields, null), staticFields);
});

test("merge never lets a dynamic field shadow a static one", () => {
  const merged = mergeDynamicConfigFields(staticFields, {
    fields: [field({ key: "definition", label: "Imposter" })],
    hiddenFieldKeys: [],
  });
  assert.deepEqual(
    merged.map((entry) => entry.label),
    ["definition", "username"],
  );
});

test("trigger values are trimmed, default-filled and leave other keys behind", () => {
  assert.deepEqual(
    dynamicTriggerValues(["definition"], staticFields, {
      definition: "  beta ",
      username: "someone",
      cookie: "secret",
    }),
    { definition: "beta" },
  );
  assert.deepEqual(
    dynamicTriggerValues(["definition"], staticFields, { definition: " " }),
    { definition: "alpha" },
  );
  assert.deepEqual(dynamicTriggerValues(["missing"], staticFields, {}), {});
});

test("trigger signature ignores key order", () => {
  assert.equal(
    dynamicTriggerSignature({ a: "1", b: "2" }),
    dynamicTriggerSignature({ b: "2", a: "1" }),
  );
  assert.notEqual(
    dynamicTriggerSignature({ a: "1" }),
    dynamicTriggerSignature({ a: "2" }),
  );
});

test("reconcile drops values of fields that disappeared and seeds new ones", () => {
  const betaAnswer: DynamicConfigFields = {
    fields: [field({ key: "sort", fieldType: "SELECT", defaultValue: "new" })],
    hiddenFieldKeys: [],
  };
  const values = { definition: "beta", cookie: "typed", sort: "old" };
  assert.deepEqual(
    reconcileDynamicConfigValues(values, alphaAnswer, betaAnswer),
    {
      definition: "beta",
      sort: "old",
    },
  );

  assert.deepEqual(
    reconcileDynamicConfigValues({ definition: "alpha" }, null, alphaAnswer),
    { definition: "alpha", cookie: "", sort: "new" },
  );
});

test("reconcile keeps stored dynamic values on the first answer for an edit", () => {
  const stored = { definition: "alpha", cookie: "", sort: "old" };
  assert.equal(reconcileDynamicConfigValues(stored, null, alphaAnswer), stored);
});

test("reconcile clears every dynamic value when the plugin stops offering fields", () => {
  assert.deepEqual(
    reconcileDynamicConfigValues(
      { definition: "gamma", cookie: "x", sort: "new", other: "kept" },
      alphaAnswer,
      null,
    ),
    { definition: "gamma", other: "kept" },
  );
});

test("hidden static values are left out of what is saved", () => {
  const values = { definition: "alpha", username: "someone" };
  assert.deepEqual(omitHiddenConfigValues(values, alphaAnswer), {
    definition: "alpha",
  });
  assert.equal(omitHiddenConfigValues(values, null), values);
});

test("only the newest request epoch is current", () => {
  const epoch = createRequestEpoch();
  const first = epoch.next();
  const second = epoch.next();
  assert.equal(epoch.isCurrent(first), false);
  assert.equal(epoch.isCurrent(second), true);
});

test("reconcile never gives an informational field a value", () => {
  const withNote: DynamicConfigFields = {
    fields: [
      field({
        key: "info_cookie",
        fieldType: "INFO",
        defaultValue: "Sign in with a browser first.",
      }),
      field({ key: "cookie", defaultValue: "" }),
    ],
    hiddenFieldKeys: [],
  };

  const values = reconcileDynamicConfigValues({}, null, withNote);

  assert.equal("info_cookie" in values, false);
  assert.equal(values.cookie, "");
});
