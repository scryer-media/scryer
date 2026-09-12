import type { WizardFacet } from "@/lib/hooks/use-external-import-setup";

/**
 * The facet colours themselves now live in `lib/facets/style`, shared with the
 * settings pickers; the wizard keeps importing them from here.
 */
export { facetPillStyle, facetStyle, type FacetStyle } from "@/lib/facets/style";

/** i18n key for a facet's display label. */
export function facetLabelKey(facet: WizardFacet): string {
  switch (facet) {
    case "MOVIE":
      return "setup.facetMovies";
    case "SERIES":
      return "setup.facetSeries";
    case "ANIME":
      return "setup.facetAnime";
  }
}
