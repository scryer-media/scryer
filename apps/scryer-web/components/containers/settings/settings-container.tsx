import { userFacingGraphQlErrorMessage } from "../../../lib/graphql/error-message.ts";

import {
  lazy,
  memo,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
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
  Inbox,
  Puzzle,
  Rss,
  Server,
  Settings2,
  ShieldCheck,
  Network,
  SlidersHorizontal,
  SquareTerminal,
  Tag,
  Timer,
  User,
  Users,
  Wrench,
} from "lucide-react";
import { INDEXER_SETTINGS_TAB_ITEMS } from "@/components/views/settings/indexer-settings-tabs";
import { UnderlineTabLink } from "@/components/common/underline-filter-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  GutterPanel,
  GutterPanelButton,
} from "@/components/common/gutter-panel";
import { GutterPanelContext } from "@/lib/context/gutter-panel-context";
import type {
  IndexerSettingsTab,
  SettingsSection,
} from "@/components/root/types";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import { useTranslate } from "@/lib/context/translate-context";
import {
  type GutterPanelSize,
  useGutterPanel,
} from "@/lib/hooks/use-gutter-panel";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  buildIndexerSettingsPath,
  indexerSettingsTabFromPath,
  INDEXER_SETTINGS_TABS,
  lifecycleRulesSectionFromPath,
  BETA_RULES_SECTIONS,
  rulesSectionFromPath,
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
  default: (
    await import("@/components/containers/settings/settings-overview-container")
  ).SettingsOverviewContainer,
}));
const SettingsSecurityContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-security-container")
  ).SettingsSecurityContainer,
}));
const SettingsUsersContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-users-container")
  ).SettingsUsersContainer,
}));
const SettingsIndexersContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-indexers-container")
  ).SettingsIndexersContainer,
}));
const SettingsIndexerSearchContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-indexer-search-container")
  ).SettingsIndexerSearchContainer,
}));
const SettingsIndexerClientsContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-indexer-clients-container")
  ).SettingsIndexerClientsContainer,
}));
const SettingsMediaServersContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-media-servers-container")
  ).SettingsMediaServersContainer,
}));
const SettingsDownloadClientsContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-download-clients-container")
  ).SettingsDownloadClientsContainer,
}));
const SettingsAcquisitionContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-acquisition-container")
  ).SettingsAcquisitionContainer,
}));
const SettingsDelayProfilesContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-delay-profiles-container")
  ).SettingsDelayProfilesContainer,
}));
const SettingsTitleTagsContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-title-tags-container")
  ).SettingsTitleTagsContainer,
}));
const SettingsSeedingProfilesContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-seeding-profiles-container")
  ).SettingsSeedingProfilesContainer,
}));
const SettingsQualityProfilesContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-quality-profiles-container")
  ).SettingsQualityProfilesContainer,
}));
const SettingsProfileContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-profile-container")
  ).SettingsProfileContainer,
}));
const SettingsRulesContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-rules-container")
  ).SettingsRulesContainer,
}));
const SettingsLifecycleRulesContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-lifecycle-rules-container")
  ).SettingsLifecycleRulesContainer,
}));
const SettingsRequestRulesContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-request-rules-container")
  ).SettingsRequestRulesContainer,
}));
const SettingsPluginsContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-plugins-container")
  ).SettingsPluginsContainer,
}));
const SettingsScriptsContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-scripts-container")
  ).SettingsScriptsContainer,
}));
const SettingsNotificationsContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-notifications-container")
  ).SettingsNotificationsContainer,
}));
const SettingsProxiesContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-proxies-container")
  ).SettingsProxiesContainer,
}));
const SettingsPostProcessingContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-post-processing-container")
  ).SettingsPostProcessingContainer,
}));
const SettingsSubtitlesContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-subtitles-container")
  ).SettingsSubtitlesContainer,
}));
const SettingsBackupsContainer = lazy(async () => ({
  default: (
    await import("@/components/containers/settings/settings-backups-container")
  ).SettingsBackupsContainer,
}));

/** DOM id of the right reference rail. Inline plugin managers are portaled here
 * so they anchor to the top of the pane. */
export const SETTINGS_REFERENCE_SLOT_ID = "settings-content-reference";
export const SETTINGS_HEADER_ACTIONS_SLOT_ID = "settings-header-actions";
const SETTINGS_PLUGINS_PANEL_ID = "settings-plugins-panel";

/**
 * The plugins panel is pinned to the right edge beside the centered settings
 * pane. The pane keeps its content width (padding included) whether the panel
 * is open or not, so the panel never narrows or shifts the content; it closes
 * behind its edge tab once the gutter cannot hold it. The pane's own right
 * padding separates the panel from the content, so the panel needs no gap of
 * its own.
 */
const SETTINGS_PLUGINS_PANEL_SIZE: GutterPanelSize = { width: 320, gap: 0 };
const STANDARD_FILTERED_PLUGIN_CONTENT_WIDTH = 1280;
const INDEXERS_FILTERED_PLUGIN_CONTENT_WIDTH = 1460;
// The Search pane carries its own refine rail beside the results table, so it
// keeps the wider pane it had before it carried the plugins rail.
const INDEXER_SEARCH_FILTERED_PLUGIN_CONTENT_WIDTH = 1640;

/// Pane tabs for the Indexers page. An indexer and the seeding profile it
/// applies are two views of the same subject, so they share a page instead of
/// scattering across the settings nav. Same shape as the Lists page's tabs.
function IndexerSettingsTabs({
  activeTab,
  t,
}: {
  activeTab: IndexerSettingsTab;
  t: ReturnType<typeof useTranslate>;
}) {
  return (
    <div className="mb-6 flex flex-wrap items-end gap-3 border-b border-[var(--scry-border3)]">
      <div
        id="settings-indexers-subnav"
        role="tablist"
        aria-label={t("settings.indexers")}
        className="relative top-px flex min-h-12 flex-wrap gap-x-2"
      >
        {INDEXER_SETTINGS_TABS.map((tab) => {
          const item = INDEXER_SETTINGS_TAB_ITEMS[tab];
          return (
            <UnderlineTabLink
              key={tab}
              id={selectorId("settings-indexers-subnav", tab)}
              to={buildIndexerSettingsPath(tab)}
              selected={activeTab === tab}
              icon={<item.icon className="h-4 w-4" />}
              label={t(item.labelKey)}
              badge={item.beta ? t("settings.pluginBeta") : undefined}
            />
          );
        })}
      </div>
    </div>
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
  pluginBlockedCount: number;
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
}: SettingsContainerProps) {
  const t = useTranslate();
  const client = useClient();
  // The Indexers page's pane lives in the path rather than in state so a pane
  // can be linked to and reloaded into.
  const pathname = useLocation().pathname;
  const indexerSettingsTab = indexerSettingsTabFromPath(pathname);
  // Same for the Lifecycle Rules page's tabs.
  const lifecycleRulesSection = lifecycleRulesSectionFromPath(pathname);
  const [
    indexerDownloadClientMappingCatalogResource,
    setIndexerDownloadClientMappingCatalogResource,
  ] = useState<IndexerDownloadClientMappingCatalogResource>({
    catalog: null,
    status: "idle",
    error: null,
  });
  const indexerMappingCatalogRequestRef = useRef(0);
  const updateIndexerDownloadClientMappingCatalog = useCallback(
    (
      updater: (
        catalog: IndexerDownloadClientMappingCatalog,
      ) => IndexerDownloadClientMappingCatalog,
    ) => {
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
      if (
        !isLatestIndexerDownloadClientCatalogRequest(
          requestSequence,
          indexerMappingCatalogRequestRef.current,
        )
      )
        return;
      setIndexerDownloadClientMappingCatalogResource(
        completeIndexerDownloadClientCatalogRequest(
          normalizeIndexerDownloadClientMappingCatalog(
            data?.indexerDownloadClientMappingCatalog,
          ),
        ),
      );
    } catch (error) {
      if (
        !isLatestIndexerDownloadClientCatalogRequest(
          requestSequence,
          indexerMappingCatalogRequestRef.current,
        )
      )
        return;
      setIndexerDownloadClientMappingCatalogResource((previous) =>
        failIndexerDownloadClientCatalogRequest(
          previous,
          userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
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
  // Pages that render an inline plugins rail anchored to the top of the content
  // pane.
  const showReferenceRail =
    settingsSection === "indexers" ||
    settingsSection === "downloadClients" ||
    settingsSection === "notifications" ||
    settingsSection === "subtitles";
  const referenceContentWidth = showReferenceRail
    ? settingsSection === "indexers"
      ? indexerSettingsTab === "search"
        ? INDEXER_SEARCH_FILTERED_PLUGIN_CONTENT_WIDTH
        : INDEXERS_FILTERED_PLUGIN_CONTENT_WIDTH
      : STANDARD_FILTERED_PLUGIN_CONTENT_WIDTH
    : null;
  const settingsContentRef = useRef<HTMLElement | null>(null);
  const settingsPaneRef = useRef<HTMLDivElement | null>(null);
  const referencePanel = useGutterPanel(
    settingsContentRef,
    settingsPaneRef,
    showReferenceRail ? SETTINGS_PLUGINS_PANEL_SIZE : null,
    settingsSection,
  );
  const setReferencePanelOpen = referencePanel.setOpen;
  // The plugin lists are portaled into the panel from the section containers,
  // so the panel's context has to wrap the content they render from.
  const referencePanelContext = useMemo(
    () => ({ close: () => setReferencePanelOpen(false) }),
    [setReferencePanelOpen],
  );

  const settingsSectionLabel =
    settingsSection === "profile"
      ? t("settings.profile")
      : settingsSection === "general"
        ? t("settings.general")
        : settingsSection === "backups"
          ? t("settings.backups")
          : settingsSection === "security"
            ? t("nav.securityAccess")
            : settingsSection === "users"
              ? t("settings.users")
              : settingsSection === "mediaServers"
                ? t("settings.mediaServers")
                : settingsSection === "indexers"
                  ? t("settings.indexers")
                  : settingsSection === "downloadClients"
                    ? t("settings.downloadClients")
                    : settingsSection === "proxies"
                      ? t("settings.proxies")
                      : settingsSection === "rules"
                        ? t("settings.rulesScoring")
                        : settingsSection === "lifecycleRules"
                          ? t("settings.lifecycleRules")
                          : settingsSection === "requestRules"
                            ? t("settings.requestRules")
                            : settingsSection === "plugins"
                              ? t("settings.plugins")
                              : settingsSection === "scripts"
                                ? t("settings.scripts")
                                : settingsSection === "notifications"
                                  ? t("settings.notifications")
                                  : settingsSection === "post-processing"
                                    ? t("settings.postProcessing")
                                    : settingsSection === "subtitles"
                                      ? t("settings.subtitles")
                                      : settingsSection === "delayProfiles"
                                        ? t("settings.delayProfiles")
                                        : settingsSection === "titleTags"
                                          ? t("settings.titleTags")
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
      section: "titleTags" as const,
      label: t("settings.titleTags"),
      icon: Tag,
      visible: canManageCatalogSettings,
    },
    {
      section: "plugins" as const,
      label: t("settings.plugins"),
      icon: Puzzle,
      visible: canManageSystemSettings,
    },
    {
      section: "scripts" as const,
      label: t("settings.scripts"),
      icon: SquareTerminal,
      visible: canManageSystemSettings,
    },
  ].filter((item) => item.visible);
  // Both kinds of rule are panes of one page, so both carry its gutter.
  const isRulesSection =
    settingsSection === "rules" ||
    settingsSection === "lifecycleRules" ||
    settingsSection === "requestRules";
  const usesAutomationHeader =
    isRulesSection ||
    settingsSection === "subtitles" ||
    settingsSection === "post-processing" ||
    settingsSection === "acquisition";
  const usesIntegrationsHeader =
    settingsSection === "downloadClients" ||
    settingsSection === "indexers" ||
    settingsSection === "proxies" ||
    settingsSection === "mediaServers" ||
    settingsSection === "notifications";
  const usesAccessHeader =
    settingsSection === "security" || settingsSection === "users";
  const usesSystemHeader = usesAccessHeader || settingsSection === "backups";
  const SettingsSectionIcon = (() => {
    switch (settingsSection) {
      case "rules":
        return SlidersHorizontal;
      case "lifecycleRules":
        return Wrench;
      case "requestRules":
        return Inbox;
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
      case "proxies":
        return Network;
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
          primarySettingsNav.find((item) => item.section === settingsSection)
            ?.icon ?? Settings2
        );
    }
  })();
  const pageLabel = settingsSectionLabel;
  const PageIcon = SettingsSectionIcon;
  const breadcrumbRootLabel = usesAutomationHeader
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
    <div
      className={cn(
        "flex min-h-0 w-full flex-1 flex-col overflow-visible bg-transparent min-[1280px]:overflow-hidden",
        "md:flex-row",
      )}
    >
      <main
        ref={settingsContentRef}
        data-slot="settings-main-scroll"
        className={cn(
          "min-w-0 flex-1 overflow-visible bg-transparent min-[1280px]:overflow-y-auto",
          showReferenceRail && "[scrollbar-gutter:stable]",
        )}
      >
        <div
          ref={settingsPaneRef}
          style={
            referenceContentWidth
              ? { maxWidth: referenceContentWidth }
              : undefined
          }
          className={cn(
            "mx-auto w-full px-4 py-5 pb-[calc(env(safe-area-inset-bottom)+5rem)] sm:px-6 md:px-[30px] md:py-[26px] md:pb-[60px]",
            showReferenceRail
              ? null
              : settingsSection === "rules" ||
                  settingsSection === "lifecycleRules" ||
                  settingsSection === "requestRules" ||
                  settingsSection === "post-processing"
                ? "max-w-none"
                : settingsSection === "users"
                  ? "max-w-[1620px]"
                  : "max-w-[1280px]",
          )}
        >
          <div className="contents">
            <div className={showReferenceRail ? "min-w-0" : "contents"}>
              <div className="mb-4 flex items-center gap-1.5 text-[12.5px] text-[var(--scry-faint)]">
                <span>{breadcrumbRootLabel}</span>
                <ChevronRight className="h-3.5 w-3.5" />
                <span className="font-semibold text-[var(--scry-accent-text)]">
                  {pageLabel}
                </span>
              </div>
              <div
                className={cn(
                  "mb-6 flex gap-3",
                  // The plugins button is small enough to share the title's row
                  // on any screen; other header actions stack below it on phones.
                  showReferenceRail
                    ? "flex-row items-center justify-between"
                    : "flex-col sm:flex-row sm:items-center sm:justify-between",
                )}
              >
                <div className="flex min-w-0 items-center gap-4">
                  <div className="flex h-[46px] w-[46px] shrink-0 items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.35),rgba(123,91,255,0.22))] text-[var(--scry-accent-text)]">
                    <PageIcon className="h-[23px] w-[23px]" />
                  </div>
                  <div className="min-w-0">
                    <h1 className="flex flex-wrap items-center gap-2.5 text-[25px] font-bold tracking-normal text-[var(--scry-ink2)]">
                      {pageLabel}
                      {isRulesSection &&
                      BETA_RULES_SECTIONS.has(
                        rulesSectionFromPath(pathname),
                      ) ? (
                        <Badge
                          id="settings-rules-beta"
                          tone="warning"
                          className="text-[11px]"
                        >
                          {t("settings.pluginBeta")}
                        </Badge>
                      ) : null}
                    </h1>
                    {!usesAutomationHeader &&
                    !usesIntegrationsHeader &&
                    !usesSystemHeader &&
                    settingsSection !== "profile" &&
                    settingsSection !== "general" &&
                    settingsSection !== "qualityProfiles" &&
                    settingsSection !== "delayProfiles" &&
                    settingsSection !== "titleTags" &&
                    settingsSection !== "plugins" &&
                    settingsSection !== "scripts" ? (
                      <p className="mt-1 max-w-[640px] text-[13.5px] text-[var(--scry-muted)]">
                        {t("settings.sectionTitle", {
                          section: settingsSectionLabel,
                        })}
                      </p>
                    ) : null}
                  </div>
                </div>
                {showReferenceRail ? (
                  <GutterPanelButton
                    panelId={SETTINGS_PLUGINS_PANEL_ID}
                    label={t("settings.plugins")}
                    icon={<Puzzle className="h-4 w-4" />}
                    open={referencePanel.open}
                    onOpenChange={setReferencePanelOpen}
                  />
                ) : settingsSection === "plugins" ? (
                  <div
                    id={SETTINGS_HEADER_ACTIONS_SLOT_ID}
                    className="flex min-h-10 shrink-0 flex-wrap items-center justify-end gap-2 sm:min-w-[29rem]"
                  />
                ) : showPluginsShortcut ? (
                  <Button
                    asChild
                    variant="primary"
                    className="h-10 shrink-0 rounded-[10px] px-3 text-[13px]"
                  >
                    <Link to="/settings/plugins">{t("settings.plugins")}</Link>
                  </Button>
                ) : null}
              </div>
              {settingsSection === "indexers" ? (
                <IndexerSettingsTabs activeTab={indexerSettingsTab} t={t} />
              ) : null}
              <GutterPanelContext.Provider
                value={showReferenceRail ? referencePanelContext : null}
              >
                <Suspense
                  fallback={
                    <div className="py-6 text-sm text-[var(--scry-muted3)]">
                      {t("label.loading")}
                    </div>
                  }
                >
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
                      ) : indexerSettingsTab === "search" ? (
                        <SettingsIndexerSearchContainer />
                      ) : indexerSettingsTab === "clients" ? (
                        <SettingsIndexerClientsContainer />
                      ) : (
                        <SettingsIndexersContainer
                          indexerSettingsTab={indexerSettingsTab}
                          providerCatalogVersion={
                            providerCatalogVersions.INDEXER
                          }
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
                        providerCatalogVersions.DOWNLOAD_CLIENT +
                        providerCatalogVersions.ARCHIVE_EXTRACTOR
                      }
                      onDownloadClientsChanged={
                        refreshIndexerDownloadClientMappingCatalog
                      }
                    />
                  ) : settingsSection === "rules" ? (
                    <SettingsRulesContainer
                      canManageCatalogSettings={canManageCatalogSettings}
                      canManageSystemSettings={canManageSystemSettings}
                    />
                  ) : settingsSection === "lifecycleRules" ? (
                    <SettingsLifecycleRulesContainer
                      section={lifecycleRulesSection}
                    />
                  ) : settingsSection === "requestRules" ? (
                    <SettingsRequestRulesContainer />
                  ) : settingsSection === "plugins" ? (
                    <SettingsPluginsContainer />
                  ) : settingsSection === "scripts" ? (
                    <SettingsScriptsContainer />
                  ) : settingsSection === "proxies" ? (
                    <SettingsProxiesContainer />
                  ) : settingsSection === "notifications" ? (
                    <SettingsNotificationsContainer
                      providerCatalogVersion={
                        providerCatalogVersions.NOTIFICATION
                      }
                    />
                  ) : settingsSection === "post-processing" ? (
                    <SettingsPostProcessingContainer />
                  ) : settingsSection === "subtitles" ? (
                    <SettingsSubtitlesContainer
                      providerCatalogVersion={providerCatalogVersions.SUBTITLE}
                    />
                  ) : settingsSection === "delayProfiles" ? (
                    <SettingsDelayProfilesContainer />
                  ) : settingsSection === "titleTags" ? (
                    <SettingsTitleTagsContainer
                      canManageCatalogSettings={canManageCatalogSettings}
                    />
                  ) : settingsSection === "acquisition" ? (
                    <SettingsAcquisitionContainer />
                  ) : (
                    <SettingsQualityProfilesContainer />
                  )}
                </Suspense>
              </GutterPanelContext.Provider>
            </div>
            {showReferenceRail ? (
              <GutterPanel
                id={SETTINGS_PLUGINS_PANEL_ID}
                label={t("settings.plugins")}
                frame={referencePanel.frame}
                open={referencePanel.open}
                onOpenChange={setReferencePanelOpen}
              >
                <div
                  id={SETTINGS_REFERENCE_SLOT_ID}
                  data-slot="settings-reference"
                  className="min-w-0"
                />
              </GutterPanel>
            ) : null}
          </div>
        </div>
      </main>
    </div>
  );
});
