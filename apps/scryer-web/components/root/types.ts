export type ViewId =
  | "dashboard"
  | "movies"
  | "series"
  | "anime"
  | "discovery"
  | "requests"
  | "activity"
  | "calendar"
  | "wanted"
  | "settings"
  | "logs"
  | "system";
export type SystemSection = "overview" | "jobs" | "recycleBin";
export type LogsSection = "logs" | "audit";
export type ActivitySection = "activity" | "import" | "history";
export type WantedSection = "wanted" | "cutoff" | "pending";
export type SettingsSection =
  | "profile"
  | "general"
  | "backups"
  | "security"
  | "users"
  | "mediaServers"
  | "indexers"
  | "downloadClients"
  | "proxies"
  | "qualityProfiles"
  | "delayProfiles"
  | "acquisition"
  | "rules"
  | "maintenanceRules"
  | "plugins"
  | "notifications"
  | "post-processing"
  | "subtitles";

/// Panes of the Rules page. Scoring and maintenance rules are two kinds of the
/// same subject, so they share one nav entry and one gutter rather than sitting
/// beside each other in the sidebar. Each pane keeps its own `SettingsSection`
/// underneath, so permissions and the settings shell are untouched.
export type RulesSection = "scoring" | "maintenance";

/// Panes of the Maintenance Rules page, reached through a second gutter. The
/// rule list is the default, so `/automation/rules/maintenance` still means
/// what a bare maintenance-rules link always meant. Exclusions live with the
/// gates because both answer "what is this instance allowed to do", not "what
/// would a rule match".
export type MaintenanceRulesSection = "rules" | "candidates" | "history" | "gates";

/// Panes of the Indexers settings page. Seeding profiles live here rather than
/// in their own settings section because a profile is only ever reached
/// through the indexer that applies it. Proxies used to be a pane too; they
/// are a settings section of their own now that download clients assign them.
export type IndexerSettingsTab = "indexers" | "search" | "seedingProfiles";
export type ContentSettingsSection =
  | "overview"
  | "import"
  | "library"
  | "general"
  | "quality"
  | "renaming"
  | "routing";

export type OverviewTitleTarget = {
  id: string;
  slug?: string | null;
  libraryId?: string | null;
  librarySlug?: string | null;
};

export type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

export type ActivityEvent = {
  id: string;
  kind: string;
  severity: string;
  channels: string[];
  eventType?: string;
  message: string;
  actorUserId?: string | null;
  titleId?: string | null;
  occurredAt?: string | null;
};

export type IndexerQueryStats = {
  indexerId: string;
  indexerName: string;
  queriesLast24H: number;
  successfulLast24H: number;
  failedLast24H: number;
  lastQueryAt: string | null;
  apiCurrent: number | null;
  apiMax: number | null;
  grabCurrent: number | null;
  grabMax: number | null;
};

export type SystemHealth = {
  serviceReady: boolean;
  dbPath: string;
  datastoreEngine: string;
  datastoreMigrationKey: string | null;
  runtimePathStyle: "UNIX" | "WINDOWS";
  totalTitles: number;
  monitoredTitles: number;
  totalUsers: number;
  titlesMovie: number;
  titlesSeries: number;
  titlesAnime: number;
  titlesOther: number;
  recentEvents: number;
  recentEventPreview: string[];
  dbMigrationVersion: string | null;
  indexerStats: IndexerQueryStats[];
};

export type SmgVersionCompatibilityNotice = {
  status: string;
  minimumVersion: string;
  yourVersion: string;
  message: string;
  upgradeDeadline: string | null;
};

export type SmgScryerUpdateNotice = {
  available: boolean;
  currentVersion: string;
  latestVersion: string;
  latestTag: string;
  releaseUrl: string | null;
  publishedAt: string | null;
  checkedAt: string;
};
