import assert from "node:assert/strict";
import test from "node:test";

import {
  AUDIO_LANGUAGES,
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

test("audio language options speak the codes the backend stores", () => {
  const options = audioLanguageOptions(ORIGINAL_LABEL);
  const codes = new Set(options.map((option) => option.code));

  // The terminological half of ISO 639-2: what a saved requirement comes back
  // as, and therefore what an option has to be keyed by for its row to tick.
  for (const code of ["hye", "eus", "deu", "fra", "ces", "nld", "zho"]) {
    assert.equal(codes.has(code), true, `missing ${code}`);
  }
  // The bibliographic spellings the picker used to offer never match a stored
  // value, so offering them is what left an entry unremovable.
  for (const code of ["arm", "baq", "ger", "fre", "cze", "dut", "chi"]) {
    assert.equal(codes.has(code), false, `still offering ${code}`);
  }
});

test("audio language options never repeat a code", () => {
  const codes = AUDIO_LANGUAGES.map((language) => language.code);

  assert.equal(new Set(codes).size, codes.length);
});

test("audio language labels resolve both halves of ISO 639-2", () => {
  assert.equal(
    formatAudioLanguageLabels(["hye", "eus"], ORIGINAL_LABEL),
    "Armenian, Basque",
  );
  // Values written before the two code sets were reconciled still read.
  assert.equal(
    formatAudioLanguageLabels(["arm", "baq"], ORIGINAL_LABEL),
    "Armenian, Basque",
  );
});
