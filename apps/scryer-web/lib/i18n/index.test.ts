import assert from "node:assert/strict";
import test from "node:test";
import {
  METADATA_LANGUAGES,
  getLocaleDictionary,
  isLocaleLoaded,
  loadLocaleDictionary,
  normalizeLocale,
  metadataLanguageForUi,
  t,
} from "./index.ts";

test("normalizes supported locale aliases and falls back to English", () => {
  assert.equal(normalizeLocale("pt-BR"), "por");
  assert.equal(normalizeLocale("zh-CN"), "zho");
  assert.equal(normalizeLocale("ZH_hk"), "zh-HK");
  assert.equal(normalizeLocale("zh-TW"), "zh-TW");
  assert.equal(normalizeLocale("zh-Hant-HK"), "zh-HK");
  assert.equal(normalizeLocale("zh-Hant"), "zh-TW");
  assert.equal(normalizeLocale("zh-Hans"), "zho");
  assert.equal(normalizeLocale("nl-NL"), "nld");
  assert.equal(normalizeLocale("nl-BE"), "nld");
  assert.equal(normalizeLocale("unknown"), "eng");
});

test("offers Dutch for metadata hydration", () => {
  assert.equal(METADATA_LANGUAGES.some(({ code }) => code === "nld"), true);
});

test("keeps regional UI languages separate from generic Chinese metadata", () => {
  assert.equal(metadataLanguageForUi("zh-HK"), "zho");
  assert.equal(metadataLanguageForUi("zh-TW"), "zho");
  assert.equal(metadataLanguageForUi("eng"), "eng");
  assert.equal(metadataLanguageForUi("EN_us"), "eng");
  assert.equal(metadataLanguageForUi("pt-BR"), "por");
  assert.equal(metadataLanguageForUi("unknown"), "eng");
  assert.deepEqual(METADATA_LANGUAGES.filter(({ code }) => code.startsWith("zh")), [
    { code: "zho", label: "中文" },
  ]);
});

test("keeps English synchronously available", () => {
  assert.equal(isLocaleLoaded("eng"), true);
  assert.equal(getLocaleDictionary("eng")["label.language"], "Language");
  assert.equal(t("label.language", "eng"), "Language");
});

test("loads and caches a deferred locale atomically", async () => {
  assert.equal(isLocaleLoaded("spa"), false);
  assert.equal(t("label.language", "spa"), "Language");

  const firstLoad = loadLocaleDictionary("spa");
  const secondLoad = loadLocaleDictionary("spa");
  assert.equal(firstLoad, secondLoad);

  const dictionary = await firstLoad;
  assert.equal(dictionary["label.language"], "Idioma");
  assert.equal(isLocaleLoaded("spa"), true);
  assert.equal(getLocaleDictionary("spa"), dictionary);
  assert.equal(t("label.language", "spa"), "Idioma");
});

test("every deferred locale has a valid loader", async () => {
  const locales = ["fra", "deu", "ita", "por", "kor", "zho", "zh-HK", "zh-TW", "jpn", "rus", "nld"];
  for (const locale of locales) {
    const dictionary = await loadLocaleDictionary(locale);
    assert.equal(typeof dictionary["label.language"], "string");
    assert.notEqual(dictionary["label.language"], "");
  }
});
