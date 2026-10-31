import type { Client, CombinedError } from "urql";

import { extractDownloadFeedbackWarning } from "@/lib/graphql/download-feedback-timeout";
import type { Facet } from "@/lib/types";
import type { DownloadQueueItem } from "@/lib/types/download-queue";

import {
  titleBySlugQuery,
  titleMoreLikeThisQuery,
  titleOverviewDownloadFeedbackQuery,
  titleRouteTargetQuery,
} from "@/lib/graphql/queries";
import type { CatalogDiscoveryItem } from "@/lib/types/discovery";

export type TitleSidePanelOverviewSnapshot<
  TTitle,
  TDiagnostics,
  TEvent,
  TBlocklist,
  TSubtitle,
> = {
  title: TTitle | null;
  acquisitionDiagnostics: TDiagnostics | null;
  titleHistory: TEvent[];
  titleReleaseBlocklist: TBlocklist[];
  externalSubtitles: TSubtitle[];
  hasDownloadClients: boolean;
};

export type TitleOverviewDownloadFeedbackSnapshot = {
  downloadQueueItems: DownloadQueueItem[];
  completedDownloadQueueItems: DownloadQueueItem[];
  downloadFeedbackWarning: string | null;
};

export type TitleOverviewSnapshot<
  TTitle,
  TDiagnostics,
  TEvent,
  TBlocklist,
  TSubtitle,
> = TitleSidePanelOverviewSnapshot<
  TTitle,
  TDiagnostics,
  TEvent,
  TBlocklist,
  TSubtitle
> &
  TitleOverviewDownloadFeedbackSnapshot;

export type ResolvedTitleOverviewTarget = {
  id: string;
  slug: string | null;
  libraryId: string | null;
  librarySlug: string | null;
};

export type ResolvedTitleRouteTarget = ResolvedTitleOverviewTarget & {
  facet: string;
};

type TitleMoreLikeThisResponse<TItem> = {
  title?: {
    moreLikeThis?: TItem[] | null;
  } | null;
};

function graphQlErrorAlias(
  error: CombinedError["graphQLErrors"][number],
): string | null {
  if (!Array.isArray(error.path) || error.path.length === 0) {
    return null;
  }

  return typeof error.path[0] === "string" ? error.path[0] : null;
}

function isTitleOverviewPartialOverviewError(
  error: CombinedError,
  data: unknown,
): boolean {
  if (error.networkError || !data || error.graphQLErrors.length === 0) {
    return false;
  }

  return error.graphQLErrors.every((graphQlError) => {
    const alias = graphQlErrorAlias(graphQlError);
    return alias === "titleHistory" || alias === "setupStatus";
  });
}

export async function fetchTitleSidePanelOverviewSnapshot<
  TTitle,
  TDiagnostics = unknown,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
>(
  client: Client,
  titleId: string,
  blocklistLimit: number,
  queryDocument: string,
): Promise<
  TitleSidePanelOverviewSnapshot<
    TTitle,
    TDiagnostics,
    TEvent,
    TBlocklist,
    TSubtitle
  >
> {
  const { data, error } = await client
    .query(
      queryDocument,
      { id: titleId, blocklistLimit },
      { requestPolicy: "network-only" },
    )
    .toPromise();

  if (error && !isTitleOverviewPartialOverviewError(error, data)) {
    throw error;
  }

  return {
    title: (data?.title ?? null) as TTitle | null,
    acquisitionDiagnostics: (data?.titleAcquisitionDiagnostics ??
      null) as TDiagnostics | null,
    titleHistory: (data?.titleHistory?.items ?? []) as TEvent[],
    titleReleaseBlocklist: (data?.titleReleaseBlocklist ?? []) as TBlocklist[],
    externalSubtitles: (data?.externalSubtitles ?? []) as TSubtitle[],
    hasDownloadClients: data?.setupStatus?.hasDownloadClients !== false,
  };
}

export async function fetchTitleMoreLikeThis<TItem = CatalogDiscoveryItem>(
  client: Client,
  titleId: string,
  limit = 12,
): Promise<TItem[]> {
  const { data, error } = await client
    .query<TitleMoreLikeThisResponse<TItem>>(
      titleMoreLikeThisQuery,
      { id: titleId, limit },
      { requestPolicy: "network-only" },
    )
    .toPromise();

  if (error) {
    throw error;
  }

  return data?.title?.moreLikeThis ?? [];
}

export function createEmptyTitleOverviewDownloadFeedbackSnapshot(): TitleOverviewDownloadFeedbackSnapshot {
  return {
    downloadQueueItems: [],
    completedDownloadQueueItems: [],
    downloadFeedbackWarning: null,
  };
}

export async function fetchTitleOverviewDownloadFeedbackSnapshot(
  client: Client,
  titleId: string,
): Promise<TitleOverviewDownloadFeedbackSnapshot> {
  const { data, error } = await client
    .query(
      titleOverviewDownloadFeedbackQuery,
      { id: titleId },
      { requestPolicy: "network-only" },
    )
    .toPromise();

  let downloadFeedbackWarning: string | null = null;
  if (error) {
    if (error.networkError || !data) {
      throw error;
    }

    downloadFeedbackWarning = extractDownloadFeedbackWarning(
      error.graphQLErrors,
      ["downloadQueueItems", "completedDownloadQueueItems"],
    );
    if (!downloadFeedbackWarning) {
      throw error;
    }
  }

  return {
    downloadQueueItems: (data?.downloadQueueItems ?? []) as DownloadQueueItem[],
    completedDownloadQueueItems: (data?.completedDownloadQueueItems ??
      []) as DownloadQueueItem[],
    downloadFeedbackWarning,
  };
}

export async function resolveTitleOverviewTargetBySlug(
  client: Client,
  facet: Facet,
  librarySlug: string | null | undefined,
  slug: string,
): Promise<ResolvedTitleOverviewTarget | null> {
  const normalizedLibrarySlug = librarySlug?.trim() || null;
  const normalizedSlug = slug.trim();
  if (!normalizedSlug) {
    return null;
  }

  const { data, error } = await client
    .query(
      titleBySlugQuery,
      { facet, librarySlug: normalizedLibrarySlug, slug: normalizedSlug },
      { requestPolicy: "network-only" },
    )
    .toPromise();

  if (error) {
    throw error;
  }

  const title = data?.titleBySlug;
  if (!title?.id) {
    return null;
  }

  return {
    id: String(title.id),
    slug:
      typeof title.slug === "string" && title.slug.trim().length > 0
        ? title.slug.trim()
        : null,
    libraryId:
      typeof title.libraryId === "string" && title.libraryId.trim().length > 0
        ? title.libraryId.trim()
        : null,
    librarySlug:
      typeof title.librarySlug === "string" &&
      title.librarySlug.trim().length > 0
        ? title.librarySlug.trim()
        : null,
  };
}

export async function resolveTitleOverviewTargetById(
  client: Client,
  id: string,
): Promise<ResolvedTitleRouteTarget | null> {
  const normalizedId = id.trim();
  if (!normalizedId) {
    return null;
  }

  const { data, error } = await client
    .query(
      titleRouteTargetQuery,
      { id: normalizedId },
      { requestPolicy: "network-only" },
    )
    .toPromise();

  if (error) {
    throw error;
  }

  const title = data?.title;
  if (!title?.id || typeof title.facet !== "string") {
    return null;
  }

  return {
    id: String(title.id),
    facet: title.facet,
    slug:
      typeof title.slug === "string" && title.slug.trim().length > 0
        ? title.slug.trim()
        : null,
    libraryId:
      typeof title.libraryId === "string" && title.libraryId.trim().length > 0
        ? title.libraryId.trim()
        : null,
    librarySlug:
      typeof title.librarySlug === "string" &&
      title.librarySlug.trim().length > 0
        ? title.librarySlug.trim()
        : null,
  };
}
