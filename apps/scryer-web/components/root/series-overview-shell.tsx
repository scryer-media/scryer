import { lazy, Suspense, useState, useCallback, useMemo } from "react";
import { ActivitySquare, Clapperboard, Film, ListChecks, MonitorCog, Settings, Tv } from "lucide-react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { RootHeader } from "@/components/root/root-header";
import { RootSidebar } from "@/components/root/root-sidebar";
import { ViewLoadingFallback } from "@/components/common/view-loading-fallback";
import { buildRouteCommands } from "@/components/root/route-commands";
import { useLanguage } from "@/lib/hooks/use-language";
import { useGlobalStatusToast } from "@/lib/hooks/use-global-status-toast";
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import type { ViewId, SettingsSection, ContentSettingsSection } from "@/components/root/types";
import { buildViewPath } from "@/lib/utils/routing";

const SeriesOverviewContainer = lazy(() =>
  import("@/components/containers/series-overview-container").then((m) => ({ default: m.SeriesOverviewContainer })),
);

const TOP_NAV_IDS: ViewId[] = [
  "movies",
  "series",
  "anime",
  "activity",
  "wanted",
  "settings",
  "system",
];

const TOP_NAV_ICONS: Record<ViewId, typeof Film> = {
  movies: Film,
  series: Tv,
  anime: Clapperboard,
  activity: ActivitySquare,
  wanted: ListChecks,
  settings: Settings,
  system: MonitorCog,
};

export function SeriesOverviewShell() {
  const [searchParams] = useSearchParams();
  const titleId = searchParams.get("id") ?? "";
  const navigate = useNavigate();

  const { uiLanguage, t } = useLanguage(searchParams);

  const [, setGlobalStatusRaw] = useState("");
  const setGlobalStatus = useGlobalStatusToast(setGlobalStatusRaw);

  const topNav = useMemo(
    () =>
      TOP_NAV_IDS.map((id) => ({
        id,
        label: t(`nav.${id}`),
        icon: TOP_NAV_ICONS[id],
      })),
    [t],
  );

  const handleTitleNotFound = useCallback(() => {
    navigate("/series", { replace: true });
  }, [navigate]);

  const navigateTo = useCallback(
    (
      nextView: ViewId,
      nextSettingsSection?: SettingsSection,
      nextContentSection?: ContentSettingsSection,
    ) => {
      const targetPath = buildViewPath(
        nextView,
        nextView === "settings" ? nextSettingsSection : undefined,
        nextView === "movies" ||
          nextView === "series" ||
          nextView === "anime"
          ? nextContentSection
          : undefined,
      );
      navigate(targetPath);
    },
    [navigate],
  );

  const routeCommandPalette = useMemo(() => {
    return buildRouteCommands({
      t,
      onNavigate: navigateTo,
    });
  }, [navigateTo, t]);

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

  return (
    <ScryerGraphqlProvider language={uiLanguage}>
    <div className="min-h-screen bg-background text-foreground">
      <RootHeader
        t={t}
        globalSearch=""
        onGlobalSearchChange={() => undefined}
        searching={false}
        globalSearchInputRef={{ current: null } as React.RefObject<HTMLInputElement | null>}
        catalogSearchResults={[]}
        metadataSearchResults={{
          movie: [],
          series: [],
          anime: [],
        }}
        isGlobalSearchPanelOpen={false}
        onOpenGlobalSearchPanel={() => undefined}
        onCloseGlobalSearchPanel={() => undefined}
        routeCommandPalette={routeCommandPaletteConfig}
        catalogQualityProfileOptions={[]}
        resolveDefaultQualityProfileIdForFacet={() => ""}
        onAddMetadataSearchResultToCatalog={async () => null}
        isMetadataSearchResultInCatalog={() => false}
      />

      <div className="mx-auto w-full max-w-[1480px] px-3 pb-10 pt-4">
        <RootSidebar
          t={t}
          topNav={topNav}
          view="series"
          settingsSection="profile"
          contentSettingsSection="overview"
          entitlements={[]}
          onNavigate={navigateTo}
        >
          <main className="min-h-[70vh]">
            <Suspense fallback={<ViewLoadingFallback />}>
            <SeriesOverviewContainer
              titleId={titleId}
              t={t}
              setGlobalStatus={setGlobalStatus}
              onTitleNotFound={handleTitleNotFound}
            />
            </Suspense>
          </main>
        </RootSidebar>
      </div>
    </div>
    </ScryerGraphqlProvider>
  );
}
