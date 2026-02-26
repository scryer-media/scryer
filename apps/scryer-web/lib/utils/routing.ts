import type { LocaleCode } from "@/lib/i18n";
import type { ContentSettingsSection, SettingsSection, ViewId } from "@/components/root/types";
import { normalizeLocale } from "@/lib/i18n";
import { AVAILABLE_LANGUAGES } from "@/lib/i18n";
import { URL_PATH_SEGMENTS } from "@/lib/constants/settings";
import { SETTINGS_SECTION_PATH_TO_ID, CONTENT_SECTION_PATH_TO_ID } from "@/lib/constants/settings";
import { isMediaView } from "@/lib/facets/registry";

export const SETTINGS_SECTION_PATH: Record<SettingsSection, string> = {
  profile: "profile",
  general: "general",
  users: "users",
  indexers: "indexers",
  downloadClients: "download-clients",
  qualityProfiles: "quality-profiles",
  acquisition: "acquisition",
};

export const CONTENT_SECTION_PATH: Record<ContentSettingsSection, string> = {
  overview: "overview",
  settings: "settings",
};

export function buildViewPath(
  nextView: ViewId,
  nextSettingsSection?: SettingsSection,
  nextContentSection?: ContentSettingsSection,
) {
  const base = `/${nextView}`;
  if (nextView === "settings" && nextSettingsSection && nextSettingsSection !== "profile") {
    return `${base}/${SETTINGS_SECTION_PATH[nextSettingsSection]}`;
  }
  if (isMediaView(nextView)) {
    if (nextContentSection && nextContentSection !== "overview") {
      return `${base}/${CONTENT_SECTION_PATH[nextContentSection]}`;
    }
  }
  return base;
}

export function isLocaleSupported(code: string): code is LocaleCode {
  return AVAILABLE_LANGUAGES.some((language) => language.code === code);
}

export function parseViewFromPath(pathname: string | null | undefined): ViewId {
  const segment = (pathname ?? "").trim().toLowerCase();
  if (!segment) {
    return "movies";
  }

  return URL_PATH_SEGMENTS.includes(segment as ViewId) ? (segment as ViewId) : "movies";
}

export function parseSettingsSectionFromPath(value: string | null): SettingsSection {
  if (!value) {
    return "profile";
  }
  return SETTINGS_SECTION_PATH_TO_ID[value] ?? "profile";
}

export function parseContentSectionFromPath(value: string | null): ContentSettingsSection {
  if (!value) {
    return "overview";
  }
  return CONTENT_SECTION_PATH_TO_ID[value] ?? "overview";
}

export function parseLanguageFromParam(value: string | null): LocaleCode | null {
  if (!value) {
    return null;
  }

  const normalized = normalizeLocale(value);
  return isLocaleSupported(normalized) ? normalized : null;
}
