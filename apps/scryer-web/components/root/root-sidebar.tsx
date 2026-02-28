
import * as React from "react";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { LucideIcon } from "lucide-react";
import type {
  ContentSettingsSection,
  SettingsSection,
  Translate,
  ViewId,
} from "@/components/root/types";
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

type NavItem = {
  id: ViewId;
  label: string;
  icon: LucideIcon;
};

type RootSidebarProps = {
  t: Translate;
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
];

function getMediaSectionChildren(
  viewId: ViewId,
  t: Translate,
): Array<{
  id: ContentSettingsSection;
  label: string;
}> {
  if (viewId === "movies") {
    return [
      { id: "overview", label: t("title.manageMovies") },
      { id: "settings", label: t("settings.moviesSettings") },
    ];
  }
  if (viewId === "series") {
    return [
      { id: "overview", label: t("title.manageSeries") },
      { id: "settings", label: t("settings.seriesSettings") },
    ];
  }
  return [
    { id: "overview", label: t("nav.anime") },
    { id: "settings", label: t("settings.animeSettings") },
  ];
}

export const RootSidebar = React.memo(function RootSidebar({
  t,
  topNav,
  view,
  settingsSection,
  contentSettingsSection,
  entitlements,
  children,
  onNavigate,
}: RootSidebarProps) {
  const isMobile = useIsMobile();

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
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                              ))
                            : getMediaSectionChildren(item.id, t).map((entry) => (
                                <SidebarMenuSubItem key={`${item.id}-${entry.id}`}>
                                  <SidebarMenuSubButton
                                    isActive={contentSettingsSection === entry.id}
                                    onClick={(event) => {
                                      handleNavigate(event, item.id, undefined, entry.id);
                                    }}
                                  >
                                    {entry.label}
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                              ))}
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
