import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AVAILABLE_LANGUAGES,
  DEFAULT_LANGUAGE,
  LocaleCode,
  getLanguageLabel,
  normalizeLocale,
  t as translate,
} from "@/lib/i18n";
import { URL_PARAM_LANGUAGE } from "@/lib/constants/settings";
import { parseLanguageFromParam } from "@/lib/utils/routing";

export const UI_LANGUAGE_STORAGE_KEY = "scryer.ui.language";

export function isLocaleSupported(code: string): code is LocaleCode {
  const normalized = normalizeLocale(code);
  return AVAILABLE_LANGUAGES.some((language) => language.code === normalized);
}

export function readStoredLanguageCode(): LocaleCode {
  if (typeof window === "undefined") {
    return DEFAULT_LANGUAGE;
  }

  const stored = window.sessionStorage.getItem(UI_LANGUAGE_STORAGE_KEY);
  if (!stored) {
    const browserLanguage = navigator.language.split("-")[0] ?? DEFAULT_LANGUAGE;
    return normalizeLocale(browserLanguage);
  }

  return normalizeLocale(stored);
}

export function writeStoredLanguageCode(code: string) {
  if (typeof window === "undefined") {
    return;
  }

  const normalized = normalizeLocale(code);
  window.sessionStorage.setItem(UI_LANGUAGE_STORAGE_KEY, normalized);
}

type UseLanguageOptions = {
  onLanguageSet?: (code: LocaleCode, label: string) => void;
};

export function useLanguage(searchParams: URLSearchParams, options: UseLanguageOptions = {}) {
  const [queryLanguage] = useState(() => searchParams.get(URL_PARAM_LANGUAGE));
  const initialLanguage = (() => {
    const fromQuery = parseLanguageFromParam(queryLanguage);
    if (fromQuery) {
      return fromQuery;
    }

    const stored = readStoredLanguageCode();
    return isLocaleSupported(stored) ? stored : DEFAULT_LANGUAGE;
  })();

  const languageMenuRef = useRef<HTMLDivElement>(null);
  const [uiLanguage, setUiLanguage] = useState<LocaleCode>(initialLanguage);
  const [isLanguageMenuOpen, setIsLanguageMenuOpen] = useState(false);
  const t = useCallback(
    (key: string, values?: Record<string, string | number | boolean | null | undefined>) =>
      translate(key, uiLanguage, values),
    [uiLanguage],
  );

  const selectedLanguage = useMemo(
    () => AVAILABLE_LANGUAGES.find((language) => language.code === uiLanguage) ?? AVAILABLE_LANGUAGES[0],
    [uiLanguage],
  );

  const setLanguagePreference = useCallback((code: string) => {
    const normalized = normalizeLocale(code);
    setUiLanguage(normalized);
    writeStoredLanguageCode(normalized);
    setIsLanguageMenuOpen(false);
    options.onLanguageSet?.(normalized, getLanguageLabel(normalized));
  }, [options]);

  const setLanguageFallback = useCallback(() => {
    if (typeof window === "undefined") {
      return;
    }

    const stored = readStoredLanguageCode();
    if (stored === uiLanguage) {
      return;
    }
    setUiLanguage(stored);
  }, [uiLanguage]);

  useEffect(() => {
    const onDocumentPointerDown = (event: PointerEvent) => {
      if (!languageMenuRef.current?.contains(event.target as Node)) {
        setIsLanguageMenuOpen(false);
      }
    };
    const onDocumentKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setIsLanguageMenuOpen(false);
      }
    };

    document.addEventListener("pointerdown", onDocumentPointerDown);
    document.addEventListener("keydown", onDocumentKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onDocumentPointerDown);
      document.removeEventListener("keydown", onDocumentKeyDown);
    };
  }, []);

  useEffect(() => {
    const queryLang = parseLanguageFromParam(searchParams.get(URL_PARAM_LANGUAGE));
    if (queryLang) {
      writeStoredLanguageCode(queryLang);
      if (queryLang !== uiLanguage) {
        setUiLanguage(queryLang);
      }
      return;
    }

    if (uiLanguage === DEFAULT_LANGUAGE) {
      writeStoredLanguageCode(DEFAULT_LANGUAGE);
    }
  }, [searchParams, uiLanguage]);

  useEffect(() => {
    writeStoredLanguageCode(uiLanguage);
    document.documentElement.lang = uiLanguage;
  }, [uiLanguage]);

  useEffect(() => {
    setLanguageFallback();
  }, [setLanguageFallback]);

  return {
    uiLanguage,
    isLanguageMenuOpen,
    setIsLanguageMenuOpen,
    languageMenuRef,
    setLanguagePreference,
    selectedLanguage,
    t,
    getLanguageLabel,
  };
}
