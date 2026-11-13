// Metadata types returned by the backend metadata proxy resolvers.
// Field names are camelCase to match async_graphql output.

import type { TitleExternalRating } from "@/lib/utils/title-ratings";

export type MetadataTvdbSearchItem = {
  tvdbId: string;
  smgId?: number | null;
  tmdbId?: number | null;
  primarySource?: string | null;
  name: string;
  imdbId: string | null;
  externalIds?: Array<{ source: string; kind?: string | null; value: string }>;
  slug: string | null;
  type: string | null;
  year: number | null;
  status: string | null;
  overview: string | null;
  popularity: number | null;
  posterUrl: string | null;
  backgroundUrl?: string | null;
  language: string | null;
  runtimeMinutes: number | null;
  sortTitle: string | null;
  rating?: number | null;
  ratingSource?: string | null;
  ratingSources?: string[];
  externalRatings?: TitleExternalRating[];
  /** Libraries of the result's facet that already hold it, limited to those
   * the viewer may view, request into or manage. Only the requester search
   * selects it. */
  presentInLibraryIds?: string[];
};
