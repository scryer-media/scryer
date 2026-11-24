import assert from "node:assert/strict";
import test from "node:test";

import {
  approvalAudioLanguageOverrides,
  catalogAudioLanguageDraftFrom,
  catalogAudioLanguagesForSubmit,
  inheritedAudioLanguagesLabel,
  INHERIT_CATALOG_AUDIO_LANGUAGES,
} from "./catalog-audio-languages.ts";

test("inheriting sends neither audio field", () => {
  assert.deepEqual(
    catalogAudioLanguagesForSubmit(INHERIT_CATALOG_AUDIO_LANGUAGES),
    {},
  );
});

test("requiring none sends an empty list, which differs from inheriting", () => {
  assert.deepEqual(
    catalogAudioLanguagesForSubmit({
      ...INHERIT_CATALOG_AUDIO_LANGUAGES,
      requiredMode: "none",
      required: ["eng"],
    }),
    { required: [] },
  );
});

test("chosen languages are sent in order, and an empty choice inherits", () => {
  assert.deepEqual(
    catalogAudioLanguagesForSubmit({
      requiredMode: "choose",
      required: ["jpn"],
      preferredMode: "choose",
      preferred: ["original", "eng"],
    }),
    { required: ["jpn"], preferred: ["original", "eng"] },
  );
  assert.deepEqual(
    catalogAudioLanguagesForSubmit({
      requiredMode: "choose",
      required: [],
      preferredMode: "choose",
      preferred: [],
    }),
    {},
  );
});

test("the inherit label names inherited languages only when they were read", () => {
  const format = (codes: string[]) => codes.join(" + ");
  assert.equal(
    inheritedAudioLanguagesLabel("Inherit", null, format, "None"),
    "Inherit",
  );
  assert.equal(
    inheritedAudioLanguagesLabel("Inherit", [], format, "None"),
    "Inherit — None",
  );
  assert.equal(
    inheritedAudioLanguagesLabel("Inherit", ["eng", "deu"], format, "None"),
    "Inherit — eng + deu",
  );
});

test("stored languages read back as the choices that send them", () => {
  for (const [required, preferred] of [
    [null, null],
    [[], null],
    [["jpn"], ["original", "eng"]],
  ] as Array<[string[] | null, string[] | null]>) {
    const draft = catalogAudioLanguageDraftFrom(required, preferred);
    const sent = catalogAudioLanguagesForSubmit(draft);
    assert.deepEqual(sent.required ?? null, required);
    assert.deepEqual(sent.preferred ?? null, preferred);
  }
});

test("an approver who leaves the requested languages alone sends neither field", () => {
  const requested = { required: ["en"], preferred: ["fr", "de"] };
  assert.deepEqual(
    approvalAudioLanguageOverrides(
      requested,
      catalogAudioLanguageDraftFrom(requested.required, requested.preferred),
    ),
    {},
  );
  const inherited = { required: null, preferred: null };
  assert.deepEqual(
    approvalAudioLanguageOverrides(
      inherited,
      catalogAudioLanguageDraftFrom(null, null),
    ),
    {},
  );
  const requiredNone = { required: [], preferred: null };
  assert.deepEqual(
    approvalAudioLanguageOverrides(
      requiredNone,
      catalogAudioLanguageDraftFrom([], null),
    ),
    {},
  );
});

test("an approver who switches a requested field to inherit sends null", () => {
  const requested = { required: ["en"], preferred: ["fr"] };
  assert.deepEqual(
    approvalAudioLanguageOverrides(requested, INHERIT_CATALOG_AUDIO_LANGUAGES),
    { requiredAudioLanguages: null, preferredAudioLanguages: null },
  );
  // Choosing with nothing ticked is the same as inheriting.
  assert.deepEqual(
    approvalAudioLanguageOverrides(requested, {
      requiredMode: "choose",
      required: [],
      preferredMode: "choose",
      preferred: [],
    }),
    { requiredAudioLanguages: null, preferredAudioLanguages: null },
  );
});

test("an approver can require none or choose their own languages", () => {
  const requested = { required: null, preferred: null };
  assert.deepEqual(
    approvalAudioLanguageOverrides(requested, {
      requiredMode: "none",
      required: [],
      preferredMode: "inherit",
      preferred: [],
    }),
    { requiredAudioLanguages: [] },
  );
  assert.deepEqual(
    approvalAudioLanguageOverrides(
      { required: ["en"], preferred: ["fr", "de"] },
      {
        requiredMode: "choose",
        required: ["ja"],
        preferredMode: "choose",
        preferred: ["de", "fr"],
      },
    ),
    { requiredAudioLanguages: ["ja"], preferredAudioLanguages: ["de", "fr"] },
  );
});
