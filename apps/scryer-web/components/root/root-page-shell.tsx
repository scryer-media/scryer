import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { ActivitySquare, Download, History, ListChecks, Loader2, MonitorCog, Settings, WifiOff, X } from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useAuth } from "@/lib/hooks/use-auth";

import { TranslateContext } from "@/lib/context/translate-context";
import { GlobalStatusContext } from "@/lib/context/global-status-context";
import { RootHeader } from "@/components/root/root-header";
import { RootSidebar } from "@/components/root/root-sidebar";
import { ViewLoadingFallback } from "@/components/common/view-loading-fallback";
import { buildRouteCommands } from "@/components/root/route-commands";

import { useGlobalStatusToast } from "@/lib/hooks/use-global-status-toast";
import { useLanguage } from "@/lib/hooks/use-language";
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import { useOnlineStatus } from "@/lib/hooks/use-online-status";
import { useInstallPrompt } from "@/lib/hooks/use-install-prompt";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import type { ViewId, SettingsSection, ContentSettingsSection } from "@/components/root/types";
import type { Facet } from "@/lib/types";
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
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";

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

const TitleHistoryContainer = lazy(() =>
  import("@/components/containers/title-history-container").then((m) => ({ default: m.TitleHistoryContainer })),
);

const GlobalSearchProvider = lazy(() =>
  import("@/components/root/global-search-provider").then((m) => ({ default: m.GlobalSearchProvider })),
);

const INSTALL_BANNER_DISMISSED_KEY = "scryer.pwa.installBannerDismissed";

function OverviewContainerForView({ view, initialEpisodeId, ...props }: { view: ViewId; titleId: string; onBackToList: () => void; onTitleNotFound: () => void; initialEpisodeId?: string | null }) {
  const facet = facetForView(view);
  if (facet?.hasEpisodes) {
    return <SeriesOverviewContainer {...props} initialEpisodeId={initialEpisodeId} />;
  }
  return <MovieOverviewContainer {...props} />;
}

/**
 * Renders the main content area.
 */
function MainContent({
  view,
  overviewTitleId,
  overviewEpisodeId,
  handleBackToList,
  handleTitleNotFound,
  settingsSection,
  userId,
  username,
  selectedLanguage,
  uiLanguage,
  setLanguagePreferenceFromShell,
  contentSettingsSection,
  handleOpenOverview,
}: {
  view: ViewId;
  overviewTitleId: string | null;
  overviewEpisodeId: string | null;
  handleBackToList: () => void;
  handleTitleNotFound: () => void;
  settingsSection: SettingsSection;
  userId: string | undefined;
  username: string | undefined;
  selectedLanguage: LanguageOption;
  uiLanguage: LocaleCode;
  setLanguagePreferenceFromShell: (code: string) => void;
  contentSettingsSection: ContentSettingsSection;
  handleOpenOverview: (targetView: ViewId, titleId: string, episodeId?: string) => void;
}) {
  if (view === "activity") {
    return <ActivityContainer key="activity" />;
  }
  if (view === "wanted") {
    return <WantedContainer key="wanted" onOpenOverview={handleOpenOverview} />;
  }
  if (view === "history") {
    return <TitleHistoryContainer key="history" />;
  }
  if (view === "system") {
    return <SystemContainer key="system" />;
  }
  if (isMediaView(view) && overviewTitleId) {
    return (
      <OverviewContainerForView
        key={`${view}-overview-${overviewTitleId}`}
        view={view}
        titleId={overviewTitleId}
        initialEpisodeId={overviewEpisodeId}
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
      view={view}
      contentSettingsSection={contentSettingsSection}
      onOpenOverview={handleOpenOverview}
    />
  );
}

export default function HomePage() {
  const { serviceRestarting, setServiceRestarting } = useBackendRestarting();
  const { user, loading: authLoading } = useAuth();
  const navigate = useNavigate();
  const [setupChecked, setSetupChecked] = useState(false);

  useEffect(() => {
    if (!serviceRestarting && !authLoading && !user) {
      navigate("/login", { replace: true });
    }
  }, [authLoading, user, navigate, serviceRestarting]);

  // Check if setup wizard needs to run (first-run detection).
  useEffect(() => {
    if (serviceRestarting || authLoading || !user || setupChecked) return;
    (async () => {
      try {
        const { data } = await import("@/lib/graphql/urql-client").then(
          (mod) => mod.backendClient.query(
            `query SetupStatus { setupStatus { setupComplete } }`,
            {},
          ).toPromise(),
        );
        if (data?.setupStatus?.setupComplete === false) {
          navigate("/setup", { replace: true });
          return;
        }
      } catch {
        // If the query fails (e.g., old backend), skip the check
      }
      setSetupChecked(true);
    })();
  }, [authLoading, user, setupChecked, navigate, serviceRestarting]);

  if (serviceRestarting) {
    return <BackendRestartOverlay />;
  }

  if (authLoading || (!setupChecked && user)) {
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
      serviceRestarting={serviceRestarting}
      setServiceRestarting={setServiceRestarting}
    />
  );
}

function AuthenticatedHomePage({
  serviceRestarting,
  setServiceRestarting,
}: {
  serviceRestarting: boolean;
  setServiceRestarting: (value: boolean) => void;
}) {
  const { user } = useAuth();
  const isOnline = useOnlineStatus();
  const { canPrompt, isInstalled, isIosSafari, promptInstall } = useInstallPrompt();

  const { pathname } = useLocation();
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();

  const { parsedView: view, parsedSettingsSection: settingsSection, parsedContentSection: contentSettingsSection } =
    useMemo(() => {
      const trimmed = pathname.replace(/^\/+|\/+$/g, "").toLowerCase();
      const segments = trimmed ? trimmed.split("/") : [];
      const parsedView = parseViewFromPath(segments[0]);
      const parsedSettingsSection: SettingsSection = parsedView === "settings"
        ? parseSettingsSectionFromPath(segments[1] ?? null)
        : "general";
      const parsedContentSection: ContentSettingsSection = isMediaView(parsedView)
        ? parseContentSectionFromPath(segments[1] ?? null, segments[2] ?? null)
        : "overview";
      return { parsedView, parsedSettingsSection, parsedContentSection };
    }, [pathname]);

  const overviewTitleId = useMemo(() => {
    if (!isMediaView(view) || contentSettingsSection !== "overview") return null;
    return searchParams.get("id")?.trim() || null;
  }, [view, contentSettingsSection, searchParams]);

  const overviewEpisodeId = useMemo(() =>
    searchParams.get("episodeId")?.trim() || null, [searchParams]);

  const {
    uiLanguage,
    setLanguagePreference,
    selectedLanguage,
    t,
    getLanguageLabel,
  } = useLanguage(searchParams);

  const [, setGlobalStatusRaw] = useState("");
  const setGlobalStatus = useGlobalStatusToast(setGlobalStatusRaw, {
    onServiceRestarting: useCallback(() => setServiceRestarting(true), [setServiceRestarting]),
  });

  const setLanguagePreferenceFromShell = useCallback(
    (code: string) => {
      setLanguagePreference(code);
      setGlobalStatus(t("status.languageChanged", { language: getLanguageLabel(code) }));
    },
    [getLanguageLabel, setLanguagePreference, t, setGlobalStatus],
  );

  const [installBannerDismissed, setInstallBannerDismissed] = useState(() => {
    if (typeof window === "undefined") {
      return false;
    }

    return window.localStorage.getItem(INSTALL_BANNER_DISMISSED_KEY) === "true";
  });
  const showInstallBanner = !isInstalled && !installBannerDismissed && (canPrompt || isIosSafari);

  useEffect(() => {
    if (!isInstalled || typeof window === "undefined") {
      return;
    }

    window.localStorage.removeItem(INSTALL_BANNER_DISMISSED_KEY);
    setInstallBannerDismissed(false);
  }, [isInstalled]);

  const dismissInstallBanner = useCallback(() => {
    setInstallBannerDismissed(true);
    if (typeof window !== "undefined") {
      window.localStorage.setItem(INSTALL_BANNER_DISMISSED_KEY, "true");
    }
  }, []);

  const onCatalogChanged = useCallback(() => {}, []);

  const activeFacet = useMemo<Facet>(() => facetForView(view)?.id ?? "movie", [view]);
  const queueFacet = activeFacet;

  const navigateTo = useCallback(
    (
      nextView: ViewId,
      nextSettingsSection?: SettingsSection,
      nextContentSection?: ContentSettingsSection,
      nextOverviewTitleId?: string | null,
      nextEpisodeId?: string | null,
    ) => {
      const isMedia = isMediaView(nextView);
      const targetPath = buildViewPath(
        nextView,
        nextView === "settings" ? nextSettingsSection : undefined,
        isMedia ? nextContentSection : undefined,
      );
      const normalizedContentSection = isMedia
        ? (nextContentSection ?? "overview")
        : "overview";
      const normalizedOverviewTitleId = (nextOverviewTitleId ?? "").trim().length > 0
        ? (nextOverviewTitleId as string).trim()
        : null;

      const nextParams = new URLSearchParams(searchParams.toString());
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
      if (nextEpisodeId) {
        nextParams.set("episodeId", nextEpisodeId);
      } else {
        nextParams.delete("episodeId");
      }

      const nextQuery = nextParams.toString();
      const nextPathWithQuery = `${targetPath}${nextQuery ? `?${nextQuery}` : ""}`;
      const currentPathWithQuery = `${pathname}${searchParams.toString() ? `?${searchParams.toString()}` : ""}`;

      if (nextPathWithQuery !== currentPathWithQuery) {
        navigate(nextPathWithQuery);
      }
    },
    [navigate, searchParams, pathname],
  );

  const handleOpenOverview = useCallback(
    (targetView: ViewId, titleId: string, episodeId?: string) => {
      if (!isMediaView(targetView)) {
        return;
      }

      navigateTo(targetView, undefined, "overview", titleId, episodeId);
    },
    [navigateTo],
  );

  const topNav = useMemo(
    () => [
      ...FACET_REGISTRY.map((f) => ({ id: f.viewId as ViewId, label: t(f.navLabelKey), icon: f.icon })),
      { id: "activity" as ViewId, label: t("nav.activity"), icon: ActivitySquare },
      { id: "wanted" as ViewId, label: t("nav.wanted"), icon: ListChecks },
      { id: "history" as ViewId, label: t("nav.history"), icon: History },
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
    <TranslateContext.Provider value={t}>
    <GlobalStatusContext.Provider value={setGlobalStatus}>
    <div className="min-h-screen bg-background text-foreground">
      {serviceRestarting && (
        <BackendRestartOverlay />
      )}
      <Suspense fallback={<ViewLoadingFallback />}>
        <GlobalSearchProvider
          activeFacet={activeFacet}
          queueFacet={queueFacet}
          uiLanguage={uiLanguage}
          onCatalogChanged={onCatalogChanged}
        >
          <RootHeader
            onOpenOverview={handleOpenOverview}
            routeCommandPalette={routeCommandPaletteConfig}
          />

          {!isOnline ? (
            <div className="flex items-center justify-center gap-2 bg-amber-900/80 px-4 py-2 text-sm text-amber-100">
              <WifiOff className="h-4 w-4 flex-none" />
              <span>{t("pwa.offline")}</span>
            </div>
          ) : null}

          {showInstallBanner ? (
            <div className="flex items-center justify-center gap-3 bg-emerald-100 dark:bg-emerald-900/60 px-4 py-2 text-sm text-emerald-800 dark:text-emerald-100">
              <Download className="h-4 w-4 flex-none" />
              <span>{isIosSafari ? t("pwa.iosInstallHint") : t("pwa.installApp")}</span>
              {canPrompt ? (
                <button
                  type="button"
                  onClick={() => void promptInstall()}
                  className="rounded-md bg-emerald-600 px-3 py-1 text-xs font-medium text-foreground hover:bg-emerald-500"
                >
                  {t("pwa.installApp")}
                </button>
              ) : null}
              <button
                type="button"
                onClick={dismissInstallBanner}
                className="ml-auto text-emerald-700 dark:text-emerald-300 hover:text-foreground"
                aria-label={t("label.dismiss")}
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          ) : null}

          <div className="mx-auto w-full max-w-[1480px] px-3 pb-10 pt-4">
            <RootSidebar
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
                    overviewTitleId={overviewTitleId}
                    overviewEpisodeId={overviewEpisodeId}
                    handleBackToList={handleBackToList}
                    handleTitleNotFound={handleTitleNotFound}
                    settingsSection={settingsSection}
                    userId={user?.id}
                    username={user?.username}
                    selectedLanguage={selectedLanguage}
                    uiLanguage={uiLanguage}
                    setLanguagePreferenceFromShell={setLanguagePreferenceFromShell}
                    contentSettingsSection={contentSettingsSection}
                    handleOpenOverview={handleOpenOverview}
                  />
                </Suspense>
              </main>
            </RootSidebar>
          </div>
        </GlobalSearchProvider>
      </Suspense>
    </div>
    </GlobalStatusContext.Provider>
    </TranslateContext.Provider>
    </ScryerGraphqlProvider>
  );
}
