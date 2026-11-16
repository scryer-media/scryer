import assert from "node:assert/strict";
import test from "node:test";

import cs from "../i18n/locales/cs.ts";
import en from "../i18n/locales/en.ts";
import ru from "../i18n/locales/ru.ts";
import zhHK from "../i18n/locales/zh_HK.ts";
import zhTW from "../i18n/locales/zh_TW.ts";
import { LIFECYCLE_SERVED_PAYLOAD } from "../utils/rule-builder-contract-fixture.ts";
import { templateRuleOutputs } from "../utils/rule-output-vocabulary.ts";
import {
  LIFECYCLE_RULE_TEMPLATES,
  lifecycleTemplateFacetLabelKey,
} from "./lifecycle-rule-templates.ts";

const DESCRIPTORS = LIFECYCLE_SERVED_PAYLOAD.outputs;
const templateActionHeads = (source: string) => templateRuleOutputs(source, DESCRIPTORS);

test("the gallery ships every starter template exactly once", () => {
  assert.equal(LIFECYCLE_RULE_TEMPLATES.length, 17);

  const ids = LIFECYCLE_RULE_TEMPLATES.map((template) => template.id);
  assert.deepEqual([...new Set(ids)], ids);

  const names = LIFECYCLE_RULE_TEMPLATES.map((template) => template.name);
  assert.deepEqual([...new Set(names)], names);

  const keys = LIFECYCLE_RULE_TEMPLATES.flatMap((template) => [
    template.titleKey,
    template.descriptionKey,
  ]);
  assert.deepEqual([...new Set(keys)], keys);
});

test("every template carries a matcher the API can accept", () => {
  for (const template of LIFECYCLE_RULE_TEMPLATES) {
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
    assert.doesNotMatch(
      template.regoSource,
      /(^|\n)match\b/,
      `${template.id} still defines the retired match rule`,
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

test("every template declares at least one served action and a non-negative grace period", () => {
  const labelled: [string, Record<string, string>][] = [
    ["en", en], ["cs", cs], ["ru", ru], ["zh_HK", zhHK], ["zh_TW", zhTW],
  ];
  for (const template of LIFECYCLE_RULE_TEMPLATES) {
    const heads = templateActionHeads(template.regoSource);
    assert.ok(heads.length > 0, `${template.id} declares no action`);
    for (const head of heads) {
      const descriptor = DESCRIPTORS.find((entry) => entry.id === head);
      assert.ok(descriptor, `${template.id} declares ${head}, which is not served`);
      for (const [name, strings] of labelled)
        assert.ok(
          strings[descriptor.labelKey],
          `${template.id} declares ${head}, which has no ${name} label`,
        );
    }
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

/// Heads that remove media, at every subject.
const FILE_DELETING_HEADS = new Set([
  "DELETE_FILES",
  "DELETE_TITLE",
  "DELETE_SEASON_FILES",
  "DELETE_EPISODE_FILES",
]);

test("each template keeps the action it was written for", () => {
  const headsById = Object.fromEntries(
    LIFECYCLE_RULE_TEMPLATES.map((template) => [
      template.id,
      templateActionHeads(template.regoSource),
    ]),
  );
  assert.deepEqual(headsById, {
    "dead-wanted-entries": ["UNMONITOR"],
    "library-aging": ["DELETE_TITLE"],
    "added-over-180-days-ago": ["UNMONITOR"],
    "oversized-releases": ["SET_QUALITY_PROFILE", "SEARCH"],
    "four-k-purge": ["SET_QUALITY_PROFILE", "SEARCH"],
    "requested-media-expiry": ["DELETE_TITLE"],
    "departed-requester": ["DELETE_TITLE"],
    "system-added-cleanup": ["UNMONITOR"],
    "watched-by-every-requester": ["DELETE_TITLE"],
    "expired-request-leases": ["DELETE_TITLE"],
    "tagged-for-removal": ["DELETE_TITLE"],
    "flag-for-review": ["ADD_TAGS"],
    "no-quality-profile": ["ADD_TAGS"],
    "keep-newest-three-downloaded-episodes": ["DELETE_EPISODE_FILES"],
    "keep-newest-two-downloaded-seasons": ["DELETE_SEASON_FILES"],
    "left-every-public-list": ["UNMONITOR"],
    "list-added-never-watched": ["ADD_TAGS"],
  });
});

test("a template that sets a quality profile is marked as needing the operator's own", () => {
  for (const template of LIFECYCLE_RULE_TEMPLATES) {
    const setsProfile = templateActionHeads(template.regoSource).includes(
      "SET_QUALITY_PROFILE",
    );
    assert.equal(
      template.requiresTargetQualityProfile === true,
      setsProfile,
      `${template.id} disagrees with its source about needing a target profile`,
    );
  }
});

test("every file-deleting template is marked and says so in its copy", () => {
  for (const template of LIFECYCLE_RULE_TEMPLATES) {
    const heads = templateActionHeads(template.regoSource);
    const deletesFiles = heads.some((head) => FILE_DELETING_HEADS.has(head));
    assert.equal(
      template.destructive === true,
      deletesFiles,
      `${template.id} disagrees with its source about deleting files`,
    );
    // The server refuses a deleting rule without a per-run maximum.
    assert.equal(
      template.maxTitlesPerRun !== undefined,
      deletesFiles,
      `${template.id} disagrees with its source about needing a per-run maximum`,
    );
    if (deletesFiles) {
      assert.match(
        en[template.descriptionKey],
        /^Caution: /,
        `${template.id} does not open its description with a caution`,
      );
    }
  }
});

test("retention examples use the rank for their exact scope", () => {
  const episodes = LIFECYCLE_RULE_TEMPLATES.find(
    (template) => template.id === "keep-newest-three-downloaded-episodes",
  );
  const seasons = LIFECYCLE_RULE_TEMPLATES.find(
    (template) => template.id === "keep-newest-two-downloaded-seasons",
  );
  // The head names the subject the action acts on, and the condition reads
  // that subject's own facts.
  assert.match(episodes?.regoSource ?? "", /\bdelete_episode_files if\b/);
  assert.match(
    episodes?.regoSource ?? "",
    /input\.episode\.facts\.episode_position_by_air_date > 3/,
  );
  assert.match(seasons?.regoSource ?? "", /\bdelete_season_files if\b/);
  assert.match(
    seasons?.regoSource ?? "",
    /input\.season\.facts\.season_position_by_air_date > 2/,
  );
});

test("every template key resolves in the default locale and in Russian", () => {
  const facets = new Set(
    LIFECYCLE_RULE_TEMPLATES.flatMap((template) => template.subjectFacets),
  );
  const keys = [
    "settings.lifecycleTemplateGallery",
    "settings.lifecycleTemplateApply",
    "settings.lifecycleTemplateGraceBadge",
    "settings.lifecycleTemplateNoGraceBadge",
    "settings.lifecycleTemplateNeedsProfileBadge",
    ...[...facets].map(lifecycleTemplateFacetLabelKey),
    ...LIFECYCLE_RULE_TEMPLATES.flatMap((template) => [
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
  assert.match(en["settings.lifecycleTemplateGraceBadge"], /\{\{count\}\}/);
  assert.match(ru["settings.lifecycleTemplateGraceBadge"], /\{\{count\}\}/);
});
