import { DEFAULT_LANGUAGE, normalizeLocale, type LocaleCode } from "./index.ts";
import { parseLanguageFromParam } from "../utils/routing.ts";

export type UiLanguageSources = {
  /** `?lang=` on the current URL. */
  query: string | null;
  /**
   * A `?lang=` seen earlier in this tab. Entry links such as `/?lang=fra`
   * redirect away from the parameter, so it keeps applying for the tab.
   */
  urlOverride: string | null;
  /**
   * The language saved on the signed-in user's profile, or its last known
   * value while the profile is still loading. Null when there is none.
   */
  profile: string | null;
  /**
   * The language last picked from the menu in this browser, kept even when
   * the pick could not be saved on a profile.
   */
  choice: string | null;
  /** The language this tab last showed. */
  cached: string | null;
  /** `navigator.language`. */
  browser: string | null;
};

/**
 * The interface language for a stored code, or null when the code is empty or
 * names a language the interface does not offer. `normalizeLocale` maps any
 * unknown code to the default language, so the default only counts when it
 * was asked for.
 */
export function recognizedLanguageCode(
  value: string | null | undefined,
): LocaleCode | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  const normalized = normalizeLocale(trimmed);
  if (normalized !== DEFAULT_LANGUAGE) {
    return normalized;
  }
  return /^en(g)?([-_]|$)/i.test(trimmed) ? normalized : null;
}

/**
 * Picks the interface language: an explicit URL choice, then the user's
 * profile, then the language picked in this browser, then what this tab last
 * showed, then the browser's language.
 */
export function resolveUiLanguage(sources: UiLanguageSources): LocaleCode {
  return (
    parseLanguageFromParam(sources.query) ??
    recognizedLanguageCode(sources.urlOverride) ??
    recognizedLanguageCode(sources.profile) ??
    recognizedLanguageCode(sources.choice) ??
    recognizedLanguageCode(sources.cached) ??
    normalizeLocale(sources.browser)
  );
}
