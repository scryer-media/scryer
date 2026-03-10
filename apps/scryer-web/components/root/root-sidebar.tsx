
import * as React from "react";
import { useClient } from "urql";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { LucideIcon } from "lucide-react";
import type {
  ContentSettingsSection,
  SettingsSection,
  Translate,
  ViewId,
} from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import {
  Sidebar,
  SidebarContent,
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
} from "@/components/ui/sidebar";
import { Collapsible, CollapsibleContent } from "@/components/ui/collapsible";
import { ChevronRight } from "lucide-react";

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
  entitlements: string[];
  children?: React.ReactNode;
  onNavigate: (
    nextView: ViewId,
    nextSettingsSection?: SettingsSection,
    nextContentSection?: ContentSettingsSection,
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
];

const MEDIA_SETTINGS_SUB_PAGES: Array<{ id: ContentSettingsSection; labelKey: string }> = [
  { id: "general", labelKey: "facetSettings.general" },
  { id: "quality", labelKey: "facetSettings.quality" },
  { id: "renaming", labelKey: "facetSettings.renaming" },
  { id: "routing", labelKey: "facetSettings.routing" },
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

export const RootSidebar = React.memo(function RootSidebar({
  topNav,
  view,
  settingsSection,
  contentSettingsSection,
  entitlements,
  children,
  onNavigate,
}: RootSidebarProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const client = useClient();
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
    ) => {
      event.preventDefault();
      onNavigate(nextView, nextSettingsSection, nextContentSection);
    },
    [onNavigate],
  );

  return (
    <SidebarProvider>
      <Sidebar
        variant="floating"
        collapsible={isMobile ? "offcanvas" : "none"}
        className="-ml-4 rounded-xl border border-border overflow-hidden"
      >
        <SidebarContent className="bg-background overflow-hidden rounded-lg">
          <SidebarGroup>
            <SidebarMenu className="space-y-1">
              {topNav.map((item, index) => {
                const Icon = item.icon;
                const isMediaSection = ["movies", "series", "anime"].includes(item.id);
                const isSettingsTop = item.id === "settings";
                const isActiveMediaSection = isMediaSection && view === item.id;
                const isActiveSettingsSection = isSettingsTop && view === "settings";
                const shouldShowChildren = isActiveMediaSection || isActiveSettingsSection;
                const showSeparator = index < topNav.length - 1;
                if (!isMediaSection && !isSettingsTop) {
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
                            : (
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
      </Sidebar>
      <SidebarInset className="relative bg-background ml-4">
        {children}
      </SidebarInset>
    </SidebarProvider>
  );
});
