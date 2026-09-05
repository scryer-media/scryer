import * as React from "react";
import type { LucideIcon } from "lucide-react";
import { useClient } from "urql";
import type {
  ActivitySection,
  ContentSettingsSection,
  LogsSection,
  SettingsSection,
  SystemSection,
  Translate,
  ViewId,
  WantedSection,
} from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  SidebarInset,
  SidebarProvider,
  SidebarTrigger,
  useSidebar,
} from "@/components/ui/sidebar";
import {
  Archive,
  Bell,
  BookOpen,
  Captions,
  ChevronDown,
  Database,
  Download,
  FileText,
  FolderCog,
  Heart,
  Inbox,
  MessagesSquare,
  Monitor,
  Moon,
  Network,
  Puzzle,
  Rss,
  Server,
  Settings2,
  Wrench,
  ShieldCheck,
  SlidersHorizontal,
  Sun,
  TextSearch,
  Timer,
  Recycle,
  User,
  Users,
} from "lucide-react";
import { useTheme } from "next-themes";
import { fromUiThemeValue, getNextTheme, toUiThemeValue } from "@/lib/theme";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { cn } from "@/lib/utils";
import type { PendingImportCounts } from "@/lib/types";
import { pendingImportCountForView } from "@/lib/types";
import { setMyUiSettingsMutation } from "@/lib/graphql/mutations";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useExperimentalFeaturesEnabled } from "@/lib/context/instance-features-context";
import {
  useUiSettings,
  uiSettingsInputFromSettings,
} from "@/lib/context/ui-settings-context";
import type { AuthUser } from "@/lib/hooks/use-auth";
import type { UiSettings } from "@/lib/types/settings";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  hasAnyAppPermission,
  hasAnyLibraryPermission,
} from "@/lib/utils/permissions";
import type { AppPermission, LibraryPermission } from "@/lib/utils/permissions";
import {
  canAccessDashboard,
  canAccessSystemSection,
} from "@/lib/utils/routes";
import { selectorId } from "@/lib/utils/dom-ids";

type NavItem = {
  id: ViewId;
  label: string;
  icon: LucideIcon;
};

type TopNavGroupDefinition = {
  id: string;
  labelKey: string;
  label?: string;
  items: TopNavGroupItemDefinition[];
};

type TopNavGroup = {
  id: string;
  label: string;
  items: TopNavGroupItem[];
};

type TopNavGroupItemDefinition =
  | { kind: "view"; id: ViewId }
  | { kind: "requests"; icon: LucideIcon }
  | { kind: "logs"; id: LogsSection; labelKey: string; icon: LucideIcon }
  | { kind: "system"; id: SystemSection; labelKey: string; icon: LucideIcon }
  | {
      kind: "settings";
      id: SettingsSection;
      labelKey?: string;
      icon: LucideIcon;
    };

/// Whether a sidebar settings entry owns the section currently open. Rules is
/// one entry over two sections, so it stays lit while either kind of rule is
/// open; every other entry is the section it names.
function isSettingsNavEntryActive(
  entryId: SettingsSection,
  settingsSection: SettingsSection,
): boolean {
  if (entryId === "rules") {
    return (
      settingsSection === "rules" ||
      settingsSection === "maintenanceRules" ||
      settingsSection === "requestRules"
    );
  }
  return entryId === settingsSection;
}

type TopNavGroupItem =
  | (NavItem & { kind: "view" })
  | {
      kind: "requests";
      id: "requests";
      label: string;
      icon: LucideIcon;
    }
  | {
      kind: "logs";
      id: LogsSection;
      label: string;
      icon: LucideIcon;
    }
  | {
      kind: "system";
      id: SystemSection;
      label: string;
      icon: LucideIcon;
    }
  | {
      kind: "settings";
      id: SettingsSection;
      label: string;
      icon: LucideIcon;
    };

type HeaderWithMobileNavigationProps = {
  mobileNavigation?: React.ReactNode;
};

const TOP_NAV_GROUPS: TopNavGroupDefinition[] = [
  {
    id: "overview",
    labelKey: "nav.group.overview",
    items: [{ kind: "view", id: "dashboard" }],
  },
  {
    id: "catalogs",
    labelKey: "nav.group.catalogs",
    items: [
      { kind: "view", id: "movies" },
      { kind: "view", id: "series" },
      { kind: "view", id: "anime" },
    ],
  },
  {
    id: "discover",
    label: "Discover",
    labelKey: "nav.group.discover",
    items: [
      { kind: "view", id: "discovery" },
      { kind: "requests", icon: Inbox },
    ],
  },
  {
    id: "automation",
    labelKey: "nav.group.automation",
    items: [
      { kind: "view", id: "wanted" },
      { kind: "view", id: "calendar" },
      { kind: "view", id: "activity" },
      { kind: "settings", id: "subtitles", icon: Captions },
      // Scoring, maintenance and request rules share this entry; the Rules
      // page's own gutter picks the kind, so the sidebar names the subject once.
      { kind: "settings", id: "rules", labelKey: "nav.rules", icon: SlidersHorizontal },
      { kind: "settings", id: "post-processing", icon: FolderCog },
    ],
  },
  {
    id: "integrations",
    labelKey: "nav.group.integrations",
    items: [
      { kind: "settings", id: "indexers", icon: Database },
      { kind: "settings", id: "downloadClients", icon: Download },
      { kind: "settings", id: "proxies", icon: Network },
      { kind: "settings", id: "mediaServers", icon: Server },
      { kind: "settings", id: "notifications", icon: Bell },
    ],
  },
  {
    id: "system",
    labelKey: "nav.group.system",
    items: [
      {
        kind: "settings",
        id: "users",
        labelKey: "nav.usersAccess",
        icon: Users,
      },
      { kind: "settings", id: "security", icon: ShieldCheck },
      { kind: "view", id: "system" },
      { kind: "system", id: "jobs", labelKey: "system.jobsTitle", icon: Timer },
      {
        kind: "system",
        id: "recycleBin",
        labelKey: "settings.recycleBin",
        icon: Recycle,
      },
      { kind: "settings", id: "backups", icon: Archive },
      { kind: "view", id: "settings" },
    ],
  },
  {
    id: "logs",
    labelKey: "nav.logs",
    items: [
      { kind: "logs", id: "logs", labelKey: "nav.serviceLogs", icon: FileText },
      { kind: "logs", id: "audit", labelKey: "nav.auditLogs", icon: TextSearch },
    ],
  },
];

const PROMOTED_SETTINGS_SHORTCUT_IDS = new Set<SettingsSection>(
  TOP_NAV_GROUPS.flatMap((group) =>
    group.items.flatMap((item) => (item.kind === "settings" ? [item.id] : [])),
  ),
);

/// Whether a promoted shortcut already owns the open section, which is what
/// keeps the Settings entry dark. Ownership runs through the same predicate the
/// shortcuts light themselves with, so an entry standing in for more than one
/// section covers all of them here too.
function isPromotedSettingsSection(settingsSection: SettingsSection): boolean {
  for (const entryId of PROMOTED_SETTINGS_SHORTCUT_IDS) {
    if (isSettingsNavEntryActive(entryId, settingsSection)) {
      return true;
    }
  }
  return false;
}
const DEFAULT_SETTINGS_SECTION_ORDER: SettingsSection[] = [
  "general",
  "profile",
  "qualityProfiles",
  "delayProfiles",
  "plugins",
];
const MEDIA_NAV_VIEW_IDS: ViewId[] = ["movies", "series", "anime"];

function sidebarPublicAssetUrl(path: string): string {
  const basePath = getRuntimeBasePath();
  return `${basePath === "/" ? "" : basePath}/${path.replace(/^\/+/, "")}`;
}

type RootSidebarProps = {
  topNav: NavItem[];
  view: ViewId;
  settingsSection: SettingsSection;
  contentSettingsSection: ContentSettingsSection;
  systemSection: SystemSection;
  logsSection: LogsSection;
  activitySection: ActivitySection;
  wantedSection: WantedSection;
  user: AuthUser;
  pendingImportCounts: PendingImportCounts | null;
  pendingMediaRequestCounts: PendingImportCounts | null;
  manualImportRequiredCount: number;
  pluginUpdateCount: number;
  header?: React.ReactNode;
  children?: React.ReactNode;
  onNavigate: (
    nextView: ViewId,
    nextSettingsSection?: SettingsSection,
    nextContentSection?: ContentSettingsSection,
    nextSystemSection?: SystemSection,
    nextWantedSection?: WantedSection,
    nextActivitySection?: ActivitySection,
    nextLogsSection?: LogsSection,
  ) => void;
};

const settingsEntries: Array<{
  id: SettingsSection;
  label: (t: Translate) => string;
  icon?: LucideIcon;
  requiredAnyAppPermission?: AppPermission[];
  requiredAnyLibraryPermission?: LibraryPermission[];
}> = [
  {
    id: "profile",
    label: (t) => t("settings.profile"),
    icon: User,
  },
  {
    id: "general",
    label: (t) => t("settings.general"),
    icon: Settings2,
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "backups",
    label: (t) => t("settings.backups"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "security",
    label: (t) => t("settings.security"),
    icon: ShieldCheck,
    requiredAnyAppPermission: [APP_PERMISSIONS.manageUsers],
  },
  {
    id: "users",
    label: (t) => t("settings.users"),
    requiredAnyAppPermission: [
      APP_PERMISSIONS.manageUsers,
      APP_PERMISSIONS.managePermissions,
    ],
  },
  {
    id: "mediaServers",
    label: (t) => t("settings.mediaServers"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "qualityProfiles",
    label: (t) => t("settings.qualityProfiles"),
    icon: SlidersHorizontal,
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "delayProfiles",
    label: (t) => t("settings.delayProfiles"),
    icon: Timer,
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "downloadClients",
    label: (t) => t("settings.downloadClients"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "acquisition",
    label: (t) => t("settings.acquisition"),
    icon: Rss,
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "indexers",
    label: (t) => t("settings.indexers"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "proxies",
    label: (t) => t("settings.proxies"),
    icon: Network,
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "rules",
    label: (t) => t("settings.rules"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "maintenanceRules",
    label: (t) => t("settings.maintenanceRules"),
    icon: Wrench,
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "requestRules",
    label: (t) => t("settings.requestRules"),
    icon: Inbox,
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "plugins",
    label: (t) => t("settings.plugins"),
    icon: Puzzle,
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "notifications",
    label: (t) => t("settings.notifications"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "post-processing",
    label: (t) => t("settings.postProcessing"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "subtitles",
    label: (t) => t("settings.subtitles"),
    icon: Captions,
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
];

const SETTINGS_NAV_GROUPS: Array<{
  id: string;
  labelKey: string;
  itemIds: SettingsSection[];
}> = [
  {
    id: "settings",
    labelKey: "nav.settings",
    itemIds: [
      "profile",
      "general",
      "qualityProfiles",
      "delayProfiles",
      "plugins",
    ],
  },
];

const MEDIA_SETTINGS_SUB_PAGES: Array<{
  id: ContentSettingsSection;
  labelKey: string;
}> = [
  { id: "library", labelKey: "nav.library" },
  { id: "general", labelKey: "facetSettings.general" },
  { id: "quality", labelKey: "facetSettings.quality" },
  { id: "renaming", labelKey: "facetSettings.renaming" },
  { id: "routing", labelKey: "facetSettings.routing" },
];

const SYSTEM_SUB_PAGES: Array<{ id: SystemSection; labelKey: string }> = [
  { id: "overview", labelKey: "system.title" },
];

const LOGS_SUB_PAGES: Array<{ id: LogsSection; labelKey: string }> = [
  { id: "logs", labelKey: "nav.serviceLogs" },
  { id: "audit", labelKey: "nav.auditLogs" },
];

const ACTIVITY_SUB_PAGES: Array<{ id: ActivitySection; labelKey: string }> = [
  { id: "import", labelKey: "activity.import" },
  { id: "activity", labelKey: "activity.activity" },
  { id: "history", labelKey: "activity.history" },
];

const WANTED_SUB_PAGES: Array<{ id: WantedSection; labelKey: string }> = [
  { id: "wanted", labelKey: "wanted.tabWanted" },
  { id: "cutoff", labelKey: "wanted.tabCutoff" },
  { id: "pending", labelKey: "wanted.tabPending" },
];

const SIDEBAR_COLLAPSED_GROUPS_STORAGE_KEY = "scryer:sidebar-collapsed-groups";

function readCollapsedTopNavGroups(): ReadonlySet<string> {
  if (typeof window === "undefined") {
    return new Set();
  }

  try {
    const storedValue = window.localStorage.getItem(
      SIDEBAR_COLLAPSED_GROUPS_STORAGE_KEY,
    );
    const parsedValue: unknown = storedValue ? JSON.parse(storedValue) : [];
    return new Set(
      Array.isArray(parsedValue)
        ? parsedValue.filter((value): value is string => typeof value === "string")
        : [],
    );
  } catch {
    return new Set();
  }
}

function persistCollapsedTopNavGroups(groups: ReadonlySet<string>) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(
      SIDEBAR_COLLAPSED_GROUPS_STORAGE_KEY,
      JSON.stringify([...groups]),
    );
  } catch {
    // Local storage can be unavailable in private browsing contexts.
  }
}

const LEAF_NAV_BADGE_BASE_CLASS =
  "ml-auto inline-flex h-4 min-w-4 items-center justify-center rounded-md px-1 text-[10px] font-medium leading-none tabular-nums";

const TOP_NAV_BUTTON_CLASS =
  "h-9 rounded-[10px] px-2.5 text-[13px] font-medium transition-colors hover:!bg-[var(--scry-hover)] hover:text-sidebar-accent-foreground data-[active=true]:bg-[linear-gradient(90deg,rgba(var(--scry-accent-rgb),0.30),rgba(var(--scry-accent-rgb),0.10))] data-[active=true]:font-semibold data-[active=true]:text-foreground data-[active=true]:shadow-[inset_2px_0_0_rgb(var(--scry-accent-rgb)),0_8px_18px_rgba(var(--scry-accent-rgb),0.16)] data-[active=true]:hover:!bg-[linear-gradient(90deg,rgba(var(--scry-accent-rgb),0.30),rgba(var(--scry-accent-rgb),0.10))] data-[active=true]:[&>svg]:text-primary";

const SUB_NAV_BUTTON_CLASS =
  "min-h-[30px] rounded-[8px] px-[11px] text-[12.5px] font-medium !text-[var(--scry-muted)] transition-colors hover:!bg-[var(--scry-hover)] hover:!text-[var(--scry-body)] data-[active=true]:!bg-[var(--scry-hover)] data-[active=true]:!bg-none data-[active=true]:font-semibold data-[active=true]:!text-[var(--scry-text2)]";

const SUB_NAV_MENU_CLASS =
  "mx-0 mb-0.5 ml-[17px] mt-0.5 gap-0.5 border-[var(--scry-border2)] px-0 py-0 pl-[11px]";

const TOP_NAV_BADGE_GROUP_CLASS =
  "pointer-events-none absolute right-1 flex items-center gap-1 select-none peer-data-[size=sm]/menu-button:top-1 peer-data-[size=default]/menu-button:top-1.5 peer-data-[size=lg]/menu-button:top-2.5 group-data-[collapsible=icon]:hidden";

const TOP_NAV_BADGE_BASE_CLASS =
  "inline-flex h-5 min-w-5 items-center justify-center rounded-md px-1 text-xs font-medium tabular-nums";

type NavBadgeTone = "cta" | "danger" | "warning" | "request";

function navBadgeToneClass(tone: NavBadgeTone) {
  switch (tone) {
    case "danger":
      return "bg-[var(--scry-danger-solid)] text-[var(--scry-danger-on-solid)]";
    case "warning":
      return "bg-[var(--scry-warning-solid)] text-[var(--scry-warning-on-solid)] peer-hover/menu-button:text-[var(--scry-warning-on-solid)] peer-data-[active=true]/menu-button:text-[var(--scry-warning-on-solid)]";
    case "request":
      return "bg-primary text-primary-foreground";
    case "cta":
    default:
      return "bg-primary text-primary-foreground";
  }
}

function FacetNavBadges({ importCount }: { importCount: number }) {
  if (importCount <= 0) {
    return null;
  }

  return (
    <div className={TOP_NAV_BADGE_GROUP_CLASS}>
      <span className={cn(TOP_NAV_BADGE_BASE_CLASS, navBadgeToneClass("cta"))}>
        {importCount}
      </span>
    </div>
  );
}

function LeafNavBadge({
  count,
  tone = "cta",
}: {
  count: number;
  tone?: NavBadgeTone;
}) {
  return (
    <span className={cn(LEAF_NAV_BADGE_BASE_CLASS, navBadgeToneClass(tone))}>
      {count}
    </span>
  );
}

function isSettingsSubPage(section: ContentSettingsSection): boolean {
  return (
    section === "library" ||
    section === "general" ||
    section === "quality" ||
    section === "renaming" ||
    section === "routing"
  );
}

function getMediaOverviewLabel(_viewId: ViewId, t: Translate): string {
  return t("nav.catalog");
}

function getMediaSettingsLabel(_viewId: ViewId, t: Translate): string {
  return t("nav.settings");
}

function getThemeLabel(theme: string | undefined, t: Translate): string {
  switch (theme) {
    case "light":
      return t("theme.light");
    case "dark":
      return t("theme.dark");
    case "pride":
      return t("theme.dark");
    default:
      return t("theme.system");
  }
}

function RootSidebarContent({
  topNav,
  view,
  settingsSection,
  contentSettingsSection,
  systemSection,
  logsSection,
  activitySection,
  wantedSection,
  user,
  pendingImportCounts,
  pendingMediaRequestCounts,
  manualImportRequiredCount,
  pluginUpdateCount,
  header,
  children,
  onNavigate,
}: RootSidebarProps) {
  const client = useClient();
  const t = useTranslate();
  const experimentalFeaturesEnabled = useExperimentalFeaturesEnabled();
  const setGlobalStatus = useGlobalStatus();
  const { isMobile, setOpenMobile } = useSidebar();
  const { theme, resolvedTheme, setTheme } = useTheme();
  const {
    uiSettings,
    uiSettingsLoaded,
    uiSettingsLoading,
    setUiSettings,
  } = useUiSettings();
  const themeSaveSequenceRef = React.useRef(0);
  const [themeMounted, setThemeMounted] = React.useState(false);
  const brandLogoUrl =
    themeMounted && resolvedTheme === "light"
      ? `${import.meta.env.BASE_URL}scryer-lockup-dark.webp`
      : `${import.meta.env.BASE_URL}scryer-lockup-light.webp`;
  const [collapsedTopNavGroups, setCollapsedTopNavGroups] = React.useState<
    ReadonlySet<string>
  >(readCollapsedTopNavGroups);
  React.useEffect(() => setThemeMounted(true), []);
  const toggleTopNavGroup = React.useCallback((groupId: string) => {
    setCollapsedTopNavGroups((previous) => {
      const next = new Set(previous);
      if (next.has(groupId)) {
        next.delete(groupId);
      } else {
        next.add(groupId);
      }
      persistCollapsedTopNavGroups(next);
      return next;
    });
  }, []);
  const cycleTheme = React.useCallback(() => {
    const nextTheme = getNextTheme(theme);
    setTheme(nextTheme);

    if (
      !uiSettingsLoaded ||
      uiSettingsLoading ||
      uiSettings.theme === toUiThemeValue(nextTheme)
    ) {
      return;
    }

    const requestId = themeSaveSequenceRef.current + 1;
    themeSaveSequenceRef.current = requestId;
    const previous = uiSettings;
    const next: UiSettings = { ...uiSettings, theme: toUiThemeValue(nextTheme) };
    setUiSettings(next);
    void client
      .mutation<{ setMyUiSettings?: UiSettings }, { input: UiSettings }>(
        setMyUiSettingsMutation,
        { input: uiSettingsInputFromSettings(next) },
      )
      .toPromise()
      .then((result) => {
        if (themeSaveSequenceRef.current !== requestId) return;
        if (result.error || !result.data?.setMyUiSettings) {
          setUiSettings(previous);
          setTheme(fromUiThemeValue(previous.theme));
          setGlobalStatus(result.error?.message ?? t("status.failedToUpdate"));
          return;
        }
        setUiSettings(result.data.setMyUiSettings);
      })
      .catch((error) => {
        if (themeSaveSequenceRef.current !== requestId) return;
        setUiSettings(previous);
        setTheme(fromUiThemeValue(previous.theme));
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      });
  }, [
    client,
    setGlobalStatus,
    setTheme,
    setUiSettings,
    t,
    theme,
    uiSettings,
    uiSettingsLoaded,
    uiSettingsLoading,
  ]);
  const displayTheme = theme === "pride" ? "dark" : theme;
  const themeLabel = getThemeLabel(displayTheme, t);
  const canManageSystemSettings = hasAnyAppPermission(user, [
    APP_PERMISSIONS.manageSystemSettings,
  ]);
  const canManageCatalogSettings = hasAnyAppPermission(user, [
    APP_PERMISSIONS.manageCatalogSettings,
  ]);
  const canManageConfig = canManageSystemSettings || canManageCatalogSettings;
  const canManageLibrarySettings =
    canManageConfig ||
    hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageLibrary);
  const canViewCatalog = hasAnyLibraryPermission(
    user,
    LIBRARY_PERMISSIONS.view,
  );
  const canManageTitle = hasAnyLibraryPermission(
    user,
    LIBRARY_PERMISSIONS.manageTitles,
  );
  const canRequestMedia = hasAnyLibraryPermission(
    user,
    LIBRARY_PERMISSIONS.request,
  );
  const canResolveImports = hasAnyLibraryPermission(
    user,
    LIBRARY_PERMISSIONS.resolveImports,
  );
  const canAccessFacetImport = canResolveImports;
  const visibleMediaSettingsSubPages = React.useMemo(
    () =>
      canManageConfig
        ? MEDIA_SETTINGS_SUB_PAGES
        : canManageLibrarySettings
          ? MEDIA_SETTINGS_SUB_PAGES.filter(
              (subPage) => subPage.id === "library",
            )
          : [],
    [canManageConfig, canManageLibrarySettings],
  );
  const canAccessMediaSettings = visibleMediaSettingsSubPages.length > 0;

  const visibleSettingsEntries = React.useMemo(
    () =>
      settingsEntries.filter(
        (entry) =>
          // Maintenance and request rules are still being finished, so their
          // shortcuts are offered only when the instance has opted in. The
          // permission each already required still applies on top.
          ((entry.id !== "maintenanceRules" && entry.id !== "requestRules") ||
            experimentalFeaturesEnabled) &&
          (!entry.requiredAnyAppPermission ||
            hasAnyAppPermission(user, entry.requiredAnyAppPermission) ||
            entry.requiredAnyLibraryPermission?.some((permission) =>
              hasAnyLibraryPermission(user, permission),
            )),
      ),
    [experimentalFeaturesEnabled, user],
  );
  const groupedSettingsEntries = React.useMemo(() => {
    const entriesById = new Map(
      visibleSettingsEntries.map((entry) => [entry.id, entry]),
    );
    return SETTINGS_NAV_GROUPS.map((group) => ({
      ...group,
      entries: group.itemIds.flatMap((id) => {
        const entry = entriesById.get(id);
        return entry ? [entry] : [];
      }),
    })).filter((group) => group.entries.length > 0);
  }, [visibleSettingsEntries]);
  const canAccessMediaTopNav =
    canViewCatalog || canResolveImports || canManageLibrarySettings;
  const defaultMediaContentSection: ContentSettingsSection = canViewCatalog
    ? "overview"
    : canResolveImports
      ? "import"
      : "library";
  const visibleTopNav = React.useMemo(
    () =>
      topNav.filter(
        (item) =>
          (!MEDIA_NAV_VIEW_IDS.includes(item.id) || canAccessMediaTopNav) &&
          (item.id !== "calendar" || canViewCatalog) &&
          (item.id !== "wanted" || canViewCatalog) &&
          (item.id !== "dashboard" || canAccessDashboard(canManageSystemSettings)) &&
          (item.id !== "system" || canManageSystemSettings) &&
          (item.id !== "activity" || canResolveImports || canManageTitle),
      ),
    [
      canAccessMediaTopNav,
      canManageSystemSettings,
      canManageTitle,
      canResolveImports,
      canViewCatalog,
      topNav,
    ],
  );
  const groupedTopNav = React.useMemo<TopNavGroup[]>(() => {
    const itemsById = new Map(visibleTopNav.map((item) => [item.id, item]));
    const settingsEntriesById = new Map(
      visibleSettingsEntries.map((entry) => [entry.id, entry]),
    );
    const groupedIds = new Set<ViewId>();
    const groups = TOP_NAV_GROUPS.map((group) => {
      const items = group.items.flatMap<TopNavGroupItem>((definition) => {
        if (definition.kind === "settings") {
          const entry = settingsEntriesById.get(definition.id);
          if (!entry) {
            return [];
          }
          return [
            {
              kind: "settings",
              id: definition.id,
              label: definition.labelKey
                ? t(definition.labelKey)
                : entry.label(t),
              icon: definition.icon,
            },
          ];
        }
        if (definition.kind === "requests") {
          if (!canManageTitle && !canRequestMedia) {
            return [];
          }
          return [
            {
              kind: "requests",
              id: "requests",
              label: t("nav.requests"),
              icon: definition.icon,
            },
          ];
        }
        if (definition.kind === "logs") {
          if (!canManageSystemSettings) {
            return [];
          }
          return [
            {
              kind: "logs",
              id: definition.id,
              label: t(definition.labelKey),
              icon: definition.icon,
            },
          ];
        }
        if (definition.kind === "system") {
          const canAccess = canAccessSystemSection(
            definition.id,
            canManageSystemSettings,
            canManageTitle,
          );
          if (!canAccess) {
            return [];
          }
          return [
            {
              kind: "system",
              id: definition.id,
              label: t(definition.labelKey),
              icon: definition.icon,
            },
          ];
        }

        const item = itemsById.get(definition.id);
        if (!item) {
          return [];
        }
        groupedIds.add(definition.id);
        return [{ kind: "view", ...item }];
      });
      return { id: group.id, label: group.label ?? t(group.labelKey), items };
    }).filter((group) => group.items.length > 0);
    const ungroupedItems = visibleTopNav
      .filter((item) => !groupedIds.has(item.id))
      .map<TopNavGroupItem>((item) => ({ kind: "view", ...item }));

    return ungroupedItems.length > 0
      ? [
          ...groups,
          { id: "more", label: t("nav.group.more"), items: ungroupedItems },
        ]
      : groups;
  }, [
    canManageSystemSettings,
    canManageTitle,
    canRequestMedia,
    t,
    visibleSettingsEntries,
    visibleTopNav,
  ]);
  const defaultSettingsSectionForTopNav = React.useMemo<SettingsSection>(() => {
    const visibleIds = new Set(visibleSettingsEntries.map((entry) => entry.id));
    return (
      DEFAULT_SETTINGS_SECTION_ORDER.find((section) =>
        visibleIds.has(section),
      ) ??
      visibleSettingsEntries[0]?.id ??
      "profile"
    );
  }, [visibleSettingsEntries]);

  const pendingImportCountForNavView = React.useCallback(
    (viewId: ViewId) => pendingImportCountForView(pendingImportCounts, viewId),
    [pendingImportCounts],
  );
  const pendingMediaRequestCountForNavView = React.useCallback(
    (viewId: ViewId) =>
      pendingImportCountForView(pendingMediaRequestCounts, viewId),
    [pendingMediaRequestCounts],
  );
  const pendingMediaRequestCount = MEDIA_NAV_VIEW_IDS.reduce(
    (total, viewId) => total + pendingMediaRequestCountForNavView(viewId),
    0,
  );
  const isRequestsSection = view === "requests";
  const activityImportBadgeCount = Math.max(0, manualImportRequiredCount);
  const hasActivityImportBadge = activityImportBadgeCount > 0;
  const visibleActivitySubPages = React.useMemo(
    () =>
      ACTIVITY_SUB_PAGES.filter(
        (entry) => entry.id !== "import" || hasActivityImportBadge,
      ),
    [hasActivityImportBadge],
  );
  const hasVisibleActivitySubnav = React.useMemo(
    () => visibleActivitySubPages.some((entry) => entry.id !== "activity"),
    [visibleActivitySubPages],
  );
  const visibleWantedSubPages = WANTED_SUB_PAGES;

  const handleNavigate = React.useCallback(
    (
      event: React.MouseEvent,
      nextView: ViewId,
      nextSettingsSection?: SettingsSection,
      nextContentSection?: ContentSettingsSection,
      nextSystemSection?: SystemSection,
      nextWantedSection?: WantedSection,
      nextActivitySection?: ActivitySection,
      nextLogsSection?: LogsSection,
    ) => {
      event.preventDefault();
      onNavigate(
        nextView,
        nextSettingsSection,
        nextContentSection,
        nextSystemSection,
        nextWantedSection,
        nextActivitySection,
        nextLogsSection,
      );
      if (isMobile) {
        setOpenMobile(false);
      }
    },
    [isMobile, onNavigate, setOpenMobile],
  );

  const currentTopLevelLabel = React.useMemo(() => {
    if (isRequestsSection) {
      return t("nav.requests");
    }
    if (view === "logs") {
      return t("nav.logs");
    }

    return (
      visibleTopNav.find((item) => item.id === view)?.label ??
      topNav.find((item) => item.id === view)?.label ??
      t("nav.library")
    );
  }, [isRequestsSection, topNav, t, view, visibleTopNav]);

  const currentSubsectionLabel = React.useMemo(() => {
    if (view === "settings") {
      return (
        visibleSettingsEntries
          .find((entry) => entry.id === settingsSection)
          ?.label(t) ?? null
      );
    }

    if (view === "movies" || view === "series" || view === "anime") {
      if (contentSettingsSection === "overview") {
        return getMediaOverviewLabel(view, t);
      }

      if (contentSettingsSection === "import") {
        return canAccessFacetImport
          ? t("nav.import")
          : getMediaOverviewLabel(view, t);
      }

      if (isSettingsSubPage(contentSettingsSection)) {
        if (!canAccessMediaSettings) {
          return getMediaOverviewLabel(view, t);
        }
        const mediaSettingsLabel = visibleMediaSettingsSubPages.find(
          (subPage) => subPage.id === contentSettingsSection,
        )?.labelKey;
        return mediaSettingsLabel
          ? t(mediaSettingsLabel)
          : getMediaOverviewLabel(view, t);
      }
    }

    if (view === "system") {
      return SYSTEM_SUB_PAGES.find((entry) => entry.id === systemSection)
        ?.labelKey
        ? t(
            SYSTEM_SUB_PAGES.find((entry) => entry.id === systemSection)!
              .labelKey,
          )
        : null;
    }

    if (view === "logs") {
      return LOGS_SUB_PAGES.find((entry) => entry.id === logsSection)?.labelKey
        ? t(LOGS_SUB_PAGES.find((entry) => entry.id === logsSection)!.labelKey)
        : null;
    }

    if (view === "activity") {
      return visibleActivitySubPages.find(
        (entry) => entry.id === activitySection,
      )?.labelKey
        ? t(
            visibleActivitySubPages.find(
              (entry) => entry.id === activitySection,
            )!.labelKey,
          )
        : null;
    }

    if (view === "wanted") {
      return visibleWantedSubPages.find((entry) => entry.id === wantedSection)
        ?.labelKey
        ? t(
            visibleWantedSubPages.find((entry) => entry.id === wantedSection)!
              .labelKey,
          )
        : null;
    }

    return null;
  }, [
    contentSettingsSection,
    settingsSection,
    systemSection,
    logsSection,
    activitySection,
    t,
    view,
    canAccessFacetImport,
    canAccessMediaSettings,
    visibleMediaSettingsSubPages,
    visibleActivitySubPages,
    visibleSettingsEntries,
    visibleWantedSubPages,
    wantedSection,
  ]);

  const mobileNavigationTrigger = (
    <SidebarTrigger
      id="root-sidebar-mobile-trigger"
      aria-label={t("nav.mobileTrigger")}
      className="size-10 rounded-[11px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[var(--scry-muted2)] shadow-none transition hover:border-[var(--scry-bhover2)] hover:bg-[var(--scry-hover)] hover:text-foreground focus-visible:ring-primary/25 min-[981px]:hidden"
    />
  );
  const canInjectMobileNavigation =
    React.isValidElement<HeaderWithMobileNavigationProps>(header) &&
    typeof header.type !== "string";
  const headerWithMobileNavigation = canInjectMobileNavigation
    ? React.cloneElement(header, { mobileNavigation: mobileNavigationTrigger })
    : header;

  return (
    <>
      <Sidebar
        variant="sidebar"
        collapsible={isMobile ? "offcanvas" : "none"}
        mobileTitle={t("nav.mobileTitle")}
        mobileDescription={t("nav.mobileDescription")}
        className="overflow-hidden border-r border-[var(--scry-border3)] bg-[var(--scry-bg)] shadow-[12px_0_40px_rgba(2,6,23,0.22)] min-[981px]:sticky min-[981px]:top-[var(--root-shell-top-offset,0px)] min-[981px]:h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:max-h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:self-start"
      >
        <SidebarHeader className="px-4 py-0">
          <div className="flex w-full items-center justify-center">
            <img
              src={brandLogoUrl}
              alt="Scryer"
              className="h-auto w-[220px] object-contain dark:drop-shadow-[0_12px_22px_rgba(var(--scry-accent-rgb),0.32)]"
            />
          </div>
        </SidebarHeader>
        <SidebarContent className="overflow-y-auto px-3 pb-3 [scrollbar-color:var(--scry-border2)_transparent] [scrollbar-width:thin] [&::-webkit-scrollbar]:w-2.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:border-[3px] [&::-webkit-scrollbar-thumb]:border-transparent [&::-webkit-scrollbar-thumb]:bg-[var(--scry-border2)] [&::-webkit-scrollbar-thumb]:bg-clip-content">
          {groupedTopNav.map((group) => {
            const isCollapsed = collapsedTopNavGroups.has(group.id);
            const groupContentId = selectorId("root-sidebar-group", group.id);

            return (
            <SidebarGroup
              key={group.id}
              className="px-0 py-1 first:pt-0"
            >
              <SidebarGroupLabel asChild className="h-auto p-0">
                <button
                  type="button"
                  className="flex w-full items-center justify-between rounded-md px-1 py-1.5 text-left text-[10.5px] font-bold uppercase tracking-[0.13em] text-[var(--scry-faint3)] hover:text-[var(--scry-ink2)] focus-visible:ring-2 focus-visible:ring-ring"
                  aria-controls={groupContentId}
                  aria-expanded={!isCollapsed}
                  onClick={() => toggleTopNavGroup(group.id)}
                >
                  {group.label}
                  <ChevronDown
                    className={cn(
                      "h-4 w-4 transition-transform",
                      isCollapsed && "-rotate-90",
                    )}
                  />
                </button>
              </SidebarGroupLabel>
              {isCollapsed ? null : (
              <SidebarMenu id={groupContentId} className="space-y-0.5">
                {group.items.map((item) => {
                  const Icon = item.icon;
                  if (item.kind === "requests") {
                    return (
                      <React.Fragment key="requests">
                        <SidebarMenuItem>
                          <SidebarMenuButton
                            id={selectorId("root-sidebar-nav", "requests")}
                            isActive={isRequestsSection}
                            className={TOP_NAV_BUTTON_CLASS}
                            onClick={(event) => {
                              handleNavigate(event, "requests");
                            }}
                          >
                            <Icon className="h-4 w-4" />
                            {item.label}
                          </SidebarMenuButton>
                          {pendingMediaRequestCount > 0 ? (
                            <SidebarMenuBadge
                              className={navBadgeToneClass("request")}
                            >
                              {pendingMediaRequestCount}
                            </SidebarMenuBadge>
                          ) : null}
                        </SidebarMenuItem>
                      </React.Fragment>
                    );
                  }
                  if (item.kind === "settings") {
                    return (
                      <React.Fragment key={`settings-${item.id}`}>
                        <SidebarMenuItem>
                          <SidebarMenuButton
                            id={selectorId(
                              "root-sidebar-settings-shortcut",
                              item.id,
                            )}
                            isActive={
                              view === "settings" &&
                              isSettingsNavEntryActive(item.id, settingsSection)
                            }
                            className={TOP_NAV_BUTTON_CLASS}
                            onClick={(event) => {
                              handleNavigate(event, "settings", item.id);
                            }}
                          >
                            <Icon className="h-4 w-4" />
                            {item.label}
                          </SidebarMenuButton>
                        </SidebarMenuItem>
                      </React.Fragment>
                    );
                  }
                  if (item.kind === "system") {
                    return (
                      <React.Fragment key={`system-${item.id}`}>
                        <SidebarMenuItem>
                          <SidebarMenuButton
                            id={selectorId(
                              "root-sidebar-system-shortcut",
                              item.id,
                            )}
                            isActive={
                              view === "system" && systemSection === item.id
                            }
                            className={TOP_NAV_BUTTON_CLASS}
                            onClick={(event) => {
                              handleNavigate(
                                event,
                                "system",
                                undefined,
                                undefined,
                                item.id,
                              );
                            }}
                          >
                            <Icon className="h-4 w-4" />
                            {item.label}
                          </SidebarMenuButton>
                        </SidebarMenuItem>
                      </React.Fragment>
                    );
                  }
                  if (item.kind === "logs") {
                    return (
                      <React.Fragment key={`logs-${item.id}`}>
                        <SidebarMenuItem>
                          <SidebarMenuButton
                            id={selectorId(
                              "root-sidebar-logs-shortcut",
                              item.id,
                            )}
                            isActive={view === "logs" && logsSection === item.id}
                            className={TOP_NAV_BUTTON_CLASS}
                            onClick={(event) => {
                              handleNavigate(
                                event,
                                "logs",
                                undefined,
                                undefined,
                                undefined,
                                undefined,
                                undefined,
                                item.id,
                              );
                            }}
                          >
                            <Icon className="h-4 w-4" />
                            {item.label}
                          </SidebarMenuButton>
                        </SidebarMenuItem>
                      </React.Fragment>
                    );
                  }

                  const isMediaSection = ["movies", "series", "anime"].includes(
                    item.id,
                  );
                  const isSettingsTop = item.id === "settings";
                  const isSystemTop = item.id === "system";
                  const isActivityTop = item.id === "activity";
                  const isActiveMediaSection =
                    isMediaSection && view === item.id && !isRequestsSection;
                  const isActiveSettingsSection =
                    isSettingsTop &&
                    view === "settings" &&
                    !isPromotedSettingsSection(settingsSection);
                  const isActiveSystemSection =
                    isSystemTop &&
                    view === "system" &&
                    systemSection === "overview";
                  const isActiveActivitySection =
                    isActivityTop && view === "activity";
                  const mediaFacetImportBadgeCount = isMediaSection
                    ? pendingImportCountForNavView(item.id)
                    : 0;
                  const hasVisibleSystemSubnav = SYSTEM_SUB_PAGES.length > 1;
                  const shouldShowChildren =
                    isActiveMediaSection ||
                    (isActiveSystemSection && hasVisibleSystemSubnav) ||
                    (isActiveActivitySection && hasVisibleActivitySubnav);
                  const hasExpandableChildren =
                    isMediaSection ||
                    (isSystemTop && hasVisibleSystemSubnav) ||
                    (isActivityTop && hasVisibleActivitySubnav);
                  if (
                    !isMediaSection &&
                    !isSettingsTop &&
                    !isSystemTop &&
                    !isActivityTop
                  ) {
                    return (
                      <React.Fragment key={item.id}>
                        <SidebarMenuItem>
                          <SidebarMenuButton
                            id={selectorId("root-sidebar-nav", item.id)}
                            isActive={view === item.id}
                            className={TOP_NAV_BUTTON_CLASS}
                            onClick={(event) => {
                              handleNavigate(event, item.id);
                            }}
                          >
                            <Icon className="h-4 w-4" />
                            {item.label}
                          </SidebarMenuButton>
                          {item.id === "activity" && hasActivityImportBadge ? (
                            <SidebarMenuBadge className="bg-primary text-primary-foreground">
                              {activityImportBadgeCount}
                            </SidebarMenuBadge>
                          ) : null}
                        </SidebarMenuItem>
                      </React.Fragment>
                    );
                  }

                  return (
                    <React.Fragment key={item.id}>
                      <SidebarMenuItem>
                        <SidebarMenuButton
                          id={selectorId("root-sidebar-nav", item.id)}
                          isActive={
                            isSettingsTop
                              ? isActiveSettingsSection
                              : isSystemTop
                                ? isActiveSystemSection
                                : isMediaSection
                                  ? isActiveMediaSection
                                  : view === item.id
                          }
                          className={TOP_NAV_BUTTON_CLASS}
                          aria-expanded={
                            hasExpandableChildren
                              ? shouldShowChildren
                              : undefined
                          }
                          onClick={(event) => {
                            if (isSettingsTop) {
                              handleNavigate(
                                event,
                                "settings",
                                defaultSettingsSectionForTopNav,
                              );
                              return;
                            }
                            if (isSystemTop) {
                              handleNavigate(
                                event,
                                "system",
                                undefined,
                                undefined,
                                "overview",
                              );
                              return;
                            }
                            if (isActivityTop) {
                              handleNavigate(
                                event,
                                "activity",
                                undefined,
                                undefined,
                                undefined,
                                undefined,
                                visibleActivitySubPages.some(
                                  (entry) => entry.id === activitySection,
                                )
                                  ? activitySection
                                  : "activity",
                              );
                              return;
                            }
                            handleNavigate(
                              event,
                              item.id,
                              undefined,
                              defaultMediaContentSection,
                            );
                          }}
                        >
                          <Icon className="h-4 w-4" />
                          <span className="min-w-0 flex-1 truncate">
                            {item.label}
                          </span>
                        </SidebarMenuButton>
                        {item.id === "activity" && hasActivityImportBadge ? (
                          <SidebarMenuBadge className="bg-primary text-primary-foreground">
                            {activityImportBadgeCount}
                          </SidebarMenuBadge>
                        ) : null}
                        {isSettingsTop && pluginUpdateCount > 0 ? (
                          <SidebarMenuBadge
                            className={cn(
                              "!top-1/2 -translate-y-1/2",
                              navBadgeToneClass("warning"),
                            )}
                          >
                            {pluginUpdateCount}
                          </SidebarMenuBadge>
                        ) : null}
                        {isMediaSection ? (
                          <FacetNavBadges
                            importCount={mediaFacetImportBadgeCount}
                          />
                        ) : null}
                      </SidebarMenuItem>

                      {shouldShowChildren ? (
                        <SidebarGroupContent>
                          <SidebarMenuSub className={SUB_NAV_MENU_CLASS}>
                            {isSettingsTop ? (
                              groupedSettingsEntries.map((group) => (
                                <React.Fragment key={group.id}>
                                  {groupedSettingsEntries.length > 1 ? (
                                    <SidebarMenuSubItem>
                                      <div className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-[0.13em] text-sidebar-foreground/40">
                                        {t(group.labelKey)}
                                      </div>
                                    </SidebarMenuSubItem>
                                  ) : null}
                                  {group.entries.map((entry) => (
                                    <SidebarMenuSubItem key={entry.id}>
                                      <SidebarMenuSubButton
                                        id={selectorId(
                                          "root-sidebar-settings",
                                          entry.id,
                                        )}
                                        isActive={settingsSection === entry.id}
                                        className={SUB_NAV_BUTTON_CLASS}
                                        onClick={(event) => {
                                          handleNavigate(
                                            event,
                                            "settings",
                                            entry.id,
                                          );
                                        }}
                                      >
                                        {entry.icon ? (
                                          <entry.icon className="h-3.5 w-3.5 shrink-0" />
                                        ) : null}
                                        <span className="min-w-0 flex-1 truncate">
                                          {entry.label(t)}
                                        </span>
                                        {entry.id === "plugins" &&
                                        pluginUpdateCount > 0 ? (
                                          <LeafNavBadge
                                            count={pluginUpdateCount}
                                            tone="warning"
                                          />
                                        ) : null}
                                      </SidebarMenuSubButton>
                                    </SidebarMenuSubItem>
                                  ))}
                                </React.Fragment>
                              ))
                            ) : isSystemTop ? (
                              SYSTEM_SUB_PAGES.map((entry) => (
                                <SidebarMenuSubItem key={entry.id}>
                                  <SidebarMenuSubButton
                                    id={selectorId(
                                      "root-sidebar-system",
                                      entry.id,
                                    )}
                                    isActive={systemSection === entry.id}
                                    className={SUB_NAV_BUTTON_CLASS}
                                    onClick={(event) => {
                                      handleNavigate(
                                        event,
                                        "system",
                                        undefined,
                                        undefined,
                                        entry.id,
                                      );
                                    }}
                                  >
                                    {t(entry.labelKey)}
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                              ))
                            ) : isActivityTop ? (
                              visibleActivitySubPages.map((entry) => (
                                <SidebarMenuSubItem key={entry.id}>
                                  <SidebarMenuSubButton
                                    id={selectorId(
                                      "root-sidebar-activity",
                                      entry.id,
                                    )}
                                    isActive={activitySection === entry.id}
                                    className={SUB_NAV_BUTTON_CLASS}
                                    onClick={(event) => {
                                      handleNavigate(
                                        event,
                                        "activity",
                                        undefined,
                                        undefined,
                                        undefined,
                                        undefined,
                                        entry.id,
                                      );
                                    }}
                                  >
                                    {t(entry.labelKey)}
                                    {entry.id === "import" &&
                                    hasActivityImportBadge ? (
                                      <LeafNavBadge
                                        count={activityImportBadgeCount}
                                      />
                                    ) : null}
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                              ))
                            ) : (
                              <>
                                {canViewCatalog ? (
                                  <SidebarMenuSubItem>
                                    <SidebarMenuSubButton
                                      id={selectorId(
                                        "root-sidebar-media",
                                        item.id,
                                        "overview",
                                      )}
                                      isActive={
                                        contentSettingsSection === "overview"
                                      }
                                      className={SUB_NAV_BUTTON_CLASS}
                                      onClick={(event) => {
                                        handleNavigate(
                                          event,
                                          item.id,
                                          undefined,
                                          "overview",
                                        );
                                      }}
                                    >
                                      {getMediaOverviewLabel(item.id, t)}
                                    </SidebarMenuSubButton>
                                  </SidebarMenuSubItem>
                                ) : null}
                                {canAccessFacetImport &&
                                mediaFacetImportBadgeCount > 0 ? (
                                  <SidebarMenuSubItem>
                                    <SidebarMenuSubButton
                                      id={selectorId(
                                        "root-sidebar-media",
                                        item.id,
                                        "import",
                                      )}
                                      isActive={
                                        contentSettingsSection === "import"
                                      }
                                      className={SUB_NAV_BUTTON_CLASS}
                                      onClick={(event) => {
                                        handleNavigate(
                                          event,
                                          item.id,
                                          undefined,
                                          "import",
                                        );
                                      }}
                                    >
                                      {t("nav.import")}
                                      <LeafNavBadge
                                        count={mediaFacetImportBadgeCount}
                                      />
                                    </SidebarMenuSubButton>
                                  </SidebarMenuSubItem>
                                ) : null}
                                {canAccessMediaSettings ? (
                                  <SidebarMenuSubItem>
                                    <SidebarMenuSubButton
                                      id={selectorId(
                                        "root-sidebar-media",
                                        item.id,
                                        "settings",
                                      )}
                                      isActive={isSettingsSubPage(
                                        contentSettingsSection,
                                      )}
                                      className={SUB_NAV_BUTTON_CLASS}
                                      onClick={(event) => {
                                        handleNavigate(
                                          event,
                                          item.id,
                                          undefined,
                                          "library",
                                        );
                                      }}
                                    >
                                      {getMediaSettingsLabel(item.id, t)}
                                    </SidebarMenuSubButton>
                                  </SidebarMenuSubItem>
                                ) : null}
                              </>
                            )}
                          </SidebarMenuSub>
                        </SidebarGroupContent>
                      ) : null}
                    </React.Fragment>
                  );
                })}
              </SidebarMenu>
              )}
            </SidebarGroup>
            );
          })}
        </SidebarContent>
        <SidebarFooter className="space-y-1.5 border-t border-[var(--scry-border3)] px-3.5 py-2.5">
          <div
            className={cn(
              "grid grid-cols-2 gap-1.5 group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:justify-center",
            )}
          >
            {!uiSettings.hideSponsorButton ? (
              <a
                id="root-sidebar-sponsor-link"
                href="https://www.scryer.media/scryer/donate/"
                target="_blank"
                rel="noopener noreferrer"
                className={cn(
                  "flex min-w-0 items-center justify-center gap-1.5 rounded-lg border border-[var(--scry-border2)] bg-[var(--scry-card2)] px-2 py-1 text-xs font-semibold text-[var(--scry-ink2)] transition hover:border-[var(--scry-accent)] hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25 group-data-[collapsible=icon]:hidden",
                )}
              >
                <Heart className="h-3.5 w-3.5 text-[var(--scry-danger-text-soft)]" />
                <span className="truncate">Sponsor</span>
              </a>
            ) : null}
            {themeMounted ? (
              <button
                id="root-sidebar-theme-toggle"
                type="button"
                onClick={cycleTheme}
                aria-label={t("theme.switchLabel", { theme: themeLabel })}
                className={cn(
                  "flex min-w-0 items-center justify-center gap-1.5 rounded-lg border border-transparent bg-transparent px-2 py-1 text-xs font-medium text-[var(--scry-muted)] transition hover:border-[var(--scry-border2)] hover:bg-[var(--scry-hover)] hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25",
                )}
              >
                {displayTheme === "light" ? (
                  <Sun className="h-4 w-4" />
                ) : displayTheme === "dark" ? (
                  <Moon className="h-4 w-4" />
                ) : (
                  <Monitor className="h-4 w-4" />
                )}
                <span className="truncate">{themeLabel}</span>
              </button>
            ) : null}
            <a
              id="root-sidebar-docs-link"
              href="https://www.scryer.media/scryer/docs/"
              target="_blank"
              rel="noopener noreferrer"
              className="flex min-w-0 items-center justify-center gap-1.5 rounded-lg border border-transparent bg-transparent px-2 py-1 text-xs font-medium text-[var(--scry-muted)] transition hover:border-[var(--scry-border2)] hover:bg-[var(--scry-hover)] hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25 group-data-[collapsible=icon]:hidden"
            >
              <BookOpen className="h-4 w-4" />
              <span className="truncate">Docs</span>
            </a>
            <div
              className={cn(
                "group/social relative min-w-0 group-data-[collapsible=icon]:hidden",
                uiSettings.hideSponsorButton && "col-span-2",
              )}
            >
              <button
                id="root-sidebar-social-links"
                type="button"
                aria-haspopup="menu"
                className="flex w-full min-w-0 items-center justify-center gap-1.5 rounded-lg border border-transparent bg-transparent px-2 py-1 text-xs font-medium text-[var(--scry-muted)] transition hover:border-[var(--scry-border2)] hover:bg-[var(--scry-hover)] hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25"
              >
                <MessagesSquare className="h-4 w-4" />
                <span className="truncate">Social</span>
              </button>
              <div className="pointer-events-none invisible absolute bottom-full left-0 z-50 w-full pb-1.5 opacity-0 transition-opacity group-hover/social:pointer-events-auto group-hover/social:visible group-hover/social:opacity-100 group-focus-within/social:pointer-events-auto group-focus-within/social:visible group-focus-within/social:opacity-100">
                <div
                  role="menu"
                  aria-label="Social links"
                  className="space-y-0.5 rounded-lg border border-[var(--scry-border2)] bg-[var(--scry-bg)] p-1 shadow-xl"
                >
                  <a
                    href="https://www.reddit.com/r/scryer_media/"
                    target="_blank"
                    rel="noopener noreferrer"
                    role="menuitem"
                    className="flex items-center gap-2 rounded-md px-2 py-1.5 text-xs font-medium text-[var(--scry-ink2)] transition hover:bg-[var(--scry-hover)] focus-visible:bg-[var(--scry-hover)] focus-visible:outline-none"
                  >
                    <img
                      src={sidebarPublicAssetUrl("social/reddit.svg")}
                      alt=""
                      aria-hidden="true"
                      className="h-4 w-4 shrink-0"
                    />
                    <span>Reddit</span>
                  </a>
                  <a
                    href="https://discord.gg/SQmtZTanqm"
                    target="_blank"
                    rel="noopener noreferrer"
                    role="menuitem"
                    className="flex items-center gap-2 rounded-md px-2 py-1.5 text-xs font-medium text-[var(--scry-ink2)] transition hover:bg-[var(--scry-hover)] focus-visible:bg-[var(--scry-hover)] focus-visible:outline-none"
                  >
                    <img
                      src={sidebarPublicAssetUrl("social/discord.svg")}
                      alt=""
                      aria-hidden="true"
                      className="h-4 w-4 shrink-0"
                    />
                    <span>Discord</span>
                  </a>
                </div>
              </div>
            </div>
          </div>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset
        data-slot="root-content-panel"
        className="relative min-w-0 min-[981px]:h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:max-h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:overflow-hidden"
      >
        {headerWithMobileNavigation}
        {canInjectMobileNavigation ? null : (
          <div className="mx-3 mb-3 mt-3 flex items-center gap-3 rounded-xl border border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] px-3 py-2 shadow-[0_12px_28px_rgba(2,6,23,0.16)] min-[981px]:hidden">
            {mobileNavigationTrigger}
            <div className="min-w-0">
              <p className="truncate text-sm font-semibold text-foreground">
                {currentTopLevelLabel}
              </p>
              {currentSubsectionLabel &&
              currentSubsectionLabel !== currentTopLevelLabel ? (
                <p className="truncate text-xs text-muted-foreground">
                  {currentSubsectionLabel}
                </p>
              ) : null}
            </div>
          </div>
        )}
        {children}
      </SidebarInset>
    </>
  );
}

export const RootSidebar = React.memo(function RootSidebar(
  props: RootSidebarProps,
) {
  return (
    <SidebarProvider
      className="h-full"
      style={
        {
          "--sidebar-width": "224px",
        } as React.CSSProperties
      }
    >
      <RootSidebarContent {...props} />
    </SidebarProvider>
  );
});
