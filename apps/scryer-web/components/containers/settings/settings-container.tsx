
import { lazy, memo, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { Link, useLocation } from "react-router";
import { useClient } from "urql";
import {
  Archive,
  Bell,
  Captions,
  ChevronRight,
  Database,
  Download,
  FolderCog,
  Puzzle,
  Rss,
  Server,
  Settings2,
  ShieldCheck,
  Network,
  SlidersHorizontal,
  Timer,
  UploadCloud,
  User,
  Users,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import type { IndexerSettingsTab, SettingsSection } from "@/components/root/types";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  buildIndexerSettingsPath,
  buildViewPath,
  indexerSettingsTabFromPath,
} from "@/lib/utils/routing";
import {
  type ProviderCatalogFamily,
  useProviderCatalogSubscription,
} from "@/lib/hooks/use-provider-catalog-subscription";
import { indexerDownloadClientMappingCatalogQuery } from "@/lib/graphql/queries";
import type {
  IndexerDownloadClientMappingCatalog,
  IndexerDownloadClientMappingCatalogResource,
} from "@/lib/types";
import {
  beginIndexerDownloadClientCatalogRequest,
  completeIndexerDownloadClientCatalogRequest,
  failIndexerDownloadClientCatalogRequest,
  isLatestIndexerDownloadClientCatalogRequest,
  normalizeIndexerDownloadClientMappingCatalog,
} from "@/lib/utils/indexer-download-client-mapping";

const SettingsOverviewContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-overview-container")).SettingsOverviewContainer,
}));
const SettingsSecurityContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-security-container")).SettingsSecurityContainer,
}));
const SettingsUsersContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-users-container")).SettingsUsersContainer,
}));
const SettingsIndexersContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-indexers-container")).SettingsIndexersContainer,
}));
const SettingsMediaServersContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-media-servers-container")).SettingsMediaServersContainer,
}));
const SettingsDownloadClientsContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-download-clients-container")).SettingsDownloadClientsContainer,
}));
const SettingsAcquisitionContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-acquisition-container")).SettingsAcquisitionContainer,
}));
const SettingsDelayProfilesContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-delay-profiles-container")).SettingsDelayProfilesContainer,
}));
const SettingsSeedingProfilesContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-seeding-profiles-container")).SettingsSeedingProfilesContainer,
}));
const SettingsQualityProfilesContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-quality-profiles-container")).SettingsQualityProfilesContainer,
}));
const SettingsProfileContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-profile-container")).SettingsProfileContainer,
}));
const SettingsRulesContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-rules-container")).SettingsRulesContainer,
}));
const SettingsPluginsContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-plugins-container")).SettingsPluginsContainer,
}));
const SettingsNotificationsContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-notifications-container")).SettingsNotificationsContainer,
}));
const SettingsPostProcessingContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-post-processing-container")).SettingsPostProcessingContainer,
}));
const SettingsSubtitlesContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-subtitles-container")).SettingsSubtitlesContainer,
}));
const SettingsBackupsContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-backups-container")).SettingsBackupsContainer,
}));

/** DOM id of the right reference rail. Inline plugin managers are portaled here
 * so they anchor to the top of the pane. */
export const SETTINGS_REFERENCE_SLOT_ID = "settings-content-reference";
export const SETTINGS_HEADER_ACTIONS_SLOT_ID = "settings-header-actions";

const DOCKED_REFERENCE_GAP_PX = 20;

type DockedReferenceLayout = {
  mainMinWidth: number;
  railMinWidth: number;
  contentClass: string;
  mainClass: string;
  railClass: string;
};

const STANDARD_FILTERED_PLUGIN_LAYOUT: DockedReferenceLayout = {
  mainMinWidth: 1080,
  railMinWidth: 288,
  contentClass: "max-w-none",
  mainClass: "min-w-0 max-w-[1280px] flex-[1_1_1280px]",
  railClass: "sticky top-[26px] z-auto min-w-[288px] max-w-[720px] flex-[1_1_720px]",
};

const INDEXERS_FILTERED_PLUGIN_LAYOUT: DockedReferenceLayout = {
  mainMinWidth: 1360,
  railMinWidth: 288,
  contentClass: "max-w-none",
  mainClass: "min-w-[1360px] max-w-[1520px] flex-[1_1_1520px]",
  railClass: "sticky top-[26px] z-auto min-w-[288px] max-w-[720px] flex-[1_1_720px]",
};

const SUBTITLES_FILTERED_PLUGIN_LAYOUT: DockedReferenceLayout = {
  mainMinWidth: 960,
  railMinWidth: 320,
  contentClass: "max-w-none",
  mainClass: "min-w-0 max-w-[1120px] flex-[1_1_960px]",
  railClass: "sticky top-[26px] z-auto min-w-[320px] max-w-[560px] flex-[1_1_560px]",
};

const INDEXER_SETTINGS_TABS: {
  tab: IndexerSettingsTab;
  labelKey: string;
  icon: LucideIcon;
}[] = [
  { tab: "indexers", labelKey: "settings.indexers", icon: Database },
  { tab: "proxies", labelKey: "settings.indexerProxies", icon: Network },
  { tab: "seedingProfiles", labelKey: "settings.seedingProfiles", icon: UploadCloud },
];

/// Pane switcher for the Indexers page. Indexers, their proxies, and the
/// seeding profiles they apply are three views of the same subject, so they
/// share a page instead of scattering across the settings nav. Same shape as
/// the Wanted view's section rail.
function IndexerSettingsSubnav({
  activeTab,
  t,
}: {
  activeTab: IndexerSettingsTab;
  t: ReturnType<typeof useTranslate>;
}) {
  return (
    <aside className="w-full shrink-0 border-b border-[var(--scry-border3)] bg-[var(--scry-surfF)] p-3 md:h-full md:w-[218px] md:overflow-y-auto md:border-b-0 md:border-r md:p-[22px_14px]">
      <nav
        id="settings-indexers-subnav"
        className="flex gap-2 overflow-x-auto pb-1 md:flex-col md:overflow-visible md:pb-0"
        aria-label={t("settings.indexers")}
      >
        {INDEXER_SETTINGS_TABS.map((item) => {
          const Icon = item.icon;
          const active = activeTab === item.tab;
          return (
            <Link
              key={item.tab}
              id={selectorId("settings-indexers-subnav", item.tab)}
              to={buildIndexerSettingsPath(item.tab)}
              aria-current={active ? "page" : undefined}
              className={cn(
                "flex h-9 shrink-0 items-center gap-2 rounded-[9px] px-3 text-[13px] font-medium text-[var(--scry-muted)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] md:w-full",
                active &&
                  "bg-[linear-gradient(90deg,rgba(var(--scry-accent-rgb),0.26),rgba(var(--scry-accent-rgb),0.08))] text-[var(--scry-ink2)] shadow-[inset_2px_0_0_var(--scry-accent-ring)]",
              )}
            >
              <Icon
                className={cn(
                  "h-[17px] w-[17px] text-[var(--scry-muted2)]",
                  active && "text-[var(--scry-accent-text)]",
                )}
              />
              <span className="whitespace-nowrap">{t(item.labelKey)}</span>
            </Link>
          );
        })}
      </nav>
    </aside>
  );
}

type SettingsContainerProps = {
  settingsSection: SettingsSection;
  userId?: string;
  username?: string;
  canManageSystemSettings: boolean;
  canManageCatalogSettings: boolean;
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
  pluginUpdateCount: number;
};

export const SettingsContainer = memo(function SettingsContainer({
  settingsSection,
  userId,
  username,
  canManageSystemSettings,
  canManageCatalogSettings,
  availableLanguages,
  selectedLanguage,
  uiLanguage,
  onSelectLanguage,
  pluginUpdateCount,
}: SettingsContainerProps) {
  const t = useTranslate();
  const client = useClient();
  // The Indexers page's pane lives in the path rather than in state so a pane
  // can be linked to and reloaded into.
  const indexerSettingsTab = indexerSettingsTabFromPath(useLocation().pathname);
  const [indexerDownloadClientMappingCatalogResource, setIndexerDownloadClientMappingCatalogResource] =
    useState<IndexerDownloadClientMappingCatalogResource>({
      catalog: null,
      status: "idle",
      error: null,
    });
  const indexerMappingCatalogRequestRef = useRef(0);
  const updateIndexerDownloadClientMappingCatalog = useCallback(
    (updater: (catalog: IndexerDownloadClientMappingCatalog) => IndexerDownloadClientMappingCatalog) => {
      setIndexerDownloadClientMappingCatalogResource((previous) =>
        previous.catalog
          ? { ...previous, catalog: updater(previous.catalog) }
          : previous,
      );
    },
    [],
  );
  const refreshIndexerDownloadClientMappingCatalog = useCallback(async () => {
    const requestSequence = indexerMappingCatalogRequestRef.current + 1;
    indexerMappingCatalogRequestRef.current = requestSequence;
    setIndexerDownloadClientMappingCatalogResource(
      beginIndexerDownloadClientCatalogRequest,
    );
    try {
      const { data, error } = await client
        .query(
          indexerDownloadClientMappingCatalogQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      if (!isLatestIndexerDownloadClientCatalogRequest(
        requestSequence,
        indexerMappingCatalogRequestRef.current,
      )) return;
      setIndexerDownloadClientMappingCatalogResource(
        completeIndexerDownloadClientCatalogRequest(
          normalizeIndexerDownloadClientMappingCatalog(
            data?.indexerDownloadClientMappingCatalog,
          ),
        ),
      );
    } catch (error) {
      if (!isLatestIndexerDownloadClientCatalogRequest(
        requestSequence,
        indexerMappingCatalogRequestRef.current,
      )) return;
      setIndexerDownloadClientMappingCatalogResource((previous) =>
        failIndexerDownloadClientCatalogRequest(
          previous,
          error instanceof Error ? error.message : t("status.failedToLoad"),
        ),
      );
    }
  }, [client, t]);
  const [providerCatalogVersions, setProviderCatalogVersions] = useState<
    Record<ProviderCatalogFamily, number>
  >({
    SUBTITLE: 0,
    NOTIFICATION: 0,
    INDEXER: 0,
    DOWNLOAD_CLIENT: 0,
    ARCHIVE_EXTRACTOR: 0,
  });
  useEffect(() => {
    if (settingsSection === "indexers") {
      void refreshIndexerDownloadClientMappingCatalog();
    }
  }, [refreshIndexerDownloadClientMappingCatalog, settingsSection]);
  const showPluginsLink =
    settingsSection === "downloadClients" ||
    settingsSection === "indexers" ||
    settingsSection === "notifications" ||
    settingsSection === "subtitles";
  const subscribeToProviderCatalog = showPluginsLink;
  // Surfaces that embed the FilteredPluginList manage plugins inline, so they
  // don't need the shortcut to the standalone Plugins page.
  const showPluginsShortcut =
    showPluginsLink &&
    settingsSection !== "indexers" &&
    settingsSection !== "downloadClients" &&
    settingsSection !== "notifications" &&
    settingsSection !== "subtitles";
  // Pages that render an inline plugins rail anchored to the top of the content pane.
  const showReferenceRail =
    settingsSection === "indexers" ||
    settingsSection === "downloadClients" ||
    settingsSection === "notifications" ||
    settingsSection === "subtitles";
  const isSubtitlesSection = settingsSection === "subtitles";
  const referenceLayout = showReferenceRail
    ? isSubtitlesSection
      ? SUBTITLES_FILTERED_PLUGIN_LAYOUT
      : settingsSection === "indexers"
        ? INDEXERS_FILTERED_PLUGIN_LAYOUT
        : STANDARD_FILTERED_PLUGIN_LAYOUT
    : null;
  const settingsContentRef = useRef<HTMLElement | null>(null);
  const [referenceRailOpen, setReferenceRailOpen] = useState(false);
  const [referenceRailDocked, setReferenceRailDocked] = useState(false);

  useEffect(() => {
    setReferenceRailOpen(false);
  }, [referenceRailDocked, settingsSection]);

  useEffect(() => {
    if (!referenceLayout) {
      setReferenceRailDocked(false);
      return;
    }

    const contentElement = settingsContentRef.current;
    if (!contentElement) {
      setReferenceRailDocked(false);
      return;
    }

    const minimumDockedWidth =
      referenceLayout.mainMinWidth +
      referenceLayout.railMinWidth +
      DOCKED_REFERENCE_GAP_PX;
    const syncReferenceRailMode = () => {
      setReferenceRailDocked(
        contentElement.getBoundingClientRect().width >= minimumDockedWidth,
      );
    };

    syncReferenceRailMode();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", syncReferenceRailMode);
      return () => window.removeEventListener("resize", syncReferenceRailMode);
    }

    const resizeObserver = new ResizeObserver(syncReferenceRailMode);
    resizeObserver.observe(contentElement);
    return () => resizeObserver.disconnect();
  }, [referenceLayout]);

  useEffect(() => {
    if (!referenceRailOpen || referenceRailDocked) {
      return;
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setReferenceRailOpen(false);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [referenceRailDocked, referenceRailOpen]);

  const settingsSectionLabel =
    settingsSection === "profile"
      ? t("settings.profile")
      : settingsSection === "general"
        ? t("settings.general")
        : settingsSection === "backups"
          ? t("settings.backups")
          : settingsSection === "security"
            ? t("settings.security")
            : settingsSection === "users"
              ? t("settings.users")
              : settingsSection === "mediaServers"
                ? t("settings.mediaServers")
                : settingsSection === "indexers"
                  ? t("settings.indexers")
                  : settingsSection === "downloadClients"
                    ? t("settings.downloadClients")
                    : settingsSection === "rules"
                      ? t("settings.rules")
                      : settingsSection === "plugins"
                        ? t("settings.plugins")
                        : settingsSection === "notifications"
                          ? t("settings.notifications")
                          : settingsSection === "post-processing"
                            ? t("settings.postProcessing")
                            : settingsSection === "subtitles"
                              ? t("settings.subtitles")
                              : settingsSection === "delayProfiles"
                                ? t("settings.delayProfiles")
                                : settingsSection === "acquisition"
                                  ? t("settings.acquisition")
                                  : t("settings.qualityProfiles");
  const primarySettingsNav = [
    {
      section: "profile" as const,
      label: t("settings.profile"),
      icon: User,
      visible: true,
    },
    {
      section: "general" as const,
      label: t("settings.general"),
      icon: Settings2,
      visible: canManageSystemSettings,
    },
    {
      section: "qualityProfiles" as const,
      label: t("settings.qualityProfiles"),
      icon: SlidersHorizontal,
      visible: canManageCatalogSettings,
    },
    {
      section: "delayProfiles" as const,
      label: t("settings.delayProfiles"),
      icon: Timer,
      visible: canManageCatalogSettings,
    },
    {
      section: "plugins" as const,
      label: t("settings.plugins"),
      icon: Puzzle,
      visible: canManageSystemSettings,
    },
  ].filter((item) => item.visible);
  const showPrimarySettingsSubnav = primarySettingsNav.some(
    (item) => item.section === settingsSection,
  );
  const usesAutomationHeader =
    settingsSection === "rules" ||
    settingsSection === "subtitles" ||
    settingsSection === "post-processing" ||
    settingsSection === "acquisition";
  const usesIntegrationsHeader =
    settingsSection === "downloadClients" ||
    settingsSection === "indexers" ||
    settingsSection === "mediaServers" ||
    settingsSection === "notifications";
  const usesAccessHeader = settingsSection === "security" || settingsSection === "users";
  const usesSystemHeader = usesAccessHeader || settingsSection === "backups";
  const SettingsSectionIcon = (() => {
    switch (settingsSection) {
      case "rules":
        return SlidersHorizontal;
      case "post-processing":
        return FolderCog;
      case "subtitles":
        return Captions;
      case "acquisition":
        return Rss;
      case "indexers":
        return Database;
      case "downloadClients":
        return Download;
      case "mediaServers":
        return Server;
      case "notifications":
        return Bell;
      case "security":
        return ShieldCheck;
      case "users":
        return Users;
      case "backups":
        return Archive;
      default:
        return (
          primarySettingsNav.find((item) => item.section === settingsSection)?.icon ??
          Settings2
        );
    }
  })();
  // The Indexers page's panes are pages in their own right, so the header and
  // the breadcrumb name the pane rather than the section that hosts it.
  const activeIndexerTab =
    settingsSection === "indexers" && indexerSettingsTab !== "indexers"
      ? INDEXER_SETTINGS_TABS.find((item) => item.tab === indexerSettingsTab)
      : undefined;
  const pageLabel = activeIndexerTab
    ? t(activeIndexerTab.labelKey)
    : settingsSectionLabel;
  const PageIcon = activeIndexerTab ? activeIndexerTab.icon : SettingsSectionIcon;
  const breadcrumbRootLabel =
    usesAutomationHeader
      ? t("nav.group.automation")
      : usesIntegrationsHeader
        ? t("nav.group.integrations")
        : usesSystemHeader
          ? t("nav.group.system")
      : t("nav.settings");

  useProviderCatalogSubscription(
    useCallback((families: ProviderCatalogFamily[]) => {
      setProviderCatalogVersions((previous) => {
        const uniqueFamilies = [...new Set(families)];
        if (uniqueFamilies.length === 0) {
          return previous;
        }

        const next = { ...previous };
        for (const family of uniqueFamilies) {
          next[family] += 1;
        }
        return next;
      });
    }, []),
    subscribeToProviderCatalog,
  );

  return (
    <div className="flex min-h-0 w-full flex-1 flex-col overflow-visible bg-transparent md:flex-row min-[981px]:overflow-hidden">
      {showPrimarySettingsSubnav ? (
        <aside
          data-slot="settings-subnav-scroll"
          className="w-full shrink-0 border-b border-[var(--scry-border3)] bg-[var(--scry-surfF)] p-3 md:h-full md:w-[218px] md:overflow-y-auto md:border-b-0 md:border-r md:p-[22px_14px]"
        >
          <nav className="flex gap-2 overflow-x-auto pb-1 md:flex-col md:overflow-visible md:pb-0">
            {primarySettingsNav.map((item) => {
              const Icon = item.icon;
              const active = settingsSection === item.section;
              return (
                <Link
                  key={item.section}
                  id={selectorId("root-sidebar-settings", item.section)}
                  to={buildViewPath("settings", item.section)}
                  className={cn(
                    "flex h-9 shrink-0 items-center gap-2 rounded-[9px] px-3 text-[13px] font-medium text-[var(--scry-muted)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] md:w-full",
                    active &&
                      "bg-[linear-gradient(90deg,rgba(var(--scry-accent-rgb),0.26),rgba(var(--scry-accent-rgb),0.08))] text-[var(--scry-ink2)] shadow-[inset_2px_0_0_var(--scry-accent-ring)]",
                  )}
                >
                  <Icon
                    className={cn(
                      "h-[17px] w-[17px] text-[var(--scry-muted2)]",
                      active && "text-[var(--scry-accent-text)]",
                    )}
                  />
                  <span className="whitespace-nowrap">{item.label}</span>
                  {item.section === "plugins" && pluginUpdateCount > 0 ? (
                    <span className="ml-auto inline-flex h-5 min-w-5 items-center justify-center rounded-md bg-[var(--scry-warning-solid)] px-1 text-xs font-medium tabular-nums text-[var(--scry-warning-on-solid)]">
                      {pluginUpdateCount}
                    </span>
                  ) : null}
                </Link>
              );
            })}
          </nav>
        </aside>
      ) : null}
      {settingsSection === "indexers" ? (
        <IndexerSettingsSubnav activeTab={indexerSettingsTab} t={t} />
      ) : null}
      <main
        ref={settingsContentRef}
        data-slot="settings-main-scroll"
        className="min-w-0 flex-1 overflow-visible bg-transparent min-[981px]:overflow-y-auto"
      >
        <div
          className={cn(
            "mx-auto w-full px-4 py-5 pb-[calc(env(safe-area-inset-bottom)+5rem)] sm:px-6 md:px-[30px] md:py-[26px] md:pb-[60px]",
            showReferenceRail
              ? referenceRailDocked
                ? referenceLayout?.contentClass
                : settingsSection === "indexers"
                  ? "max-w-none"
                  : "max-w-[1280px]"
              : settingsSection === "rules" ||
                  settingsSection === "post-processing"
                ? "max-w-none"
                : settingsSection === "users"
                  ? "max-w-[1620px]"
                  : "max-w-[1280px]",
          )}
        >
          <div
            className={cn(
              showReferenceRail && referenceRailDocked
                ? "flex items-start justify-center gap-5"
                : "contents",
            )}
          >
            <div
              className={cn(
                showReferenceRail && referenceRailDocked
                  ? referenceLayout?.mainClass
                  : showReferenceRail
                    ? "min-w-0"
                  : "contents",
              )}
            >
          <div className="mb-4 flex items-center gap-1.5 text-[12.5px] text-[var(--scry-faint)]">
            <span>{breadcrumbRootLabel}</span>
            <ChevronRight className="h-3.5 w-3.5" />
            {activeIndexerTab ? (
              <>
                <Link
                  to={buildIndexerSettingsPath("indexers")}
                  className="transition hover:text-[var(--scry-ink2)]"
                >
                  {settingsSectionLabel}
                </Link>
                <ChevronRight className="h-3.5 w-3.5" />
              </>
            ) : null}
            <span className="font-semibold text-[var(--scry-accent-text)]">{pageLabel}</span>
          </div>
          <div
            className={cn(
              "mb-6 flex gap-3",
              showReferenceRail && !referenceRailDocked
                ? "flex-row items-center justify-between"
                : "flex-col sm:flex-row sm:items-center sm:justify-between",
            )}
          >
            <div className="flex min-w-0 items-center gap-4">
              <div className="flex h-[46px] w-[46px] shrink-0 items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.35),rgba(123,91,255,0.22))] text-[var(--scry-accent-text)]">
                <PageIcon className="h-[23px] w-[23px]" />
              </div>
              <div className="min-w-0">
                <h1 className="text-[25px] font-bold tracking-normal text-[var(--scry-ink2)]">
                  {pageLabel}
                </h1>
                {!usesAutomationHeader &&
                !usesIntegrationsHeader &&
                !usesSystemHeader &&
                settingsSection !== "profile" &&
                settingsSection !== "general" &&
                settingsSection !== "qualityProfiles" &&
                settingsSection !== "delayProfiles" &&
                settingsSection !== "plugins" ? (
                  <p className="mt-1 max-w-[640px] text-[13.5px] text-[var(--scry-muted)]">
                    {t("settings.sectionTitle", { section: settingsSectionLabel })}
                  </p>
                ) : null}
              </div>
            </div>
            {showReferenceRail && !referenceRailDocked ? (
              <Button
                type="button"
                variant="primary"
                className="h-10 w-auto shrink-0 self-start rounded-[10px] px-3 text-[13px]"
                onClick={() => setReferenceRailOpen(true)}
                aria-expanded={referenceRailOpen}
                aria-controls={SETTINGS_REFERENCE_SLOT_ID}
              >
                <Puzzle className="h-4 w-4" />
                {t("settings.plugins")}
              </Button>
            ) : settingsSection === "plugins" ? (
              <div
                id={SETTINGS_HEADER_ACTIONS_SLOT_ID}
                className="flex min-h-10 shrink-0 flex-wrap items-center justify-end gap-2 sm:min-w-[29rem]"
              />
            ) : showPluginsShortcut ? (
              <Button asChild variant="primary" className="h-10 shrink-0 rounded-[10px] px-3 text-[13px]">
                <Link to="/settings/plugins">{t("settings.plugins")}</Link>
              </Button>
            ) : null}
          </div>
          <Suspense fallback={<div className="py-6 text-sm text-[var(--scry-muted3)]">{t("label.loading")}</div>}>
          {settingsSection === "profile" ? (
            <SettingsProfileContainer
              userId={userId}
              username={username}
            />
          ) : settingsSection === "general" ? (
            <SettingsOverviewContainer
              availableLanguages={availableLanguages}
              selectedLanguage={selectedLanguage}
              uiLanguage={uiLanguage}
              onSelectLanguage={onSelectLanguage}
            />
          ) : settingsSection === "backups" ? (
            <SettingsBackupsContainer />
          ) : settingsSection === "security" ? (
            <SettingsSecurityContainer />
          ) : settingsSection === "users" ? (
            <SettingsUsersContainer />
          ) : settingsSection === "mediaServers" ? (
            <SettingsMediaServersContainer />
          ) : settingsSection === "indexers" ? (
            <>
              {indexerSettingsTab === "seedingProfiles" ? (
                <SettingsSeedingProfilesContainer />
              ) : (
                <SettingsIndexersContainer
                  indexerSettingsTab={indexerSettingsTab}
                  providerCatalogVersion={providerCatalogVersions.INDEXER}
                  indexerDownloadClientMappingCatalogResource={
                    indexerDownloadClientMappingCatalogResource
                  }
                  updateIndexerDownloadClientMappingCatalog={
                    updateIndexerDownloadClientMappingCatalog
                  }
                  refreshIndexerDownloadClientMappingCatalog={
                    refreshIndexerDownloadClientMappingCatalog
                  }
                />
              )}
            </>
          ) : settingsSection === "downloadClients" ? (
            <SettingsDownloadClientsContainer
              providerCatalogVersion={
                providerCatalogVersions.DOWNLOAD_CLIENT
                + providerCatalogVersions.ARCHIVE_EXTRACTOR
              }
              onDownloadClientsChanged={refreshIndexerDownloadClientMappingCatalog}
            />
          ) : settingsSection === "rules" ? (
            <SettingsRulesContainer />
          ) : settingsSection === "plugins" ? (
            <SettingsPluginsContainer />
          ) : settingsSection === "notifications" ? (
            <SettingsNotificationsContainer
              providerCatalogVersion={providerCatalogVersions.NOTIFICATION}
            />
          ) : settingsSection === "post-processing" ? (
            <SettingsPostProcessingContainer />
          ) : settingsSection === "subtitles" ? (
            <SettingsSubtitlesContainer
              providerCatalogVersion={providerCatalogVersions.SUBTITLE}
            />
          ) : settingsSection === "delayProfiles" ? (
            <SettingsDelayProfilesContainer />
          ) : settingsSection === "acquisition" ? (
            <SettingsAcquisitionContainer />
          ) : (
            <SettingsQualityProfilesContainer />
          )}
          </Suspense>
            </div>
            {showReferenceRail ? (
              <>
                {!referenceRailDocked ? (
                  <button
                    type="button"
                    aria-label={t("label.close")}
                    className={cn(
                      "fixed inset-0 z-40 bg-black/45 backdrop-blur-[2px] transition-opacity",
                      referenceRailOpen
                        ? "opacity-100"
                        : "pointer-events-none opacity-0",
                    )}
                    onClick={() => setReferenceRailOpen(false)}
                  />
                ) : null}
                <aside
                  aria-label={t("settings.plugins")}
                  className={cn(
                    referenceRailDocked
                      ? referenceLayout?.railClass
                      : "fixed bottom-4 right-4 top-[118px] z-50 flex w-[min(420px,calc(100vw-2rem))] min-w-0 flex-col gap-3 overflow-y-auto rounded-[16px] border border-[var(--scry-border)] bg-[var(--scry-bg)] p-3 shadow-[0_20px_50px_rgba(0,0,0,0.38)] transition duration-200",
                    !referenceRailDocked &&
                      (referenceRailOpen
                        ? "translate-x-0 opacity-100"
                        : "pointer-events-none translate-x-[calc(100%+1rem)] opacity-0"),
                  )}
                >
                  {!referenceRailDocked ? (
                    <div className="flex items-center justify-between gap-3 rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-bg)] px-3 py-2 shadow-[0_10px_24px_rgba(0,0,0,0.16)]">
                      <div className="flex min-w-0 items-center gap-2 text-[15px] font-semibold text-[var(--scry-ink2)]">
                        <Puzzle className="h-4 w-4 shrink-0" />
                        <span className="truncate">{t("settings.plugins")}</span>
                      </div>
                      <IconButton
                        id="settings-plugins-panel-close"
                        label={t("label.close")}
                        tone="neutral"
                        onClick={() => setReferenceRailOpen(false)}
                      >
                        <X className="h-4 w-4" />
                      </IconButton>
                    </div>
                  ) : null}
                  <div
                    id={SETTINGS_REFERENCE_SLOT_ID}
                    data-slot="settings-reference"
                    className="min-w-0"
                  />
                </aside>
              </>
            ) : null}
          </div>
        </div>
      </main>
    </div>
  );
});
