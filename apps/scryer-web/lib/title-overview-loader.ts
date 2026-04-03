import type { Client } from "urql";

import { titleOverviewInitQuery } from "@/lib/graphql/queries";

type TitleOverviewSnapshot<TTitle, TEvent, TBlocklist, TSubtitle> = {
  title: TTitle | null;
  titleEvents: TEvent[];
  titleReleaseBlocklist: TBlocklist[];
  subtitleDownloads: TSubtitle[];
};

// Canonical base loader for title overview pages. Overview containers may
// derive view-specific state locally, but should not duplicate the underlying
// network-only title detail fetch and normalization.
export async function fetchTitleOverviewSnapshot<
  TTitle,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
>(
  client: Client,
  titleId: string,
  blocklistLimit: number,
): Promise<TitleOverviewSnapshot<TTitle, TEvent, TBlocklist, TSubtitle>> {
  const { data, error } = await client
    .query(
      titleOverviewInitQuery,
      { id: titleId, blocklistLimit },
      { requestPolicy: "network-only" },
    )
    .toPromise();

  if (error) {
    throw error;
  }

  return {
    title: (data?.title ?? null) as TTitle | null,
    titleEvents: (data?.titleEvents ?? []) as TEvent[],
    titleReleaseBlocklist: (data?.titleReleaseBlocklist ?? []) as TBlocklist[],
    subtitleDownloads: (data?.subtitleDownloads ?? []) as TSubtitle[],
  };
}
