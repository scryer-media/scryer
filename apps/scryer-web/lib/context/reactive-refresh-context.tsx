import { createContext, useContext } from "react";

import type {
  TitleCatalogTitleProjection,
  TitleSidePanelOverviewProjection,
} from "@/lib/graphql/queries";
import type { ReactiveRefreshRegistration } from "@/lib/reactive/domain-event-feed";
import type { ImportRecord, TitleRecord } from "@/lib/types";
import type {
  TitleOverviewDownloadFeedbackSnapshot,
  TitleSidePanelOverviewSnapshot,
} from "@/lib/title-overview-loader";

type ReactiveRefreshErrorHandler = (error: unknown) => void;

export type QueueCatalogTitlesRefreshOptions = {
  facet?: string | null;
  apply: (titles: TitleRecord[]) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type QueueCatalogTitleRefreshOptions = {
  titleId: string;
  projection?: TitleCatalogTitleProjection;
  apply: (title: TitleRecord | null, requestEpoch: number) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type QueueTitleSidePanelOverviewRefreshOptions<
  TTitle = unknown,
  TDiagnostics = unknown,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
> = {
  titleId: string;
  blocklistLimit: number;
  projection: TitleSidePanelOverviewProjection;
  apply: (
    snapshot: TitleSidePanelOverviewSnapshot<
      TTitle,
      TDiagnostics,
      TEvent,
      TBlocklist,
      TSubtitle
    >,
  ) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type QueueTitleOverviewDownloadFeedbackRefreshOptions = {
  titleId: string;
  apply: (snapshot: TitleOverviewDownloadFeedbackSnapshot) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type QueueImportHistoryRefreshOptions = {
  limit?: number | null;
  apply: (records: ImportRecord[]) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type ReactiveRefreshContextValue = {
  /**
   * Register a targeted refresh: `run` fires (coalesced) whenever a
   * `domainEventFeed` event satisfies `predicate`. Returns an unregister fn.
   * Prefer the prebuilt predicates in `@/lib/reactive/domain-event-feed`
   * (forTitle, forEventTypes, forStreamKind, anyOf/allOf/not) over `always`.
   */
  registerReactiveRefresh: (
    registration: ReactiveRefreshRegistration,
  ) => () => void;
  queueCatalogTitlesRefresh: (
    options: QueueCatalogTitlesRefreshOptions,
  ) => void;
  queueCatalogTitleRefresh: (options: QueueCatalogTitleRefreshOptions) => void;
  queueTitleSidePanelOverviewRefresh: <
    TTitle = unknown,
    TDiagnostics = unknown,
    TEvent = unknown,
    TBlocklist = unknown,
    TSubtitle = unknown,
  >(
    options: QueueTitleSidePanelOverviewRefreshOptions<
      TTitle,
      TDiagnostics,
      TEvent,
      TBlocklist,
      TSubtitle
    >,
  ) => void;
  queueTitleOverviewDownloadFeedbackRefresh: (
    options: QueueTitleOverviewDownloadFeedbackRefreshOptions,
  ) => void;
  queueImportHistoryRefresh: (
    options: QueueImportHistoryRefreshOptions,
  ) => void;
};

export const ReactiveRefreshContext =
  createContext<ReactiveRefreshContextValue | null>(null);

export function reactiveRefreshEpoch(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

export function useReactiveRefresh(): ReactiveRefreshContextValue {
  const value = useContext(ReactiveRefreshContext);
  if (!value) {
    throw new Error(
      "useReactiveRefresh must be used within ReactiveRefreshContext.Provider",
    );
  }
  return value;
}

/**
 * Like [`useReactiveRefresh`] but returns `null` when no provider is mounted.
 * For hooks that must also work above the authenticated shell (where the
 * provider lives) and degrade to a legacy transport there.
 */
export function useReactiveRefreshOptional(): ReactiveRefreshContextValue | null {
  return useContext(ReactiveRefreshContext);
}
