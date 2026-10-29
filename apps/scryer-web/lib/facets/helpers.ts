import { resolveDefaultMonitorType } from "../utils/default-monitor-type";
import { facetById, facetForView } from "./registry";
import type { Translate } from "@/components/root/types";
import type { Facet } from "@/lib/types/titles";
import type { ViewId } from "@/components/root/types";
import type { MetadataCatalogMonitorType } from "@/lib/hooks/use-global-search";

export function sectionLabelForFacet(t: Translate, facetId: Facet): string {
  const def = facetById(facetId);
  return def ? t(def.searchLabelKey) : facetId;
}

export function viewAllLabelForFacet(t: Translate, facetId: Facet): string {
  return t(`search.viewAll.${facetId}`);
}

export function viewFromFacet(facetId: Facet): ViewId {
  const def = facetById(facetId);
  return (def?.viewId ?? "movies") as ViewId;
}

export function defaultMonitorTypeForFacet(facetId: Facet, saved?: string | null): MetadataCatalogMonitorType {
  return resolveDefaultMonitorType(facetId, saved, facetById(facetId)?.defaultMonitorType ?? "MONITORED") as MetadataCatalogMonitorType;
}

export function facetFromView(viewId: string): Facet | undefined {
  return facetForView(viewId)?.id;
}
