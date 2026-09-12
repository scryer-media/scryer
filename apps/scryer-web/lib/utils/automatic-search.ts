import type { JobRun } from "@/lib/types";

export function acquisitionProgress(run: JobRun): Record<string, unknown> {
  const value = run.progressJson;
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown> : {};
}

export function matchesActiveAutomaticSearch(run: JobRun, titleId: string, season?: number): boolean {
  if (run.jobKey !== "ACQUISITION_SEARCH" || !["QUEUED", "DISCOVERING", "RUNNING"].includes(run.status)) return false;
  const search = acquisitionProgress(run).search as Record<string, unknown> | undefined;
  return search?.titleId === titleId &&
    (season === undefined || search.seasonNumber == null || search.seasonNumber === season);
}

export function shouldRestoreAutomaticSearch(run: JobRun): boolean {
  const search = acquisitionProgress(run).search as Record<string, unknown> | undefined;
  return search?.intent === "AUTOMATIC" && typeof search.titleId === "string" &&
    matchesActiveAutomaticSearch(run, search.titleId);
}

export function parseSearchSeason(value: string | null | undefined): number | null {
  if (!value || !/^\d+$/.test(value.trim())) return null;
  const season = Number(value.trim());
  return Number.isSafeInteger(season) && season <= 2147483647 ? season : null;
}

export function automaticSearchOutcomeKey(run: JobRun): string | null {
  if (run.jobKey !== "ACQUISITION_SEARCH") return null;
  switch (acquisitionProgress(run).outcome) {
    case "no_eligible_work": return "wanted.searchNoEligibleWork";
    case "no_acceptable_releases": return "wanted.searchNoAcceptableReleases";
    case "downloads_submitted": return "wanted.searchJobComplete";
    case "cancelled": return "wanted.searchJobCancelled";
    case "failed": return "wanted.searchJobComplete";
    default: return null;
  }
}
