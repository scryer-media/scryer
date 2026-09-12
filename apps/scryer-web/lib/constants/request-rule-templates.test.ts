import assert from "node:assert/strict";
import test from "node:test";

import requestInputContract from "../contracts/request-input-contract.json" with { type: "json" };
import en from "../i18n/locales/en.ts";
import ru from "../i18n/locales/ru.ts";
import {
  applyRequestersToSource,
  requestRuleNamesRequesters,
} from "../utils/request-rule-sets.ts";
import { REQUEST_RULE_TEMPLATES } from "./request-rule-templates.ts";

/// The five worked examples, byte-for-byte as `REQUEST_RULE_EXAMPLES` pins them
/// in `crates/scryer-rules/src/request.rs`. They are repeated here rather than
/// derived from the gallery so that editing the gallery cannot quietly edit the
/// expectation too: the API's own fixture test validates exactly these strings,
/// and a template that diverges from them is a template that will not save.
const PINNED_SOURCES: Record<string, string> = {
  "named-requesters-family-rated":
    "package rules\nimport rego.v1\n\nrequesters := {\"alice\", \"bob\", \"carol\"}\n\napprove if {\n\tinput.requester.username in requesters\n\tinput.facts.certification_rank <= 2\n}\n\ntags contains \"family\" if {\n\tinput.facts.certification_rank <= 1\n}\n",
  "short-lease-approval":
    "package rules\nimport rego.v1\n\napprove if {\n\tinput.requester.username == \"bob\"\n\tnot input.request.lease_forever\n\tinput.request.lease_days <= 14\n}\n",
  "low-resolution-approval":
    "package rules\nimport rego.v1\n\napprove if {\n\tinput.requester.username == \"alice\"\n\tinput.facts.quality_profile_max_resolution <= 720\n}\n",
  "deny-adult-content":
    "package rules\nimport rego.v1\n\ndeny if {\n\tinput.facts.is_adult\n}\n\nreasons contains \"adult_content\" if {\n\tinput.facts.is_adult\n}\n",
  "monthly-approval-quota":
    "package rules\nimport rego.v1\n\nmanual if {\n\tinput.facts.approved_last_30d >= 5\n}\n",
};

test("the gallery ships the five worked examples exactly once, in order", () => {
  assert.deepEqual(
    REQUEST_RULE_TEMPLATES.map((template) => template.id),
    [
      "named-requesters-family-rated",
      "short-lease-approval",
      "low-resolution-approval",
      "deny-adult-content",
      "monthly-approval-quota",
    ],
  );

  const names = REQUEST_RULE_TEMPLATES.map((template) => template.name);
  assert.deepEqual([...new Set(names)], names);

  const keys = REQUEST_RULE_TEMPLATES.flatMap((template) => [
    template.titleKey,
    template.descriptionKey,
  ]);
  assert.deepEqual([...new Set(keys)], keys);
});

test("every template's matcher is the pinned worked example, byte for byte", () => {
  for (const template of REQUEST_RULE_TEMPLATES) {
    assert.equal(
      template.regoSource,
      PINNED_SOURCES[template.id],
      `${template.id} has drifted from the matcher the API validates`,
    );
  }
});

test("every template carries a matcher the API can accept", () => {
  for (const template of REQUEST_RULE_TEMPLATES) {
    assert.ok(
      template.regoSource.startsWith("package rules\nimport rego.v1\n"),
      `${template.id} does not open with the pinned package and import lines`,
    );
    assert.ok(
      template.regoSource.endsWith("\n"),
      `${template.id} does not end in a newline`,
    );
    /// The API refuses a rule that can never vote, so every template has to
    /// define at least one of the four decision rules.
    assert.match(
      template.regoSource,
      /(^|\n)(approve|deny|manual|tags) /,
      `${template.id} defines no decision rule`,
    );
    /// Templates are pinned byte-for-byte, so nothing may re-indent them.
    assert.equal(
      template.regoSource.includes("\n    "),
      false,
      `${template.id} is indented with spaces rather than tabs`,
    );
  }
});

test("person-targeted templates are the ones that read the requester", () => {
  for (const template of REQUEST_RULE_TEMPLATES) {
    assert.equal(
      template.personTargeted === true,
      template.regoSource.includes("input.requester."),
      `${template.id} disagrees with its matcher about naming people`,
    );
    assert.equal(
      template.namesRequesters === true,
      requestRuleNamesRequesters(template.regoSource),
      `${template.id} disagrees about whether the user picker can write to it`,
    );
  }

  assert.deepEqual(
    REQUEST_RULE_TEMPLATES.filter((template) => template.personTargeted).map(
      (template) => template.id,
    ),
    [
      "named-requesters-family-rated",
      "short-lease-approval",
      "low-resolution-approval",
    ],
  );
});

test("the user picker rewrites both ways a template names people", () => {
  const set = REQUEST_RULE_TEMPLATES.find(
    (template) => template.id === "named-requesters-family-rated",
  )!;
  assert.equal(
    applyRequestersToSource(set.regoSource, ["dana"]),
    set.regoSource.replace(
      '{"alice", "bob", "carol"}',
      '{"dana"}',
    ),
  );

  const literal = REQUEST_RULE_TEMPLATES.find(
    (template) => template.id === "short-lease-approval",
  )!;
  assert.equal(
    applyRequestersToSource(literal.regoSource, ["dana"]),
    literal.regoSource.replace('== "bob"', '== "dana"'),
  );
  /// One name stays an equality; several become a set membership, which is the
  /// only honest way to say "any of these" with an equality operator.
  assert.equal(
    applyRequestersToSource(literal.regoSource, ["dana", "erin"]),
    literal.regoSource.replace('== "bob"', 'in {"dana", "erin"}'),
  );

  /// A matcher that names nobody has nowhere to write, and says so rather than
  /// leaving the placeholder in place.
  const contentOnly = REQUEST_RULE_TEMPLATES.find(
    (template) => template.id === "deny-adult-content",
  )!;
  assert.equal(applyRequestersToSource(contentOnly.regoSource, ["dana"]), null);
});

test("every template key resolves in the default locale and in Russian", () => {
  const keys = [
    "settings.requestTemplateGallery",
    "settings.requestTemplateGalleryDescription",
    "settings.requestTemplateApply",
    "settings.requestTemplatePersonTargetedBadge",
    ...REQUEST_RULE_TEMPLATES.flatMap((template) => [
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

test("every request contract key resolves in the default locale", () => {
  const contract = requestInputContract as {
    sections: Array<{
      titleKey: string;
      fields: Array<{ descKey: string }>;
    }>;
  };

  const missing: string[] = [];
  for (const section of contract.sections) {
    if (typeof en[section.titleKey] !== "string" || !en[section.titleKey]) {
      missing.push(`eng -> ${section.titleKey}`);
    }
    for (const field of section.fields) {
      if (typeof en[field.descKey] !== "string" || !en[field.descKey]) {
        missing.push(`eng -> ${field.descKey}`);
      }
    }
  }

  assert.deepEqual(missing, []);
});
