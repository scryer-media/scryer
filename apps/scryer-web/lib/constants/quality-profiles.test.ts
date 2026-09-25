import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import { SCORING_PERSONA_CHOICES } from "./quality-profiles.ts";

test("scoring persona choices cover every persona id with an English label", () => {
  assert.deepEqual(
    SCORING_PERSONA_CHOICES.map((choice) => choice.value),
    ["BALANCED", "AUDIOPHILE", "EFFICIENT", "COMPATIBLE"],
  );
  for (const choice of SCORING_PERSONA_CHOICES) {
    assert.ok(en[choice.labelKey], `missing English label for ${choice.labelKey}`);
  }
});
