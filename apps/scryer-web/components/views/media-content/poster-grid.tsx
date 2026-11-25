import * as React from "react";
import { useLocation } from "react-router";
import { useTranslate } from "@/lib/context/translate-context";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import {
  persistOverviewWindowScroll,
  useOverviewElementScrollRestoration,
} from "@/lib/hooks/use-overview-window-scroll-restoration";
import type { TitleRecord } from "@/lib/types";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitleCard } from "@/components/title-card";
import { titleDownloadActivityCornerBadge } from "@/components/common/title-download-activity";
import { titleOverviewOpenButtonId } from "@/lib/utils/dom-ids";
import {
  TitleCollectionEmptyState,
  TitleCollectionLoadingState,
} from "./title-table-shared";

type PosterGridProps = {
  titles: TitleRecord[];
  catalogInitialLoadComplete?: boolean;
  catalogHasMoreTitles?: boolean;
  catalogLoadingMoreTitles?: boolean;
  onCatalogEndReached?: () => Promise<void> | void;
  onOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
  selectedTitleId?: string | null;
  contextPanelId?: string;
  onSelectTitle?: (title: TitleRecord) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => void;
  isDeletingById: Record<string, boolean>;
  overviewTargetView: ViewId;
  showScanLibraryAction?: boolean;
  showConfigureRootsAction?: boolean;
  configureRootsReason?: "missing" | "invalid";
  configureRootsHref?: string;
  onScanLibrary?: () => Promise<void> | void;
  scanLibraryLoading?: boolean;
  scanLibraryDisabled?: boolean;
  scanLibraryNotice?: string | null;
  scrollContainer?: boolean;
  /** Titles with live, pending download work — shown as a pulsing card badge. */
  activeDownloadTitleIds?: ReadonlySet<string>;
};

export const PosterGrid = React.memo(function PosterGrid({
  titles,
  catalogInitialLoadComplete = true,
  catalogHasMoreTitles = false,
  catalogLoadingMoreTitles = false,
  onCatalogEndReached,
  onOpenOverview,
  selectedTitleId,
  contextPanelId,
  onSelectTitle,
  overviewTargetView,
  showScanLibraryAction = false,
  showConfigureRootsAction = false,
  configureRootsReason = "missing",
  configureRootsHref,
  onScanLibrary,
  scanLibraryLoading = false,
  scanLibraryDisabled = false,
  scanLibraryNotice,
  scrollContainer = false,
  activeDownloadTitleIds,
}: PosterGridProps) {
  const t = useTranslate();
  const endSentinelRef = React.useRef<HTMLDivElement | null>(null);
  const gridRef = React.useRef<HTMLDivElement | null>(null);
  useOverviewElementScrollRestoration({
    enabled: scrollContainer && catalogInitialLoadComplete && titles.length > 0,
    ready: catalogInitialLoadComplete,
    storageKeySuffix: "poster-grid",
    scrollRef: gridRef,
  });

  React.useEffect(() => {
    if (
      !catalogHasMoreTitles ||
      catalogLoadingMoreTitles ||
      !onCatalogEndReached
    ) {
      return;
    }

    const sentinel = endSentinelRef.current;
    if (!sentinel) {
      return;
    }

    const scrollRoot = sentinel.closest<HTMLElement>(
      "[data-catalog-scroll-root='true']",
    );
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          void onCatalogEndReached();
        }
      },
      { root: scrollRoot, rootMargin: "1200px 0px" },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [
    catalogHasMoreTitles,
    catalogLoadingMoreTitles,
    onCatalogEndReached,
    titles.length,
  ]);

  if (!catalogInitialLoadComplete) {
    return <TitleCollectionLoadingState />;
  }

  if (titles.length === 0) {
    return (
      <TitleCollectionEmptyState
        t={t}
        showScanAction={showScanLibraryAction}
        showConfigureRootsAction={showConfigureRootsAction}
        configureRootsReason={configureRootsReason}
        configureRootsHref={configureRootsHref}
        scanLoading={scanLibraryLoading}
        scanDisabled={scanLibraryDisabled}
        scanNotice={scanLibraryNotice}
        onScan={onScanLibrary}
      />
    );
  }

  return (
    <div
      ref={gridRef}
      data-catalog-scroll-root={scrollContainer ? "true" : undefined}
      className={
        scrollContainer
          ? "poster-grid-container min-h-0 sm:h-full sm:overflow-y-auto"
          : "poster-grid-container"
      }
    >
      <div className="poster-grid-responsive grid justify-start gap-3.5">
        {titles.map((title) => (
          <PosterCard
            key={title.id}
            title={title}
            onOpenOverview={onOpenOverview}
            selected={selectedTitleId === title.id}
            contextPanelId={contextPanelId}
            onSelectTitle={onSelectTitle}
            overviewTargetView={overviewTargetView}
            downloadActive={activeDownloadTitleIds?.has(title.id) ?? false}
          />
        ))}
      </div>
      {catalogHasMoreTitles ? (
        <div ref={endSentinelRef} aria-hidden="true" className="h-12" />
      ) : null}
    </div>
  );
});

type PosterCardProps = {
  title: TitleRecord;
  onOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
  selected: boolean;
  contextPanelId?: string;
  onSelectTitle?: (title: TitleRecord) => void;
  overviewTargetView: ViewId;
  downloadActive: boolean;
};

const PosterCard = React.memo(function PosterCard({
  title,
  onOpenOverview,
  selected,
  contextPanelId,
  onSelectTitle,
  overviewTargetView,
  downloadActive,
}: PosterCardProps) {
  const location = useLocation();
  const t = useTranslate();
  const posterUrl = selectPosterVariantUrl(title.posterUrl, "w250");
  const contextPanelControlsId =
    selected && onSelectTitle ? contextPanelId : undefined;
  const handleActivate = React.useCallback(() => {
    if (onSelectTitle) {
      persistOverviewWindowScroll(location.pathname);
      onSelectTitle(title);
      return;
    }
    persistOverviewWindowScroll(location.pathname);
    onOpenOverview(overviewTargetView, title);
  }, [
    location.pathname,
    onOpenOverview,
    onSelectTitle,
    overviewTargetView,
    title,
  ]);

  return (
    <div className="cv-auto-poster">
      <TitleCard
        title={title.name}
        year={title.year ?? null}
        posterUrl={posterUrl}
        metadataFetchedAt={title.metadataFetchedAt}
        createdAt={title.createdAt}
        monitored={title.monitored}
        selected={selected}
        cornerBadge={
          downloadActive ? titleDownloadActivityCornerBadge(t) : null
        }
        revealTextOnHover
        emptyLabel={t("label.noArt")}
        onOpen={handleActivate}
        interactiveProps={{
          id: titleOverviewOpenButtonId(title.id),
          "aria-current": selected ? "true" : undefined,
          "aria-controls": contextPanelControlsId,
        }}
      />
    </div>
  );
});
