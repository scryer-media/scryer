import type { LocaleDictionary } from "./types";
import { DEFAULT_LANGUAGE, interpolate } from "./types";
import en from "./locales/en";
export { DEFAULT_LANGUAGE } from "./types";

// Interface copy is maintained in English only for now.

export type LocaleCode = "eng";

export type LanguageOption = {
  code: LocaleCode;
  label: string;
};

type LocaleMap = Record<LocaleCode, LocaleDictionary>;

const LOCALE_ALIASES: Record<string, LocaleCode> = {
  en: "eng",
};

const locales: LocaleMap = {
  eng: en,
};

export const AVAILABLE_LANGUAGES: LanguageOption[] = [
  { code: "eng", label: "English" },
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
