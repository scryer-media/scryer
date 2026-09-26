import type { ExternalId, Facet } from "./titles";

export type ListScope = "PUBLIC" | "PERSONAL";
export type ListMode = "SEARCH" | "ADD" | "HOLD" | "REQUEST" | "DISCOVER";
export type ListOnLeave = "KEEP" | "LOG" | "UNMONITOR" | "TAG";
export type ListSyncState = "OK" | "NEW" | "FAIL" | "OFF";
export type ListPolicy = "AUTO" | "APPROVAL" | "NONE";
export type ListExclusionScopeKind = "ALL_LISTS" | "LIST";
export type ListSyncRunOutcome = "SUCCEEDED" | "FAILED" | "SKIPPED";
export type ListMembershipState =
  | "IN_LIBRARY"
  | "ADDED"
  | "REQUESTED"
  | "HELD"
  | "REJECTED"
  | "FILTERED"
  | "EXCLUDED"
  | "UNRESOLVED"
  | "BLOCKED_PERMISSION"
  | "DISCOVER"
  | "PENDING";
export type ListFilterKind =
  | "RATING_AT_LEAST"
  | "RELEASE_YEAR"
  | "EXCLUDE_GENRES"
  | "FORMAT"
  | "LANGUAGE"
  | "SKIP_ON_MY_STREAMING_SERVICES"
  | "RELEASED_ONLY"
  | "DIRECTOR_CREDITS_ONLY"
  | "NOT_SEQUEL_WITHOUT_BASE";
export type ListAuthBadge =
  | "NO_ACCOUNT"
  | "NO_ACCOUNT_NEEDS_VALUE"
  | "MEMBER_ACCOUNT"
  | "SERVER_API_KEY";
export type ListSourceParamType = "TEXT" | "URL" | "ENUM" | "SEASON";
export type ListNoteTone = "INFO" | "WARN" | "BAD";

export type ListParam = { key: string; value: string };

export type ListProviderTile = { bg: string; ink: string; abbr: string };

export type ListSourceParamDefinition = {
  key: string;
  label: string;
  type: ListSourceParamType;
  options: string[];
  required: boolean;
};

export type ListProviderItem = {
  id: string;
  name: string;
  description: string | null;
  kinds: Facet[];
  sourceType: string;
  params: ListSourceParamDefinition[];
  personal: boolean;
  defaultIntervalSeconds: number;
};

export type ListProviderGroup = {
  label: string;
  authBadge: ListAuthBadge;
  items: ListProviderItem[];
};

export type ListUrlPattern = {
  pattern: string;
  sourceType: string;
  captures: Array<{ group: string; param: string }>;
};

export type ListProviderManifest = {
  providerType: string;
  name: string;
  summary: string | null;
  blurb: string | null;
  tile: ListProviderTile | null;
  coverage: Facet[];
  groups: ListProviderGroup[];
  notes: Array<{ tone: ListNoteTone; textKey: string }>;
  urlPatterns: ListUrlPattern[];
  configFields: ListProviderSettingField[];
};

export type ListProviderSettingFieldType =
  | "STRING"
  | "PASSWORD"
  | "MULTILINE"
  | "BOOL"
  | "SELECT"
  | "FILTERED_SELECT"
  | "NUMBER"
  | "PATH"
  | "TAG";

export type ListProviderSettingField = {
  key: string;
  label: string;
  helpText: string | null;
  type: ListProviderSettingFieldType;
  required: boolean;
  secret: boolean;
  isSet: boolean;
  value: string | null;
};

export type ListProviderSettings = {
  providerType: string;
  fields: ListProviderSettingField[];
};

export type ListProviderSettingChange = { key: string; value: string | null };

export type TitleListMembership = {
  subscriptionId: string;
  name: string;
  state: ListMembershipState;
  addedByList: boolean;
  leftAt: string | null;
};

export type ListRoute = {
  kind: Facet;
  libraryId: string;
  qualityProfileId: string | null;
  rootFolderId: string | null;
  monitorType: string;
  minAvailability: string | null;
  useSeasonFolders: boolean | null;
  releaseNumbering: string | null;
  tags: string[];
};

export type ListFilter = {
  kind: ListFilterKind;
  scale: string | null;
  value: number | null;
  from: number | null;
  to: number | null;
  values: string[];
};

export type ListCounts = {
  total: number;
  inLibrary: number;
  added: number;
  requested: number;
  held: number;
  filtered: number;
  excluded: number;
  unresolved: number;
};

export type ListSyncStatus = {
  state: ListSyncState;
  lastAt: string | null;
  nextAt: string | null;
  errorMessage: string | null;
  errorAt: string | null;
  pausedUntil: string | null;
};

export type ListSubscription = {
  id: string;
  scope: ListScope;
  name: string;
  providerUrl: string | null;
  source: {
    provider: string;
    sourceType: string;
    params: ListParam[];
  };
  kinds: Facet[];
  enabled: boolean;
  mode: ListMode;
  routes: ListRoute[];
  filters: ListFilter[];
  maxPerSync: number | null;
  onLeave: ListOnLeave;
  intervalSeconds: number;
  sync: ListSyncStatus;
  counts: ListCounts;
  createdAt: string;
  updatedAt: string;
};

export type ListPreviewItem = {
  itemKey: string;
  displayTitle: string;
  year: number | null;
  kind: Facet | null;
  posterUrl: string | null;
};

export type ListPreview = {
  recognized: boolean;
  provider: string | null;
  sourceType: string | null;
  params: ListParam[];
  name: string | null;
  kinds: Facet[];
  total: number;
  inLibrary: number;
  filtered: number;
  excluded: number;
  unresolved: number;
  wouldAdd: ListPreviewItem[];
};

export type ListMembership = {
  itemKey: string;
  rank: number | null;
  season: number | null;
  kind: Facet;
  state: ListMembershipState;
  stateReason: string | null;
  displayTitle: string | null;
  year: number | null;
  titleId: string | null;
  requestId: string | null;
  addedByList: boolean;
  firstSeenAt: string;
  lastSeenAt: string;
  leftAt: string | null;
};

export type ListMembershipPage = {
  totalCount: number;
  items: ListMembership[];
};

export type ListSyncRun = {
  id: string;
  startedAt: string;
  finishedAt: string | null;
  outcome: ListSyncRunOutcome;
  counts: ListCounts;
  errorMessage: string | null;
};

export type ListExclusion = {
  id: string;
  kind: Facet;
  externalIds: ExternalId[];
  displayTitle: string;
  year: number | null;
  scope: ListExclusionScopeKind;
  subscriptionId: string | null;
  subscriptionName: string | null;
  createdAt: string;
};

export type MemberListPolicy = {
  user: { id: string; username: string };
  policy: ListPolicy;
  listRequestsLast30d: number;
};

/** The editable part of a subscription, shared by follow and edit. */
export type ListSubscriptionDraft = {
  name: string;
  kinds: Facet[];
  mode: ListMode;
  routes: ListRoute[];
  filters: ListFilter[];
  maxPerSync: number | null;
  onLeave: ListOnLeave;
};

/** Where a new subscription reads from: a manifest item or a recognised URL. */
export type ListSourceDraft = {
  provider: string;
  sourceType: string;
  params: ListParam[];
  url: string | null;
};
