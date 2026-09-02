import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AVAILABLE_LANGUAGES,
  DEFAULT_LANGUAGE,
  getLanguageLabel,
  isLocaleLoaded,
  loadLocaleDictionary,
  normalizeLocale,
  t as translate,
} from "@/lib/i18n";
import type { LocaleCode } from "@/lib/i18n";
import { URL_PARAM_LANGUAGE } from "@/lib/constants/settings";
import { parseLanguageFromParam } from "@/lib/utils/routing";
import { toast } from "sonner";

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
  const onLanguageSet = options.onLanguageSet;
  const [queryLanguage] = useState(() => searchParams.get(URL_PARAM_LANGUAGE));
  const initialLanguage = (() => {
    const fromQuery = parseLanguageFromParam(queryLanguage);
    if (fromQuery && isLocaleLoaded(fromQuery)) {
      return fromQuery;
    }

    const stored = readStoredLanguageCode();
    return isLocaleSupported(stored) && isLocaleLoaded(stored) ? stored : DEFAULT_LANGUAGE;
  })();

  const languageMenuRef = useRef<HTMLDivElement>(null);
  const languageRequestRef = useRef(0);
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

  const requestLanguage = useCallback(
    (code: string, notify: boolean) => {
      const normalized = normalizeLocale(code);
      const requestId = ++languageRequestRef.current;
      setIsLanguageMenuOpen(false);

      void loadLocaleDictionary(normalized)
        .then(() => {
          if (requestId !== languageRequestRef.current) {
            return;
          }
          setUiLanguage(normalized);
          writeStoredLanguageCode(normalized);
          if (notify) {
            onLanguageSet?.(normalized, getLanguageLabel(normalized));
          }
        })
        .catch(() => {
          if (requestId !== languageRequestRef.current) {
            return;
          }
          toast.error(`Failed to load ${getLanguageLabel(normalized)} translations.`);
        });
    },
    [onLanguageSet],
  );

  const setLanguagePreference = useCallback(
    (code: string) => requestLanguage(code, true),
    [requestLanguage],
  );

  const setLanguageFallback = useCallback(() => {
    if (typeof window === "undefined") {
      return;
    }

    const stored = readStoredLanguageCode();
    if (stored === uiLanguage) {
      return;
    }
    requestLanguage(stored, false);
  }, [requestLanguage, uiLanguage]);

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
    if (!queryLang) {
      setLanguageFallback();
    }
  }, [searchParams, setLanguageFallback]);

  useEffect(() => {
    const queryLang = parseLanguageFromParam(searchParams.get(URL_PARAM_LANGUAGE));
    if (queryLang) {
      if (queryLang !== uiLanguage) {
        requestLanguage(queryLang, false);
      } else {
        writeStoredLanguageCode(queryLang);
      }
      return;
    }

    if (uiLanguage === DEFAULT_LANGUAGE) {
      writeStoredLanguageCode(DEFAULT_LANGUAGE);
    }
  }, [requestLanguage, searchParams, uiLanguage]);

  useEffect(() => {
    writeStoredLanguageCode(uiLanguage);
    document.documentElement.lang = uiLanguage;
  }, [uiLanguage]);

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
