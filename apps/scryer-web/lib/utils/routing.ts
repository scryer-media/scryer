import type { LocaleCode } from "../i18n/index.ts";
import type {
  ActivitySection,
  ContentSettingsSection,
  IndexerSettingsTab,
  ListsSection,
  LogsSection,
  LifecycleRulesSection,
  RequestRulesSection,
  RulesSection,
  ScoringRulesSection,
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
  proxies: "proxies",
  qualityProfiles: "quality-profiles",
  delayProfiles: "delay-profiles",
  titleTags: "tags",
  acquisition: "acquisition",
  rules: "rules",
  lifecycleRules: "lifecycle-rules",
  requestRules: "request-rules",
  plugins: "plugins",
  scripts: "scripts",
  listProviderApps: "lists/provider-apps",
  notifications: "notifications",
  "post-processing": "post-processing",
  subtitles: "subtitles",
};

const AUTOMATION_SETTINGS_SECTION_PATH: Partial<
  Record<SettingsSection, string>
> = {
  subtitles: "subtitles",
  "post-processing": "post-processing",
};

/// Panes of the Rules page. Scoring, lifecycle and request rules all hang off
/// `/automation/rules`, so the sidebar carries one entry with the kinds listed
/// under it.
export const RULES_SECTION_PATH: Record<RulesSection, string> = {
  scoring: "scoring",
  lifecycle: "lifecycle",
  request: "request",
};

const RULES_SECTION_BY_SEGMENT: Record<string, RulesSection> = {
  scoring: "scoring",
  "scoring-rules": "scoring",
  lifecycle: "lifecycle",
  // Lifecycle rules were called maintenance rules; links that still name
  // them redirect to the lifecycle pane.
  maintenance: "lifecycle",
  "maintenance-rules": "lifecycle",
  maintenancerules: "lifecycle",
  request: "request",
  "request-rules": "request",
  requestrules: "request",
};

/// Panes of the Lifecycle Rules page. The rule list is the default and has no
/// segment of its own, so `/automation/rules/lifecycle` keeps resolving to
/// the list an existing lifecycle-rules link expected.
export const LIFECYCLE_RULES_SECTION_PATH: Record<
  LifecycleRulesSection,
  string
> = {
  rules: "",
  try: "try",
  candidates: "candidates",
  history: "history",
};

const LIFECYCLE_RULES_SECTION_BY_SEGMENT: Record<
  string,
  LifecycleRulesSection
> = {
  rules: "rules",
  try: "try",
  candidates: "candidates",
  history: "history",
  runs: "history",
  // Panes that were folded into others redirect rather than 404ing on links
  // people already hold: the instance switches now head every pane, and
  // exclusions sit under the candidates they were made from.
  gates: "rules",
  exclusions: "candidates",
};

/// Panes of the Scoring Rules page. The rule list is the default and adds no
/// segment, so `/automation/rules/scoring` still opens it.
export const SCORING_RULES_SECTION_PATH: Record<ScoringRulesSection, string> = {
  rules: "",
  community: "community",
};

const SCORING_RULES_SECTION_BY_SEGMENT: Record<string, ScoringRulesSection> = {
  rules: "rules",
  community: "community",
};

/// Panes of the Request Rules page. As on Lifecycle, the rule list is the
/// default and adds no segment, so `/automation/rules/request` still opens it.
export const REQUEST_RULES_SECTION_PATH: Record<RequestRulesSection, string> = {
  rules: "",
  try: "try",
  decisions: "decisions",
};

const REQUEST_RULES_SECTION_BY_SEGMENT: Record<string, RequestRulesSection> = {
  rules: "rules",
  try: "try",
  decisions: "decisions",
};

/// Which kinds of rule the Rules page offers, in rail order. Lifecycle and
/// request rules are in beta and badged as such, but no longer wait behind the
/// experimental-features switch.
export const RULES_SECTIONS: readonly RulesSection[] = [
  "scoring",
  "lifecycle",
  "request",
];

/// Rule kinds still in beta. The rail badges these.
export const BETA_RULES_SECTIONS: ReadonlySet<RulesSection> = new Set([
  "lifecycle",
  "request",
]);

/// The `rules` settings section is the Scoring pane, `lifecycleRules` the
/// Lifecycle pane and `requestRules` the Request pane. Which `SettingsSection`
/// a pane maps to still decides its permissions, so the three stay distinct
/// underneath one nav entry.
export const RULES_SECTION_BY_SETTINGS_SECTION: Partial<
  Record<SettingsSection, RulesSection>
> = {
  rules: "scoring",
  lifecycleRules: "lifecycle",
  requestRules: "request",
};

/// Path of one Rules pane. Lifecycle and request rules carry their own pane
/// as a fourth segment; each rule list is the default and adds none.
export function buildRulesPath(
  section: "lifecycle",
  subSection?: LifecycleRulesSection,
): string;
export function buildRulesPath(
  section: "request",
  subSection?: RequestRulesSection,
): string;
export function buildRulesPath(
  section: "scoring",
  subSection?: ScoringRulesSection,
): string;
export function buildRulesPath(section: RulesSection): string;
export function buildRulesPath(
  section: RulesSection,
  subSection?:
    LifecycleRulesSection | RequestRulesSection | ScoringRulesSection,
): string {
  const base = `/automation/rules/${RULES_SECTION_PATH[section]}`;
  let segment = "";
  if (section === "lifecycle") {
    segment =
      LIFECYCLE_RULES_SECTION_PATH[
        (subSection as LifecycleRulesSection | undefined) ?? "rules"
      ];
  } else if (section === "request") {
    segment =
      REQUEST_RULES_SECTION_PATH[
        (subSection as RequestRulesSection | undefined) ?? "rules"
      ];
  } else if (section === "scoring") {
    segment =
      SCORING_RULES_SECTION_PATH[
        (subSection as ScoringRulesSection | undefined) ?? "rules"
      ];
  }
  return segment ? `${base}/${segment}` : base;
}

export function rulesSectionFromPath(pathname: string): RulesSection {
  const segments = pathname.split("/").filter(Boolean);
  const rulesAt = segments.findIndex(
    (segment) => segment.toLowerCase() === "rules",
  );
  if (rulesAt < 0) {
    return "scoring";
  }
  return (
    RULES_SECTION_BY_SEGMENT[segments[rulesAt + 1]?.toLowerCase() ?? ""] ??
    "scoring"
  );
}

export function lifecycleRulesSectionFromPath(
  pathname: string,
): LifecycleRulesSection {
  const segments = pathname.split("/").filter(Boolean);
  const rulesAt = segments.findIndex(
    (segment) => segment.toLowerCase() === "rules",
  );
  if (rulesAt < 0) {
    return "rules";
  }
  return (
    LIFECYCLE_RULES_SECTION_BY_SEGMENT[
      segments[rulesAt + 2]?.toLowerCase() ?? ""
    ] ?? "rules"
  );
}

export function scoringRulesSectionFromPath(
  pathname: string,
): ScoringRulesSection {
  const segments = pathname.split("/").filter(Boolean);
  const rulesAt = segments.findIndex(
    (segment) => segment.toLowerCase() === "rules",
  );
  if (rulesAt < 0) {
    return "rules";
  }
  return (
    SCORING_RULES_SECTION_BY_SEGMENT[
      segments[rulesAt + 2]?.toLowerCase() ?? ""
    ] ?? "rules"
  );
}

export function requestRulesSectionFromPath(
  pathname: string,
): RequestRulesSection {
  const segments = pathname.split("/").filter(Boolean);
  const rulesAt = segments.findIndex(
    (segment) => segment.toLowerCase() === "rules",
  );
  if (rulesAt < 0) {
    return "rules";
  }
  return (
    REQUEST_RULES_SECTION_BY_SEGMENT[
      segments[rulesAt + 2]?.toLowerCase() ?? ""
    ] ?? "rules"
  );
}

/// Panes of the Lists page. Public lists are the default and add no segment.
export const LISTS_SECTION_PATH: Record<ListsSection, string> = {
  public: "",
  personal: "personal",
  providerApps: "provider-apps",
  exclusions: "exclusions",
};

const LISTS_SECTION_BY_SEGMENT: Record<string, ListsSection> = {
  personal: "personal",
  "provider-apps": "providerApps",
  exclusions: "exclusions",
};

export function buildListsPath(section: ListsSection): string {
  if (section === "providerApps") return "/settings/lists/provider-apps";
  const segment = LISTS_SECTION_PATH[section];
  return segment ? `/lists/${segment}` : "/lists";
}

/** Path of a discovery home section's full grid. */
export function buildDiscoverySectionPath(sectionId: string): string {
  return `/discovery/section/${encodeURIComponent(sectionId)}`;
}

/**
 * The section id a `/discovery/section/:sectionId` path opens, or null for
 * the discovery home and any other path. The path is split while still
 * encoded and only the id segment is decoded, exactly once, so an encoded
 * `/` or `%` stays part of the id.
 */
export function discoverySectionIdFromPath(pathname: string): string | null {
  const segments = pathname.split("/").filter(Boolean);
  const discoveryAt = segments.findIndex(
    (segment) => segment.toLowerCase() === "discovery",
  );
  if (
    discoveryAt < 0 ||
    segments.length !== discoveryAt + 3 ||
    segments[discoveryAt + 1]?.toLowerCase() !== "section"
  ) {
    return null;
  }
  try {
    const sectionId = decodeURIComponent(segments[discoveryAt + 2] ?? "");
    return sectionId.trim() ? sectionId : null;
  } catch {
    return null;
  }
}

export function listsSectionFromPath(pathname: string): ListsSection {
  const segments = pathname.split("/").filter(Boolean);
  const listsAt = segments.findIndex(
    (segment) => segment.toLowerCase() === "lists",
  );
  if (listsAt < 0) {
    return "public";
  }
  return (
    LISTS_SECTION_BY_SEGMENT[segments[listsAt + 1]?.toLowerCase() ?? ""] ??
    "public"
  );
}

const INTEGRATIONS_SETTINGS_SECTION_PATH: Partial<
  Record<SettingsSection, string>
> = {
  indexers: "indexers",
  downloadClients: "download-clients",
  proxies: "proxies",
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
  search: "search",
  clients: "clients",
  seedingProfiles: "seeding-profiles",
};

const INDEXER_TAB_BY_SEGMENT: Record<string, IndexerSettingsTab> = {
  search: "search",
  clients: "clients",
  "seeding-profiles": "seedingProfiles",
  seedingprofiles: "seedingProfiles",
};

/// Segments that used to name the Indexers page's proxies pane. Proxies are a
/// section of their own now, so these are links people already have rather
/// than panes, and they redirect instead of resolving.
const MOVED_INDEXER_PROXY_SEGMENTS = new Set(["proxies", "indexer-proxies"]);

/// Path of one Indexers pane. Seeding profiles used to be a settings section of
/// its own, so `/settings/seeding-profiles` still redirects here.
export function buildIndexerSettingsPath(tab: IndexerSettingsTab): string {
  const segment = INDEXER_TAB_PATH[tab];
  return segment
    ? `/integrations/indexers/${segment}`
    : "/integrations/indexers";
}

export function indexerSettingsTabFromPath(
  pathname: string,
): IndexerSettingsTab {
  const segments = pathname.split("/").filter(Boolean);
  const indexerAt = segments.findIndex(
    (segment) => segment.toLowerCase() === "indexers",
  );
  if (indexerAt < 0) {
    return "indexers";
  }
  return (
    INDEXER_TAB_BY_SEGMENT[segments[indexerAt + 1]?.toLowerCase() ?? ""] ??
    "indexers"
  );
}

/// Which panes the Indexers page offers on this instance.
///
/// Indexer search stays available; client management requires the instance opt-in.
export const INDEXER_SETTINGS_TABS: readonly IndexerSettingsTab[] = [
  "indexers",
  "search",
  "clients",
  "seedingProfiles",
];

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
    // Scoring and lifecycle rules are panes of one page, so their canonical
    // paths nest under `/automation/rules` rather than sitting beside it.
    const rulesSection = RULES_SECTION_BY_SETTINGS_SECTION[nextSettingsSection];
    if (rulesSection) {
      return buildRulesPath(rulesSection);
    }
    const automationPath =
      AUTOMATION_SETTINGS_SECTION_PATH[nextSettingsSection];
    if (automationPath) {
      return `/automation/${automationPath}`;
    }
    const integrationsPath =
      INTEGRATIONS_SETTINGS_SECTION_PATH[nextSettingsSection];
    if (integrationsPath) {
      return `/integrations/${integrationsPath}`;
    }
    const systemPath = SYSTEM_SETTINGS_SECTION_PATH[nextSettingsSection];
    if (systemPath) {
      return `/system/${systemPath}`;
    }
    return `${base}/${SETTINGS_SECTION_PATH[nextSettingsSection]}`;
  }
  if (
    nextView === "system" &&
    nextSystemSection &&
    nextSystemSection !== "overview"
  ) {
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
  // Acquisition lived under Automation before it was listed in Settings; the
  // old path redirects so existing links and the e2e harness keep reaching it.
  acquisition: "acquisition",
  subtitles: "subtitles",
  "post-processing": "post-processing",
  "post-procesing": "post-processing",
};

/// Segments that named the maintenance rules page, from before lifecycle
/// rules were renamed and became a pane of the Rules page. They redirect
/// rather than resolve.
const MOVED_LIFECYCLE_RULES_SEGMENTS = new Set([
  "maintenance-rules",
  "maintenancerules",
]);

/// `/settings/...` segments that now name a Rules pane. Both were settings
/// sections of their own before the two kinds of rule shared a page.
const MOVED_SETTINGS_RULES_BY_SEGMENT: Record<string, RulesSection> = {
  rules: "scoring",
  // Lifecycle rules were called maintenance rules before.
  "maintenance-rules": "lifecycle",
  maintenancerules: "lifecycle",
  // Request rules never had a settings section of their own, but the section
  // path exists for the settings shell, so the spelling it implies resolves
  // rather than 404ing.
  "request-rules": "request",
  requestrules: "request",
};
const INTEGRATION_SETTINGS_BY_SEGMENT: Record<string, SettingsSection> = {
  indexers: "indexers",
  "download-clients": "downloadClients",
  downloadclients: "downloadClients",
  proxies: "proxies",
  "indexer-proxies": "proxies",
  "media-servers": "mediaServers",
  mediaservers: "mediaServers",
  notifications: "notifications",
};
const LOCAL_SETTINGS_BY_SEGMENT: Record<string, SettingsSection> = {
  profile: "profile",
  general: "general",
  acquisition: "acquisition",
  "quality-profiles": "qualityProfiles",
  qualityprofiles: "qualityProfiles",
  "delay-profiles": "delayProfiles",
  delayprofiles: "delayProfiles",
  tags: "titleTags",
  "title-tags": "titleTags",
  titletags: "titleTags",
  plugins: "plugins",
  scripts: "scripts",
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
  const normalizedSearch =
    search && !search.startsWith("?") ? `?${search}` : search;
  const normalizedHash = hash && !hash.startsWith("#") ? `#${hash}` : hash;
  return `${normalizedSearch}${normalizedHash}`;
}

function redirectTo(
  path: string,
  search: string,
  hash: string,
): AppRouteResolution {
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

/// `/automation/rules/...` — the Rules page and its two levels of panes.
///
/// A bare `/automation/rules` names the page rather than a pane, so it lands on
/// Scoring rather than 404ing. Lifecycle and request rules take a further
/// segment for their own pane, as does Scoring for its Community pane, and
/// the rule list is the pane a segment-less path means.
function resolveRulesRoute(
  currentPath: string,
  rawSegments: string[],
  normalizedSegments: string[],
  search: string,
  hash: string,
): AppRouteResolution {
  if (rawSegments.length === 2) {
    return redirectTo(buildRulesPath("scoring"), search, hash);
  }

  const rulesSection = RULES_SECTION_BY_SEGMENT[normalizedSegments[2] ?? ""];
  if (!rulesSection) {
    return { kind: "not-found" };
  }

  if (rawSegments.length > 4) {
    return { kind: "not-found" };
  }

  if (rulesSection === "scoring") {
    const scoringSection =
      rawSegments.length === 3
        ? "rules"
        : SCORING_RULES_SECTION_BY_SEGMENT[normalizedSegments[3] ?? ""];
    if (!scoringSection) {
      return { kind: "not-found" };
    }
    return canonicalOrRedirect(
      currentPath,
      parsedRoute(buildRulesPath("scoring", scoringSection), "settings", {
        settingsSection: "rules",
      }),
      search,
      hash,
    );
  }

  if (rulesSection === "request") {
    const requestSection =
      rawSegments.length === 3
        ? "rules"
        : REQUEST_RULES_SECTION_BY_SEGMENT[normalizedSegments[3] ?? ""];
    if (!requestSection) {
      return { kind: "not-found" };
    }
    return canonicalOrRedirect(
      currentPath,
      parsedRoute(buildRulesPath("request", requestSection), "settings", {
        settingsSection: "requestRules",
      }),
      search,
      hash,
    );
  }

  const lifecycleSection =
    rawSegments.length === 3
      ? "rules"
      : LIFECYCLE_RULES_SECTION_BY_SEGMENT[normalizedSegments[3] ?? ""];
  if (!lifecycleSection) {
    return { kind: "not-found" };
  }

  return canonicalOrRedirect(
    currentPath,
    parsedRoute(buildRulesPath("lifecycle", lifecycleSection), "settings", {
      settingsSection: "lifecycleRules",
    }),
    search,
    hash,
  );
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
    return canonicalOrRedirect(
      currentPath,
      parsedRoute(`/${view}`, view),
      search,
      hash,
    );
  }

  const section = normalizedSegments[1];
  if (rawSegments.length === 2) {
    if (section === "overview") {
      return redirectTo(`/${view}`, search, hash);
    }
    if (section === "import") {
      return canonicalOrRedirect(
        currentPath,
        parsedRoute(`/${view}/import`, view, {
          contentSettingsSection: "import",
        }),
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
      parsedRoute(canonicalPath, view, {
        contentSettingsSection: contentSection,
      }),
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
  const normalizedSegments = rawSegments.map((segment) =>
    segment.toLowerCase(),
  );
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

  if (
    root === "settings" &&
    normalizedSegments[1] === "lists" &&
    normalizedSegments[2] === "provider-apps" &&
    rawSegments.length === 3
  ) {
    return canonicalOrRedirect(
      currentPath,
      parsedRoute(buildListsPath("providerApps"), "lists"),
      search,
      hash,
    );
  }

  if (root === "lists") {
    if (rawSegments.length > 2) {
      return { kind: "not-found" };
    }
    const segment = normalizedSegments[1] ?? "";
    if (segment === "public") {
      return redirectTo(buildListsPath("public"), search, hash);
    }
    const section: ListsSection | undefined = segment
      ? LISTS_SECTION_BY_SEGMENT[segment]
      : "public";
    if (!section) {
      return { kind: "not-found" };
    }
    return canonicalOrRedirect(
      currentPath,
      parsedRoute(buildListsPath(section), "lists"),
      search,
      hash,
    );
  }

  if (root === "discovery" && rawSegments.length > 1) {
    const sectionId = discoverySectionIdFromPath(currentPath);
    if (sectionId === null) {
      return { kind: "not-found" };
    }
    return canonicalOrRedirect(
      currentPath,
      parsedRoute(buildDiscoverySectionPath(sectionId), "discovery"),
      search,
      hash,
    );
  }

  if (
    root === "discovery" ||
    root === "requests" ||
    root === "calendar" ||
    root === "api-explorer"
  ) {
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
    if (
      rawSegments.length > 2 ||
      !["activity", "import", "history"].includes(section)
    ) {
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
          buildViewPath(
            "activity",
            undefined,
            undefined,
            undefined,
            undefined,
            "history",
          ),
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

    if (section === "rules") {
      return resolveRulesRoute(
        currentPath,
        rawSegments,
        normalizedSegments,
        search,
        hash,
      );
    }

    if (MOVED_LIFECYCLE_RULES_SEGMENTS.has(section)) {
      return redirectTo(buildRulesPath("lifecycle"), search, hash);
    }

    const settingsSection = AUTOMATION_SETTINGS_BY_SEGMENT[section];
    if (!settingsSection || rawSegments.length !== 2) {
      return { kind: "not-found" };
    }
    return canonicalOrRedirect(
      currentPath,
      settingsRoute(settingsSection),
      search,
      hash,
    );
  }

  if (root === "integrations") {
    const settingsSection =
      INTEGRATION_SETTINGS_BY_SEGMENT[normalizedSegments[1] ?? ""];
    if (!settingsSection) {
      return { kind: "not-found" };
    }
    // The Indexers page carries its seeding-profiles pane as a third segment;
    // every other integrations section is a bare two-segment path.
    if (settingsSection === "indexers" && rawSegments.length === 3) {
      const paneSegment = normalizedSegments[2] ?? "";
      if (MOVED_INDEXER_PROXY_SEGMENTS.has(paneSegment)) {
        return redirectTo(buildViewPath("settings", "proxies"), search, hash);
      }
      const tab = INDEXER_TAB_BY_SEGMENT[paneSegment];
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
    return canonicalOrRedirect(
      currentPath,
      settingsRoute(settingsSection),
      search,
      hash,
    );
  }

  if (root === "wanted") {
    const wantedSegment = normalizedSegments[1] ?? "items";
    if (wantedSegment === "history" && rawSegments.length === 2) {
      return redirectTo(
        buildViewPath(
          "activity",
          undefined,
          undefined,
          undefined,
          undefined,
          "history",
        ),
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
          buildViewPath(
            "activity",
            undefined,
            undefined,
            undefined,
            undefined,
            "history",
          ),
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
      return canonicalOrRedirect(
        currentPath,
        settingsRoute(localSection),
        search,
        hash,
      );
    }
    const movedRulesSection = MOVED_SETTINGS_RULES_BY_SEGMENT[section];
    if (movedRulesSection) {
      return redirectTo(buildRulesPath(movedRulesSection), search, hash);
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
        parsedRoute("/system/recycle-bin", "system", {
          systemSection: "recycleBin",
        }),
        search,
        hash,
      );
    }
    const systemSetting =
      SYSTEM_SETTINGS_BY_SEGMENT[section === "backups" ? "backup" : section];
    if (systemSetting) {
      return canonicalOrRedirect(
        currentPath,
        settingsRoute(systemSetting),
        search,
        hash,
      );
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

export function parseLanguageFromParam(
  value: string | null,
): LocaleCode | null {
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
export const ENABLE_FORM_LOGIN_LOCATION_STATE = {
  enableFormLogin: true,
} as const;

export function readEnableFormLoginIntent(state: unknown): boolean {
  return (
    typeof state === "object" &&
    state !== null &&
    (state as { enableFormLogin?: unknown }).enableFormLogin === true
  );
}
