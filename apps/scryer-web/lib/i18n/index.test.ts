import assert from "node:assert/strict";
import test from "node:test";
import {
  getLocaleDictionary,
  isLocaleLoaded,
  loadLocaleDictionary,
  normalizeLocale,
  t,
} from "./index.ts";

test("normalizes supported locale aliases and falls back to English", () => {
  assert.equal(normalizeLocale("pt-BR"), "por");
  assert.equal(normalizeLocale("zh-CN"), "zho");
  assert.equal(normalizeLocale("unknown"), "eng");
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
  const locales = ["fra", "deu", "ita", "por", "kor", "zho", "jpn", "rus"];
  for (const locale of locales) {
    const dictionary = await loadLocaleDictionary(locale);
    assert.equal(typeof dictionary["label.language"], "string");
    assert.notEqual(dictionary["label.language"], "");
  }
});
