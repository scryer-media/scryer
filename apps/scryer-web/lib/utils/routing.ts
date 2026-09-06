import type { LocaleCode } from "../i18n/index.ts";
import type {
  ActivitySection,
  ContentSettingsSection,
  IndexerSettingsTab,
  LogsSection,
  SettingsSection,
  SystemSection,
  ViewId,
  WantedSection,
} from "@/components/root/types";
import { normalizeLocale } from "../i18n/index.ts";
import { AVAILABLE_LANGUAGES } from "../i18n/index.ts";
import { isMediaView } from "../facets/registry.ts";

export const SETTINGS_SECTION_PATH: Record<SettingsSection, string> = {
  profile: "profile",
  general: "general",
  backups: "backups",
  security: "security",
  users: "users",
  mediaServers: "media-servers",
  indexers: "indexers",
  downloadClients: "download-clients",
  qualityProfiles: "quality-profiles",
  delayProfiles: "delay-profiles",
  acquisition: "acquisition",
  rules: "rules",
  plugins: "plugins",
  notifications: "notifications",
  "post-processing": "post-processing",
  subtitles: "subtitles",
};

const AUTOMATION_SETTINGS_SECTION_PATH: Partial<Record<SettingsSection, string>> = {
  acquisition: "acquisition",
  rules: "rules",
  subtitles: "subtitles",
  "post-processing": "post-processing",
};

const INTEGRATIONS_SETTINGS_SECTION_PATH: Partial<Record<SettingsSection, string>> = {
  indexers: "indexers",
  downloadClients: "download-clients",
  mediaServers: "media-servers",
  notifications: "notifications",
};

const SYSTEM_SETTINGS_SECTION_PATH: Partial<Record<SettingsSection, string>> = {
  users: "users",
  security: "security",
  backups: "backup",
};

/// Panes of the Indexers settings page. The default pane has no segment, so
/// `/integrations/indexers` keeps meaning what it always meant.
export const INDEXER_TAB_PATH: Record<IndexerSettingsTab, string> = {
  indexers: "",
  proxies: "proxies",
  seedingProfiles: "seeding-profiles",
};

const INDEXER_TAB_BY_SEGMENT: Record<string, IndexerSettingsTab> = {
  proxies: "proxies",
  "indexer-proxies": "proxies",
  "seeding-profiles": "seedingProfiles",
  seedingprofiles: "seedingProfiles",
};

/// Path of one Indexers pane. Seeding profiles used to be a settings section of
/// its own, so `/settings/seeding-profiles` still redirects here.
export function buildIndexerSettingsPath(tab: IndexerSettingsTab): string {
  const segment = INDEXER_TAB_PATH[tab];
  return segment ? `/integrations/indexers/${segment}` : "/integrations/indexers";
}

export function indexerSettingsTabFromPath(pathname: string): IndexerSettingsTab {
  const segments = pathname.split("/").filter(Boolean);
  const indexerAt = segments.findIndex((segment) => segment.toLowerCase() === "indexers");
  if (indexerAt < 0) {
    return "indexers";
  }
  return INDEXER_TAB_BY_SEGMENT[segments[indexerAt + 1]?.toLowerCase() ?? ""] ?? "indexers";
}

export const CONTENT_SECTION_PATH: Record<ContentSettingsSection, string> = {
  overview: "overview",
  import: "import",
  library: "settings/library",
  general: "settings/general",
  quality: "settings/quality",
  renaming: "settings/renaming",
  routing: "settings/routing",
};

export const WANTED_SECTION_PATH: Record<WantedSection, string> = {
  wanted: "items",
  cutoff: "cutoff-unmet",
  pending: "pending",
};

export const ACTIVITY_SECTION_PATH: Record<ActivitySection, string> = {
  activity: "activity",
  import: "import",
  history: "history",
};

export const LOGS_SECTION_PATH: Record<LogsSection, string> = {
  logs: "logs",
  audit: "audit",
};

export const SYSTEM_SECTION_PATH: Record<SystemSection, string> = {
  overview: "overview",
  jobs: "jobs",
  recycleBin: "recycle-bin",
};

const MEDIA_RESERVED_OVERVIEW_SEGMENTS = new Set([
  "overview",
  "import",
  "requests",
  "settings",
  "media",
]);

export function buildViewPath(
  nextView: ViewId,
  nextSettingsSection?: SettingsSection,
  nextContentSection?: ContentSettingsSection,
  nextSystemSection?: SystemSection,
  nextWantedSection?: WantedSection,
  nextActivitySection?: ActivitySection,
  nextLogsSection?: LogsSection,
) {
  const base = `/${nextView}`;
  if (nextView === "settings" && nextSettingsSection) {
    const automationPath = AUTOMATION_SETTINGS_SECTION_PATH[nextSettingsSection];
    if (automationPath) {
      return `/automation/${automationPath}`;
    }
    const integrationsPath = INTEGRATIONS_SETTINGS_SECTION_PATH[nextSettingsSection];
    if (integrationsPath) {
      return `/integrations/${integrationsPath}`;
    }
    const systemPath = SYSTEM_SETTINGS_SECTION_PATH[nextSettingsSection];
    if (systemPath) {
      return `/system/${systemPath}`;
    }
    return `${base}/${SETTINGS_SECTION_PATH[nextSettingsSection]}`;
  }
  if (nextView === "system" && nextSystemSection && nextSystemSection !== "overview") {
    return `${base}/${SYSTEM_SECTION_PATH[nextSystemSection]}`;
  }
  if (nextView === "logs") {
    const logsSection = nextLogsSection ?? "logs";
    if (logsSection !== "logs") {
      return `${base}/${LOGS_SECTION_PATH[logsSection]}`;
    }
    return base;
  }
  if (nextView === "activity") {
    const activitySection = nextActivitySection ?? "activity";
    if (activitySection !== "activity") {
      return `${base}/${ACTIVITY_SECTION_PATH[activitySection]}`;
    }
    return base;
  }
  if (nextView === "wanted") {
    return `/automation/wanted/${WANTED_SECTION_PATH[nextWantedSection ?? "wanted"]}`;
  }
  if (isMediaView(nextView)) {
    if (nextContentSection && nextContentSection !== "overview") {
      return `${base}/${CONTENT_SECTION_PATH[nextContentSection]}`;
    }
  }
  return base;
}

function defaultLibrarySlugForView(view: ViewId): string | null {
  return isMediaView(view) ? view : null;
}

function decodePathSegment(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

export function buildOverviewDetailPath(
  view: ViewId,
  librarySlug: string | null | undefined,
  titleSlug: string | null | undefined,
) {
  const normalizedLibrarySlug = librarySlug?.trim();
  const normalizedTitleSlug = titleSlug?.trim();
  if (!normalizedLibrarySlug || !normalizedTitleSlug) {
    return `/${view}`;
  }
  const defaultLibrarySlug = defaultLibrarySlugForView(view);
  if (
    defaultLibrarySlug &&
    normalizedLibrarySlug.toLowerCase() === defaultLibrarySlug &&
    !MEDIA_RESERVED_OVERVIEW_SEGMENTS.has(normalizedTitleSlug.toLowerCase())
  ) {
    return `/${view}/${encodeURIComponent(normalizedTitleSlug)}`;
  }
  return `/${view}/${encodeURIComponent(normalizedLibrarySlug)}/${encodeURIComponent(normalizedTitleSlug)}`;
}

export function isLocaleSupported(code: string): code is LocaleCode {
  return AVAILABLE_LANGUAGES.some((language) => language.code === code);
}

export type ParsedAppRoute = {
  canonicalPath: string;
  view: ViewId;
  settingsSection: SettingsSection;
  contentSettingsSection: ContentSettingsSection;
  systemSection: SystemSection;
  logsSection: LogsSection;
  activitySection: ActivitySection;
  wantedSection: WantedSection;
  overviewLibrarySlug: string | null;
  overviewTitleSlug: string | null;
};

export type AppRouteResolution =
  | { kind: "canonical"; route: ParsedAppRoute }
  | { kind: "redirect"; to: string }
  // `/` cannot be resolved by path alone: admins land on the dashboard and
  // everyone else on the catalog, and permissions only exist once the auth
  // bootstrap has run inside the shell. Parsing stays pure by naming the
  // undecided case; the shell picks the destination the same way it already
  // picks one for a route the user may not access.
  | { kind: "landing" }
  | { kind: "not-found" };

const MEDIA_SETTINGS_SECTIONS = new Set<ContentSettingsSection>([
  "library",
  "general",
  "quality",
  "renaming",
  "routing",
]);
const AUTOMATION_SETTINGS_BY_SEGMENT: Record<string, SettingsSection> = {
  acquisition: "acquisition",
  rules: "rules",
  subtitles: "subtitles",
  "post-processing": "post-processing",
  "post-procesing": "post-processing",
};
const INTEGRATION_SETTINGS_BY_SEGMENT: Record<string, SettingsSection> = {
  indexers: "indexers",
  "download-clients": "downloadClients",
  downloadclients: "downloadClients",
  "media-servers": "mediaServers",
  mediaservers: "mediaServers",
  notifications: "notifications",
};
const LOCAL_SETTINGS_BY_SEGMENT: Record<string, SettingsSection> = {
  profile: "profile",
  general: "general",
  "quality-profiles": "qualityProfiles",
  qualityprofiles: "qualityProfiles",
  "delay-profiles": "delayProfiles",
  delayprofiles: "delayProfiles",
  plugins: "plugins",
};
const SYSTEM_SETTINGS_BY_SEGMENT: Record<string, SettingsSection> = {
  users: "users",
  security: "security",
  backup: "backups",
};
const WANTED_SECTION_BY_SEGMENT: Record<string, WantedSection> = {
  items: "wanted",
  "wanted-items": "wanted",
  wanted: "wanted",
  "cutoff-unmet": "cutoff",
  cutoff: "cutoff",
  pending: "pending",
};

function parsedRoute(
  canonicalPath: string,
  view: ViewId,
  overrides: Partial<Omit<ParsedAppRoute, "canonicalPath" | "view">> = {},
): ParsedAppRoute {
  return {
    canonicalPath,
    view,
    settingsSection: "profile",
    contentSettingsSection: "overview",
    systemSection: "overview",
    logsSection: "logs",
    activitySection: "activity",
    wantedSection: "wanted",
    overviewLibrarySlug: null,
    overviewTitleSlug: null,
    ...overrides,
  };
}

function locationSuffix(search: string, hash: string): string {
  const normalizedSearch = search && !search.startsWith("?") ? `?${search}` : search;
  const normalizedHash = hash && !hash.startsWith("#") ? `#${hash}` : hash;
  return `${normalizedSearch}${normalizedHash}`;
}

function redirectTo(path: string, search: string, hash: string): AppRouteResolution {
  return { kind: "redirect", to: `${path}${locationSuffix(search, hash)}` };
}

function canonicalOrRedirect(
  currentPath: string,
  route: ParsedAppRoute,
  search: string,
  hash: string,
): AppRouteResolution {
  if (currentPath !== route.canonicalPath) {
    return redirectTo(route.canonicalPath, search, hash);
  }
  return { kind: "canonical", route };
}

function settingsRoute(section: SettingsSection): ParsedAppRoute {
  return parsedRoute(buildViewPath("settings", section), "settings", {
    settingsSection: section,
  });
}

function resolveMediaRoute(
  currentPath: string,
  view: ViewId,
  rawSegments: string[],
  normalizedSegments: string[],
  search: string,
  hash: string,
): AppRouteResolution {
  if (rawSegments.length === 1) {
    return canonicalOrRedirect(currentPath, parsedRoute(`/${view}`, view), search, hash);
  }

  const section = normalizedSegments[1];
  if (rawSegments.length === 2) {
    if (section === "overview") {
      return redirectTo(`/${view}`, search, hash);
    }
    if (section === "import") {
      return canonicalOrRedirect(
        currentPath,
        parsedRoute(`/${view}/import`, view, { contentSettingsSection: "import" }),
        search,
        hash,
      );
    }
    if (section === "requests") {
      return redirectTo("/requests", search, hash);
    }
    if (section === "settings" || section === "media") {
      return redirectTo(`/${view}/settings/library`, search, hash);
    }

    const titleSlug = decodePathSegment(rawSegments[1]);
    const canonicalPath = buildOverviewDetailPath(view, view, titleSlug);
    return canonicalOrRedirect(
      currentPath,
      parsedRoute(canonicalPath, view, {
        overviewLibrarySlug: view,
        overviewTitleSlug: titleSlug,
      }),
      search,
      hash,
    );
  }

  if (rawSegments.length !== 3) {
    return { kind: "not-found" };
  }

  if (section === "settings") {
    const contentSection = normalizedSegments[2] as ContentSettingsSection;
    if (!MEDIA_SETTINGS_SECTIONS.has(contentSection)) {
      return { kind: "not-found" };
    }
    const canonicalPath = `/${view}/${CONTENT_SECTION_PATH[contentSection]}`;
    return canonicalOrRedirect(
      currentPath,
      parsedRoute(canonicalPath, view, { contentSettingsSection: contentSection }),
      search,
      hash,
    );
  }

  if (MEDIA_RESERVED_OVERVIEW_SEGMENTS.has(section)) {
    return { kind: "not-found" };
  }

  const librarySlug = decodePathSegment(rawSegments[1]);
  const titleSlug = decodePathSegment(rawSegments[2]);
  const canonicalPath = buildOverviewDetailPath(view, librarySlug, titleSlug);
  return canonicalOrRedirect(
    currentPath,
    parsedRoute(canonicalPath, view, {
      overviewLibrarySlug: librarySlug,
      overviewTitleSlug: titleSlug,
    }),
    search,
    hash,
  );
}

export function resolveAppRoute(
  pathname: string | null | undefined,
  search = "",
  hash = "",
): AppRouteResolution {
  const currentPath = pathname?.trim() || "/";
  const trimmed = currentPath.replace(/^\/+|\/+$/g, "");
  const rawSegments = trimmed ? trimmed.split("/").filter(Boolean) : [];
  const normalizedSegments = rawSegments.map((segment) => segment.toLowerCase());
  const root = normalizedSegments[0] ?? "";

  if (!root) {
    return { kind: "landing" };
  }

  if (root === "dashboard") {
    if (rawSegments.length !== 1) {
      return { kind: "not-found" };
    }
    return canonicalOrRedirect(
      currentPath,
      parsedRoute("/dashboard", "dashboard"),
      search,
      hash,
    );
  }

  if (isMediaView(root)) {
    return resolveMediaRoute(
      currentPath,
      root as ViewId,
      rawSegments,
      normalizedSegments,
      search,
      hash,
    );
  }

  if (root === "discovery" || root === "requests" || root === "calendar") {
    if (rawSegments.length !== 1) {
      return { kind: "not-found" };
    }
    return canonicalOrRedirect(
      currentPath,
      parsedRoute(`/${root}`, root as ViewId),
      search,
      hash,
    );
  }

  if (root === "activity") {
    const section = normalizedSegments[1] ?? "activity";
    if (rawSegments.length > 2 || !["activity", "import", "history"].includes(section)) {
      return { kind: "not-found" };
    }
    const activitySection = section as ActivitySection;
    const canonicalPath = buildViewPath(
      "activity",
      undefined,
      undefined,
      undefined,
      undefined,
      activitySection,
    );
    return canonicalOrRedirect(
      currentPath,
      parsedRoute(canonicalPath, "activity", { activitySection }),
      search,
      hash,
    );
  }

  if (root === "automation") {
    const section = normalizedSegments[1] ?? "";
    if (section === "wanted") {
      const wantedSegment = normalizedSegments[2] ?? "items";
      if (wantedSegment === "history" && rawSegments.length === 3) {
        return redirectTo(
          buildViewPath("activity", undefined, undefined, undefined, undefined, "history"),
          search,
          hash,
        );
      }
      const wantedSection = WANTED_SECTION_BY_SEGMENT[wantedSegment];
      if (!wantedSection || rawSegments.length > 3) {
        return { kind: "not-found" };
      }
      const canonicalPath = buildViewPath(
        "wanted",
        undefined,
        undefined,
        undefined,
        wantedSection,
      );
      return canonicalOrRedirect(
        currentPath,
        parsedRoute(canonicalPath, "wanted", { wantedSection }),
        search,
        hash,
      );
    }

    const settingsSection = AUTOMATION_SETTINGS_BY_SEGMENT[section];
    if (!settingsSection || rawSegments.length !== 2) {
      return { kind: "not-found" };
    }
    return canonicalOrRedirect(currentPath, settingsRoute(settingsSection), search, hash);
  }

  if (root === "integrations") {
    const settingsSection = INTEGRATION_SETTINGS_BY_SEGMENT[normalizedSegments[1] ?? ""];
    if (!settingsSection) {
      return { kind: "not-found" };
    }
    // The Indexers page carries panes (proxies, seeding profiles) as a third
    // segment; every other integrations section is a bare two-segment path.
    if (settingsSection === "indexers" && rawSegments.length === 3) {
      const tab = INDEXER_TAB_BY_SEGMENT[normalizedSegments[2] ?? ""];
      if (!tab) {
        return { kind: "not-found" };
      }
      return canonicalOrRedirect(
        currentPath,
        parsedRoute(buildIndexerSettingsPath(tab), "settings", {
          settingsSection,
        }),
        search,
        hash,
      );
    }
    if (rawSegments.length !== 2) {
      return { kind: "not-found" };
    }
    return canonicalOrRedirect(currentPath, settingsRoute(settingsSection), search, hash);
  }

  if (root === "wanted") {
    const wantedSegment = normalizedSegments[1] ?? "items";
    if (wantedSegment === "history" && rawSegments.length === 2) {
      return redirectTo(
        buildViewPath("activity", undefined, undefined, undefined, undefined, "history"),
        search,
        hash,
      );
    }
    const wantedSection = WANTED_SECTION_BY_SEGMENT[wantedSegment];
    if (!wantedSection || rawSegments.length > 2) {
      return { kind: "not-found" };
    }
    return redirectTo(
      buildViewPath("wanted", undefined, undefined, undefined, wantedSection),
      search,
      hash,
    );
  }

  if (root === "history") {
    return rawSegments.length === 1
      ? redirectTo(
          buildViewPath("activity", undefined, undefined, undefined, undefined, "history"),
          search,
          hash,
        )
      : { kind: "not-found" };
  }

  if (root === "settings") {
    if (rawSegments.length === 1) {
      return redirectTo("/settings/profile", search, hash);
    }
    if (rawSegments.length !== 2) {
      return { kind: "not-found" };
    }
    const section = normalizedSegments[1];
    const localSection = LOCAL_SETTINGS_BY_SEGMENT[section];
    if (localSection) {
      return canonicalOrRedirect(currentPath, settingsRoute(localSection), search, hash);
    }
    const movedSection =
      AUTOMATION_SETTINGS_BY_SEGMENT[section] ??
      INTEGRATION_SETTINGS_BY_SEGMENT[section] ??
      SYSTEM_SETTINGS_BY_SEGMENT[section === "backups" ? "backup" : section];
    if (movedSection) {
      return redirectTo(buildViewPath("settings", movedSection), search, hash);
    }
    if (section === "recycle-bin" || section === "recyclebin") {
      return redirectTo("/system/recycle-bin", search, hash);
    }
    return { kind: "not-found" };
  }

  if (root === "system") {
    const section = normalizedSegments[1] ?? "overview";
    if (rawSegments.length > 2) {
      return { kind: "not-found" };
    }
    if (section === "overview") {
      return canonicalOrRedirect(
        currentPath,
        parsedRoute("/system", "system"),
        search,
        hash,
      );
    }
    if (section === "jobs") {
      return canonicalOrRedirect(
        currentPath,
        parsedRoute("/system/jobs", "system", { systemSection: "jobs" }),
        search,
        hash,
      );
    }
    if (section === "recycle-bin" || section === "recyclebin") {
      return canonicalOrRedirect(
        currentPath,
        parsedRoute("/system/recycle-bin", "system", { systemSection: "recycleBin" }),
        search,
        hash,
      );
    }
    const systemSetting = SYSTEM_SETTINGS_BY_SEGMENT[section === "backups" ? "backup" : section];
    if (systemSetting) {
      return canonicalOrRedirect(currentPath, settingsRoute(systemSetting), search, hash);
    }
    if (section === "logs") {
      return redirectTo("/logs", search, hash);
    }
    if (section === "audit") {
      return redirectTo("/logs/audit", search, hash);
    }
    return { kind: "not-found" };
  }

  if (root === "logs") {
    const section = normalizedSegments[1] ?? "logs";
    if (rawSegments.length > 2) {
      return { kind: "not-found" };
    }
    if (["logs", "service", "service-logs"].includes(section)) {
      return canonicalOrRedirect(
        currentPath,
        parsedRoute("/logs", "logs"),
        search,
        hash,
      );
    }
    if (["audit", "audit-logs"].includes(section)) {
      return canonicalOrRedirect(
        currentPath,
        parsedRoute("/logs/audit", "logs", { logsSection: "audit" }),
        search,
        hash,
      );
    }
    return { kind: "not-found" };
  }

  return { kind: "not-found" };
}

export function parseLanguageFromParam(value: string | null): LocaleCode | null {
  if (!value) {
    return null;
  }

  const normalized = normalizeLocale(value);
  return isLocaleSupported(normalized) ? normalized : null;
}

/**
 * Navigation state the header's "Enable login" entry attaches when it sends
 * the user to Security, so the page opens the enable flow on arrival.
 */
export const ENABLE_FORM_LOGIN_LOCATION_STATE = { enableFormLogin: true } as const;

export function readEnableFormLoginIntent(state: unknown): boolean {
  return (
    typeof state === "object" &&
    state !== null &&
    (state as { enableFormLogin?: unknown }).enableFormLogin === true
  );
}
