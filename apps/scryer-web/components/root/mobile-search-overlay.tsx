import * as React from "react";
import {
  Eraser,
  Search,
  X,
} from "lucide-react";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import {
  groupRouteCommandItems,
  routeCommandDisplayLabel,
  type RouteCommandItem,
} from "@/components/common/route-command-types";
import { HorizontalRail } from "@/components/common/horizontal-scroll-fade";
import { UnderlineFilterButton } from "@/components/common/underline-filter-button";
import { IconButton } from "@/components/ui/icon-button";
import {
  SearchCatalogResultButton,
  SearchEmptyState,
  SearchFooterTip,
  SearchMetadataPosterButton,
  SearchRouteCommandButton,
  SearchSectionLoading,
} from "@/components/root/global-search-parts";
import {
  buildCatalogSearchSections,
  buildGlobalSearchTabs,
  buildMetadataResultCounts,
  buildMetadataSearchActionState,
  countHiddenCatalogResultsForFilters,
  countHiddenMetadataResultsForFilters,
  countHiddenRouteCommandResultsForFilters,
  countMetadataResults,
  countVisibleCatalogResults,
  filterGlobalSearchRouteCommands,
  GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT,
  getMetadataSectionFacetsForFilters,
  getVisibleCatalogFacetsForFilters,
  getVisibleCatalogResultsForFilters,
  getVisibleMetadataResultsForFilters,
  getVisibleRouteCommandResultsForFilters,
  isGlobalSearchFilterSelected,
  normalizeGlobalSearchFilterSelection,
  toggleGlobalSearchFilterSelection,
  type GlobalSearchFilterKey,
  type GlobalSearchTabKey,
} from "@/components/root/global-search-model";
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import type {
  MetadataCatalogAddOptions,
  MetadataCatalogRequestOptions,
} from "@/lib/hooks/use-global-search";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import {
  sectionLabelForFacet,
  viewAllLabelForFacet,
  viewFromFacet,
} from "@/lib/facets/helpers";
import { useSearchContext } from "@/lib/context/search-context";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import {
  globalSearchMetadataResultId,
  selectorId,
} from "@/lib/utils/dom-ids";
import {
  AddToCatalogDialog,
  EMPTY_SEARCH_RESULT,
} from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";

type MobileSearchOverlayProps = {
  canViewCatalog: boolean;
  onClose: () => void;
  routeCommandItems?: RouteCommandItem[];
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
};

export function MobileSearchOverlay({
  canViewCatalog,
  onClose,
  routeCommandItems = [],
  onOpenOverview,
}: MobileSearchOverlayProps) {
  const searchState = useSearchContext();
  const {
    globalSearch,
    globalSearchInputRef,
    catalogSearchResults,
    metadataSearchResults,
    catalogSearchLoading,
    metadataSearchLoading,
    searching,
  } = searchState;
  const t = useTranslate();
  const searchOverlayPlaceholder = canViewCatalog
    ? t("search.overlayPlaceholder")
    : t("search.overlayPlaceholderNoLibrary");
  const searchSubtitle = canViewCatalog
    ? t("search.subtitle")
    : t("search.subtitleNoLibrary");
  const searchMinimumQueryHint = canViewCatalog
    ? t("search.minimumQueryHint")
    : t("search.minimumQueryHintNoLibrary");
  const searchEmptyHint = canViewCatalog
    ? t("search.emptyHint")
    : t("search.emptyHintNoLibrary");
  const searchTipTitles = canViewCatalog
    ? t("search.tipTitles")
    : t("search.tipTitlesNoLibrary");
  const searchTipTabs = canViewCatalog
    ? t("search.tipTabs")
    : t("search.tipTabsNoLibrary");
  const trimmedGlobalSearch = globalSearch.trim();
  const hasMinimumGlobalSearchQuery = trimmedGlobalSearch.length >= 2;
  const overlayRef = React.useRef<HTMLDivElement>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const mobileSearchResultsRef = React.useRef<HTMLDivElement>(null);
  const mobileSearchTabRefs = React.useRef<
    Partial<Record<GlobalSearchTabKey, HTMLButtonElement | null>>
  >({});
  const [activeFilters, setActiveFilters] = React.useState<
    GlobalSearchFilterKey[]
  >([]);
  const [addDialogTarget, setAddDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
  const [requestDialogTarget, setRequestDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
  const closingAfterSuccessfulActionRef = React.useRef(false);
  const setMobileSearchInputRef = React.useCallback(
    (node: HTMLInputElement | null) => {
      inputRef.current = node;
      globalSearchInputRef.current = node;
    },
    [globalSearchInputRef],
  );

  // Focus the input when the overlay mounts.
  // Mobile Safari restricts focus() to user-gesture contexts, so we also
  // use autoFocus on the input and retry with a short delay as a fallback.
  React.useEffect(() => {
    inputRef.current?.focus();
    const timer = setTimeout(() => inputRef.current?.focus(), 50);
    return () => clearTimeout(timer);
  }, []);

  // Prevent body scroll while overlay is open
  React.useEffect(() => {
    const original = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = original;
    };
  }, []);

  const catalogSearchSections = React.useMemo(
    () => buildCatalogSearchSections(catalogSearchResults, globalSearch),
    [catalogSearchResults, globalSearch],
  );

  const metadataResultCounts = React.useMemo(
    () => buildMetadataResultCounts(metadataSearchResults),
    [metadataSearchResults],
  );

  const metadataResultCount = React.useMemo(
    () => countMetadataResults(metadataResultCounts),
    [metadataResultCounts],
  );
  const routeCommandResults = React.useMemo(
    () => filterGlobalSearchRouteCommands(routeCommandItems, globalSearch),
    [globalSearch, routeCommandItems],
  );
  const visibleRouteCommandResults = React.useMemo(
    () =>
      getVisibleRouteCommandResultsForFilters(
        activeFilters,
        routeCommandResults,
      ),
    [activeFilters, routeCommandResults],
  );
  const visibleCatalogFacets = React.useMemo(
    () => getVisibleCatalogFacetsForFilters(activeFilters, canViewCatalog),
    [activeFilters, canViewCatalog],
  );
  const metadataSectionFacets = React.useMemo(
    () =>
      getMetadataSectionFacetsForFilters({
        selectedFilters: activeFilters,
        metadataSearchLoading,
        metadataResultCounts,
      }),
    [activeFilters, metadataResultCounts, metadataSearchLoading],
  );

  const visibleCatalogCount = React.useMemo(
    () =>
      countVisibleCatalogResults(visibleCatalogFacets, catalogSearchSections),
    [catalogSearchSections, visibleCatalogFacets],
  );

  const visibleCatalogResults = React.useMemo(() => {
    return getVisibleCatalogResultsForFilters({
      selectedFilters: activeFilters,
      canViewCatalog,
      catalogSearchSections,
      visibleCatalogFacets,
      allLimit: GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT,
    });
  }, [activeFilters, canViewCatalog, catalogSearchSections, visibleCatalogFacets]);
  const visibleCatalogResultCount = canViewCatalog
    ? catalogSearchResults.length
    : 0;
  const hiddenCatalogResultCount = countHiddenCatalogResultsForFilters(
    activeFilters,
    visibleCatalogCount,
    visibleCatalogResults,
  );

  const mobileSearchTabs = React.useMemo(
    () =>
      buildGlobalSearchTabs({
        canViewCatalog,
        catalogSearchSections,
        metadataResultCount,
        metadataResultCounts,
        routeCommandResultCount: routeCommandResults.length,
        visibleCatalogResultCount,
        t,
      }),
    [
      canViewCatalog,
      catalogSearchSections,
      metadataResultCount,
      metadataResultCounts,
      routeCommandResults.length,
      t,
      visibleCatalogResultCount,
    ],
  );
  const searchStatusLabel = React.useMemo(() => {
    const isLoading =
      searching ||
      (canViewCatalog && catalogSearchLoading) ||
      metadataSearchLoading;
    if (!trimmedGlobalSearch) {
      return searchSubtitle;
    }
    if (!hasMinimumGlobalSearchQuery && routeCommandResults.length === 0) {
      return searchMinimumQueryHint;
    }
    if (isLoading) {
      return t("search.statusLoading", { query: trimmedGlobalSearch });
    }

    const resultCount =
      visibleCatalogResultCount +
      metadataResultCount +
      routeCommandResults.length;
    if (resultCount === 0) {
      return t("search.statusNoResults", { query: trimmedGlobalSearch });
    }
    return resultCount === 1
      ? t("search.statusResultOne", { query: trimmedGlobalSearch })
      : t("search.statusResultOther", {
          count: String(resultCount),
          query: trimmedGlobalSearch,
        });
  }, [
    canViewCatalog,
    catalogSearchLoading,
    hasMinimumGlobalSearchQuery,
    metadataResultCount,
    metadataSearchLoading,
    searchMinimumQueryHint,
    searchSubtitle,
    searching,
    routeCommandResults.length,
    t,
    trimmedGlobalSearch,
    visibleCatalogResultCount,
  ]);

  const focusMobileSearchFilter = React.useCallback(
    (nextTab: GlobalSearchTabKey) => {
      const nextTabElement = mobileSearchTabRefs.current[nextTab];
      nextTabElement?.focus();
      nextTabElement?.scrollIntoView({ block: "nearest", inline: "nearest" });
    },
    [],
  );

  const toggleMobileSearchFilter = React.useCallback(
    (key: GlobalSearchTabKey) => {
      setActiveFilters((selectedFilters) =>
        toggleGlobalSearchFilterSelection(
          selectedFilters,
          key,
          mobileSearchTabs,
        ),
      );
    },
    [mobileSearchTabs],
  );

  const handleMobileSearchTabKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent<HTMLButtonElement>,
      currentTab: GlobalSearchTabKey,
    ) => {
      const tabKeys = mobileSearchTabs.map((tab) => tab.key);
      if (tabKeys.length === 0) {
        return;
      }

      const currentIndex = tabKeys.indexOf(currentTab);
      const safeIndex = currentIndex === -1 ? 0 : currentIndex;
      let nextTab: GlobalSearchTabKey | null = null;

      if (event.key === "ArrowRight") {
        nextTab = tabKeys[(safeIndex + 1) % tabKeys.length] ?? null;
      } else if (event.key === "ArrowLeft") {
        nextTab =
          tabKeys[(safeIndex - 1 + tabKeys.length) % tabKeys.length] ?? null;
      } else if (event.key === "Home") {
        nextTab = tabKeys[0] ?? null;
      } else if (event.key === "End") {
        nextTab = tabKeys[tabKeys.length - 1] ?? null;
      }

      if (!nextTab) {
        return;
      }

      event.preventDefault();
      focusMobileSearchFilter(nextTab);
    },
    [focusMobileSearchFilter, mobileSearchTabs],
  );

  React.useEffect(() => {
    setActiveFilters((selectedFilters) =>
      normalizeGlobalSearchFilterSelection(selectedFilters, mobileSearchTabs),
    );
  }, [mobileSearchTabs]);

  React.useEffect(() => {
    mobileSearchResultsRef.current?.scrollTo({ left: 0, top: 0 });
  }, [activeFilters, globalSearch]);

  const {
    catalogConfigLoading,
    ensureCatalogConfigReady,
    isCatalogConfigReady,
    resolveDefaultQualityProfileIdForFacet,
    addMetadataSearchResultToCatalog,
    requestMetadataSearchResult,
    isMetadataSearchResultInCatalog,
    catalogQualityProfileOptions,
    librariesByFacet,
    rootFoldersByFacet,
    requestableLibrariesByFacet,
    setGlobalSearch,
    clearGlobalSearch,
    resetGlobalSearch,
    forceSearchGlobal,
  } = searchState;
  const isAddDialogConfigReady = addDialogTarget
    ? isCatalogConfigReady(addDialogTarget.facet)
    : true;

  const getMobileSearchResultButtons = React.useCallback(() => {
    const resultRoot = mobileSearchResultsRef.current;
    if (!resultRoot) {
      return [];
    }

    return Array.from(
      resultRoot.querySelectorAll<HTMLButtonElement>(
        "[data-mobile-global-search-result='true']:not(:disabled)",
      ),
    );
  }, []);

  const focusMobileSearchResult = React.useCallback(
    (position: "first" | "last") => {
      const buttons = getMobileSearchResultButtons();
      if (buttons.length === 0) {
        return false;
      }

      buttons[position === "first" ? 0 : buttons.length - 1]?.focus();
      return true;
    },
    [getMobileSearchResultButtons],
  );

  const focusRelativeMobileSearchResult = React.useCallback(
    (currentButton: HTMLButtonElement, delta: 1 | -1) => {
      const buttons = getMobileSearchResultButtons();
      if (buttons.length === 0) {
        return false;
      }

      const currentIndex = buttons.indexOf(currentButton);
      if (currentIndex === -1) {
        buttons[delta > 0 ? 0 : buttons.length - 1]?.focus();
        return true;
      }

      const nextIndex =
        (currentIndex + delta + buttons.length) % buttons.length;
      buttons[nextIndex]?.focus();
      return true;
    },
    [getMobileSearchResultButtons],
  );

  const handleMobileSearchInputKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
        return;
      }

      if (event.key === "Enter") {
        if (event.nativeEvent.isComposing) {
          return;
        }
        event.preventDefault();
        if (focusMobileSearchResult("first")) {
          return;
        }
        void forceSearchGlobal(event.currentTarget.value);
        return;
      }

      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        const didFocus = focusMobileSearchResult(
          event.key === "ArrowDown" ? "first" : "last",
        );
        if (didFocus) {
          event.preventDefault();
        }
      }
    },
    [focusMobileSearchResult, forceSearchGlobal, onClose],
  );

  const handleMobileSearchResultKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
        return;
      }

      if (event.key === "Home" || event.key === "End") {
        const didFocus = focusMobileSearchResult(
          event.key === "Home" ? "first" : "last",
        );
        if (didFocus) {
          event.preventDefault();
        }
        return;
      }

      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        const didFocus = focusRelativeMobileSearchResult(
          event.currentTarget,
          event.key === "ArrowDown" ? 1 : -1,
        );
        if (didFocus) {
          event.preventDefault();
        }
      }
    },
    [focusMobileSearchResult, focusRelativeMobileSearchResult, onClose],
  );

  const isNestedSearchDialogOpen =
    addDialogTarget !== null || requestDialogTarget !== null;

  const handleMobileOverlayKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (isNestedSearchDialogOpen) {
        return;
      }

      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }

      if (event.key !== "Tab") {
        return;
      }

      const overlay = overlayRef.current;
      if (!overlay) {
        return;
      }

      const activeElement = document.activeElement;
      if (
        activeElement instanceof Element &&
        !overlay.contains(activeElement) &&
        activeElement.closest(
          "[data-slot='popover-content'], [data-slot='select-content'], [data-slot='dialog-content']",
        )
      ) {
        return;
      }

      const focusableElements = Array.from(
        overlay.querySelectorAll<HTMLElement>(
          "a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex='-1'])",
        ),
      ).filter((element) => element.offsetParent !== null);

      if (focusableElements.length === 0) {
        event.preventDefault();
        overlay.focus();
        return;
      }

      const firstElement = focusableElements[0];
      const lastElement = focusableElements[focusableElements.length - 1];

      if (!activeElement || !overlay.contains(activeElement)) {
        event.preventDefault();
        firstElement?.focus();
        return;
      }

      if (event.shiftKey && activeElement === firstElement) {
        event.preventDefault();
        lastElement?.focus();
        return;
      }

      if (!event.shiftKey && activeElement === lastElement) {
        event.preventDefault();
        firstElement?.focus();
      }
    },
    [isNestedSearchDialogOpen, onClose],
  );

  const handleOpenAddDialog = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: Facet) => {
      setAddDialogTarget({ result, facet });
      void ensureCatalogConfigReady(facet);
    },
    [ensureCatalogConfigReady],
  );

  const handleAddDialogSubmit = React.useCallback(
    async (
      result: MetadataTvdbSearchItem,
      facet: Facet,
      options: MetadataCatalogAddOptions,
    ) => {
      // The overlay stays up so a second title can go straight in; the success
      // toast owns the jump to the one that was just added.
      return addMetadataSearchResultToCatalog(result, facet, options, {
        onViewInCatalog: (titleId) => {
          const selectedLibrary = librariesByFacet[facet].find(
            (library) => library.id === options.libraryId,
          );
          resetGlobalSearch();
          onClose();
          onOpenOverview?.(viewFromFacet(facet), {
            id: titleId,
            slug: result.slug ?? null,
            libraryId: selectedLibrary?.id ?? options.libraryId ?? null,
            librarySlug: selectedLibrary?.slug ?? null,
          });
        },
      });
    },
    [
      addMetadataSearchResultToCatalog,
      librariesByFacet,
      onClose,
      onOpenOverview,
      resetGlobalSearch,
    ],
  );

  const handleRequestDialogSubmit = React.useCallback(
    async (
      result: MetadataTvdbSearchItem,
      facet: Facet,
      options: MetadataCatalogRequestOptions,
    ) => {
      const accepted = await requestMetadataSearchResult(
        result,
        facet,
        options,
      );
      if (accepted) {
        resetGlobalSearch();
        closingAfterSuccessfulActionRef.current = true;
      }
      return accepted;
    },
    [requestMetadataSearchResult, resetGlobalSearch],
  );

  const restoreMobileSearchInputFocus = React.useCallback(() => {
    if (typeof window === "undefined") {
      return;
    }

    window.requestAnimationFrame(() => inputRef.current?.focus());
  }, [inputRef]);

  const handleAddDialogOpenChange = React.useCallback(
    (open: boolean) => {
      if (open) {
        return;
      }
      setAddDialogTarget(null);
      // Adding no longer closes the overlay, so the caret always goes back to
      // the search box ready for the next title.
      restoreMobileSearchInputFocus();
    },
    [restoreMobileSearchInputFocus],
  );

  const handleRequestDialogOpenChange = React.useCallback(
    (open: boolean) => {
      if (open) {
        return;
      }
      setRequestDialogTarget(null);
      if (closingAfterSuccessfulActionRef.current) {
        closingAfterSuccessfulActionRef.current = false;
        onClose();
        return;
      }
      restoreMobileSearchInputFocus();
    },
    [onClose, restoreMobileSearchInputFocus],
  );

  const handleRouteCommandSelect = React.useCallback(
    (item: RouteCommandItem) => {
      resetGlobalSearch();
      onClose();
      item.onSelect();
    },
    [onClose, resetGlobalSearch],
  );

  const renderRouteCommandItem = React.useCallback(
    (item: RouteCommandItem) => {
      const Icon = item.icon ?? Search;
      const description = item.description.trim();
      const groupLabel = item.groupLabel?.trim() || null;
      const displayLabel = routeCommandDisplayLabel(item);
      const showDescription =
        description.length > 0 && description !== displayLabel.trim();
      const commandLabel = [
        displayLabel,
        showDescription ? description : null,
        groupLabel,
      ]
        .filter(Boolean)
        .join(": ");

      return (
        <SearchRouteCommandButton
          key={item.id}
          Icon={Icon}
          ariaLabel={commandLabel}
          description={description}
          displayLabel={displayLabel}
          onClick={() => handleRouteCommandSelect(item)}
          onKeyDown={handleMobileSearchResultKeyDown}
          resultAttribute="data-mobile-global-search-result"
          showDescription={showDescription}
          surface="mobile"
        />
      );
    },
    [handleMobileSearchResultKeyDown, handleRouteCommandSelect],
  );

  const renderCatalogItem = React.useCallback(
    (
      title: import("@/lib/types").TitleRecord,
      facet: "MOVIE" | "SERIES" | "ANIME",
    ) => {
      const targetView: ViewId =
        facet === "SERIES" ? "series" : facet === "ANIME" ? "anime" : "movies";
      const tvdbId = (title.externalIds ?? [])
        .find((externalId) => externalId.source.toLowerCase() === "tvdb")
        ?.value.trim();
      const posterUrl = selectPosterVariantUrl(title.posterUrl, "w70");
      const facetLabel = sectionLabelForFacet(t, facet);
      const libraryLabel = title.libraryName?.trim() || null;
      const statusLabel = title.contentStatus?.trim() || null;
      const secondaryParts = [
        title.year ? String(title.year) : null,
        libraryLabel && libraryLabel !== facetLabel ? libraryLabel : null,
        statusLabel,
        tvdbId ? `TVDB ${tvdbId}` : null,
      ].filter(Boolean);
      const viewTitleLabel = `${t("search.view")}: ${title.name}`;

      return (
        <SearchCatalogResultButton
          id={selectorId("global-search-catalog-result", facet, title.id)}
          key={title.id}
          onClick={() => {
            resetGlobalSearch();
            onOpenOverview?.(targetView, {
              id: title.id,
              slug: title.slug ?? null,
              libraryId: title.libraryId,
              librarySlug: title.librarySlug ?? null,
            });
          }}
          onKeyDown={handleMobileSearchResultKeyDown}
          ariaLabel={viewTitleLabel}
          createdAt={title.createdAt}
          emptyLabel={t("label.noArt")}
          externalIds={title.externalIds}
          facet={facet}
          facetLabel={facetLabel}
          metadataFetchedAt={title.metadataFetchedAt}
          monitoredLabel={
            title.monitored ? t("search.monitored") : t("search.unmonitored")
          }
          posterAlt={t("media.posterAlt", { name: title.name })}
          posterUrl={posterUrl}
          resultAttribute="data-mobile-global-search-result"
          secondaryParts={secondaryParts}
          surface="mobile"
          titleId={title.id}
          titleName={title.name}
          viewLabel={t("search.view")}
          year={title.year}
        />
      );
    },
    [handleMobileSearchResultKeyDown, onOpenOverview, resetGlobalSearch, t],
  );

  const renderMetadataItem = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: "MOVIE" | "SERIES" | "ANIME") => {
      const {
        actionTitle,
        disabled,
        isInCatalog,
        isUnavailable,
        opensRequestDialog,
      } = buildMetadataSearchActionState({
        isInCatalog: isMetadataSearchResultInCatalog(facet, result),
        canAdd:
          catalogQualityProfileOptions.length > 0 &&
          (librariesByFacet[facet].length > 0 ||
            rootFoldersByFacet[facet].some((rootFolder) =>
              Boolean(rootFolder.path.trim()),
            )),
        canRequest: requestableLibrariesByFacet[facet].length > 0,
        resultName: result.name,
        t,
      });
      const posterUrl = selectPosterVariantUrl(result.posterUrl, "w250");
      const handleMetadataAction = () => {
        if (disabled) {
          return;
        }

        if (opensRequestDialog) {
          setRequestDialogTarget({ result, facet });
          return;
        }

        handleOpenAddDialog(result, facet);
      };
      const actionKind = isInCatalog
        ? "inCatalog"
        : isUnavailable
          ? "unavailable"
          : opensRequestDialog
            ? "request"
            : "add";

      return (
        <SearchMetadataPosterButton
          id={globalSearchMetadataResultId(facet, result)}
          key={`${facet}-${result.smgId ?? result.tvdbId ?? result.name}`}
          onClick={handleMetadataAction}
          onKeyDown={handleMobileSearchResultKeyDown}
          disabled={disabled}
          actionKind={actionKind}
          actionTitle={actionTitle}
          facet={facet}
          imdbId={result.imdbId}
          name={result.name}
          posterUrl={posterUrl}
          resultAttribute="data-mobile-global-search-result"
          smgId={result.smgId}
          tvdbId={result.tvdbId}
          year={result.year}
          yearLabel={result.year ? result.year : t("label.yearUnknown")}
        />
      );
    },
    [
      handleOpenAddDialog,
      handleMobileSearchResultKeyDown,
      isMetadataSearchResultInCatalog,
      catalogQualityProfileOptions.length,
      librariesByFacet,
      rootFoldersByFacet,
      requestableLibrariesByFacet,
      t,
    ],
  );

  const renderMetadataSection = (
    items: MetadataTvdbSearchItem[],
    facet: Facet,
    _section: string,
    loading: boolean,
  ) => {
    if (!loading && items.length === 0) return null;
    const visibleItems = getVisibleMetadataResultsForFilters(
      activeFilters,
      items,
    );
    const hiddenItemCount = countHiddenMetadataResultsForFilters(
      activeFilters,
      items,
      visibleItems,
    );
    const facetConfig = FACET_REGISTRY.find((f) => f.id === facet);
    const facetLabel = facetConfig
      ? t(facetConfig.navLabelKey)
      : sectionLabelForFacet(t, facet);
    const viewAllFacetLabel = viewAllLabelForFacet(t, facet);
    const resultCountLabel =
      items.length === 1
        ? t("search.resultCountOne")
        : t("search.resultCountOther", { count: String(items.length) });
    return (
      <section key={`metadata-${facet}`} className="space-y-3">
        <div className="flex items-baseline justify-between gap-3">
          <div className="flex min-w-0 items-baseline gap-2">
            <h3 className="truncate text-[15px] font-bold text-[var(--scry-ink2)]">
              {facetLabel}
            </h3>
            <span className="shrink-0 text-xs text-[var(--scry-muted3)]">
              {loading ? t("search.metadataSearch") : resultCountLabel}
            </span>
          </div>
          {!loading && hiddenItemCount > 0 ? (
            <button
              type="button"
              className="text-xs font-medium text-[var(--scry-accent-ring)]"
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => {
                setActiveFilters([facet]);
                focusMobileSearchFilter(facet);
              }}
              aria-label={viewAllFacetLabel}
            >
              {viewAllFacetLabel}
            </button>
          ) : null}
        </div>
        {loading ? (
          <SearchSectionLoading compact label={t("label.loading")} />
        ) : (
          <HorizontalRail
            className="flex gap-3 overflow-x-auto pb-1 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
            fadeClassName="to-[var(--scry-bg)]"
          >
            {visibleItems.map((result) => renderMetadataItem(result, facet))}
          </HorizontalRail>
        )}
      </section>
    );
  };

  const showCatalogSection =
    canViewCatalog && (catalogSearchLoading || visibleCatalogCount > 0);
  const showMetadataSection = metadataSectionFacets.length > 0;
  const showRouteCommandSection = visibleRouteCommandResults.length > 0;
  const hiddenRouteCommandResultCount = countHiddenRouteCommandResultsForFilters(
    activeFilters,
    routeCommandResults,
    visibleRouteCommandResults,
  );
  const showSectionResults =
    showCatalogSection || showMetadataSection || showRouteCommandSection;

  return (
    <div
      id="mobile-global-search-panel"
      ref={overlayRef}
      data-slot="mobile-global-search-overlay"
      className="fixed inset-0 z-50 flex flex-col items-center bg-[rgba(2,4,10,0.66)] px-3 pb-safe pt-safe-comfort text-foreground backdrop-blur-md"
      role={isNestedSearchDialogOpen ? undefined : "dialog"}
      aria-modal={isNestedSearchDialogOpen ? undefined : true}
      aria-label={t("search.title")}
      aria-describedby="mobile-global-search-description"
      tabIndex={-1}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
      onKeyDown={handleMobileOverlayKeyDown}
    >
      <p id="mobile-global-search-description" className="sr-only">
        {searchSubtitle}
      </p>
      <p
        id="mobile-global-search-status"
        className="sr-only"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {searchStatusLabel}
      </p>
      <div
        data-slot="mobile-global-search-panel"
        className="flex min-h-0 w-full max-w-[920px] flex-1 flex-col overflow-hidden rounded-[18px] border border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] shadow-[0_30px_90px_rgba(0,0,0,0.62)]"
        onMouseDown={(event) => event.stopPropagation()}
      >
        {/* Sticky search header */}
        <div
          data-slot="mobile-global-search-header"
          className="flex items-center gap-[13px] border-b border-[var(--scry-border)] px-[14px] py-4"
        >
          <Search className="h-[21px] w-[21px] shrink-0 text-[var(--scry-accent-ring)]" />
          <input
            type="search"
            ref={setMobileSearchInputRef}
            value={globalSearch}
            onChange={(e) => setGlobalSearch(e.target.value)}
            onKeyDown={handleMobileSearchInputKeyDown}
            className="h-8 min-w-0 flex-1 appearance-none border-0 !bg-transparent px-0 text-[16px] text-[var(--scry-ink2)] shadow-none outline-none placeholder:text-[16px] placeholder:text-[var(--scry-muted3)] focus:!bg-transparent focus:outline-none focus-visible:!bg-transparent focus-visible:ring-0 [&::-webkit-search-cancel-button]:hidden [&::-webkit-search-decoration]:hidden"
            style={{
              WebkitAppearance: "none",
              background: "transparent",
            }}
            placeholder={searchOverlayPlaceholder}
            aria-label={searchOverlayPlaceholder}
            aria-controls="mobile-global-search-results-panel"
            aria-describedby="mobile-global-search-description mobile-global-search-status"
            autoComplete="off"
            data-1p-ignore="true"
            data-lpignore="true"
            data-bwignore="true"
            data-form-type="other"
            data-protonpass-ignore="true"
            autoFocus
          />
          {globalSearch ? (
            <IconButton
              type="button"
              label={t("label.clear")}
              appearance="ghost"
              className="h-[26px] w-[26px] flex-none rounded-[7px] bg-[var(--scry-kbdbg)]"
              onClick={() => {
                clearGlobalSearch();
                inputRef.current?.focus();
              }}
            >
              <Eraser className="h-3.5 w-3.5" />
            </IconButton>
          ) : null}
          <kbd
            aria-hidden="true"
            className="flex h-[30px] flex-none items-center rounded-[7px] border border-[var(--scry-kbdbd)] bg-[var(--scry-kbdbg)] px-2 text-[11px] font-medium leading-none text-[var(--scry-faint2)]"
          >
            ESC
          </kbd>
          <IconButton
            type="button"
            onClick={onClose}
            label={t("label.close")}
            appearance="ghost"
            className="h-[34px] w-[34px] flex-none rounded-lg bg-[var(--scry-kbdbg)]"
            aria-keyshortcuts="Escape"
          >
            <X className="h-4 w-4" />
          </IconButton>
        </div>

        <div
          data-slot="mobile-global-search-tabs"
          className="flex gap-x-5 gap-y-1 overflow-x-auto border-b border-[var(--scry-border)] px-[14px] py-2 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
          role="group"
          aria-label={t("search.title")}
        >
          {mobileSearchTabs.map((tab) => (
            <UnderlineFilterButton
              id={`mobile-global-search-tab-${tab.key}`}
              key={tab.key}
              ref={(node) => {
                mobileSearchTabRefs.current[tab.key] = node;
              }}
              selected={isGlobalSearchFilterSelected(activeFilters, tab.key)}
              label={tab.label}
              count={tab.count}
              aria-controls="mobile-global-search-results-panel"
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => toggleMobileSearchFilter(tab.key)}
              onKeyDown={(event) =>
                handleMobileSearchTabKeyDown(event, tab.key)
              }
            />
          ))}
        </div>

        {/* Scrollable results */}
        <div
          ref={mobileSearchResultsRef}
          id="mobile-global-search-results-panel"
          data-slot="mobile-global-search-results"
          role="region"
          aria-label={t("search.title")}
          className="min-h-0 flex-1 overflow-y-auto px-[14px] py-[18px] [scrollbar-color:var(--scry-border2)_transparent] [scrollbar-width:thin] [&::-webkit-scrollbar]:w-2.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:border-[3px] [&::-webkit-scrollbar-thumb]:border-transparent [&::-webkit-scrollbar-thumb]:bg-[var(--scry-border2)] [&::-webkit-scrollbar-thumb]:bg-clip-content"
        >
          {showSectionResults ? (
            <div className="space-y-6">
              {showCatalogSection ? (
                <section className="space-y-3">
                  <div className="flex items-baseline justify-between gap-3">
                    <div className="min-w-0">
                      <h3 className="text-[15px] font-bold text-[var(--scry-ink2)]">
                        {t("search.inLibrary")}
                      </h3>
                      <p className="truncate text-xs text-[var(--scry-muted3)]">
                        {t("search.alreadyInCollection")}
                      </p>
                    </div>
                    <div className="flex shrink-0 items-center gap-3">
                      {!catalogSearchLoading &&
                      hiddenCatalogResultCount > 0 ? (
                        <button
                          type="button"
                          className="text-xs font-medium text-[var(--scry-accent-ring)]"
                          onMouseDown={(event) => event.preventDefault()}
                          onClick={() => {
                            setActiveFilters(["library"]);
                            focusMobileSearchFilter("library");
                          }}
                          aria-label={`${t("search.viewAll")} ${t("search.inLibrary")}`}
                        >
                          {t("search.viewAll")}
                        </button>
                      ) : null}
                      <span className="text-xs font-medium tabular-nums text-[var(--scry-muted3)]">
                        {visibleCatalogCount === 1
                          ? t("search.resultCountOne")
                          : t("search.resultCountOther", {
                              count: String(visibleCatalogCount),
                            })}
                      </span>
                    </div>
                  </div>
                  {catalogSearchLoading ? (
                    <SearchSectionLoading compact label={t("label.loading")} />
                  ) : visibleCatalogResults.length === 0 ? (
                    <p className="rounded-[12px] border border-dashed border-[var(--scry-border2)] bg-[var(--scry-surfC)] px-4 py-5 text-sm text-[var(--scry-muted3)]">
                      {!hasMinimumGlobalSearchQuery
                        ? searchMinimumQueryHint
                        : t("search.noCatalogMatches")}
                    </p>
                  ) : (
                    <div className="space-y-2">
                      {visibleCatalogResults.map(({ facet, title }) =>
                        renderCatalogItem(title, facet),
                      )}
                    </div>
                  )}
                </section>
              ) : null}

              {showMetadataSection ? (
                <div className="space-y-5">
                  {metadataSectionFacets.map((f) =>
                    renderMetadataSection(
                      metadataSearchResults[f.metadataKey] ?? [],
                      f.id,
                      f.metadataKey,
                      metadataSearchLoading,
                    ),
                  )}
                </div>
              ) : null}
              {visibleRouteCommandResults.length > 0 ? (
                <section className="space-y-3">
                  <div className="flex items-baseline justify-between gap-3">
                    <div className="min-w-0">
                      <h3 className="truncate text-[15px] font-bold text-[var(--scry-ink2)]">
                        {t("search.actionsAndSettings")}
                      </h3>
                      <p className="truncate text-xs text-[var(--scry-muted3)]">
                        {routeCommandResults.length === 1
                          ? t("search.resultCountOne")
                          : t("search.resultCountOther", {
                              count: String(routeCommandResults.length),
                        })}
                      </p>
                    </div>
                    {hiddenRouteCommandResultCount > 0 ? (
                      <button
                        type="button"
                        className="shrink-0 text-xs font-medium text-[var(--scry-accent-ring)]"
                        onMouseDown={(event) => event.preventDefault()}
                        onClick={() => {
                          setActiveFilters(["actions"]);
                          focusMobileSearchFilter("actions");
                        }}
                        aria-label={`${t("search.viewAll")} ${t("search.actionsAndSettings")}`}
                      >
                        {t("search.viewAll")}
                      </button>
                    ) : null}
                  </div>
                  <div className="space-y-3">
                    {groupRouteCommandItems(visibleRouteCommandResults).map(
                      (group) => (
                        <div
                          key={group.groupLabel ?? "ungrouped"}
                          className="space-y-2"
                        >
                          {group.groupLabel ? (
                            <p className="text-[11px] font-semibold uppercase tracking-wide text-[var(--scry-muted3)]">
                              {group.groupLabel}
                            </p>
                          ) : null}
                          <div className="space-y-2">
                            {group.items.map(renderRouteCommandItem)}
                          </div>
                        </div>
                      ),
                    )}
                  </div>
                </section>
              ) : null}
              <SearchFooterTip
                canViewCatalog={canViewCatalog}
                footerTip={t("search.footerTip")}
                searchTipsLabel={t("search.searchTips")}
                surface="mobile"
                tipIndexers={t("search.tipIndexers")}
                tipTabs={searchTipTabs}
                tipTitles={searchTipTitles}
              />
            </div>
          ) : searching ? (
            <div className="py-6">
              <SearchSectionLoading compact label={t("label.searching")} />
            </div>
          ) : trimmedGlobalSearch ? (
            <SearchEmptyState
              description={
                !hasMinimumGlobalSearchQuery
                  ? searchMinimumQueryHint
                  : searchEmptyHint
              }
              icon={hasMinimumGlobalSearchQuery ? "searchX" : "search"}
              title={
                !hasMinimumGlobalSearchQuery
                  ? t("search.minimumQueryTitle")
                  : t("search.noMatchesFor", { query: trimmedGlobalSearch })
              }
            />
          ) : (
            <SearchEmptyState
              description={searchEmptyHint}
              icon="search"
              title={searchOverlayPlaceholder}
            />
          )}
        </div>
      </div>
      <AddToCatalogDialog
        open={addDialogTarget !== null}
        onOpenChange={handleAddDialogOpenChange}
        result={addDialogTarget?.result ?? EMPTY_SEARCH_RESULT}
        facet={addDialogTarget?.facet ?? "SERIES"}
        catalogQualityProfileOptions={catalogQualityProfileOptions}
        catalogConfigLoading={
          Boolean(addDialogTarget) &&
          catalogConfigLoading &&
          !isAddDialogConfigReady
        }
        defaultQualityProfileId={resolveDefaultQualityProfileIdForFacet(
          addDialogTarget?.facet ?? "SERIES",
        )}
        manageableLibraries={
          librariesByFacet[addDialogTarget?.facet ?? "SERIES"]
        }
        rootFolderOptions={
          rootFoldersByFacet[addDialogTarget?.facet ?? "SERIES"]
        }
        onAdd={handleAddDialogSubmit}
      />
      <RequestMediaDialog
        open={requestDialogTarget !== null}
        onOpenChange={handleRequestDialogOpenChange}
        result={requestDialogTarget?.result ?? EMPTY_SEARCH_RESULT}
        facet={requestDialogTarget?.facet ?? "SERIES"}
        requestableLibraries={
          requestableLibrariesByFacet[requestDialogTarget?.facet ?? "SERIES"]
        }
        qualityProfileOptions={catalogQualityProfileOptions}
        onRequest={handleRequestDialogSubmit}
      />
    </div>
  );
}
