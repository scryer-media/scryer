import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useClient } from "urql";
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
import { recognizedLanguageCode, resolveUiLanguage } from "@/lib/i18n/ui-language";
import { URL_PARAM_LANGUAGE } from "@/lib/constants/settings";
import {
  isSignedOutLoginSurface,
  uiSettingsInputFromSettings,
  useUiSettings,
} from "@/lib/context/ui-settings-context";
import { setMyUiSettingsMutation } from "@/lib/graphql/mutations";
import type { SetMyUiSettingsInput, UiSettings } from "@/lib/types/settings";
import { parseLanguageFromParam } from "@/lib/utils/routing";
import { toast } from "sonner";

export const UI_LANGUAGE_STORAGE_KEY = "scryer.ui.language";
/** A `?lang=` choice that keeps applying for the rest of the tab. */
const UI_LANGUAGE_URL_OVERRIDE_KEY = "scryer.ui.language.url";
/**
 * The last profile language this browser saw, so a new tab paints in it
 * before the profile has loaded. The profile stays authoritative.
 */
const UI_LANGUAGE_PROFILE_HINT_KEY = "scryer.ui.language.profile";

type StorageArea = "session" | "local";

function readStorageItem(area: StorageArea, key: string): string | null {
  try {
    const storage = area === "session" ? window.sessionStorage : window.localStorage;
    return storage.getItem(key);
  } catch {
    return null;
  }
}

function writeStorageItem(area: StorageArea, key: string, value: string | null) {
  try {
    const storage = area === "session" ? window.sessionStorage : window.localStorage;
    if (value === null) {
      storage.removeItem(key);
    } else {
      storage.setItem(key, value);
    }
  } catch {
    // Storage only speeds up the first paint; the profile remains authoritative.
  }
}

export function isLocaleSupported(code: string): code is LocaleCode {
  const normalized = normalizeLocale(code);
  return AVAILABLE_LANGUAGES.some((language) => language.code === normalized);
}

/**
 * The language to show without a `?lang=` on the URL. `profileLanguage` is the
 * loaded profile value (null when the profile has none); leave it undefined
 * while the profile is loading to use the last value this browser saw. The
 * signed-out login page has no profile and keeps the tab and browser choice.
 */
export function readStoredLanguageCode(profileLanguage?: string | null): LocaleCode {
  if (typeof window === "undefined") {
    return DEFAULT_LANGUAGE;
  }

  let profile: string | null;
  if (profileLanguage !== undefined) {
    profile = profileLanguage;
  } else if (isSignedOutLoginSurface()) {
    profile = null;
  } else {
    profile = readStorageItem("local", UI_LANGUAGE_PROFILE_HINT_KEY);
  }
  return resolveUiLanguage({
    query: null,
    urlOverride: readStorageItem("session", UI_LANGUAGE_URL_OVERRIDE_KEY),
    profile,
    cached: readStorageItem("session", UI_LANGUAGE_STORAGE_KEY),
    browser: navigator.language,
  });
}

export function writeStoredLanguageCode(code: string) {
  if (typeof window === "undefined") {
    return;
  }

  writeStorageItem("session", UI_LANGUAGE_STORAGE_KEY, normalizeLocale(code));
}

type UseLanguageOptions = {
  onLanguageSet?: (code: LocaleCode, label: string) => void;
};

export function useLanguage(searchParams: URLSearchParams, options: UseLanguageOptions = {}) {
  const onLanguageSet = options.onLanguageSet;
  const client = useClient();
  const { uiSettings, uiSettingsLoaded, setUiSettings } = useUiSettings();
  // Undefined until the signed-in profile has loaded.
  const profileLanguage = uiSettingsLoaded ? uiSettings.language : undefined;
  const [queryLanguage] = useState(() => searchParams.get(URL_PARAM_LANGUAGE));
  const initialLanguage = (() => {
    const fromQuery = parseLanguageFromParam(queryLanguage);
    if (fromQuery && isLocaleLoaded(fromQuery)) {
      return fromQuery;
    }

    const stored = readStoredLanguageCode(profileLanguage);
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

  // An explicit choice ends any `?lang=` override for the tab and, once
  // signed in, is saved on the profile so every browser picks it up.
  const saveLanguageChoice = useCallback(
    (code: LocaleCode) => {
      writeStorageItem("session", UI_LANGUAGE_URL_OVERRIDE_KEY, null);
      if (!uiSettingsLoaded) {
        return;
      }
      writeStorageItem("local", UI_LANGUAGE_PROFILE_HINT_KEY, code);
      setUiSettings({ ...uiSettings, language: code });
      void client
        .mutation<{ setMyUiSettings?: UiSettings }, { input: SetMyUiSettingsInput }>(
          setMyUiSettingsMutation,
          { input: uiSettingsInputFromSettings(uiSettings, code) },
        )
        .toPromise()
        .then((result) => {
          if (result.error || !result.data?.setMyUiSettings) {
            toast.error(translate("status.languageSaveFailed", code));
            return;
          }
          setUiSettings(result.data.setMyUiSettings);
        });
    },
    [client, setUiSettings, uiSettings, uiSettingsLoaded],
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
            saveLanguageChoice(normalized);
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
    [onLanguageSet, saveLanguageChoice],
  );

  const setLanguagePreference = useCallback(
    (code: string) => requestLanguage(code, true),
    [requestLanguage],
  );

  const setLanguageFallback = useCallback(() => {
    if (typeof window === "undefined") {
      return;
    }

    const stored = readStoredLanguageCode(profileLanguage);
    if (stored === uiLanguage) {
      return;
    }
    requestLanguage(stored, false);
  }, [profileLanguage, requestLanguage, uiLanguage]);

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
    if (profileLanguage !== undefined) {
      writeStorageItem(
        "local",
        UI_LANGUAGE_PROFILE_HINT_KEY,
        recognizedLanguageCode(profileLanguage),
      );
    }
  }, [profileLanguage]);

  useEffect(() => {
    const queryLang = parseLanguageFromParam(searchParams.get(URL_PARAM_LANGUAGE));
    if (!queryLang) {
      setLanguageFallback();
    }
  }, [searchParams, setLanguageFallback]);

  useEffect(() => {
    const queryLang = parseLanguageFromParam(searchParams.get(URL_PARAM_LANGUAGE));
    if (queryLang) {
      writeStorageItem("session", UI_LANGUAGE_URL_OVERRIDE_KEY, queryLang);
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
