import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type { MediaSettings } from "@/lib/types/settings";

export function normalizeFillerPolicy(value: string | null | undefined) {
  return value?.trim().toUpperCase() === "SKIP_FILLER"
    ? "SKIP_FILLER"
    : "DOWNLOAD_ALL";
}

export function normalizeRenameCollisionPolicy(
  value: string | null | undefined,
) {
  const normalized = value?.trim().toUpperCase();
  return normalized === "ERROR" || normalized === "REPLACE_IF_BETTER"
    ? normalized
    : "SKIP";
}

export function normalizeRenameMissingMetadataPolicy(
  value: string | null | undefined,
) {
  return value?.trim().toUpperCase() === "SKIP" ? "SKIP" : "FALLBACK_TITLE";
}

export function normalizeRecapPolicy(value: string | null | undefined) {
  return value?.trim().toUpperCase() === "SKIP_RECAP"
    ? "SKIP_RECAP"
    : "DOWNLOAD_ALL";
}

export function normalizeAnimeMediaSettings(
  settings: Partial<
    Pick<
      MediaSettings,
      | "fillerPolicy"
      | "recapPolicy"
      | "monitorSpecials"
      | "interSeasonMovies"
      | "monitorFillerMovies"
    >
  >,
) {
  return {
    fillerPolicy: normalizeFillerPolicy(settings.fillerPolicy),
    recapPolicy: normalizeRecapPolicy(settings.recapPolicy),
    monitorSpecials: settings.monitorSpecials ? "true" : "false",
    interSeasonMovies: settings.interSeasonMovies === false ? "false" : "true",
    monitorFillerMovies: settings.monitorFillerMovies ? "true" : "false",
  };
}

export function facetScopedMediaSettingsScopeId(
  mediaSettings: Pick<MediaSettings, "scope">,
): ViewCategoryId {
  return mediaSettings.scope;
}

export function updateFacetScopedStringRecord(
  previous: Record<ViewCategoryId, string>,
  scopeId: ViewCategoryId,
  nextValue: string,
): Record<ViewCategoryId, string> {
  if (previous[scopeId] === nextValue) {
    return previous;
  }
  return { ...previous, [scopeId]: nextValue };
}

export function updateFacetScopedStringArrayRecord(
  previous: Record<ViewCategoryId, string[]>,
  scopeId: ViewCategoryId,
  nextValues: string[],
): Record<ViewCategoryId, string[]> {
  const currentValues = previous[scopeId] ?? [];
  const same =
    currentValues.length === nextValues.length &&
    currentValues.every((value, index) => value === nextValues[index]);
  if (same) {
    return previous;
  }
  return { ...previous, [scopeId]: nextValues };
}
