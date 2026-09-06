import * as React from "react";
import {
  ChevronsUpDown,
  Eraser,
  LogOut,
  Search,
  ShieldCheck,
  User,
  X,
} from "lucide-react";
import { createPortal } from "react-dom";
import { useNavigate } from "react-router";
import { useClient } from "urql";
import { HorizontalRail } from "@/components/common/horizontal-scroll-fade";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import {
  groupRouteCommandItems,
  routeCommandDisplayLabel,
  type RouteCommandItem,
} from "@/components/common/route-command-types";
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
  countHiddenRouteCommandResultsForFilters,
  countMetadataResults,
  countVisibleCatalogResults,
  filterGlobalSearchRouteCommands,
  GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT,
  GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT,
  getMetadataSectionFacetsForFilters,
  getVisibleCatalogFacetsForFilters,
  getVisibleCatalogResultsForFilters,
  getVisibleRouteCommandResultsForFilters,
  isGlobalSearchFilterSelected,
  normalizeGlobalSearchFilterSelection,
  toggleGlobalSearchFilterSelection,
  type GlobalSearchFilterKey,
  type GlobalSearchTabKey,
} from "@/components/root/global-search-model";
import { useTranslate } from "@/lib/context/translate-context";
import { linkedAccountsQuery } from "@/lib/graphql/queries";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import type { LinkedAccount } from "@/lib/types/settings";
import type {
  MetadataCatalogAddOptions,
  MetadataCatalogRequestOptions,
} from "@/lib/hooks/use-global-search";
import { useAuth } from "@/lib/hooks/use-auth";
import {
  ROOT_SHELL_MOBILE_BREAKPOINT,
  useIsMobile,
} from "@/lib/hooks/use-mobile";
import { MobileSearchOverlay } from "@/components/root/mobile-search-overlay";
import {
  sectionLabelForFacet,
  viewFromFacet,
} from "@/lib/facets/helpers";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { useSearchContext } from "@/lib/context/search-context";
import { cn } from "@/lib/utils";
import {
  ENABLE_FORM_LOGIN_LOCATION_STATE,
  buildViewPath,
} from "@/lib/utils/routing";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  hasAnyAppPermission,
  hasAnyLibraryPermission,
} from "@/lib/utils/permissions";
import {
  globalSearchMetadataResultId,
  selectorId,
} from "@/lib/utils/dom-ids";
import {
  AddToCatalogDialog,
  EMPTY_SEARCH_RESULT,
} from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";

type RootHeaderProps = {
  mobileNavigation?: React.ReactNode;
  routeCommandItems?: RouteCommandItem[];
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
};

function hashAccountBadgeSeed(seed: string): number {
  let hash = 2166136261;
  for (let index = 0; index < seed.length; index += 1) {
    hash ^= seed.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function buildAccountBadgeStyle(seed: string): React.CSSProperties {
  const hash = hashAccountBadgeSeed(seed || "?");
  const hue = hash % 360;
  const secondaryHue = (hue + 42 + ((hash >>> 8) % 48)) % 360;

  return {
    background: `linear-gradient(135deg, hsl(${hue} 82% 58%), hsl(${secondaryHue} 86% 62%))`,
    boxShadow: `inset 0 1px 0 rgba(255,255,255,0.28), 0 8px 18px hsla(${hue},82%,42%,0.28)`,
  };
}

function selectAccountAvatar(accounts: LinkedAccount[]): LinkedAccount | null {
  const hasAvatar = (account: LinkedAccount) =>
    Boolean(account.avatarUrl?.trim());

  return (
    accounts.find(
      (account) => account.provider === "JELLYFIN" && hasAvatar(account),
    ) ??
    accounts.find(
      (account) => account.provider === "PLEX" && hasAvatar(account),
    ) ??
    null
  );
}

export const RootHeader = React.memo(function RootHeader({
  mobileNavigation,
  routeCommandItems = [],
  onOpenOverview,
}: RootHeaderProps) {
  const searchState = useSearchContext();
  const {
    resolveDefaultQualityProfileIdForFacet,
    addMetadataSearchResultToCatalog,
    requestMetadataSearchResult,
    clearGlobalSearch,
    resetGlobalSearch,
    openGlobalSearchPanel,
    closeGlobalSearchPanel,
    forceSearchGlobal,
    setGlobalSearch,
    globalSearchInputRef,
    isMetadataSearchResultInCatalog,
    catalogQualityProfileOptions,
    catalogConfigLoading,
    ensureCatalogConfigReady,
    isCatalogConfigReady,
    librariesByFacet,
    rootFoldersByFacet,
    requestableLibrariesByFacet,
    catalogSearchResults,
    catalogSearchLoading,
    metadataSearchResults,
    metadataSearchLoading,
    isGlobalSearchPanelOpen,
    globalSearch,
    searching,
  } = searchState;
  const t = useTranslate();
  const isMobile = useIsMobile(ROOT_SHELL_MOBILE_BREAKPOINT);
  const navigate = useNavigate();
  const client = useClient();
  const { token, user, logout, effectiveFormLoginEnabled } = useAuth();
  const canViewCatalog = user
    ? hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.view)
    : false;
  // Form login off means nobody has to sign in yet. Offer a one-click jump to
  // Security so the admin who can turn it on does not have to hunt for it.
  const canEnableFormLogin =
    effectiveFormLoginEnabled === false &&
    hasAnyAppPermission(user, [APP_PERMISSIONS.manageUsers]);
  const searchTriggerPlaceholder = canViewCatalog
    ? t("search.globalPlaceholder")
    : t("search.globalPlaceholderNoLibrary");
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
  const searchShortcutHint = React.useMemo(() => {
    if (typeof navigator === "undefined") {
      return t("search.shortcutHint");
    }
    const platformSignal =
      `${navigator.platform} ${navigator.userAgent}`.toLowerCase();
    return platformSignal.includes("mac") ||
      platformSignal.includes("iphone") ||
      platformSignal.includes("ipad") ||
      platformSignal.includes("ipod")
      ? t("search.shortcutHint")
      : t("search.shortcutHintControl");
  }, [t]);
  const headerRef = React.useRef<HTMLElement>(null);
  const searchShellRef = React.useRef<HTMLDivElement>(null);
  const searchTriggerRef = React.useRef<HTMLButtonElement>(null);
  const searchPanelRef = React.useRef<HTMLDivElement>(null);
  const searchResultsRef = React.useRef<HTMLDivElement>(null);
  const wasGlobalSearchPanelOpenRef = React.useRef(isGlobalSearchPanelOpen);
  const desktopSearchTabRefs = React.useRef<
    Partial<Record<GlobalSearchTabKey, HTMLButtonElement | null>>
  >({});
  const lastScrollYRef = React.useRef(0);
  const [accountMenuOpen, setAccountMenuOpen] = React.useState(false);
  const [accountLinkedAccounts, setAccountLinkedAccounts] = React.useState<
    LinkedAccount[]
  >([]);
  const [desktopSearchFilters, setDesktopSearchFilters] = React.useState<
    GlobalSearchFilterKey[]
  >([]);
  const [mobileHeaderHeight, setMobileHeaderHeight] = React.useState(0);
  const [isMobileHeaderVisible, setIsMobileHeaderVisible] =
    React.useState(true);
  React.useEffect(() => {
    if (!user?.id) {
      setAccountLinkedAccounts([]);
      return;
    }

    let cancelled = false;
    void client
      .query<{ linkedAccounts?: LinkedAccount[] }>(linkedAccountsQuery, {
        userId: user.id,
      })
      .toPromise()
      .then((result) => {
        if (cancelled) {
          return;
        }
        setAccountLinkedAccounts(
          result.error ? [] : (result.data?.linkedAccounts ?? []),
        );
      })
      .catch(() => {
        if (!cancelled) {
          setAccountLinkedAccounts([]);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [client, user?.id]);
  const readPageScrollTop = React.useCallback(() => {
    if (typeof window === "undefined") {
      return 0;
    }

    return Math.max(
      window.scrollY,
      document.scrollingElement?.scrollTop ?? 0,
      document.documentElement?.scrollTop ?? 0,
      document.body?.scrollTop ?? 0,
    );
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
        desktopSearchFilters,
        routeCommandResults,
        GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT,
      ),
    [desktopSearchFilters, routeCommandResults],
  );
  const visibleCatalogFacets = React.useMemo(
    () => getVisibleCatalogFacetsForFilters(desktopSearchFilters, canViewCatalog),
    [canViewCatalog, desktopSearchFilters],
  );
  const metadataSectionFacets = React.useMemo(
    () =>
      getMetadataSectionFacetsForFilters({
        selectedFilters: desktopSearchFilters,
        metadataSearchLoading,
        metadataResultCounts,
      }),
    [desktopSearchFilters, metadataResultCounts, metadataSearchLoading],
  );
  const visibleCatalogCount = React.useMemo(
    () =>
      countVisibleCatalogResults(visibleCatalogFacets, catalogSearchSections),
    [catalogSearchSections, visibleCatalogFacets],
  );
  const visibleCatalogResultCount = canViewCatalog
    ? catalogSearchResults.length
    : 0;
  const visibleCatalogResults = React.useMemo(() => {
    return getVisibleCatalogResultsForFilters({
      selectedFilters: desktopSearchFilters,
      canViewCatalog,
      catalogSearchSections,
      visibleCatalogFacets,
      allLimit: GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT,
    });
  }, [
    canViewCatalog,
    catalogSearchSections,
    desktopSearchFilters,
    visibleCatalogFacets,
  ]);
  const hiddenCatalogResultCount = countHiddenCatalogResultsForFilters(
    desktopSearchFilters,
    visibleCatalogCount,
    visibleCatalogResults,
  );
  const showCatalogSection =
    canViewCatalog && (catalogSearchLoading || visibleCatalogCount > 0);
  const showRouteCommandSection = visibleRouteCommandResults.length > 0;
  const hiddenRouteCommandResultCount = countHiddenRouteCommandResultsForFilters(
    desktopSearchFilters,
    routeCommandResults,
    visibleRouteCommandResults,
  );
  const showSectionResults =
    showCatalogSection ||
    metadataSectionFacets.length > 0 ||
    showRouteCommandSection;
  const desktopSearchTabs = React.useMemo(
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
  const [addDialogTarget, setAddDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
  const [requestDialogTarget, setRequestDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
  const closingRequestDialogAfterSuccessfulActionRef = React.useRef(false);
  const isAddDialogConfigReady = addDialogTarget
    ? isCatalogConfigReady(addDialogTarget.facet)
    : true;

  const handleOpenAddDialog = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: Facet) => {
      setAddDialogTarget({ result, facet });
      void ensureCatalogConfigReady(facet);
    },
    [ensureCatalogConfigReady],
  );

  const handleOpenRequestDialog = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: Facet) => {
      setRequestDialogTarget({ result, facet });
    },
    [],
  );

  const restoreGlobalSearchInputFocus = React.useCallback(() => {
    if (isMobile || typeof window === "undefined") {
      return;
    }

    window.requestAnimationFrame(() => {
      globalSearchInputRef.current?.focus();
      globalSearchInputRef.current?.select();
    });
  }, [globalSearchInputRef, isMobile]);

  const handleAddDialogOpenChange = React.useCallback(
    (open: boolean) => {
      if (open) {
        return;
      }
      setAddDialogTarget(null);
      // Adding no longer navigates away, so the caret always goes back to the
      // search box ready for the next title.
      restoreGlobalSearchInputFocus();
    },
    [restoreGlobalSearchInputFocus],
  );

  const handleRequestDialogOpenChange = React.useCallback(
    (open: boolean) => {
      if (open) {
        return;
      }
      setRequestDialogTarget(null);
      if (closingRequestDialogAfterSuccessfulActionRef.current) {
        closingRequestDialogAfterSuccessfulActionRef.current = false;
        return;
      }
      restoreGlobalSearchInputFocus();
    },
    [restoreGlobalSearchInputFocus],
  );

  const handleAddDialogSubmit = React.useCallback(
    async (
      result: MetadataTvdbSearchItem,
      facet: Facet,
      options: MetadataCatalogAddOptions,
    ) => {
      // The panel stays open so a second title can go straight in; the success
      // toast owns the jump to the one that was just added.
      return addMetadataSearchResultToCatalog(result, facet, options, {
        onViewInCatalog: (titleId) => {
          const selectedLibrary = librariesByFacet[facet].find(
            (library) => library.id === options.libraryId,
          );
          resetGlobalSearch();
          globalSearchInputRef.current?.blur();
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
      globalSearchInputRef,
      librariesByFacet,
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
        globalSearchInputRef.current?.blur();
        closingRequestDialogAfterSuccessfulActionRef.current = true;
      }
      return accepted;
    },
    [globalSearchInputRef, requestMetadataSearchResult, resetGlobalSearch],
  );

  const handleRouteCommandSelect = React.useCallback(
    (item: RouteCommandItem) => {
      resetGlobalSearch();
      globalSearchInputRef.current?.blur();
      item.onSelect();
    },
    [globalSearchInputRef, resetGlobalSearch],
  );

  const handleOpenProfile = React.useCallback(() => {
    setAccountMenuOpen(false);
    navigate(buildViewPath("settings", "profile"));
  }, [navigate]);

  const handleEnableLogin = React.useCallback(() => {
    setAccountMenuOpen(false);
    navigate(buildViewPath("settings", "security"), {
      state: ENABLE_FORM_LOGIN_LOCATION_STATE,
    });
  }, [navigate]);

  const handleLogout = React.useCallback(() => {
    setAccountMenuOpen(false);
    logout();
    navigate("/login", { replace: true });
  }, [logout, navigate]);

  const handleSearchChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setGlobalSearch(event.target.value);
      openGlobalSearchPanel(isMobile || undefined);
    },
    [isMobile, openGlobalSearchPanel, setGlobalSearch],
  );

  const restoreSearchTriggerFocus = React.useCallback(() => {
    if (typeof window === "undefined") {
      return;
    }

    if (isMobile) {
      window.requestAnimationFrame(() => searchTriggerRef.current?.focus());
      return;
    }

    const focusTrigger = () => {
      const activeElement = document.activeElement;
      if (
        activeElement &&
        activeElement !== document.body &&
        !searchPanelRef.current?.contains(activeElement)
      ) {
        return;
      }
      searchTriggerRef.current?.focus();
    };

    window.requestAnimationFrame(() => {
      focusTrigger();
      window.setTimeout(focusTrigger, 0);
      window.setTimeout(focusTrigger, 100);
    });
  }, [isMobile]);

  const handleSearchEscape = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key !== "Escape") {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      closeGlobalSearchPanel();
      globalSearchInputRef.current?.blur();
      restoreSearchTriggerFocus();
    },
    [closeGlobalSearchPanel, globalSearchInputRef, restoreSearchTriggerFocus],
  );

  const handleClearSearch = React.useCallback(() => {
    clearGlobalSearch();
    globalSearchInputRef.current?.focus();
  }, [clearGlobalSearch, globalSearchInputRef]);

  const handleCloseGlobalSearchPanel = React.useCallback(() => {
    closeGlobalSearchPanel();
    globalSearchInputRef.current?.blur();
    restoreSearchTriggerFocus();
  }, [closeGlobalSearchPanel, globalSearchInputRef, restoreSearchTriggerFocus]);

  React.useEffect(() => {
    const wasOpen = wasGlobalSearchPanelOpenRef.current;
    wasGlobalSearchPanelOpenRef.current = isGlobalSearchPanelOpen;
    if (
      wasOpen &&
      !isGlobalSearchPanelOpen &&
      addDialogTarget === null &&
      requestDialogTarget === null
    ) {
      restoreSearchTriggerFocus();
    }
  }, [
    addDialogTarget,
    isGlobalSearchPanelOpen,
    requestDialogTarget,
    restoreSearchTriggerFocus,
  ]);

  const handleSearchPanelKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (event.key !== "Tab") {
        return;
      }

      const panel = searchPanelRef.current;
      if (!panel) {
        return;
      }

      const activeElement = document.activeElement;
      if (
        activeElement instanceof Element &&
        !panel.contains(activeElement) &&
        activeElement.closest(
          "[data-slot='popover-content'], [data-slot='select-content'], [data-slot='dialog-content']",
        )
      ) {
        return;
      }

      const focusableElements = Array.from(
        panel.querySelectorAll<HTMLElement>(
          "a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex='-1'])",
        ),
      ).filter((element) => element.offsetParent !== null);

      if (focusableElements.length === 0) {
        event.preventDefault();
        panel.focus();
        return;
      }

      const firstElement = focusableElements[0];
      const lastElement = focusableElements[focusableElements.length - 1];

      if (!activeElement || !panel.contains(activeElement)) {
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
    [],
  );

  const getSearchResultButtons = React.useCallback(() => {
    const resultRoot = searchResultsRef.current;
    if (!resultRoot) {
      return [];
    }

    return Array.from(
      resultRoot.querySelectorAll<HTMLButtonElement>(
        "[data-global-search-result='true']:not(:disabled)",
      ),
    );
  }, []);

  const focusSearchResult = React.useCallback(
    (position: "first" | "last") => {
      const buttons = getSearchResultButtons();
      if (buttons.length === 0) {
        return false;
      }

      buttons[position === "first" ? 0 : buttons.length - 1]?.focus();
      return true;
    },
    [getSearchResultButtons],
  );

  const focusRelativeSearchResult = React.useCallback(
    (currentButton: HTMLButtonElement, delta: 1 | -1) => {
      const buttons = getSearchResultButtons();
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
    [getSearchResultButtons],
  );

  const handleDesktopSearchKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "Escape") {
        handleSearchEscape(event);
        return;
      }

      if (event.key === "Enter") {
        if (event.nativeEvent.isComposing) {
          return;
        }
        event.preventDefault();
        if (focusSearchResult("first")) {
          return;
        }
        void forceSearchGlobal();
        return;
      }

      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        const didFocus = focusSearchResult(
          event.key === "ArrowDown" ? "first" : "last",
        );
        if (didFocus) {
          event.preventDefault();
        }
      }
    },
    [focusSearchResult, forceSearchGlobal, handleSearchEscape],
  );

  const handleSearchResultKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (event.key === "Escape") {
        event.preventDefault();
        handleCloseGlobalSearchPanel();
        return;
      }

      if (event.key === "Home" || event.key === "End") {
        const didFocus = focusSearchResult(
          event.key === "Home" ? "first" : "last",
        );
        if (didFocus) {
          event.preventDefault();
        }
        return;
      }

      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        const didFocus = focusRelativeSearchResult(
          event.currentTarget,
          event.key === "ArrowDown" ? 1 : -1,
        );
        if (didFocus) {
          event.preventDefault();
        }
      }
    },
    [
      focusRelativeSearchResult,
      focusSearchResult,
      handleCloseGlobalSearchPanel,
    ],
  );

  const focusDesktopSearchFilter = React.useCallback(
    (nextTab: GlobalSearchTabKey) => {
      const nextTabElement = desktopSearchTabRefs.current[nextTab];
      nextTabElement?.focus();
      nextTabElement?.scrollIntoView({ block: "nearest", inline: "nearest" });
    },
    [],
  );

  const toggleDesktopSearchFilter = React.useCallback(
    (key: GlobalSearchTabKey) => {
      setDesktopSearchFilters((selectedFilters) =>
        toggleGlobalSearchFilterSelection(
          selectedFilters,
          key,
          desktopSearchTabs,
        ),
      );
    },
    [desktopSearchTabs],
  );

  const handleDesktopSearchTabKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent<HTMLButtonElement>,
      currentTab: GlobalSearchTabKey,
    ) => {
      const tabKeys = desktopSearchTabs.map((tab) => tab.key);
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
      focusDesktopSearchFilter(nextTab);
    },
    [desktopSearchTabs, focusDesktopSearchFilter],
  );

  React.useEffect(() => {
    setDesktopSearchFilters((selectedFilters) =>
      normalizeGlobalSearchFilterSelection(selectedFilters, desktopSearchTabs),
    );
  }, [desktopSearchTabs]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen) {
      setDesktopSearchFilters([]);
    }
  }, [isGlobalSearchPanelOpen]);

  React.useEffect(() => {
    if (isGlobalSearchPanelOpen && accountMenuOpen) {
      setAccountMenuOpen(false);
    }
  }, [accountMenuOpen, isGlobalSearchPanelOpen]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }
    searchResultsRef.current?.scrollTo({ left: 0, top: 0 });
  }, [desktopSearchFilters, globalSearch, isGlobalSearchPanelOpen, isMobile]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const frameId = window.requestAnimationFrame(() => {
      globalSearchInputRef.current?.focus();
      globalSearchInputRef.current?.select();
    });
    return () => window.cancelAnimationFrame(frameId);
  }, [globalSearchInputRef, isGlobalSearchPanelOpen, isMobile]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const originalOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = originalOverflow;
    };
  }, [isGlobalSearchPanelOpen, isMobile]);

  React.useEffect(() => {
    const headerElement = headerRef.current;
    if (!headerElement) {
      return;
    }

    const updateHeaderHeight = () => {
      const height = headerElement.getBoundingClientRect().height;
      document.documentElement.style.setProperty(
        "--root-header-height",
        `${height}px`,
      );
      setMobileHeaderHeight(isMobile ? height : 0);
    };

    updateHeaderHeight();

    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", updateHeaderHeight);
      return () => window.removeEventListener("resize", updateHeaderHeight);
    }

    const resizeObserver = new ResizeObserver(() => updateHeaderHeight());
    resizeObserver.observe(headerElement);
    return () => {
      resizeObserver.disconnect();
      document.documentElement.style.removeProperty("--root-header-height");
    };
  }, [isMobile]);

  React.useEffect(() => {
    if (!isMobile || accountMenuOpen || isGlobalSearchPanelOpen) {
      setIsMobileHeaderVisible(true);
    }
  }, [accountMenuOpen, isGlobalSearchPanelOpen, isMobile]);

  React.useEffect(() => {
    if (!isMobile) {
      return;
    }

    lastScrollYRef.current = readPageScrollTop();
    let frameId: number | null = null;

    const updateHeaderVisibility = () => {
      frameId = null;
      const nextScrollY = readPageScrollTop();
      const previousScrollY = lastScrollYRef.current;
      const delta = nextScrollY - previousScrollY;

      if (Math.abs(delta) < 12) {
        return;
      }

      lastScrollYRef.current = nextScrollY;

      if (nextScrollY <= Math.max(mobileHeaderHeight, 24)) {
        setIsMobileHeaderVisible(true);
        return;
      }

      if (accountMenuOpen || isGlobalSearchPanelOpen) {
        setIsMobileHeaderVisible(true);
        return;
      }

      setIsMobileHeaderVisible(delta < 0);
    };

    const handleScroll = () => {
      if (frameId !== null) {
        return;
      }
      frameId = window.requestAnimationFrame(updateHeaderVisibility);
    };

    const scrollTargets = new Set<EventTarget>([window, document]);
    if (document.scrollingElement) {
      scrollTargets.add(document.scrollingElement);
    }
    for (const target of scrollTargets) {
      target.addEventListener("scroll", handleScroll, { passive: true });
    }

    return () => {
      if (frameId !== null) {
        window.cancelAnimationFrame(frameId);
      }
      for (const target of scrollTargets) {
        target.removeEventListener("scroll", handleScroll);
      }
    };
  }, [
    accountMenuOpen,
    isGlobalSearchPanelOpen,
    isMobile,
    mobileHeaderHeight,
    readPageScrollTop,
  ]);

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
          onKeyDown={handleSearchResultKeyDown}
          resultAttribute="data-global-search-result"
          showDescription={showDescription}
          surface="desktop"
        />
      );
    },
    [handleRouteCommandSelect, handleSearchResultKeyDown],
  );

  const renderCatalogSection = React.useCallback(
    (items: import("@/lib/types").TitleRecord[], facet: Facet) => {
      return items.map((title) => {
        const targetView: ViewId = viewFromFacet(facet);
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
              globalSearchInputRef.current?.blur();
              onOpenOverview?.(targetView, {
                id: title.id,
                slug: title.slug ?? null,
                libraryId: title.libraryId,
                librarySlug: title.librarySlug ?? null,
              });
            }}
            onKeyDown={handleSearchResultKeyDown}
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
            resultAttribute="data-global-search-result"
            secondaryParts={secondaryParts}
            surface="desktop"
            titleId={title.id}
            titleName={title.name}
            viewLabel={t("search.view")}
            year={title.year}
          />
        );
      });
    },
    [
      globalSearchInputRef,
      handleSearchResultKeyDown,
      onOpenOverview,
      resetGlobalSearch,
      t,
    ],
  );

  const handleSearchPanelBackdropMouseDown = handleCloseGlobalSearchPanel;

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const handleGlobalSearchPanelPointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      const targetElement = target instanceof Element ? target : null;
      if (target && searchShellRef.current?.contains(target)) {
        return;
      }
      if (target && searchPanelRef.current?.contains(target)) {
        return;
      }
      if (
        targetElement?.closest(
          "[data-slot='select-content'], [data-slot='popover-content']",
        )
      ) {
        return;
      }
      if (
        targetElement?.closest(
          "[data-slot='dialog-overlay'], [data-slot='dialog-content']",
        )
      ) {
        return;
      }
      handleCloseGlobalSearchPanel();
    };

    window.addEventListener("pointerdown", handleGlobalSearchPanelPointerDown);
    return () =>
      window.removeEventListener(
        "pointerdown",
        handleGlobalSearchPanelPointerDown,
      );
  }, [handleCloseGlobalSearchPanel, isMobile, isGlobalSearchPanelOpen]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const handleGlobalSearchPanelEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") {
        return;
      }
      if (addDialogTarget !== null || requestDialogTarget !== null) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      handleCloseGlobalSearchPanel();
    };

    window.addEventListener("keydown", handleGlobalSearchPanelEscape);
    return () =>
      window.removeEventListener("keydown", handleGlobalSearchPanelEscape);
  }, [
    addDialogTarget,
    handleCloseGlobalSearchPanel,
    isMobile,
    isGlobalSearchPanelOpen,
    requestDialogTarget,
  ]);

  const renderMetadataSection = React.useCallback(
    (items: MetadataTvdbSearchItem[], facet: Facet, _section: string) => {
      return items.map((result) => {
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
            handleOpenRequestDialog(result, facet);
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
            onKeyDown={handleSearchResultKeyDown}
            disabled={disabled}
            actionKind={actionKind}
            actionTitle={actionTitle}
            facet={facet}
            imdbId={result.imdbId}
            name={result.name}
            posterUrl={posterUrl}
            resultAttribute="data-global-search-result"
            smgId={result.smgId}
            tvdbId={result.tvdbId}
            year={result.year}
            yearLabel={result.year ? result.year : t("label.yearUnknown")}
          />
        );
      });
    },
    [
      handleOpenAddDialog,
      handleOpenRequestDialog,
      handleSearchResultKeyDown,
      isMetadataSearchResultInCatalog,
      catalogQualityProfileOptions.length,
      librariesByFacet,
      rootFoldersByFacet,
      requestableLibrariesByFacet,
      t,
    ],
  );
  const accountInitial = (user?.username.trim().charAt(0) || "?").toUpperCase();
  const accountBadgeStyle = React.useMemo(
    () => buildAccountBadgeStyle(user?.id || user?.username || "?"),
    [user?.id, user?.username],
  );
  const accountAvatar = React.useMemo(
    () => selectAccountAvatar(accountLinkedAccounts),
    [accountLinkedAccounts],
  );

  return (
    <>
      <header
        ref={headerRef}
        data-slot="root-header"
        className={cn(
          "relative z-50 border-b border-[var(--scry-border3)] bg-transparent pt-safe-comfort px-safe backdrop-blur transition-transform duration-200 ease-out",
          isMobile
            ? "fixed inset-x-0 top-[var(--root-shell-top-offset,0px)]"
            : "sticky top-0",
          isMobile
            ? !isMobileHeaderVisible
              ? "-translate-y-full"
              : "translate-y-0"
            : null,
        )}
      >
        <div className="flex w-full items-center gap-3 px-[15px] py-[11px] sm:gap-[18px] sm:px-[26px] sm:py-3.5 min-[981px]:grid min-[981px]:grid-cols-[minmax(0,1fr)_minmax(0,660px)_minmax(0,1fr)]">
          {mobileNavigation ? (
            <div className="shrink-0 min-[981px]:hidden">
              {mobileNavigation}
            </div>
          ) : null}
          <div className="relative flex min-w-0 flex-1 items-center gap-3 min-[981px]:col-start-2 min-[981px]:w-full min-[981px]:-translate-x-[calc(var(--sidebar-width,0px)/2)]">
            <div ref={searchShellRef} className="relative w-full max-w-[660px]">
              {isMobile ? (
                <button
                  id="global-search-mobile-trigger"
                  ref={searchTriggerRef}
                  type="button"
                  className="relative flex h-11 w-full items-center gap-3 overflow-hidden rounded-[13px] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),1),rgba(var(--scry-accent-rgb),0.48),rgba(var(--scry-accent-rgb),0.96))] py-0 pl-11 pr-3 text-left text-[15px] font-medium text-[var(--scry-ink2)] shadow-[0_0_0_1px_rgba(var(--scry-accent-rgb),0.16),0_10px_24px_rgba(var(--scry-accent-rgb),0.12)] transition before:pointer-events-none before:absolute before:inset-[2px] before:rounded-[11px] before:bg-[var(--scry-bg)] before:content-[''] active:shadow-[0_0_0_1px_rgba(var(--scry-accent-rgb),0.28),0_12px_28px_rgba(var(--scry-accent-rgb),0.16)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)]"
                  onClick={() => openGlobalSearchPanel(true)}
                  aria-label={t("search.title")}
                  aria-haspopup="dialog"
                  aria-expanded={isGlobalSearchPanelOpen}
                  aria-controls="mobile-global-search-panel"
                  aria-keyshortcuts="Meta+K Control+K /"
                >
                  <Search className="absolute left-4 top-1/2 z-10 h-5 w-5 -translate-y-1/2 shrink-0 text-primary" />
                  <span className="relative z-10 min-w-0 flex-1 truncate">
                    {globalSearch.trim() || searchTriggerPlaceholder}
                  </span>
                </button>
              ) : (
                <button
                  id="global-search-trigger"
                  ref={searchTriggerRef}
                  type="button"
                  className={cn(
                    "relative flex h-12 w-full items-center gap-3 overflow-hidden rounded-[13px] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),1),rgba(var(--scry-accent-rgb),0.48),rgba(var(--scry-accent-rgb),0.96))] py-0 pl-12 pr-4 text-left text-[15px] font-medium text-[var(--scry-ink2)] shadow-[0_0_0_1px_rgba(var(--scry-accent-rgb),0.16),0_12px_28px_rgba(var(--scry-accent-rgb),0.12)] transition before:pointer-events-none before:absolute before:inset-[2px] before:rounded-[11px] before:bg-[var(--scry-bg)] before:content-[''] hover:shadow-[0_0_0_1px_rgba(var(--scry-accent-rgb),0.28),0_14px_32px_rgba(var(--scry-accent-rgb),0.16)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)]",
                    isGlobalSearchPanelOpen
                      ? "before:bg-[rgba(var(--scry-accent-rgb),0.08)] shadow-[0_0_0_1px_rgba(var(--scry-accent-rgb),0.30),0_16px_36px_rgba(var(--scry-accent-rgb),0.18)]"
                      : null,
                  )}
                  onClick={() => openGlobalSearchPanel(true)}
                  aria-label={t("search.title")}
                  aria-haspopup="dialog"
                  aria-expanded={isGlobalSearchPanelOpen}
                  aria-controls="global-search-panel"
                  aria-keyshortcuts="Meta+K Control+K /"
                >
                  <Search className="absolute left-4 top-1/2 z-10 h-5 w-5 -translate-y-1/2 shrink-0 text-primary" />
                  <span className="relative z-10 min-w-0 flex-1 truncate">
                    {globalSearch.trim() || searchTriggerPlaceholder}
                  </span>
                  <kbd className="relative z-10 rounded-md border border-primary/45 bg-primary/10 px-2 py-0.5 text-[11px] font-semibold text-primary">
                    {searchShortcutHint}
                  </kbd>
                </button>
              )}
              {isGlobalSearchPanelOpen &&
              !isMobile &&
              typeof document !== "undefined"
                ? createPortal(
                    <div
                      data-slot="global-search-overlay"
                      className="fixed inset-0 z-[60] flex flex-col items-center bg-[rgba(2,4,10,0.66)] px-5 pb-7 pt-[max(4rem,calc(var(--root-header-height,0px)+1rem))] backdrop-blur-md motion-safe:animate-in motion-safe:fade-in-0"
                      onMouseDown={handleSearchPanelBackdropMouseDown}
                    >
                      <div
                        ref={searchPanelRef}
                        id="global-search-panel"
                        data-slot="global-search-panel"
                        role="dialog"
                        aria-modal="true"
                        aria-label={t("search.title")}
                        aria-describedby="global-search-description"
                        tabIndex={-1}
                        className="flex max-h-[calc(100dvh-7rem)] w-[min(920px,calc(100vw-2.5rem))] flex-col overflow-hidden rounded-[18px] border border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] shadow-[0_40px_120px_rgba(0,0,0,0.7)] outline-none motion-safe:animate-in motion-safe:slide-in-from-top-3"
                        onMouseDown={(event) => event.stopPropagation()}
                        onKeyDown={handleSearchPanelKeyDown}
                      >
                        <div className="flex items-center gap-[13px] border-b border-[var(--scry-border)] px-[18px] py-4">
                          <div className="relative flex h-12 min-w-0 flex-1 items-center gap-3 rounded-[14px] border-2 border-primary/70 bg-[var(--scry-bg)] px-4 shadow-none dark:shadow-[0_0_0_1px_rgba(var(--scry-accent-rgb),0.10),0_12px_28px_rgba(var(--scry-accent-rgb),0.08)]">
                            <Search className="h-[22px] w-[22px] shrink-0 text-primary" />
                            <input
                              type="text"
                              id="global-search-input"
                              ref={globalSearchInputRef}
                              autoFocus
                              value={globalSearch}
                              onChange={handleSearchChange}
                              onKeyDown={handleDesktopSearchKeyDown}
                              data-ui="global-search"
                              className="block h-10 min-w-0 flex-1 appearance-none border-0 !bg-transparent bg-none px-0 text-[18px] font-medium text-[var(--scry-ink2)] caret-primary shadow-none outline-none placeholder:text-[18px] placeholder:font-normal placeholder:text-[var(--scry-muted3)] focus:!bg-transparent focus:bg-none focus:outline-none focus-visible:!bg-transparent focus-visible:bg-none focus-visible:outline-none"
                              style={{
                                WebkitAppearance: "none",
                                background: "transparent",
                                backgroundColor: "transparent",
                                backgroundImage: "none",
                                border: 0,
                                boxShadow: "none",
                                outline: 0,
                              }}
                              placeholder={searchOverlayPlaceholder}
                              aria-label={searchOverlayPlaceholder}
                              aria-controls="global-search-results-panel"
                              aria-describedby="global-search-description global-search-status"
                              autoComplete="off"
                              data-1p-ignore="true"
                              data-lpignore="true"
                              data-bwignore="true"
                              data-form-type="other"
                              data-protonpass-ignore="true"
                            />
                            {globalSearch ? (
                              <IconButton
                                id="global-search-clear"
                                type="button"
                                label={t("label.clear")}
                                appearance="ghost"
                                className="h-[28px] w-[28px] shrink-0 rounded-[8px] bg-primary/10 text-primary"
                                onClick={handleClearSearch}
                              >
                                <Eraser className="h-3.5 w-3.5" />
                              </IconButton>
                            ) : null}
                          </div>
                          <p
                            id="global-search-description"
                            className="sr-only"
                          >
                            {globalSearch.trim()
                              ? t("search.subtitleWithQuery", {
                                  query: globalSearch.trim(),
                                })
                              : searchSubtitle}
                          </p>
                          <p
                            id="global-search-status"
                            className="sr-only"
                            role="status"
                            aria-live="polite"
                            aria-atomic="true"
                          >
                            {searchStatusLabel}
                          </p>
                          <kbd className="rounded-[7px] border border-primary/35 bg-primary/10 px-2 py-1 text-[11px] font-semibold text-primary">
                            ESC
                          </kbd>
                          <IconButton
                            type="button"
                            label={t("label.close")}
                            appearance="ghost"
                            className="h-[30px] w-[30px] shrink-0 rounded-lg bg-[var(--scry-kbdbg)]"
                            onClick={handleCloseGlobalSearchPanel}
                          >
                            <X className="h-4 w-4" />
                          </IconButton>
                        </div>
                        <div
                          className="flex gap-x-5 gap-y-1 overflow-x-auto border-b border-[var(--scry-border)] px-[18px] py-2 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
                          role="group"
                          aria-label={t("search.title")}
                        >
                          {desktopSearchTabs.map((tab) => (
                            <UnderlineFilterButton
                              id={`global-search-tab-${tab.key}`}
                              key={tab.key}
                              ref={(node) => {
                                desktopSearchTabRefs.current[tab.key] = node;
                              }}
                              selected={isGlobalSearchFilterSelected(
                                desktopSearchFilters,
                                tab.key,
                              )}
                              label={tab.label}
                              count={tab.count}
                              aria-controls="global-search-results-panel"
                              onMouseDown={(event) => event.preventDefault()}
                              onClick={() => toggleDesktopSearchFilter(tab.key)}
                              onKeyDown={(event) =>
                                handleDesktopSearchTabKeyDown(event, tab.key)
                              }
                            />
                          ))}
                        </div>
                        <div
                          ref={searchResultsRef}
                          id="global-search-results-panel"
                          data-slot="global-search-results"
                          role="region"
                          aria-label={t("search.title")}
                          className="min-h-0 flex-1 overflow-y-auto p-[18px] [scrollbar-color:var(--scry-border2)_transparent] [scrollbar-width:thin] [&::-webkit-scrollbar]:w-2.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:border-[3px] [&::-webkit-scrollbar-thumb]:border-transparent [&::-webkit-scrollbar-thumb]:bg-[var(--scry-border2)] [&::-webkit-scrollbar-thumb]:bg-clip-content"
                        >
                          {showSectionResults ? (
                            <div className="space-y-6">
                              {showCatalogSection ? (
                                <section
                                  id="global-search-catalog-section"
                                  className="space-y-3"
                                >
                                  <div className="flex items-baseline justify-between gap-3">
                                    <div className="flex items-baseline gap-2">
                                      <h3 className="text-[15px] font-bold text-[var(--scry-ink2)]">
                                        {t("search.inLibrary")}
                                      </h3>
                                      <span className="text-xs text-[var(--scry-muted3)]">
                                        {t("search.alreadyInCollection")}
                                      </span>
                                    </div>
                                    <div className="flex shrink-0 items-center gap-3">
                                      {!catalogSearchLoading &&
                                      hiddenCatalogResultCount > 0 ? (
                                        <button
                                          type="button"
                                          className="text-xs font-medium text-[var(--scry-accent-ring)] transition hover:text-[var(--scry-accent-text)]"
                                          onMouseDown={(event) =>
                                            event.preventDefault()
                                          }
                                          onClick={() => {
                                            setDesktopSearchFilters([
                                              "library",
                                            ]);
                                            focusDesktopSearchFilter("library");
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
                                              count:
                                                String(visibleCatalogCount),
                                            })}
                                      </span>
                                    </div>
                                  </div>
                                  {catalogSearchLoading ? (
                                    <SearchSectionLoading
                                      label={t("label.loading")}
                                    />
                                  ) : visibleCatalogResults.length === 0 ? (
                                    <p className="rounded-[12px] border border-dashed border-[var(--scry-border2)] bg-[var(--scry-surfC)] px-4 py-5 text-sm text-[var(--scry-muted3)]">
                                      {!hasMinimumGlobalSearchQuery
                                        ? searchMinimumQueryHint
                                        : t("search.noCatalogMatches")}
                                    </p>
                                  ) : (
                                    <div className="grid gap-3 xl:grid-cols-2">
                                      {visibleCatalogResults.flatMap(
                                        ({ facet, title }) =>
                                          renderCatalogSection([title], facet),
                                      )}
                                    </div>
                                  )}
                                </section>
                              ) : null}
                              {metadataSectionFacets.length > 0 ? (
                                <div className="space-y-5">
                                  {metadataSectionFacets.map((f) => {
                                    const items =
                                      metadataSearchResults[f.metadataKey] ??
                                      [];
                                    const facetLabel = t(f.navLabelKey);
                                    const resultCountLabel =
                                      items.length === 1
                                        ? t("search.resultCountOne")
                                        : t("search.resultCountOther", {
                                            count: String(items.length),
                                          });
                                    return (
                                      <section
                                        key={f.id}
                                        id={selectorId(
                                          "global-search-metadata-section",
                                          f.id,
                                        )}
                                        className="space-y-3"
                                      >
                                        <div className="flex items-baseline justify-between gap-3">
                                          <div className="flex min-w-0 items-baseline gap-2">
                                            <h3 className="truncate text-[15px] font-bold text-[var(--scry-ink2)]">
                                              {facetLabel}
                                            </h3>
                                            <span className="shrink-0 text-xs text-[var(--scry-muted3)]">
                                              {metadataSearchLoading
                                                ? t("search.metadataSearch")
                                                : resultCountLabel}
                                            </span>
                                          </div>
                                        </div>
                                        {metadataSearchLoading ? (
                                          <SearchSectionLoading
                                            label={t("label.loading")}
                                          />
                                        ) : items.length === 0 ? (
                                          <p className="rounded-[12px] border border-dashed border-[var(--scry-border2)] bg-[var(--scry-surfC)] px-4 py-5 text-sm text-[var(--scry-muted3)]">
                                            {t("search.noMetadataMatches")}
                                          </p>
                                        ) : (
                                          desktopSearchFilters.includes(f.id) ? (
                                            <div className="flex flex-wrap gap-3 pb-1">
                                              {renderMetadataSection(
                                                items,
                                                f.id,
                                                f.metadataKey,
                                              )}
                                            </div>
                                          ) : (
                                            <HorizontalRail
                                              className="flex gap-3 overflow-x-auto overscroll-x-contain pb-1 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
                                              fadeClassName="to-[var(--scry-bg)]"
                                            >
                                              {renderMetadataSection(
                                                items,
                                                f.id,
                                                f.metadataKey,
                                              )}
                                            </HorizontalRail>
                                          )
                                        )}
                                      </section>
                                    );
                                  })}
                                </div>
                              ) : null}
                              {showRouteCommandSection ? (
                                <section className="space-y-3">
                                  <div className="flex items-baseline justify-between gap-3">
                                    <div className="flex min-w-0 items-baseline gap-2">
                                      <h3 className="truncate text-[15px] font-bold text-[var(--scry-ink2)]">
                                        {t("search.actionsAndSettings")}
                                      </h3>
                                      <span className="shrink-0 text-xs text-[var(--scry-muted3)]">
                                        {routeCommandResults.length === 1
                                          ? t("search.resultCountOne")
                                          : t("search.resultCountOther", {
                                              count: String(
                                                routeCommandResults.length,
                                              ),
                                        })}
                                      </span>
                                    </div>
                                    {hiddenRouteCommandResultCount > 0 ? (
                                      <button
                                        type="button"
                                        className="shrink-0 text-xs font-medium text-[var(--scry-accent-ring)] transition hover:text-[var(--scry-accent-text)]"
                                        onMouseDown={(event) =>
                                          event.preventDefault()
                                        }
                                        onClick={() => {
                                          setDesktopSearchFilters(["actions"]);
                                          focusDesktopSearchFilter("actions");
                                        }}
                                        aria-label={`${t("search.viewAll")} ${t("search.actionsAndSettings")}`}
                                      >
                                        {t("search.viewAll")}
                                      </button>
                                    ) : null}
                                  </div>
                                  <div className="space-y-3">
                                    {groupRouteCommandItems(
                                      visibleRouteCommandResults,
                                    ).map((group) => (
                                      <div
                                        key={group.groupLabel ?? "ungrouped"}
                                        className="space-y-2"
                                      >
                                        {group.groupLabel ? (
                                          <p className="text-[11px] font-semibold uppercase tracking-wide text-[var(--scry-muted3)]">
                                            {group.groupLabel}
                                          </p>
                                        ) : null}
                                        <div className="grid gap-2 sm:grid-cols-2">
                                          {group.items.map(
                                            renderRouteCommandItem,
                                          )}
                                        </div>
                                      </div>
                                    ))}
                                  </div>
                                </section>
                              ) : null}
                              <SearchFooterTip
                                canViewCatalog={canViewCatalog}
                                footerTip={t("search.footerTip")}
                                searchTipsLabel={t("search.searchTips")}
                                surface="desktop"
                                tipIndexers={t("search.tipIndexers")}
                                tipTabs={searchTipTabs}
                                tipTitles={searchTipTitles}
                              />
                            </div>
                          ) : searching ? (
                            <div className="py-12">
                              <SearchSectionLoading
                                label={t("label.searching")}
                              />
                            </div>
                          ) : (
                            <SearchEmptyState
                              className="py-14"
                              description={
                                trimmedGlobalSearch &&
                                !hasMinimumGlobalSearchQuery
                                  ? searchMinimumQueryHint
                                  : searchEmptyHint
                              }
                              icon={
                                hasMinimumGlobalSearchQuery
                                  ? "searchX"
                                  : "search"
                              }
                              title={
                                hasMinimumGlobalSearchQuery
                                  ? t("search.noMatchesFor", {
                                      query: trimmedGlobalSearch,
                                    })
                                  : trimmedGlobalSearch
                                    ? t("search.minimumQueryTitle")
                                    : searchOverlayPlaceholder
                              }
                            />
                          )}
                        </div>
                      </div>
                    </div>,
                    document.body,
                  )
                : null}
            </div>
          </div>
          {user ? (
            <div className="shrink-0 min-[981px]:col-start-3 min-[981px]:justify-self-end">
              <Popover open={accountMenuOpen} onOpenChange={setAccountMenuOpen}>
              <PopoverTrigger asChild>
                <Button
                  id="account-menu-trigger"
                  type="button"
                  variant="ghost"
                  className={cn(
                    "h-10 shrink-0 gap-2 rounded-[11px] border border-[rgba(var(--scry-accent-rgb),0.48)] bg-[rgba(var(--scry-accent-rgb),0.08)] p-[5px] pr-2 text-[var(--scry-body)] shadow-[0_0_0_1px_rgba(var(--scry-accent-rgb),0.08)] transition hover:border-[rgba(var(--scry-accent-rgb),0.68)] hover:bg-[rgba(var(--scry-accent-rgb),0.12)] hover:text-[var(--scry-ink2)] sm:h-10",
                    accountMenuOpen
                      ? "border-[rgba(var(--scry-accent-rgb),0.78)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-ink2)] shadow-[0_0_0_1px_rgba(var(--scry-accent-rgb),0.22)]"
                      : null,
                  )}
                  aria-label={t("profile.accountInfo")}
                  aria-haspopup="dialog"
                  aria-controls="account-menu-content"
                  aria-expanded={accountMenuOpen}
                >
                  {accountAvatar?.avatarUrl?.trim() ? (
                    <img
                      src={accountAvatar.avatarUrl}
                      alt=""
                      className="h-[30px] w-[30px] shrink-0 rounded-lg border border-white/20 object-cover shadow-[0_8px_18px_rgba(0,0,0,0.24)]"
                      loading="lazy"
                    />
                  ) : (
                    <span
                      className="flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-lg text-[13px] font-extrabold text-white"
                      style={accountBadgeStyle}
                    >
                      {accountInitial}
                    </span>
                  )}
                  <span className="hidden min-w-0 flex-col items-start leading-tight sm:flex">
                    <span className="max-w-32 truncate text-[12.5px] font-semibold text-[var(--scry-ink2)]">
                      {user.username}
                    </span>
                  </span>
                  <ChevronsUpDown className="h-4 w-4 text-[var(--scry-faint)]" />
                </Button>
              </PopoverTrigger>
              <PopoverContent
                id="account-menu-content"
                align="end"
                sideOffset={8}
                className="w-[min(18rem,calc(100vw-1rem))] border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-2 shadow-[0_18px_48px_rgba(0,0,0,0.36)] sm:w-56"
              >
                <div className="border-b border-[var(--scry-border)] px-3 pb-2 pt-1.5">
                  <p className="truncate text-sm font-medium text-foreground">
                    {user.username}
                  </p>
                </div>
                <div className="space-y-1 pt-2">
                  <Button
                    id="account-menu-profile"
                    type="button"
                    variant="ghost"
                    className="h-11 w-full justify-start rounded-[9px] px-3 text-sm font-medium text-[var(--scry-body)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] focus-visible:ring-primary/25"
                    onClick={handleOpenProfile}
                  >
                    <User className="mr-2 h-4 w-4 text-[var(--scry-accent-ring)]" />
                    {t("settings.profile")}
                  </Button>
                  {canEnableFormLogin ? (
                    <Button
                      id="account-menu-enable-login"
                      type="button"
                      variant="ghost"
                      className="h-11 w-full justify-start rounded-[9px] px-3 text-sm font-medium text-[var(--scry-body)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] focus-visible:ring-primary/25"
                      onClick={handleEnableLogin}
                    >
                      <ShieldCheck className="mr-2 h-4 w-4 text-[var(--scry-accent-ring)]" />
                      {t("settings.enableFormLogin")}
                    </Button>
                  ) : null}
                  {token && effectiveFormLoginEnabled !== false ? (
                    <Button
                      id="account-menu-logout"
                      type="button"
                      variant="ghost"
                      className="h-11 w-full justify-start rounded-[9px] px-3 text-sm font-medium text-destructive transition hover:bg-destructive/10 hover:text-destructive focus-visible:ring-destructive/25"
                      onClick={handleLogout}
                    >
                      <LogOut className="mr-2 h-4 w-4" />
                      {t("auth.logoutButton")}
                    </Button>
                  ) : null}
                </div>
              </PopoverContent>
              </Popover>
            </div>
          ) : null}
        </div>
      </header>
      {isMobile ? (
        <div
          aria-hidden="true"
          className="shrink-0"
          style={{ height: mobileHeaderHeight }}
        />
      ) : null}
      {isGlobalSearchPanelOpen && isMobile ? (
        <MobileSearchOverlay
          canViewCatalog={canViewCatalog}
          onClose={handleCloseGlobalSearchPanel}
          onOpenOverview={onOpenOverview}
          routeCommandItems={routeCommandItems}
        />
      ) : null}
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
    </>
  );
});
