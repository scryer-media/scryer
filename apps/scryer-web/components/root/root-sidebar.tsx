
import * as React from "react";
import { useClient } from "urql";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { LucideIcon } from "lucide-react";
import type {
  ContentSettingsSection,
  SettingsSection,
  SystemSection,
  Translate,
  ViewId,
} from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  SidebarInset,
  SidebarProvider,
  SidebarSeparator,
  SidebarTrigger,
  useSidebar,
} from "@/components/ui/sidebar";
import { Collapsible, CollapsibleContent } from "@/components/ui/collapsible";
import { ChevronRight, Monitor, Moon, Rainbow, Sun } from "lucide-react";
import { useTheme } from "next-themes";
import { getNextTheme, getThemeLabel } from "@/lib/theme";
import { cn } from "@/lib/utils";

type NavItem = {
  id: ViewId;
  label: string;
  icon: LucideIcon;
};

type RootSidebarProps = {
  topNav: NavItem[];
  view: ViewId;
  settingsSection: SettingsSection;
  contentSettingsSection: ContentSettingsSection;
  systemSection: SystemSection;
  entitlements: string[];
  children?: React.ReactNode;
  onNavigate: (
    nextView: ViewId,
    nextSettingsSection?: SettingsSection,
    nextContentSection?: ContentSettingsSection,
    nextSystemSection?: SystemSection,
  ) => void;
};

const settingsEntries: Array<{
  id: SettingsSection;
  label: (t: Translate) => string;
  requiredEntitlement?: string;
}> = [
  {
    id: "profile",
    label: (t) => t("settings.profile"),
  },
  {
    id: "general",
    label: (t) => t("settings.general"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "users",
    label: (t) => t("settings.users"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "qualityProfiles",
    label: (t) => t("settings.qualityProfiles"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "delayProfiles",
    label: (t) => t("settings.delayProfiles"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "downloadClients",
    label: (t) => t("settings.downloadClients"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "indexers",
    label: (t) => t("settings.indexers"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "rules",
    label: (t) => t("settings.rules"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "plugins",
    label: (t) => t("settings.plugins"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "notifications",
    label: (t) => t("settings.notifications"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "post-processing",
    label: (t) => t("settings.postProcessing"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "subtitles",
    label: (t) => t("settings.subtitles"),
    requiredEntitlement: "manage_config",
  },
  {
    id: "recycleBin",
    label: (t) => t("settings.recycleBin"),
    requiredEntitlement: "manage_config",
  },
];

const MEDIA_SETTINGS_SUB_PAGES: Array<{ id: ContentSettingsSection; labelKey: string }> = [
  { id: "general", labelKey: "facetSettings.general" },
  { id: "quality", labelKey: "facetSettings.quality" },
  { id: "renaming", labelKey: "facetSettings.renaming" },
  { id: "routing", labelKey: "facetSettings.routing" },
];

const SYSTEM_SUB_PAGES: Array<{ id: SystemSection; labelKey: string }> = [
  { id: "overview", labelKey: "system.title" },
  { id: "jobs", labelKey: "system.jobsTitle" },
];

function isSettingsSubPage(section: ContentSettingsSection): boolean {
  return section === "settings" || section === "general" || section === "quality" || section === "renaming" || section === "routing";
}

function getMediaOverviewLabel(_viewId: ViewId, t: Translate): string {
  return t("nav.library");
}

function getMediaSettingsLabel(_viewId: ViewId, t: Translate): string {
  return t("nav.settings");
}

function RootSidebarContent({
  topNav,
  view,
  settingsSection,
  contentSettingsSection,
  systemSection,
  entitlements,
  children,
  onNavigate,
}: RootSidebarProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const { setOpenMobile } = useSidebar();
  const client = useClient();
  const { theme, setTheme } = useTheme();
  const [themeMounted, setThemeMounted] = React.useState(false);
  React.useEffect(() => setThemeMounted(true), []);
  const cycleTheme = React.useCallback(() => {
    setTheme(getNextTheme(theme));
  }, [theme, setTheme]);
  const [pluginUpgradeCount, setPluginUpgradeCount] = React.useState(0);

  React.useEffect(() => {
    const q = `query { plugins { updateAvailable } }`;
    client.query(q, {}).toPromise().then(({ data }) => {
      if (data?.plugins) {
        setPluginUpgradeCount(
          (data.plugins as Array<{ updateAvailable: boolean }>).filter((p) => p.updateAvailable).length,
        );
      }
    }).catch(() => { /* ignore */ });

    const onPluginUpdate = (e: Event) => {
      setPluginUpgradeCount((e as CustomEvent<number>).detail);
    };
    window.addEventListener("scryer:pluginUpgradeCount", onPluginUpdate);
    return () => window.removeEventListener("scryer:pluginUpgradeCount", onPluginUpdate);
  }, [client]);

  const visibleSettingsEntries = React.useMemo(
    () => settingsEntries.filter((e) => !e.requiredEntitlement || entitlements.includes(e.requiredEntitlement)),
    [entitlements],
  );

  const handleNavigate = React.useCallback(
    (
      event: React.MouseEvent,
      nextView: ViewId,
      nextSettingsSection?: SettingsSection,
      nextContentSection?: ContentSettingsSection,
      nextSystemSection?: SystemSection,
    ) => {
      event.preventDefault();
      onNavigate(nextView, nextSettingsSection, nextContentSection, nextSystemSection);
      if (isMobile) {
        setOpenMobile(false);
      }
    },
    [isMobile, onNavigate, setOpenMobile],
  );

  const currentTopLevelLabel = React.useMemo(
    () => topNav.find((item) => item.id === view)?.label ?? t("nav.library"),
    [topNav, t, view],
  );

  const currentSubsectionLabel = React.useMemo(() => {
    if (view === "settings") {
      return visibleSettingsEntries.find((entry) => entry.id === settingsSection)?.label(t) ?? null;
    }

    if (view === "movies" || view === "series" || view === "anime") {
      if (contentSettingsSection === "overview") {
        return getMediaOverviewLabel(view, t);
      }

      if (isSettingsSubPage(contentSettingsSection)) {
        const mediaSettingsLabel = MEDIA_SETTINGS_SUB_PAGES.find(
          (subPage) => subPage.id === contentSettingsSection,
        )?.labelKey;
        return mediaSettingsLabel ? t(mediaSettingsLabel) : getMediaSettingsLabel(view, t);
      }
    }

    if (view === "system") {
      return SYSTEM_SUB_PAGES.find((entry) => entry.id === systemSection)?.labelKey
        ? t(SYSTEM_SUB_PAGES.find((entry) => entry.id === systemSection)!.labelKey)
        : null;
    }

    return null;
  }, [contentSettingsSection, settingsSection, systemSection, t, view, visibleSettingsEntries]);

  return (
    <>
      <Sidebar
        variant="floating"
        collapsible={isMobile ? "offcanvas" : "none"}
        className="overflow-hidden rounded-xl border border-border md:-ml-4"
      >
        <SidebarContent className="overflow-y-auto rounded-lg bg-background">
          <SidebarGroup>
            <SidebarMenu className="space-y-1">
              {topNav.map((item, index) => {
                const Icon = item.icon;
                const isMediaSection = ["movies", "series", "anime"].includes(item.id);
                const isSettingsTop = item.id === "settings";
                const isSystemTop = item.id === "system";
                const isActiveMediaSection = isMediaSection && view === item.id;
                const isActiveSettingsSection = isSettingsTop && view === "settings";
                const isActiveSystemSection = isSystemTop && view === "system";
                const shouldShowChildren =
                  isActiveMediaSection || isActiveSettingsSection || isActiveSystemSection;
                const showSeparator = index < topNav.length - 1;
                if (!isMediaSection && !isSettingsTop && !isSystemTop) {
                  return (
                    <React.Fragment key={item.id}>
                      <SidebarMenuItem>
                        <SidebarMenuButton
                          isActive={view === item.id}
                          onClick={(event) => {
                            handleNavigate(event, item.id);
                          }}
                        >
                          <Icon className="h-4 w-4" />
                          {item.label}
                        </SidebarMenuButton>
                      </SidebarMenuItem>

                      {showSeparator ? <SidebarSeparator /> : null}
                    </React.Fragment>
                  );
                }

                return (
                    <React.Fragment key={item.id}>
                      <SidebarMenuItem>
                        <SidebarMenuButton
                          isActive={view === item.id}
                          onClick={(event) => {
                            if (isSettingsTop) {
                              handleNavigate(event, "settings", settingsSection);
                              return;
                            }
                            if (isSystemTop) {
                              handleNavigate(event, "system", undefined, undefined, systemSection);
                              return;
                            }
                            handleNavigate(event, item.id, undefined, contentSettingsSection);
                          }}
                        >
                          <Icon className="h-4 w-4" />
                          {item.label}
                        </SidebarMenuButton>
                      </SidebarMenuItem>

                    {shouldShowChildren ? (
                      <SidebarGroupContent>
                        <SidebarMenuSub>
                          {isSettingsTop
                            ? visibleSettingsEntries.map((entry) => (
                              <SidebarMenuSubItem key={entry.id}>
                                  <SidebarMenuSubButton
                                    isActive={settingsSection === entry.id}
                                    onClick={(event) => {
                                      handleNavigate(event, "settings", entry.id);
                                    }}
                                  >
                                    {entry.label(t)}
                                    {entry.id === "plugins" && pluginUpgradeCount > 0 ? (
                                      <span className="ml-auto inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-red-600 px-1 text-[10px] font-medium leading-none text-white">
                                        {pluginUpgradeCount}
                                      </span>
                                    ) : null}
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                              ))
                            : isSystemTop ? (
                              SYSTEM_SUB_PAGES.map((entry) => (
                                <SidebarMenuSubItem key={entry.id}>
                                  <SidebarMenuSubButton
                                    isActive={systemSection === entry.id}
                                    onClick={(event) => {
                                      handleNavigate(event, "system", undefined, undefined, entry.id);
                                    }}
                                  >
                                    {t(entry.labelKey)}
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                              ))
                            ) : (
                              <>
                                <SidebarMenuSubItem>
                                  <SidebarMenuSubButton
                                    isActive={contentSettingsSection === "overview"}
                                    onClick={(event) => {
                                      handleNavigate(event, item.id, undefined, "overview");
                                    }}
                                  >
                                    {getMediaOverviewLabel(item.id, t)}
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                                <SidebarMenuSubItem>
                                  <Collapsible open={isSettingsSubPage(contentSettingsSection)}>
                                    <SidebarMenuSubButton
                                      isActive={isSettingsSubPage(contentSettingsSection)}
                                      onClick={(event) => {
                                        handleNavigate(event, item.id, undefined, "general");
                                      }}
                                    >
                                      {getMediaSettingsLabel(item.id, t)}
                                      <ChevronRight className={`ml-auto h-3 w-3 transition-transform ${isSettingsSubPage(contentSettingsSection) ? "rotate-90" : ""}`} />
                                    </SidebarMenuSubButton>
                                    <CollapsibleContent>
                                      <SidebarMenuSub>
                                        {MEDIA_SETTINGS_SUB_PAGES.map((subPage) => (
                                          <SidebarMenuSubItem key={subPage.id}>
                                            <SidebarMenuSubButton
                                              isActive={contentSettingsSection === subPage.id}
                                              onClick={(event) => {
                                                handleNavigate(event, item.id, undefined, subPage.id);
                                              }}
                                            >
                                              {t(subPage.labelKey)}
                                            </SidebarMenuSubButton>
                                          </SidebarMenuSubItem>
                                        ))}
                                      </SidebarMenuSub>
                                    </CollapsibleContent>
                                  </Collapsible>
                                </SidebarMenuSubItem>
                              </>
                            )}
                        </SidebarMenuSub>
                      </SidebarGroupContent>
                    ) : null}

                    {showSeparator ? <SidebarSeparator /> : null}
                  </React.Fragment>
                );
              })}
            </SidebarMenu>
          </SidebarGroup>
        </SidebarContent>
        <SidebarFooter className="px-2 py-1.5">
          {themeMounted ? (
            <button
              type="button"
              onClick={cycleTheme}
              aria-label={`Switch theme (current: ${getThemeLabel(theme)})`}
              className={cn(
                "flex w-fit items-center gap-2 rounded-md px-2 py-1.5 text-sm text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                theme === "pride" && "text-pink-200 hover:text-pink-100",
              )}
            >
              {theme === "light" ? (
                <Sun className="h-4 w-4" />
              ) : theme === "dark" ? (
                <Moon className="h-4 w-4" />
              ) : theme === "pride" ? (
                <Rainbow className="h-4 w-4" />
              ) : (
                <Monitor className="h-4 w-4" />
              )}
              Theme
            </button>
          ) : null}
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="relative bg-background md:ml-4">
        <div className="mb-3 flex items-center gap-3 rounded-xl border border-border bg-card/80 px-3 py-2 md:hidden">
          <SidebarTrigger className="size-9 rounded-lg border border-border bg-background text-foreground shadow-none" />
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-foreground">{currentTopLevelLabel}</p>
            {currentSubsectionLabel && currentSubsectionLabel !== currentTopLevelLabel ? (
              <p className="truncate text-xs text-muted-foreground">{currentSubsectionLabel}</p>
            ) : null}
          </div>
        </div>
        {children}
      </SidebarInset>
    </>
  );
}

export const RootSidebar = React.memo(function RootSidebar(props: RootSidebarProps) {
  return (
    <SidebarProvider>
      <RootSidebarContent {...props} />
    </SidebarProvider>
  );
});
