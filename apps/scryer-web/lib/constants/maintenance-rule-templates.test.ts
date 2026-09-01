import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import ru from "../i18n/locales/ru.ts";
import type { MaintenanceActionKind } from "../types/maintenance-rule-sets.ts";
import { actionKindLabelKey } from "../utils/maintenance-rule-sets.ts";
import {
  MAINTENANCE_RULE_TEMPLATES,
  maintenanceTemplateFacetLabelKey,
} from "./maintenance-rule-templates.ts";

/// The wire names the web types define. A template may only name one of these:
/// anything else would build an action payload the API rejects.
const ACTION_KINDS: MaintenanceActionKind[] = [
  "DO_NOTHING",
  "UNMONITOR_SCOPE_KEEP_FILES",
  "DELETE_TITLE_AND_FILES",
  "UNMONITOR_TITLE_DELETE_ALL_FILES",
  "UNMONITOR_SHOW_DELETE_EXISTING_FILES",
  "UNMONITOR_SCOPE_DELETE_FILES",
  "UNMONITOR_SEASON_DELETE_FILES_THEN_DELETE_SHOW_IF_EMPTY",
  "UNMONITOR_SEASON_THEN_UNMONITOR_SHOW_IF_EMPTY",
  "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
];

test("the gallery ships every starter template exactly once", () => {
  assert.equal(MAINTENANCE_RULE_TEMPLATES.length, 10);

  const ids = MAINTENANCE_RULE_TEMPLATES.map((template) => template.id);
  assert.deepEqual([...new Set(ids)], ids);

  const names = MAINTENANCE_RULE_TEMPLATES.map((template) => template.name);
  assert.deepEqual([...new Set(names)], names);

  const keys = MAINTENANCE_RULE_TEMPLATES.flatMap((template) => [
    template.titleKey,
    template.descriptionKey,
  ]);
  assert.deepEqual([...new Set(keys)], keys);
});

test("every template carries a matcher the API can accept", () => {
  for (const template of MAINTENANCE_RULE_TEMPLATES) {
    assert.ok(
      template.regoSource.trim().length > 0,
      `${template.id} has an empty matcher`,
    );
    assert.ok(
      template.regoSource.endsWith("\n"),
      `${template.id} does not end in a newline`,
    );
    assert.ok(
      template.regoSource.startsWith("package rules\nimport rego.v1\n"),
      `${template.id} does not open with the pinned package and import lines`,
    );
    assert.match(
      template.regoSource,
      /(^|\n)match if /,
      `${template.id} defines no match rule`,
    );
    /// Templates are pinned byte-for-byte against the matcher fixtures the API
    /// validates, so nothing may re-indent them into spaces.
    assert.equal(
      template.regoSource.includes("\n    "),
      false,
      `${template.id} is indented with spaces rather than tabs`,
    );
  }
});

test("every template names a real action kind and a non-negative grace period", () => {
  for (const template of MAINTENANCE_RULE_TEMPLATES) {
    assert.ok(
      ACTION_KINDS.includes(template.actionKind),
      `${template.id} names an unknown action kind`,
    );
    assert.ok(
      actionKindLabelKey(template.actionKind),
      `${template.id} names an action kind with no label`,
    );
    assert.ok(
      Number.isInteger(template.graceDays) && template.graceDays >= 0,
      `${template.id} has an unusable grace period`,
    );
    assert.ok(
      template.subjectFacets.length > 0,
      `${template.id} declares no subject facet`,
    );
  }
});

test("a template never picks the quality profile the operator has to choose", () => {
  for (const template of MAINTENANCE_RULE_TEMPLATES) {
    const changesProfile =
      template.actionKind === "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED";
    assert.equal(
      template.requiresTargetQualityProfile === true,
      changesProfile,
      `${template.id} disagrees with its action about needing a target profile`,
    );
    assert.equal(
      template.targetQualityProfileId,
      undefined,
      `${template.id} pins a quality profile that only the operator can pick`,
    );
  }
});

test("every file-deleting template is marked and says so in its copy", () => {
  for (const template of MAINTENANCE_RULE_TEMPLATES) {
    const deletesFiles = template.actionKind === "DELETE_TITLE_AND_FILES";
    assert.equal(
      template.destructive === true,
      deletesFiles,
      `${template.id} disagrees with its action about deleting files`,
    );
    if (deletesFiles) {
      assert.match(
        en[template.descriptionKey],
        /^Caution: /,
        `${template.id} does not open its description with a caution`,
      );
    }
  }

  assert.deepEqual(
    MAINTENANCE_RULE_TEMPLATES.filter((template) => template.destructive).map(
      (template) => template.id,
    ),
    [
      "library-aging",
      "requested-media-expiry",
      "departed-requester",
      "watched-by-every-requester",
    ],
  );
});

test("every template key resolves in the default locale and in Russian", () => {
  const facets = new Set(
    MAINTENANCE_RULE_TEMPLATES.flatMap((template) => template.subjectFacets),
  );
  const keys = [
    "settings.maintenanceTemplateGallery",
    "settings.maintenanceTemplateGalleryDescription",
    "settings.maintenanceTemplateApply",
    "settings.maintenanceTemplateGraceBadge",
    "settings.maintenanceTemplateNoGraceBadge",
    "settings.maintenanceTemplateDestructiveBadge",
    "settings.maintenanceTemplateNeedsProfileBadge",
    ...[...facets].map(maintenanceTemplateFacetLabelKey),
    ...MAINTENANCE_RULE_TEMPLATES.flatMap((template) => [
      template.titleKey,
      template.descriptionKey,
    ]),
  ];

  const missing: string[] = [];
  for (const key of keys) {
    if (typeof en[key] !== "string" || en[key].length === 0) {
      missing.push(`eng -> ${key}`);
    }
    if (typeof ru[key] !== "string" || ru[key].length === 0) {
      missing.push(`rus -> ${key}`);
    }
  }

  assert.deepEqual(missing, []);
});

test("the grace badge interpolates the day count", () => {
  assert.match(en["settings.maintenanceTemplateGraceBadge"], /\{\{count\}\}/);
  assert.match(ru["settings.maintenanceTemplateGraceBadge"], /\{\{count\}\}/);
});
