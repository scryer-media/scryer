import * as React from "react";
import { useClient } from "urql";

import {
  metadataMovieSummaryQuery,
  metadataSeriesSummaryQuery,
} from "@/lib/graphql/queries";
import type { ListTitleMetadataLookup } from "@/lib/utils/lists";
import type { TitleExternalRating } from "@/lib/utils/title-ratings";

export type ListTitleMetadata = {
  overview: string | null;
  posterUrl: string | null;
  backgroundUrl: string | null;
  runtimeMinutes: number | null;
  externalRatings: TitleExternalRating[];
  genresAndThemes: string[];
};

/**
 * What the metadata service knows of a listed title: its description, artwork,
 * ratings and genres. Nothing is read until `request` is called, and it is read
 * once; a title the service cannot describe stays null.
 */
export function useListTitleMetadata(lookup: ListTitleMetadataLookup | null) {
  const client = useClient();
  const [metadata, setMetadata] = React.useState<ListTitleMetadata | null>(
    null,
  );
  const requested = React.useRef(false);

  const request = () => {
    if (requested.current || !lookup) return;
    requested.current = true;
    void client
      .query(
        lookup.movie ? metadataMovieSummaryQuery : metadataSeriesSummaryQuery,
        {
          input: lookup.movie
            ? lookup.input
            : { ...lookup.input, includeEpisodes: false },
        },
      )
      .toPromise()
      .then(({ data }) => {
        const found = lookup.movie ? data?.metadataMovie : data?.metadataSeries;
        if (!found) return;
        setMetadata({
          overview: found.overview || null,
          posterUrl: found.posterUrl || null,
          backgroundUrl: found.backgroundUrl || null,
          runtimeMinutes: found.runtimeMinutes ?? null,
          externalRatings: found.externalRatings ?? [],
          genresAndThemes: found.genresAndThemes ?? [],
        });
      });
  };

  return [metadata, request] as const;
}
