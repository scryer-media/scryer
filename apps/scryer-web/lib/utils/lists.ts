import type {
  ListCounts,
  ListExclusionScopeKind,
  ListFilter,
  ListMembershipState,
  ListMode,
  ListOnLeave,
  ListParam,
  ListProviderGroup,
  ListProviderItem,
  ListProviderManifest,
  ListProviderSettingChange,
  ListProviderSettingField,
  ListProviderTile,
  ListRoute,
  ListSourceDraft,
  ListSourceParamDefinition,
  ListSubscription,
  ListSubscriptionDraft,
  ListSyncRunOutcome,
  ListSyncState,
  TitleListMembership,
} from "../types/lists.ts";
import type { ExternalId, Facet } from "../types/titles.ts";
import { selectorToken } from "./dom-ids.ts";

export type ListTone = "neutral" | "positive" | "warning" | "negative" | "info" | "outline";

/** Modes offered for public lists. `REQUEST` belongs to personal lists only. */
export const PUBLIC_LIST_MODES: readonly ListMode[] = ["SEARCH", "ADD", "HOLD", "DISCOVER"];

/** Discover-only membership has no destination yet, so it cannot be chosen. */
export function isListModeSelectable(mode: ListMode): boolean {
  return mode === "SEARCH" || mode === "ADD" || mode === "HOLD";
}

export const LIST_ON_LEAVE_OPTIONS: readonly ListOnLeave[] = ["KEEP", "LOG", "UNMONITOR", "TAG"];

export const LIST_KINDS: readonly Facet[] = ["MOVIE", "SERIES", "ANIME"];

function camel(value: string): string {
  return value.toLowerCase().replace(/_([a-z0-9])/g, (_match, next: string) => next.toUpperCase());
}

export function listModeLabelKey(mode: ListMode): string {
  return `lists.mode.${camel(mode)}`;
}

export function listModeHelpKey(mode: ListMode): string {
  return `lists.modeHelp.${camel(mode)}`;
}

export function listOnLeaveLabelKey(onLeave: ListOnLeave): string {
  return `lists.onLeave.${camel(onLeave)}`;
}

export function listSyncStateLabelKey(state: ListSyncState): string {
  return `lists.syncState.${camel(state)}`;
}

export function listSyncStateTone(state: ListSyncState): ListTone {
  switch (state) {
    case "OK":
      return "positive";
    case "NEW":
      return "info";
    case "FAIL":
      return "negative";
    case "OFF":
      return "neutral";
  }
}

export function listMembershipStateLabelKey(state: ListMembershipState): string {
  return `lists.membershipState.${camel(state)}`;
}

export function listMembershipStateTone(state: ListMembershipState): ListTone {
  switch (state) {
    case "IN_LIBRARY":
    case "ADDED":
      return "positive";
    case "REQUESTED":
    case "HELD":
    case "PENDING":
      return "info";
    case "FILTERED":
    case "EXCLUDED":
    case "DISCOVER":
      return "neutral";
    case "UNRESOLVED":
    case "BLOCKED_PERMISSION":
      return "warning";
    case "REJECTED":
      return "negative";
  }
}

export function listSyncRunOutcomeLabelKey(outcome: ListSyncRunOutcome): string {
  return `lists.runOutcome.${camel(outcome)}`;
}

export function listSyncRunOutcomeTone(outcome: ListSyncRunOutcome): ListTone {
  switch (outcome) {
    case "SUCCEEDED":
      return "positive";
    case "FAILED":
      return "negative";
    case "SKIPPED":
      return "neutral";
  }
}

export function listKindLabelKey(kind: Facet): string {
  return `lists.kind.${camel(kind)}`;
}

export type ListCoverageSegmentKey =
  | "inLibrary"
  | "added"
  | "requested"
  | "held"
  | "filtered"
  | "excluded"
  | "unresolved";

export const LIST_COVERAGE_ORDER: readonly ListCoverageSegmentKey[] = [
  "inLibrary",
  "added",
  "requested",
  "held",
  "filtered",
  "excluded",
  "unresolved",
];

export type ListCoverageSegment = {
  key: ListCoverageSegmentKey;
  count: number;
  /** Share of the list's total, 0..1. */
  fraction: number;
};

/**
 * The coverage bar: one segment per non-empty count, in a fixed order, sized
 * against the list total. Counts larger than the total (a list that shrank
 * between syncs) are scaled down so the bar never overflows.
 */
export function listCoverageSegments(counts: ListCounts): ListCoverageSegment[] {
  const values = LIST_COVERAGE_ORDER.map((key) => ({ key, count: Math.max(0, counts[key] ?? 0) }));
  const sum = values.reduce((acc, entry) => acc + entry.count, 0);
  const denominator = Math.max(counts.total, sum);
  if (denominator <= 0) {
    return [];
  }
  return values
    .filter((entry) => entry.count > 0)
    .map((entry) => ({ ...entry, fraction: entry.count / denominator }));
}

export function listCoverageSegmentLabelKey(key: ListCoverageSegmentKey): string {
  return `lists.counts.${key}`;
}

export type ListIntervalParts = { key: string; count: number };

/** A provider interval in the largest whole unit (days, hours, minutes). */
export function listIntervalParts(seconds: number): ListIntervalParts {
  const safe = Math.max(0, Math.round(seconds));
  if (safe >= 86_400 && safe % 86_400 === 0) {
    return { key: "lists.interval.days", count: safe / 86_400 };
  }
  if (safe >= 3_600 && safe % 3_600 === 0) {
    return { key: "lists.interval.hours", count: safe / 3_600 };
  }
  return { key: "lists.interval.minutes", count: Math.max(1, Math.round(safe / 60)) };
}

/**
 * The public catalog: member-account groups and personal items belong to the
 * personal tab and are dropped, along with groups left empty.
 */
export function publicProviderGroups(manifest: ListProviderManifest): ListProviderGroup[] {
  return manifest.groups
    .filter((group) => group.authBadge !== "MEMBER_ACCOUNT")
    .map((group) => ({ ...group, items: group.items.filter((item) => !item.personal) }))
    .filter((group) => group.items.length > 0);
}

export function publicProviders(manifests: readonly ListProviderManifest[]): ListProviderManifest[] {
  return manifests
    .map((manifest) => ({ ...manifest, groups: publicProviderGroups(manifest) }))
    .filter((manifest) => manifest.groups.length > 0);
}

export function findProviderItem(
  manifests: readonly ListProviderManifest[],
  provider: string,
  sourceType: string,
): { manifest: ListProviderManifest; item: ListProviderItem } | null {
  for (const manifest of manifests) {
    if (manifest.providerType !== provider) continue;
    for (const group of manifest.groups) {
      const item = group.items.find((entry) => entry.sourceType === sourceType);
      if (item) return { manifest, item };
    }
  }
  return null;
}

/**
 * Provider URL patterns are written for the server's regex engine. Rewrite the
 * two constructs JavaScript spells differently: `(?P<name>` groups and a
 * leading `(?i)` flag.
 */
export function listUrlPatternToRegExp(pattern: string): RegExp | null {
  let source = pattern.replace(/\(\?P</g, "(?<");
  let flags = "";
  if (source.startsWith("(?i)")) {
    source = source.slice(4);
    flags = "i";
  }
  try {
    return new RegExp(source, flags);
  } catch {
    return null;
  }
}

export type ListUrlRecognition = {
  manifest: ListProviderManifest;
  source: ListSourceDraft;
};

/**
 * Instant client-side recognition for the add-by-URL box. The server preview
 * stays authoritative; this only drives the recognition strip.
 */
export function recognizeListUrl(
  rawUrl: string,
  manifests: readonly ListProviderManifest[],
): ListUrlRecognition | null {
  const url = rawUrl.trim();
  if (!url) return null;
  for (const manifest of manifests) {
    for (const urlPattern of manifest.urlPatterns) {
      const regex = listUrlPatternToRegExp(urlPattern.pattern);
      const match = regex?.exec(url);
      if (!match) continue;
      const params: ListParam[] = [];
      for (const capture of urlPattern.captures) {
        const value = match.groups?.[capture.group];
        if (value) params.push({ key: capture.param, value: decodeURIComponent(value) });
      }
      return {
        manifest,
        source: { provider: manifest.providerType, sourceType: urlPattern.sourceType, params, url },
      };
    }
  }
  return null;
}

/** Required parameter keys that are still empty. */
export function missingSourceParams(
  definitions: readonly ListSourceParamDefinition[],
  params: readonly ListParam[],
): string[] {
  return definitions
    .filter((definition) => definition.required)
    .filter((definition) => !params.find((param) => param.key === definition.key && param.value.trim()))
    .map((definition) => definition.key);
}

export function setListParam(params: readonly ListParam[], key: string, value: string): ListParam[] {
  const next = params.filter((param) => param.key !== key);
  next.push({ key, value });
  return next;
}

export function providerTileStyle(tile: ListProviderTile | null): { background: string; color: string } | null {
  if (!tile) return null;
  return { background: tile.bg, color: tile.ink };
}

export function providerTileAbbreviation(manifest: Pick<ListProviderManifest, "name" | "tile">): string {
  if (manifest.tile?.abbr) return manifest.tile.abbr;
  return manifest.name.slice(0, 2).toUpperCase();
}

export type ListRouteLibrary = {
  id: string;
  facet: Facet;
  isDefault: boolean;
  qualityProfileId?: string | null;
  roots: Array<{ id: string; isDefault: boolean }>;
};

/** A new route for a kind, seeded from that kind's default library. */
export function defaultListRoute(
  kind: Facet,
  libraries: readonly ListRouteLibrary[],
  monitorType: string,
): ListRoute {
  const candidates = libraries.filter((library) => library.facet === kind);
  const library = candidates.find((entry) => entry.isDefault) ?? candidates[0];
  const root = library?.roots.find((entry) => entry.isDefault) ?? library?.roots[0];
  return {
    kind,
    libraryId: library?.id ?? "",
    qualityProfileId: null,
    rootFolderId: root?.id ?? null,
    monitorType,
    minAvailability: kind === "MOVIE" ? "announced" : null,
    useSeasonFolders: kind === "MOVIE" ? null : true,
    releaseNumbering: kind === "MOVIE" ? null : "AUTO",
    tags: [],
  };
}

export function emptyListDraft(name: string, kinds: readonly Facet[]): ListSubscriptionDraft {
  return {
    name,
    kinds: [...kinds],
    mode: "ADD",
    routes: [],
    filters: [],
    maxPerSync: null,
    onLeave: "KEEP",
  };
}

export function subscriptionToDraft(subscription: ListSubscription): ListSubscriptionDraft {
  return {
    name: subscription.name,
    kinds: [...subscription.kinds],
    mode: subscription.mode,
    routes: subscription.routes.map((route) => ({ ...route, tags: [...route.tags] })),
    filters: subscription.filters.map((filter) => ({ ...filter, values: [...filter.values] })),
    maxPerSync: subscription.maxPerSync,
    onLeave: subscription.onLeave,
  };
}

/** Problems that block saving, as i18n keys. */
export function listDraftProblems(draft: ListSubscriptionDraft): string[] {
  const problems: string[] = [];
  if (!draft.name.trim()) problems.push("lists.follow.problem.name");
  if (draft.kinds.length === 0) problems.push("lists.follow.problem.kinds");
  if (!isListModeSelectable(draft.mode)) problems.push("lists.follow.problem.mode");
  for (const kind of draft.kinds) {
    const route = draft.routes.find((entry) => entry.kind === kind);
    if (!route || !route.libraryId) {
      problems.push("lists.follow.problem.route");
      break;
    }
  }
  if (draft.maxPerSync !== null && draft.maxPerSync < 1) problems.push("lists.follow.problem.maxPerSync");
  return problems;
}

type ListRouteInput = ListRoute;
type ListFilterInput = ListFilter;

export type UpdateListSubscriptionInput = {
  name: string;
  kinds: Facet[];
  mode: ListMode;
  routes: ListRouteInput[];
  filters: ListFilterInput[];
  maxPerSync: number | null;
  onLeave: ListOnLeave;
};

export type SubscribeListInput = UpdateListSubscriptionInput & {
  scope: "PUBLIC";
  provider: string;
  sourceType: string;
  params: ListParam[];
  url: string | null;
};

/** Only routes for kinds the list follows are sent; unrouted kinds are never added. */
export function draftToUpdateInput(draft: ListSubscriptionDraft): UpdateListSubscriptionInput {
  return {
    name: draft.name.trim(),
    kinds: [...draft.kinds],
    mode: draft.mode,
    routes: draft.routes
      .filter((route) => draft.kinds.includes(route.kind))
      .map((route) => ({ ...route, tags: [...route.tags] })),
    filters: draft.filters.map((filter) => ({ ...filter, values: [...filter.values] })),
    maxPerSync: draft.maxPerSync,
    onLeave: draft.onLeave,
  };
}

export function draftToSubscribeInput(source: ListSourceDraft, draft: ListSubscriptionDraft): SubscribeListInput {
  return {
    scope: "PUBLIC",
    provider: source.provider,
    sourceType: source.sourceType,
    params: source.params.filter((param) => param.value.trim()).map((param) => ({ ...param })),
    url: source.url,
    ...draftToUpdateInput(draft),
  };
}

export type AddListExclusionInput = {
  kind: Facet;
  externalIds: ExternalId[];
  displayTitle: string;
  year: number | null;
  scope: ListExclusionScopeKind;
  subscriptionId: string | null;
};

/** The all-lists exclusion offered when a title is deleted. */
export function exclusionInputFromTitle(title: {
  facet: Facet;
  name: string;
  year?: number | null;
  externalIds?: readonly ExternalId[] | null;
}): AddListExclusionInput | null {
  const externalIds = (title.externalIds ?? [])
    .filter((entry) => entry.source.trim() && entry.value.trim())
    .map((entry) => ({ source: entry.source, value: entry.value }));
  if (externalIds.length === 0) return null;
  return {
    kind: title.facet,
    externalIds,
    displayTitle: title.name,
    year: title.year ?? null,
    scope: "ALL_LISTS",
    subscriptionId: null,
  };
}

/** Parse "tmdb:603, imdb:tt0133093" into external ids; unknown shapes are skipped. */
export function parseExternalIdList(raw: string): ExternalId[] {
  return raw
    .split(/[\s,]+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .map((token) => {
      const separator = token.indexOf(":");
      if (separator <= 0 || separator === token.length - 1) return null;
      return { source: token.slice(0, separator).toLowerCase(), value: token.slice(separator + 1) };
    })
    .filter((entry): entry is ExternalId => entry !== null);
}

export function findListFilter(filters: readonly ListFilter[], kind: ListFilter["kind"]): ListFilter | null {
  return filters.find((filter) => filter.kind === kind) ?? null;
}

/** Replace (or with `null`, remove) the filter of one kind, keeping the others in order. */
export function withListFilter(
  filters: readonly ListFilter[],
  kind: ListFilter["kind"],
  next: Omit<ListFilter, "kind"> | null,
): ListFilter[] {
  const index = filters.findIndex((filter) => filter.kind === kind);
  if (next === null) {
    return filters.filter((filter) => filter.kind !== kind);
  }
  const filter: ListFilter = { kind, ...next };
  if (index < 0) return [...filters, filter];
  return filters.map((entry, position) => (position === index ? filter : entry));
}

export const EMPTY_LIST_FILTER: Omit<ListFilter, "kind"> = {
  scale: null,
  value: null,
  from: null,
  to: null,
  values: [],
};

export function splitListValues(raw: string): string[] {
  return raw
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean);
}

/** Stable DOM id for one membership row in a list's detail panel. */
export function listMembershipRowId(subscriptionId: string, itemKey: string): string {
  return `list-membership-${selectorToken(subscriptionId)}-${selectorToken(itemKey)}`;
}

/** What a "Sync now" click is waiting to see change. */
export type ListSyncWatchSnapshot = {
  lastAt: string | null;
  state: ListSyncState | null;
  /** Sync run ids the detail panel showed; null when the panel was not loaded. */
  runIds: readonly string[] | null;
};

/**
 * Whether a requested sync has visibly finished: a run id the baseline did not
 * have, or a change to the subscription's last sync time or sync state.
 */
export function listSyncWatchSettled(
  baseline: ListSyncWatchSnapshot,
  current: ListSyncWatchSnapshot,
): boolean {
  if (baseline.runIds && current.runIds) {
    const known = new Set(baseline.runIds);
    if (current.runIds.some((id) => !known.has(id))) return true;
  }
  if (current.lastAt !== null && current.lastAt !== baseline.lastAt) return true;
  if (current.state !== null && baseline.state !== null && current.state !== baseline.state) return true;
  return false;
}

const LIST_SYNC_POLL_DELAYS_MS = [1_000, 2_000, 3_000, 5_000, 8_000] as const;
const LIST_SYNC_POLL_MAX_DELAY_MS = 10_000;
/** How long a "Sync now" click keeps refreshing before leaving it to the next visit. */
export const LIST_SYNC_POLL_BUDGET_MS = 180_000;

/**
 * Delay before the next refresh after a "Sync now" click, backing off to a
 * ceiling. Returns null once the polling budget is spent.
 */
export function listSyncPollDelayMs(attempt: number, elapsedMs: number): number | null {
  const delay =
    LIST_SYNC_POLL_DELAYS_MS[Math.max(0, attempt)] ?? LIST_SYNC_POLL_MAX_DELAY_MS;
  if (elapsedMs + delay > LIST_SYNC_POLL_BUDGET_MS) return null;
  return delay;
}

/** What the settings form holds for one field the user touched. */
export type ListProviderSettingEdit = { value: string; clear: boolean };

/**
 * The changes to send for a provider's settings. Untouched fields are left
 * out so the server keeps them. A blank secret keeps the stored secret unless
 * the user chose Clear; a blank plain value clears it.
 */
export function listProviderSettingChanges(
  fields: readonly ListProviderSettingField[],
  edits: Readonly<Record<string, ListProviderSettingEdit>>,
): ListProviderSettingChange[] {
  const changes: ListProviderSettingChange[] = [];
  for (const field of fields) {
    const edit = edits[field.key];
    if (!edit) continue;
    const value = edit.value.trim();
    if (field.secret) {
      if (edit.clear) {
        if (field.isSet) changes.push({ key: field.key, value: null });
      } else if (value !== "") {
        changes.push({ key: field.key, value });
      }
      continue;
    }
    const current = (field.value ?? "").trim();
    if (value === current) continue;
    changes.push({ key: field.key, value: value === "" ? null : value });
  }
  return changes;
}

/** Required fields that would be empty after saving these edits. */
export function listProviderSettingsMissing(
  fields: readonly ListProviderSettingField[],
  edits: Readonly<Record<string, ListProviderSettingEdit>>,
): string[] {
  return fields
    .filter((field) => {
      if (!field.required) return false;
      const edit = edits[field.key];
      if (!edit) return !field.isSet;
      if (field.secret) return edit.clear || (!field.isSet && edit.value.trim() === "");
      return edit.value.trim() === "";
    })
    .map((field) => field.key);
}

export type TitleListProvenance = { kind: "added" | "left"; name: string };

/**
 * The line a title page shows about the list that added it: the list still
 * holding it, or, once every list that added it has dropped it, the one it
 * left most recently. Null when no list added the title.
 */
export function titleListProvenance(memberships: readonly TitleListMembership[]): TitleListProvenance | null {
  const addedBy = memberships.filter((membership) => membership.addedByList);
  const current = addedBy.find((membership) => !membership.leftAt);
  if (current) return { kind: "added", name: current.name };
  if (addedBy.length === 0) return null;
  const latest = addedBy.reduce((best, membership) =>
    (membership.leftAt ?? "") > (best.leftAt ?? "") ? membership : best,
  );
  return { kind: "left", name: latest.name };
}
