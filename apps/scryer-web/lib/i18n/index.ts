import type { LocaleDictionary } from "./types";
import { DEFAULT_LANGUAGE, interpolate } from "./types";
import en from "./locales/en";
import es from "./locales/es";
import fr from "./locales/fr";
import de from "./locales/de";
import it from "./locales/it";
import pt_BR from "./locales/pt_BR";
import ko from "./locales/ko";
import zh_CN from "./locales/zh_CN";
import ja from "./locales/ja";
export { DEFAULT_LANGUAGE } from "./types";

export type LocaleCode = "eng" | "spa" | "fra" | "deu" | "ita" | "por" | "kor" | "zho" | "jpn";

export type LanguageOption = {
  code: LocaleCode;
  label: string;
};

type LocaleMap = Record<LocaleCode, LocaleDictionary>;

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
};

const locales: LocaleMap = {
  eng: en,
  spa: es,
  fra: fr,
  deu: de,
  ita: it,
  por: pt_BR,
  kor: ko,
  zho: zh_CN,
  jpn: ja,
};

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
