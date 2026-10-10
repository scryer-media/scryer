import assert from "node:assert/strict";
import test from "node:test";
import {
  recognizedLanguageCode,
  resolveUiLanguage,
  type UiLanguageSources,
} from "./ui-language.ts";

const NONE: UiLanguageSources = {
  query: null,
  urlOverride: null,
  profile: null,
  choice: null,
  cached: null,
  browser: null,
};

test("a URL language wins over every saved preference", () => {
  assert.equal(
    resolveUiLanguage({
      query: "fra",
      urlOverride: "ita",
      profile: "deu",
      choice: "kor",
      cached: "spa",
      browser: "ja-JP",
    }),
    "fra",
  );
});

test("an earlier URL language in the tab still wins over the profile", () => {
  assert.equal(
    resolveUiLanguage({ ...NONE, urlOverride: "ita", profile: "deu", cached: "spa" }),
    "ita",
  );
});

test("the profile language wins over the tab cache and the browser", () => {
  assert.equal(
    resolveUiLanguage({ ...NONE, profile: "deu", cached: "spa", browser: "ja-JP" }),
    "deu",
  );
  // A profile saved as English is honoured, not mistaken for an unknown code.
  assert.equal(
    resolveUiLanguage({ ...NONE, profile: "en-US", cached: "spa", browser: "ja-JP" }),
    "eng",
  );
});

test("without a profile language the language picked in this browser decides", () => {
  assert.equal(resolveUiLanguage({ ...NONE, choice: "eng", browser: "es-ES" }), "eng");
  assert.equal(
    resolveUiLanguage({ ...NONE, choice: "ita", cached: "spa", browser: "es-ES" }),
    "ita",
  );
});

test("a profile language wins over the language picked in this browser", () => {
  assert.equal(
    resolveUiLanguage({ ...NONE, profile: "deu", choice: "eng", browser: "es-ES" }),
    "deu",
  );
});

test("a URL language wins over the profile and the picked language", () => {
  const saved = { ...NONE, profile: "deu", choice: "eng", browser: "es-ES" };
  assert.equal(resolveUiLanguage({ ...saved, query: "fra" }), "fra");
  assert.equal(resolveUiLanguage({ ...saved, urlOverride: "ita" }), "ita");
});

test("without a profile language the tab cache, then the browser, decide", () => {
  assert.equal(resolveUiLanguage({ ...NONE, cached: "spa", browser: "ja-JP" }), "spa");
  assert.equal(resolveUiLanguage({ ...NONE, browser: "ja-JP" }), "jpn");
  assert.equal(resolveUiLanguage(NONE), "eng");
});

test("an unknown stored language falls through instead of forcing English", () => {
  assert.equal(
    resolveUiLanguage({ ...NONE, profile: "tlh", cached: "zz-QQ", browser: "nl-BE" }),
    "nld",
  );
  assert.equal(recognizedLanguageCode("tlh"), null);
  assert.equal(recognizedLanguageCode("  "), null);
  assert.equal(recognizedLanguageCode("zh-hant-hk"), "zh-HK");
  assert.equal(resolveUiLanguage({ ...NONE, browser: "tlh" }), "eng");
});
