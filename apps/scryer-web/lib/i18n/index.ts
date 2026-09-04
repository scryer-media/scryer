import type { LocaleDictionary } from "./types.ts";
import { DEFAULT_LANGUAGE, interpolate } from "./types.ts";

import en from "./locales/en.ts";
export { DEFAULT_LANGUAGE } from "./types.ts";

export type LocaleCode =
  | "eng"
  | "spa"
  | "fra"
  | "deu"
  | "ita"
  | "por"
  | "kor"
  | "zho"
  | "jpn"
  | "rus";

export type LanguageOption = {
  code: LocaleCode;
  label: string;
};

type DeferredLocaleCode = Exclude<LocaleCode, "eng">;
type LocaleModule = { default: LocaleDictionary };
type LocaleLoader = () => Promise<LocaleModule>;

const LOCALE_ALIASES: Record<string, LocaleCode> = {
  en: "eng",
  es: "spa",
  fr: "fra",
  de: "deu",
  it: "ita",
  pt: "por",
  "pt-br": "por",
  ko: "kor",
  zh: "zho",
  "zh-cn": "zho",
  ja: "jpn",

  ru: "rus",
  "ru-ru": "rus",
};

const localeLoaders = new Map<DeferredLocaleCode, LocaleLoader>([
  ["spa", () => import("./locales/es.ts")],
  ["fra", () => import("./locales/fr.ts")],
  ["deu", () => import("./locales/de.ts")],
  ["ita", () => import("./locales/it.ts")],
  ["por", () => import("./locales/pt_BR.ts")],
  ["kor", () => import("./locales/ko.ts")],
  ["zho", () => import("./locales/zh_CN.ts")],
  ["jpn", () => import("./locales/ja.ts")],
  ["rus", () => import("./locales/ru.ts")],
]);

const locales = new Map<LocaleCode, LocaleDictionary>([["eng", en]]);
const localeLoads = new Map<LocaleCode, Promise<LocaleDictionary>>();

export const AVAILABLE_LANGUAGES: LanguageOption[] = [
  { code: "eng", label: "English" },
  { code: "fra", label: "Fran\u00e7ais" },
  { code: "deu", label: "Deutsch" },
  { code: "spa", label: "Español" },
  { code: "ita", label: "Italiano" },
  { code: "por", label: "Português (Brasil)" },
  { code: "kor", label: "한국어" },
  { code: "zho", label: "简体中文" },
  { code: "jpn", label: "日本語" },
  { code: "rus", label: "Русский" },
];

export function getLanguageLabel(code: string): string {
  const normalized = normalizeLocale(code);
  return AVAILABLE_LANGUAGES.find((language) => language.code === normalized)?.label ?? normalized;
}

const FALLBACK: LocaleDictionary = en;

export function getLocaleDictionary(code: string | null | undefined): LocaleDictionary {
  if (!code) {
    return FALLBACK;
  }
  const key = normalizeLocale(code);
  return locales.get(key) ?? FALLBACK;
}

export function isLocaleLoaded(code: string | null | undefined): boolean {
  if (!code) {
    return true;
  }
  return locales.has(normalizeLocale(code));
}

export function loadLocaleDictionary(
  code: string | null | undefined,
): Promise<LocaleDictionary> {
  const key = normalizeLocale(code);
  const loaded = locales.get(key);
  if (loaded) {
    return Promise.resolve(loaded);
  }

  const pending = localeLoads.get(key);
  if (pending) {
    return pending;
  }

  if (key === "eng") {
    return Promise.resolve(FALLBACK);
  }
  const loader = localeLoaders.get(key);
  if (!loader) {
    return Promise.reject(new Error(`No locale loader configured for ${key}.`));
  }
  const load = loader()
    .then(({ default: dictionary }) => {
      locales.set(key, dictionary);
      localeLoads.delete(key);
      return dictionary;
    })
    .catch((error: unknown) => {
      localeLoads.delete(key);
      throw error;
    });
  localeLoads.set(key, load);
  return load;
}

export function normalizeLocale(code?: string | null): LocaleCode {
  const normalized = code?.toLowerCase().trim();
  if (!normalized) {
    return DEFAULT_LANGUAGE;
  }
  const root = normalized.split("-")[0]!;
  if (AVAILABLE_LANGUAGES.some(({ code: localeCode }) => localeCode === root)) {
    return root as LocaleCode;
  }
  const alias = LOCALE_ALIASES[root];
  if (alias) {
    return alias;
  }
  return DEFAULT_LANGUAGE;
}

export function t(key: string, code: string, values?: Record<string, string | number | boolean | null | undefined>): string {
  const locale = getLocaleDictionary(code);
  const template = locale[key] ?? FALLBACK[key] ?? key;
  return interpolate(template, values);
}
