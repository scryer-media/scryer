import type { ViewId } from "../../components/root/types.ts";
import { facetById } from "../facets/registry.ts";
import { buildOverviewDetailPath } from "./routing.ts";

export type CalendarEventHrefItem = {
  id: string;
  titleId: string;
  titleFacet: string;
  titleSlug?: string | null;
  librarySlug?: string | null;
};

export function buildCalendarEventHref(
  item: CalendarEventHrefItem,
): string | null {
  const facet = facetById(item.titleFacet.toUpperCase());
  const titleId = item.titleId.trim();
  if (!facet || !titleId) return null;

  const librarySlug = item.librarySlug?.trim() || null;
  const titleSlug = item.titleSlug?.trim() || null;
  const hasSlugRoute = Boolean(librarySlug && titleSlug);
  const path = buildOverviewDetailPath(
    facet.viewId as ViewId,
    librarySlug,
    titleSlug,
  );
  const params = new URLSearchParams();

  if (!hasSlugRoute) {
    params.set("id", titleId);
  }
  if (facet.id !== "MOVIE" && item.id.trim()) {
    params.set("episodeId", item.id.trim());
  }

  const query = params.toString();
  return `${path}${query ? `?${query}` : ""}`;
}
