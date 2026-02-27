import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { ActivitySquare, Download, ListChecks, Loader2, MonitorCog, Settings, WifiOff, X } from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useAuth } from "@/lib/hooks/use-auth";

import { RootHeader } from "@/components/root/root-header";
import { RootSidebar } from "@/components/root/root-sidebar";
import { ViewLoadingFallback } from "@/components/common/view-loading-fallback";
import { buildRouteCommands } from "@/components/root/route-commands";

import { useGlobalStatusToast } from "@/lib/hooks/use-global-status-toast";
import { useLanguage } from "@/lib/hooks/use-language";
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import { useOnlineStatus } from "@/lib/hooks/use-online-status";
import { useInstallPrompt } from "@/lib/hooks/use-install-prompt";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { ViewId, SettingsSection, ContentSettingsSection } from "@/components/root/types";
import type { UseGlobalSearchResult } from "@/lib/hooks/use-global-search";
import type {
  HomePageRouteState,
  Facet,
} from "@/lib/types";
import {
  URL_PARAM_CONTENT_SECTION_DEPRECATED,
  URL_PARAM_LANGUAGE,
  URL_PARAM_SETTINGS_SECTION_DEPRECATED,
  URL_PARAM_VIEW_DEPRECATED,
} from "@/lib/constants/settings";
import { AVAILABLE_LANGUAGES } from "@/lib/i18n";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";

import {
  buildViewPath,
  parseContentSectionFromPath,
  parseSettingsSectionFromPath,
  parseViewFromPath,
} from "@/lib/utils/routing";
import { FACET_REGISTRY, isMediaView, facetForView } from "@/lib/facets/registry";

const MediaContentContainer = lazy(() =>
  import("@/components/containers/media-content-container").then((m) => ({ default: m.MediaContentContainer })),
);

const MovieOverviewContainer = lazy(() =>
  import("@/components/containers/movie-overview-container").then((m) => ({ default: m.MovieOverviewContainer })),
);

const SeriesOverviewContainer = lazy(() =>
  import("@/components/containers/series-overview-container").then((m) => ({ default: m.SeriesOverviewContainer })),
);

const SettingsContainer = lazy(() =>
  import("@/components/containers/settings/settings-container").then((m) => ({ default: m.SettingsContainer })),
);

const ActivityContainer = lazy(() =>
  import("@/components/containers/activity-container").then((m) => ({ default: m.ActivityContainer })),
);

const SystemContainer = lazy(() =>
  import("@/components/containers/system-container").then((m) => ({ default: m.SystemContainer })),
);

const WantedContainer = lazy(() =>
  import("@/components/containers/wanted-container").then((m) => ({ default: m.WantedContainer })),
);

const GlobalSearchProvider = lazy(() =>
  import("@/components/root/global-search-provider").then((m) => ({ default: m.GlobalSearchProvider })),
);

function OverviewContainerForView({ view, ...props }: { view: ViewId; titleId: string; t: any; setGlobalStatus: (s: string) => void; onBackToList: () => void; onTitleNotFound: () => void }) {
  const facet = facetForView(view);
  if (facet?.hasEpisodes) {
    return <SeriesOverviewContainer {...props} />;
  }
  return <MovieOverviewContainer {...props} />;
}

/**
 * Synchronises the active facet (derived from the current view) with the search
 * state that lives inside the lazy-loaded GlobalSearchProvider.  Extracted into
 * its own component so that the useEffect can reference the search-state setters
 * that are only available inside the provider's render-prop.
 */
function ActiveFacetSync({
  activeFacet,
  setQueueFacet,
  setTvdbCandidates,
  setSearchResults,
  setSelectedTvdbId,
}: {
  activeFacet: Facet;
  setQueueFacet: (f: Facet) => void;
  setTvdbCandidates: UseGlobalSearchResult["setTvdbCandidates"];
  setSearchResults: UseGlobalSearchResult["setSearchResults"];
  setSelectedTvdbId: UseGlobalSearchResult["setSelectedTvdbId"];
}) {
  useEffect(() => {
    setQueueFacet(activeFacet);
    setTvdbCandidates([]);
    setSearchResults([]);
    setSelectedTvdbId(null);
  }, [activeFacet, setQueueFacet, setTvdbCandidates, setSearchResults, setSelectedTvdbId]);

  return null;
}

/**
 * Renders the main content area.  Extracted so that views needing search state
 * (MediaContentContainer) can receive it via the searchState prop which comes
 * from the GlobalSearchProvider render-prop, while non-search views are
 * unaffected.
 */
function MainContent({
  view,
  t,
  setGlobalStatus,
  overviewTitleId,
  handleBackToList,
  handleTitleNotFound,
  settingsSection,
  userId,
  username,
  selectedLanguage,
  uiLanguage,
  setLanguagePreferenceFromShell,
  contentSettingsSection,
  queueFacet,
  setQueueFacet,
  searchState,
  handleOpenOverview,
  catalogChangeSignal,
}: {
  view: ViewId;
  t: (key: string, values?: Record<string, string | number | boolean | null | undefined>) => string;
  setGlobalStatus: (status: string) => void;
  overviewTitleId: string | null;
  handleBackToList: () => void;
  handleTitleNotFound: () => void;
  settingsSection: SettingsSection;
  userId: string | undefined;
  username: string | undefined;
  selectedLanguage: LanguageOption;
  uiLanguage: LocaleCode;
  setLanguagePreferenceFromShell: (code: string) => void;
  contentSettingsSection: ContentSettingsSection;
  queueFacet: Facet;
  setQueueFacet: (f: Facet) => void;
  searchState: UseGlobalSearchResult;
  handleOpenOverview: (targetView: ViewId, titleId: string) => void;
  catalogChangeSignal: number;
}) {
  if (view === "activity") {
    return <ActivityContainer key="activity" t={t} setGlobalStatus={setGlobalStatus} />;
  }
  if (view === "wanted") {
    return <WantedContainer key="wanted" t={t} setGlobalStatus={setGlobalStatus} />;
  }
  if (view === "system") {
    return <SystemContainer key="system" t={t} setGlobalStatus={setGlobalStatus} />;
  }
  if (isMediaView(view) && overviewTitleId) {
    return (
      <OverviewContainerForView
        key={`${view}-overview-${overviewTitleId}`}
        view={view}
        titleId={overviewTitleId}
        t={t}
        setGlobalStatus={setGlobalStatus}
        onBackToList={handleBackToList}
        onTitleNotFound={handleTitleNotFound}
      />
    );
  }
  if (view === "settings") {
    return (
      <SettingsContainer
        key="settings"
        settingsSection={settingsSection}
        t={t}
        setGlobalStatus={setGlobalStatus}
        userId={userId}
        username={username}
        availableLanguages={AVAILABLE_LANGUAGES}
        selectedLanguage={selectedLanguage}
        uiLanguage={uiLanguage}
        onSelectLanguage={setLanguagePreferenceFromShell}
      />
    );
  }
  return (
    <MediaContentContainer
      key={`${view}-${contentSettingsSection}`}
      t={t}
      view={view}
      contentSettingsSection={contentSettingsSection}
      setGlobalStatus={setGlobalStatus}
      queueFacet={queueFacet}
      setQueueFacet={setQueueFacet}
      runTvdbSearch={searchState.runTvdbSearch}
      runSearch={searchState.runSearch}
      searchNzbForSelectedTvdb={searchState.searchNzbForSelectedTvdb}
      selectedTvdb={searchState.selectedTvdb}
      tvdbCandidates={searchState.tvdbCandidates}
      selectedTvdbId={searchState.selectedTvdbId}
      selectTvdbCandidate={searchState.selectTvdbCandidate}
      searchResults={searchState.searchResults}
      onOpenOverview={handleOpenOverview}
      catalogChangeSignal={catalogChangeSignal}
    />
  );
}

export default function HomePage({
  initialView,
  initialSettingsSection,
  initialContentSection,
}: HomePageRouteState = {}) {
  const { user, loading: authLoading } = useAuth();
  const navigate = useNavigate();

  useEffect(() => {
    if (!authLoading && !user) {
      navigate("/login", { replace: true });
    }
  }, [authLoading, user, navigate]);

  if (authLoading) {
    return (
        <div className="flex min-h-screen items-center justify-center bg-background text-foreground">
        <Loader2 className="h-6 w-6 animate-spin text-emerald-700 dark:text-emerald-300" />
      </div>
    );
  }

  if (!user) {
    return null;
  }

  return (
    <AuthenticatedHomePage
      initialView={initialView}
      initialSettingsSection={initialSettingsSection}
      initialContentSection={initialContentSection}
    />
  );
}

function AuthenticatedHomePage({
  initialView,
  initialSettingsSection,
  initialContentSection,
}: HomePageRouteState = {}) {
  const { user } = useAuth();
  const isMobile = useIsMobile();
  const isOnline = useOnlineStatus();
  const { canPrompt, isInstalled, promptInstall } = useInstallPrompt();

  const { pathname } = useLocation();
  const [searchParams] = useSearchParams();
  const pathnameSegments = useMemo(() => {
    const trimmedPath = pathname?.replace(/^\/+|\/+$/g, "").toLowerCase();
    return trimmedPath ? trimmedPath.split("/") : [];
  }, [pathname]);

  const deriveSectionsFromPath = useCallback((segments: string[]) => {
    const parsedView = parseViewFromPath(segments[0]);
    const parsedSettingsSection = parsedView === "settings"
      ? parseSettingsSectionFromPath(segments[1] ?? null)
      : "general";
    const parsedContentSection = isMediaView(parsedView)
        ? parseContentSectionFromPath(segments[1] ?? null)
        : "overview";

    return {
      parsedView,
      parsedSettingsSection,
      parsedContentSection,
    };
  }, []);

  const initialParsedSections = useMemo(
    () => deriveSectionsFromPath(pathnameSegments),
    [deriveSectionsFromPath, pathnameSegments],
  );

  const initialResolvedView = useMemo(
    () => initialView ?? initialParsedSections.parsedView,
    [initialParsedSections.parsedView, initialView],
  );

  const initialResolvedSettingsSection = useMemo(
    () =>
      initialResolvedView === "settings"
        ? (initialSettingsSection ?? initialParsedSections.parsedSettingsSection)
        : "general",
    [initialParsedSections.parsedSettingsSection, initialResolvedView, initialSettingsSection],
  );

  const initialResolvedContentSection = useMemo(
    () =>
      isMediaView(initialResolvedView)
        ? (initialContentSection ?? initialParsedSections.parsedContentSection)
        : "overview",
    [initialContentSection, initialParsedSections.parsedContentSection, initialResolvedView],
  );

  const initialResolvedOverviewTitleId = useMemo(() => {
    if (!isMediaView(initialResolvedView)) {
      return null;
    }

    if (initialResolvedContentSection !== "overview") {
      return null;
    }

    const nextTitleId = searchParams.get("id")?.trim();
    return nextTitleId && nextTitleId.length > 0 ? nextTitleId : null;
  }, [initialResolvedContentSection, initialResolvedView, searchParams]);

  const [view, setView] = useState<ViewId>(initialResolvedView);
  const [settingsSection, setSettingsSection] = useState<SettingsSection>(initialResolvedSettingsSection);
  const [contentSettingsSection, setContentSettingsSection] = useState<ContentSettingsSection>(initialResolvedContentSection);
  const [overviewTitleId, setOverviewTitleId] = useState<string | null>(initialResolvedOverviewTitleId);

  const parseOverviewTitleId = useCallback(
    (
      nextView: ViewId,
      nextContentSection: ContentSettingsSection,
      nextSearch: string,
    ) => {
      if (
        nextContentSection !== "overview" ||
        !isMediaView(nextView)
      ) {
        return null;
      }

      const nextTitleId = new URLSearchParams(nextSearch).get("id")?.trim();
      return nextTitleId && nextTitleId.length > 0 ? nextTitleId : null;
    },
    [],
  );

  useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    const onPopState = () => {
      const trimmedPath = window.location.pathname.replace(/^\/+|\/+$/g, "").toLowerCase();
      const segments = trimmedPath ? trimmedPath.split("/") : [];
      const parsed = deriveSectionsFromPath(segments);

      const nextView = parsed.parsedView;
      const nextSettingsSection = nextView === "settings"
        ? parsed.parsedSettingsSection
        : "general";
      const nextContentSection = isMediaView(nextView)
          ? parsed.parsedContentSection
          : "overview";
      const nextOverviewTitleId = parseOverviewTitleId(nextView, nextContentSection, window.location.search);

      setView(nextView);
      setSettingsSection(nextSettingsSection);
      setContentSettingsSection(nextContentSection);
      setOverviewTitleId(nextOverviewTitleId);
    };

    window.addEventListener("popstate", onPopState);
    onPopState();

    return () => {
      window.removeEventListener("popstate", onPopState);
    };
  }, [deriveSectionsFromPath, parseOverviewTitleId]);

  const {
    uiLanguage,
    setLanguagePreference,
    selectedLanguage,
    t,
    getLanguageLabel,
  } = useLanguage(searchParams);

  const [globalStatus, setGlobalStatusRaw] = useState(() => t("label.ready"));
  const setGlobalStatus = useGlobalStatusToast(setGlobalStatusRaw);

  const setLanguagePreferenceFromShell = useCallback(
    (code: string) => {
      setLanguagePreference(code);
      setGlobalStatus(t("status.languageChanged", { language: getLanguageLabel(code) }));
    },
    [getLanguageLabel, setLanguagePreference, t],
  );

  const [queueFacet, setQueueFacet] = useState<Facet>("movie");
  const [catalogChangeSignal, setCatalogChangeSignal] = useState(0);
  const [installBannerDismissed, setInstallBannerDismissed] = useState(false);

  const onCatalogChanged = useCallback(() => setCatalogChangeSignal((v) => v + 1), []);

  const activeFacet = useMemo<Facet>(() => facetForView(view)?.id ?? "movie", [view]);

  const navigateTo = useCallback(
    (
      nextView: ViewId,
      nextSettingsSection?: SettingsSection,
      nextContentSection?: ContentSettingsSection,
      nextOverviewTitleId?: string | null,
    ) => {
      const isMedia = isMediaView(nextView);
      const targetPath = buildViewPath(
        nextView,
        nextView === "settings" ? nextSettingsSection : undefined,
        isMedia ? nextContentSection : undefined,
      );
      const normalizedSettingsSection =
        nextView === "settings" ? (nextSettingsSection ?? "general") : "general";
      const normalizedContentSection = isMedia
        ? (nextContentSection ?? "overview")
        : "overview";
      const normalizedOverviewTitleId = (nextOverviewTitleId ?? "").trim().length > 0
        ? (nextOverviewTitleId as string).trim()
        : null;

      setView(nextView);
      setSettingsSection(normalizedSettingsSection);
      setContentSettingsSection(normalizedContentSection);
      setOverviewTitleId(
        normalizedContentSection === "overview" && isMedia
          ? normalizedOverviewTitleId
          : null,
      );

      if (typeof window === "undefined") {
        return;
      }

      const nextParams = new URLSearchParams(window.location.search);
      nextParams.delete(URL_PARAM_VIEW_DEPRECATED);
      nextParams.delete(URL_PARAM_SETTINGS_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_CONTENT_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_LANGUAGE);
      if (
        normalizedOverviewTitleId &&
        isMedia &&
        normalizedContentSection === "overview"
      ) {
        nextParams.set("id", normalizedOverviewTitleId);
      } else {
        nextParams.delete("id");
      }

      const nextQuery = nextParams.toString();
      const nextPathWithQuery = `${targetPath}${nextQuery ? `?${nextQuery}` : ""}`;
      const currentPath = `${window.location.pathname}${window.location.search ? `?${window.location.search}` : ""}`;

      if (nextPathWithQuery !== currentPath) {
        window.history.pushState({}, "", nextPathWithQuery);
      }
    },
    [],
  );

  const handleOpenOverview = useCallback(
    (targetView: ViewId, titleId: string) => {
      if (!isMediaView(targetView)) {
        return;
      }

      navigateTo(targetView, undefined, "overview", titleId);
    },
    [navigateTo],
  );

  const topNav = useMemo(
    () => [
      ...FACET_REGISTRY.map((f) => ({ id: f.viewId as ViewId, label: t(f.navLabelKey), icon: f.icon })),
      { id: "activity" as ViewId, label: t("nav.activity"), icon: ActivitySquare },
      { id: "wanted" as ViewId, label: t("nav.wanted"), icon: ListChecks },
      { id: "settings" as ViewId, label: t("nav.settings"), icon: Settings },
      { id: "system" as ViewId, label: t("nav.system"), icon: MonitorCog },
    ],
    [t],
  );

  const routeCommandPalette = useMemo(
    () => buildRouteCommands({
      t,
      onNavigate: navigateTo,
    }),
    [navigateTo, t],
  );

  const routeCommandPaletteConfig = useMemo(
    () => ({
      title: t("command.paletteTitle"),
      description: t("command.paletteDescription"),
      placeholder: t("command.palettePlaceholder"),
      noResultsText: t("command.paletteNoResults"),
      groupLabel: t("command.paletteGroup"),
      items: routeCommandPalette,
    }),
    [routeCommandPalette, t],
  );

  const entitlements = useMemo(() => user?.entitlements ?? [], [user?.entitlements]);

  const handleBackToList = useCallback(
    () => navigateTo(view, undefined, "overview"),
    [navigateTo, view],
  );

  const handleTitleNotFound = useCallback(
    () => navigateTo(view, undefined, "overview"),
    [navigateTo, view],
  );

  return (
    <ScryerGraphqlProvider language={uiLanguage}>
    <div className="min-h-screen bg-background text-foreground">
      <Suspense fallback={<ViewLoadingFallback />}>
        <GlobalSearchProvider
          t={t}
          setGlobalStatus={setGlobalStatus}
          queueFacet={queueFacet}
          uiLanguage={uiLanguage}
          onCatalogChanged={onCatalogChanged}
        >
          {(searchState) => (
            <>
              <ActiveFacetSync
                activeFacet={activeFacet}
                setQueueFacet={setQueueFacet}
                setTvdbCandidates={searchState.setTvdbCandidates}
                setSearchResults={searchState.setSearchResults}
                setSelectedTvdbId={searchState.setSelectedTvdbId}
              />
              <RootHeader
                t={t}
                globalSearch={searchState.globalSearch}
                onGlobalSearchChange={searchState.setGlobalSearch}
                routeCommandPalette={routeCommandPaletteConfig}
                catalogSearchResults={searchState.catalogSearchResults}
                metadataSearchResults={searchState.metadataSearchResults}
                isGlobalSearchPanelOpen={searchState.isGlobalSearchPanelOpen}
                onOpenGlobalSearchPanel={searchState.openGlobalSearchPanel}
                onCloseGlobalSearchPanel={searchState.closeGlobalSearchPanel}
                catalogQualityProfileOptions={searchState.catalogQualityProfileOptions}
                resolveDefaultQualityProfileIdForFacet={searchState.resolveDefaultQualityProfileIdForFacet}
                onAddMetadataSearchResultToCatalog={searchState.addMetadataSearchResultToCatalog}
                isMetadataSearchResultInCatalog={searchState.isMetadataSearchResultInCatalog}
                searching={searchState.searching}
                globalSearchInputRef={searchState.globalSearchInputRef}
                globalStatus={globalStatus}
                onOpenOverview={handleOpenOverview}
              />

              {!isOnline ? (
                <div className="flex items-center justify-center gap-2 bg-amber-900/80 px-4 py-2 text-sm text-amber-100">
                  <WifiOff className="h-4 w-4 flex-none" />
                  <span>{t("pwa.offline")}</span>
                </div>
              ) : null}

              {isMobile && canPrompt && !isInstalled && !installBannerDismissed ? (
                <div className="flex items-center justify-center gap-3 bg-emerald-100 dark:bg-emerald-900/60 px-4 py-2 text-sm text-emerald-800 dark:text-emerald-100">
                  <Download className="h-4 w-4 flex-none" />
                  <span>{t("pwa.installApp")}</span>
                  <button
                    type="button"
                    onClick={() => void promptInstall()}
                    className="rounded-md bg-emerald-600 px-3 py-1 text-xs font-medium text-foreground hover:bg-emerald-500"
                  >
                    {t("pwa.installApp")}
                  </button>
                  <button
                    type="button"
                    onClick={() => setInstallBannerDismissed(true)}
                    className="ml-auto text-emerald-700 dark:text-emerald-300 hover:text-foreground"
                    aria-label={t("label.dismiss")}
                  >
                    <X className="h-4 w-4" />
                  </button>
                </div>
              ) : null}

              <div className="mx-auto w-full max-w-[1480px] px-3 pb-10 pt-4">
                <RootSidebar
                  t={t}
                  topNav={topNav}
                  view={view}
                  settingsSection={settingsSection}
                  contentSettingsSection={contentSettingsSection}
                  entitlements={entitlements}
                  onNavigate={navigateTo}
                >
                  <main className="min-h-[70vh]">
                    <Suspense fallback={<ViewLoadingFallback />}>
                      <MainContent
                        view={view}
                        t={t}
                        setGlobalStatus={setGlobalStatus}
                        overviewTitleId={overviewTitleId}
                        handleBackToList={handleBackToList}
                        handleTitleNotFound={handleTitleNotFound}
                        settingsSection={settingsSection}
                        userId={user?.id}
                        username={user?.username}
                        selectedLanguage={selectedLanguage}
                        uiLanguage={uiLanguage}
                        setLanguagePreferenceFromShell={setLanguagePreferenceFromShell}
                        contentSettingsSection={contentSettingsSection}
                        queueFacet={queueFacet}
                        setQueueFacet={setQueueFacet}
                        searchState={searchState}
                        handleOpenOverview={handleOpenOverview}
                        catalogChangeSignal={catalogChangeSignal}
                      />
                    </Suspense>
                  </main>
                </RootSidebar>
              </div>
            </>
          )}
        </GlobalSearchProvider>
      </Suspense>
    </div>
    </ScryerGraphqlProvider>
  );
}
