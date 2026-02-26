export type ViewId = "movies" | "series" | "anime" | "activity" | "wanted" | "settings" | "system";
export type SettingsSection =
  | "profile"
  | "general"
  | "users"
  | "indexers"
  | "downloadClients"
  | "qualityProfiles"
  | "acquisition";
export type ContentSettingsSection = "overview" | "settings";

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

export type SystemHealth = {
  serviceReady: boolean;
  dbPath: string;
  totalTitles: number;
  monitoredTitles: number;
  totalUsers: number;
  titlesMovie: number;
  titlesTv: number;
  titlesAnime: number;
  titlesOther: number;
  recentEvents: number;
  recentEventPreview: string[];
};
