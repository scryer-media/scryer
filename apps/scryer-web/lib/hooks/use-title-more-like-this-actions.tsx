import * as React from "react";
import { useNavigate } from "react-router";
import { useClient } from "urql";

import {
  AddToCatalogDialog,
  EMPTY_SEARCH_RESULT,
} from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";
import type { TitleMoreLikeThisStripActions } from "@/components/views/title-more-like-this-strip";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useSearchContext } from "@/lib/context/search-context";
import { useTranslate } from "@/lib/context/translate-context";
import { titleRouteTargetQuery } from "@/lib/graphql/queries";
import type { CatalogDiscoveryItem, Facet } from "@/lib/types";
import {
  discoveryItemFacet,
  metadataResultForDiscoveryItem,
} from "@/lib/utils/discovery-actions";
import { buildOverviewDetailPath } from "@/lib/utils/routing";
import type { ViewId } from "@/components/root/types";

type DiscoveryDialogTarget = {
  result: ReturnType<typeof metadataResultForDiscoveryItem>;
  facet: Facet;
};

type TitleRouteTarget = {
  id: string;
  facet: string;
  slug?: string | null;
  libraryId?: string | null;
  librarySlug?: string | null;
};

type UseTitleMoreLikeThisActionsOptions = {
  canAddItems?: boolean;
  canRequestItems?: boolean;
  onCatalogChanged?: () => Promise<void> | void;
};

function viewForFacet(facet: string | null | undefined): ViewId | null {
  switch (facet?.trim().toLowerCase()) {
    case "movie":
      return "movies";
    case "series":
      return "series";
    case "anime":
      return "anime";
    default:
      return null;
  }
}

function routePathForTitle(target: TitleRouteTarget, view: ViewId): string {
  const slug = target.slug?.trim() || null;
  const librarySlug = target.librarySlug?.trim() || null;
  const path = buildOverviewDetailPath(view, librarySlug, slug);
  if (slug && librarySlug) {
    return path;
  }
  const params = new URLSearchParams();
  params.set("id", target.id);
  return `${path}?${params.toString()}`;
}

export function useTitleMoreLikeThisActions({
  canAddItems = false,
  canRequestItems = false,
  onCatalogChanged,
}: UseTitleMoreLikeThisActionsOptions = {}): {
  stripProps: TitleMoreLikeThisStripActions;
  dialogs: React.ReactNode;
} {
  const client = useClient();
  const navigate = useNavigate();
  const search = useSearchContext();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const [addDialogTarget, setAddDialogTarget] =
    React.useState<DiscoveryDialogTarget | null>(null);
  const [requestDialogTarget, setRequestDialogTarget] =
    React.useState<DiscoveryDialogTarget | null>(null);

  const canAddItem = React.useCallback(
    (item: CatalogDiscoveryItem) => {
      const facet = discoveryItemFacet(item);
      return facet
        ? canAddItems || (search.librariesByFacet[facet] ?? []).length > 0
        : false;
    },
    [canAddItems, search.librariesByFacet],
  );

  const canRequestItem = React.useCallback(
    (item: CatalogDiscoveryItem) => {
      const facet = discoveryItemFacet(item);
      return facet
        ? canRequestItems ||
            (search.requestableLibrariesByFacet[facet] ?? []).length > 0
        : false;
    },
    [canRequestItems, search.requestableLibrariesByFacet],
  );

  const handleAction = React.useCallback(
    async (item: CatalogDiscoveryItem) => {
      if (item.ownedInInput) {
        return;
      }
      const facet = discoveryItemFacet(item);
      if (!facet) {
        setGlobalStatus(t("status.apiError"), { level: "ERROR" });
        return;
      }
      try {
        await search.ensureCatalogConfigReady(facet);
        const target = {
          result: metadataResultForDiscoveryItem(item),
          facet,
        };
        if ((search.librariesByFacet[facet] ?? []).length > 0) {
          setAddDialogTarget(target);
        } else if ((search.requestableLibrariesByFacet[facet] ?? []).length > 0) {
          setRequestDialogTarget(target);
        } else {
          setGlobalStatus(t("status.permissionDenied"));
        }
      } catch (caught) {
        setGlobalStatus(
          caught instanceof Error ? caught.message : t("status.apiError"),
          { level: "ERROR" },
        );
      }
    },
    [
      search,
      setGlobalStatus,
      t,
    ],
  );

  const handleOpenResolved = React.useCallback(
    async (item: CatalogDiscoveryItem) => {
      const titleId = item.resolvedTitleId?.trim();
      if (!titleId) {
        return;
      }
      try {
        const { data, error } = await client
          .query(
            titleRouteTargetQuery,
            { id: titleId },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) {
          throw error;
        }
        const target = data?.title as TitleRouteTarget | null | undefined;
        const view = viewForFacet(target?.facet);
        if (!target || !view) {
          setGlobalStatus(t("status.apiError"), { level: "ERROR" });
          return;
        }
        navigate(routePathForTitle(target, view));
      } catch (caught) {
        setGlobalStatus(
          caught instanceof Error ? caught.message : t("status.apiError"),
          { level: "ERROR" },
        );
      }
    },
    [client, navigate, setGlobalStatus, t],
  );

  const addFacet = addDialogTarget?.facet ?? "MOVIE";
  const addResult = addDialogTarget?.result ?? EMPTY_SEARCH_RESULT;
  const requestFacet = requestDialogTarget?.facet ?? "MOVIE";
  const requestResult = requestDialogTarget?.result ?? EMPTY_SEARCH_RESULT;

  return {
    stripProps: {
      canAddItem,
      canRequestItem,
      onAction: handleAction,
      onOpenResolved: handleOpenResolved,
    },
    dialogs: (
      <>
        <AddToCatalogDialog
          open={addDialogTarget !== null}
          onOpenChange={(open) => {
            if (!open) {
              setAddDialogTarget(null);
            }
          }}
          result={addResult}
          facet={addFacet}
          catalogQualityProfileOptions={search.catalogQualityProfileOptions}
          catalogConfigLoading={search.catalogConfigLoading}
          defaultQualityProfileId={search.resolveDefaultQualityProfileIdForFacet(
            addFacet,
          )}
          manageableLibraries={search.librariesByFacet[addFacet] ?? []}
          rootFolderOptions={search.rootFoldersByFacet[addFacet] ?? []}
          onAdd={async (result, facet, options) => {
            const titleId = await search.addMetadataSearchResultToCatalog(
              result,
              facet,
              options,
            );
            if (titleId) {
              await onCatalogChanged?.();
            }
            return titleId;
          }}
        />
        <RequestMediaDialog
          open={requestDialogTarget !== null}
          onOpenChange={(open) => {
            if (!open) {
              setRequestDialogTarget(null);
            }
          }}
          result={requestResult}
          facet={requestFacet}
          requestableLibraries={
            search.requestableLibrariesByFacet[requestFacet] ?? []
          }
          qualityProfileOptions={search.catalogQualityProfileOptions}
          onRequest={async (result, facet, options) => {
            const accepted = await search.requestMetadataSearchResult(
              result,
              facet,
              options,
            );
            if (accepted) {
              await onCatalogChanged?.();
            }
            return accepted;
          }}
        />
      </>
    ),
  };
}
