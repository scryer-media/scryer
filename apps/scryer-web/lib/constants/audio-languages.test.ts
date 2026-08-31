import assert from "node:assert/strict";
import test from "node:test";

import {
  audioLanguageOptions,
  formatAudioLanguageLabels,
} from "./audio-languages.ts";
import { SUBTITLE_LANGUAGES } from "./subtitle-languages.ts";

const ORIGINAL_LABEL = "Original language (per title)";

test("audio language options pin Original before concrete languages", () => {
  const options = audioLanguageOptions(ORIGINAL_LABEL);

  assert.equal(options[0]?.code, "original");
  assert.equal(options[0]?.name, ORIGINAL_LABEL);
  assert.ok(options.some((option) => option.code === "eng"));
  assert.equal(
    SUBTITLE_LANGUAGES.some((option) => option.code === "original"),
    false,
  );
});

test("audio language labels render Original and concrete codes readably", () => {
  assert.equal(
    formatAudioLanguageLabels(["original", "jpn"], ORIGINAL_LABEL),
    `${ORIGINAL_LABEL}, Japanese`,
  );
  assert.equal(formatAudioLanguageLabels(["ORIGINAL"], ORIGINAL_LABEL), ORIGINAL_LABEL);
});
