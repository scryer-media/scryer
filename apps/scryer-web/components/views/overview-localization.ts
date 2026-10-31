import type { Translate } from "@/components/root/types";
import { sectionLabelForFacet } from "@/lib/facets/helpers";
import type { Facet } from "@/lib/types/titles";

function isFacet(value: string): value is Facet {
  return value === "movie" || value === "series" || value === "anime";
}

export function localizedFacetLabel(
  t: Translate,
  facet: Facet | string,
): string {
  if (isFacet(facet)) {
    return sectionLabelForFacet(t, facet);
  }
  return facet.charAt(0).toUpperCase() + facet.slice(1);
}

export function localizedTitleStatus(
  t: Translate,
  status: string | null | undefined,
): string | null {
  const trimmed = status?.trim();
  if (!trimmed) return null;

  switch (trimmed.toLowerCase()) {
    case "ended":
    case "finished":
      return t("title.ended");
    case "continuing":
    case "returning":
      return t("title.continuing");
    case "upcoming":
      return t("title.upcoming");
    case "released":
      return t("settings.minAvailability.released");
    case "announced":
      return t("settings.minAvailability.announced");
    default:
      return trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
  }
}

export function localizedWantedStatus(
  t: Translate,
  status: string | null | undefined,
): string | null {
  const trimmed = status?.trim();
  if (!trimmed) return null;

  switch (trimmed.toLowerCase()) {
    case "wanted":
      return t("wanted.status.wanted");
    case "grabbed":
      return t("wanted.status.grabbed");
    case "completed":
      return t("wanted.status.completed");
    case "paused":
      return t("wanted.status.paused");
    default:
      return trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
  }
}
