import type { LocaleDictionary } from "./types";
import { DEFAULT_LANGUAGE, interpolate } from "./types";
import en from "./locales/en";
export { DEFAULT_LANGUAGE } from "./types";

// Interface copy is intentionally maintained in en.ts only.
// Other locale files are UI selector targets for future data-language
// behavior and currently alias to English for interface text.

export type LocaleCode =
  | "eng"
  | "spa"
  | "zho"
  | "hin"
  | "ara"
  | "fra"
  | "rus"
  | "por"
  | "jpn"
  | "deu";

export type LanguageOption = {
  code: LocaleCode;
  label: string;
};

type LocaleMap = Record<LocaleCode, LocaleDictionary>;

const LOCALE_ALIASES: Record<string, LocaleCode> = {
  en: "eng",
  es: "spa",
  zh: "zho",
  hi: "hin",
  ar: "ara",
  fr: "fra",
  ru: "rus",
  pt: "por",
  de: "deu",
  ja: "jpn",
};

const locales: LocaleMap = {
  eng: en,
  spa: en,
  zho: en,
  hin: en,
  ara: en,
  fra: en,
  rus: en,
  por: en,
  jpn: en,
  deu: en,
};

export const AVAILABLE_LANGUAGES: LanguageOption[] = [
  { code: "eng", label: "English" },
  { code: "zho", label: "Chinese" },
  { code: "spa", label: "Spanish" },
  { code: "hin", label: "Hindi" },
  { code: "ara", label: "Arabic" },
  { code: "fra", label: "French" },
  { code: "rus", label: "Russian" },
  { code: "por", label: "Portuguese" },
  { code: "jpn", label: "Japanese" },
  { code: "deu", label: "German" },
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
  return locales[key] ?? FALLBACK;
}

export function normalizeLocale(code?: string | null): LocaleCode {
  const normalized = code?.toLowerCase().trim();
  if (!normalized) {
    return DEFAULT_LANGUAGE;
  }
  const root = normalized.split("-")[0]!;
  if (root in locales) {
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
