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
  MemberListPolicy,
  TitleListMembership,
} from "../types/lists.ts";
import type { ExternalId, Facet } from "../types/titles.ts";
import { allOf, forEventTypes, forTitle, type DomainEventPredicate } from "../reactive/domain-event-feed.ts";
import { selectorToken } from "./dom-ids.ts";
import { facetById } from "../facets/registry.ts";
import { metadataResultExternalIds, type MetadataResultIdentity } from "./metadata-result-external-ids.ts";
import { getPluginLogoSources } from "./plugin-logos.ts";
import { ratingSourceInfo } from "./title-ratings.ts";

export type ListTone = "neutral" | "positive" | "warning" | "negative" | "info" | "accent" | "outline";

export function listMembershipTitleHref(kind: Facet, titleId: string | null, seriesMovieLinkId?: string | null): string | null {
  const facet = facetById(kind);
  const id = titleId?.trim();
  if (!facet || !id) return null;
  const params = new URLSearchParams({ id });
  if (seriesMovieLinkId) params.set("seriesMovie", seriesMovieLinkId);
  return `/${facet.viewId}?${params}`;
}

/** Modes offered for public lists. `REQUEST` belongs to personal lists only. */
export const PUBLIC_LIST_MODES: readonly ListMode[] = ["SEARCH", "ADD", "HOLD", "DISCOVER"];

/** Discover-only membership has no destination yet, so it cannot be chosen. */
export function isListModeSelectable(mode: ListMode): boolean {
  return mode === "SEARCH" || mode === "ADD" || mode === "HOLD" || mode === "REQUEST";
}

export const LIST_ON_LEAVE_OPTIONS: readonly ListOnLeave[] = ["KEEP", "LOG", "UNMONITOR", "TAG"];

export const LIST_KINDS: readonly Facet[] = ["MOVIE", "SERIES", "ANIME"];

/** Only media enums duplicate Include; status, credits and order remain source options. */
export function isListMediaParam(param: ListSourceParamDefinition): boolean {
  return param.type === "ENUM" && ["type", "kind"].includes(param.key)
    && param.options.length > 0
    && param.options.every((option) => ["all", "movie", "movies", "series", "shows", "anime"].includes(option));
}

/** Preserve the effective scope of older follows that saved both selectors. */
export function listKindsFromSourceParams(kinds: readonly Facet[], definitions: readonly ListSourceParamDefinition[], params: readonly ListParam[], legacy = false): Facet[] {
  const selector = definitions.find(isListMediaParam);
  const selected = selector && (params.find((param) => param.key === selector.key)?.value
    ?? (legacy && !selector.options.includes("all") ? selector.options[0] : undefined));
  if (!selected || selected === "all" || selected === "__include__") return [...kinds];
  const allowed: readonly Facet[] = selected === "anime" ? ["ANIME", "MOVIE"]
    : ["movie", "movies"].includes(selected) ? ["MOVIE"]
      : selector?.options.includes("anime") ? ["SERIES"] : ["SERIES", "ANIME"];
  return kinds.filter((kind) => allowed.includes(kind));
}

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

export function listSyncStateLabelKey(state: ListSyncState, lastAt?: string | null): string {
  // Switching a list back on makes it new to the scheduler again; one that has
  // synced before is only waiting for its next sync.
  if (state === "NEW" && lastAt) return "lists.syncState.waiting";
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

/** The sync state a list's row shows: a switched-off list is off whatever its last sync was. */
export function shownListSyncState(subscription: Pick<ListSubscription, "enabled" | "sync">): ListSyncState {
  return subscription.enabled ? subscription.sync.state : "OFF";
}

export type ListSortKey = "name" | "sync";
export type ListSort = { key: ListSortKey; descending: boolean };

/** Sorting by last sync groups lists by how it went: failed, waiting, synced, then switched off. */
const LIST_SYNC_STATE_ORDER: Record<ListSyncState, number> = { FAIL: 0, NEW: 1, OK: 2, OFF: 3 };

function listSyncTime(subscription: Pick<ListSubscription, "sync">): number {
  const time = subscription.sync.lastAt ? Date.parse(subscription.sync.lastAt) : Number.NaN;
  return Number.isNaN(time) ? Number.NEGATIVE_INFINITY : time;
}

/**
 * The followed lists in the order the table shows them; with no sort chosen
 * they keep the order they came in. Within one sync state the most recently
 * synced list comes first, and lists that tie fall back to their names.
 */
export function sortListSubscriptions<T extends Pick<ListSubscription, "name" | "enabled" | "sync">>(
  subscriptions: readonly T[],
  sort: ListSort | null,
): T[] {
  if (!sort) return [...subscriptions];
  const factor = sort.descending ? -1 : 1;
  const byName = (a: T, b: T) => a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true });
  return [...subscriptions].sort((a, b) => {
    if (sort.key === "name") return byName(a, b) * factor;
    const state = LIST_SYNC_STATE_ORDER[shownListSyncState(a)] - LIST_SYNC_STATE_ORDER[shownListSyncState(b)];
    if (state !== 0) return state * factor;
    const time = listSyncTime(b) - listSyncTime(a);
    if (time !== 0 && !Number.isNaN(time)) return time * factor;
    return byName(a, b);
  });
}

export function listMembershipStateLabelKey(state: ListMembershipState): string {
  return `lists.membershipState.${camel(state)}`;
}

/**
 * Why a list entry is in its state, as the server records it: a short code
 * naming the filter it failed or the action that did not go through.
 */
const LIST_MEMBERSHIP_REASON_KEYS: Readonly<Record<string, string>> = {
  media_type_not_included: "lists.reason.mediaTypeNotIncluded",
  no_route: "lists.reason.noRoute",
  duplicate_target: "lists.reason.duplicateTarget",
  specials: "lists.reason.specials",
  filler: "lists.reason.filler",
  recap: "lists.reason.recap",
  rating: "lists.reason.rating",
  missing_rating: "lists.reason.missingRating",
  genre: "lists.reason.genre",
  missing_genre: "lists.reason.missingGenre",
  release_year: "lists.reason.releaseYear",
  missing_release_year: "lists.reason.missingReleaseYear",
  format: "lists.reason.format",
  language: "lists.reason.language",
  missing_language: "lists.reason.missingLanguage",
  streaming_service: "lists.reason.streamingService",
  unreleased: "lists.reason.unreleased",
  missing_unreleased: "lists.reason.missingReleaseDate",
  director_credit: "lists.reason.directorCredit",
  sequel_without_base: "lists.reason.sequelWithoutBase",
  ambiguous_series_movie: "lists.reason.ambiguousSeriesMovie",
  not_permitted: "lists.reason.notPermitted",
  rejected: "lists.reason.refused",
  not_found: "lists.reason.notFound",
  action_failed: "lists.reason.actionFailed",
  request_rejected: "lists.reason.requestRejected",
};

/** The translation key for a list entry's reason, or null for one this client does not know. */
export function listMembershipReasonKey(reason: string | null | undefined): string | null {
  return reason && Object.hasOwn(LIST_MEMBERSHIP_REASON_KEYS, reason) ? LIST_MEMBERSHIP_REASON_KEYS[reason] : null;
}

export function listMembershipStateTone(state: ListMembershipState): ListTone {
  switch (state) {
    case "IN_LIBRARY":
      return "accent";
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

const COVERAGE_MEMBERSHIP_STATE: Record<ListCoverageSegmentKey, ListMembershipState> = {
  inLibrary: "IN_LIBRARY",
  added: "ADDED",
  requested: "REQUESTED",
  held: "HELD",
  filtered: "FILTERED",
  excluded: "EXCLUDED",
  unresolved: "UNRESOLVED",
};

/**
 * A coverage segment takes the tone of the pill a title in that state wears,
 * so the bar and the rows under it cannot drift apart.
 */
export function listCoverageSegmentTone(key: ListCoverageSegmentKey): ListTone {
  return listMembershipStateTone(COVERAGE_MEMBERSHIP_STATE[key]);
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

/**
 * Whether a followed list already reads this exact source. The server refuses
 * a second follow of the same provider, source type and parameters.
 */
export function isListSourceFollowed(
  subscriptions: readonly Pick<ListSubscription, "source">[],
  source: { provider: string; sourceType: string; params: readonly ListParam[] },
): boolean {
  const paramsKey = (params: readonly ListParam[]) =>
    params.map((param) => `${param.key}\u0000${param.value.trim()}`).sort().join("\u0001");
  const wanted = paramsKey(source.params);
  return subscriptions.some(
    (subscription) =>
      subscription.source.provider === source.provider &&
      subscription.source.sourceType === source.sourceType &&
      paramsKey(subscription.source.params) === wanted,
  );
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
 * The kinds the follow form offers. The server keeps a follow to the kinds its
 * own source declares, so the catalog entry for that source type is the guide;
 * the provider's overall coverage is only a fallback when the source is not in
 * the catalog. Kinds an existing follow already saved stay offered so the list
 * can always be saved again.
 */
export function followListOfferedKinds({
  requestedKinds,
  sourceKinds,
  providerCoverage,
  savedKinds,
}: {
  requestedKinds?: readonly Facet[] | null;
  sourceKinds?: readonly Facet[] | null;
  providerCoverage?: readonly Facet[] | null;
  savedKinds?: readonly Facet[] | null;
}): Facet[] {
  const base = requestedKinds?.length
    ? requestedKinds
    : sourceKinds?.length
      ? sourceKinds
      : providerCoverage?.length
        ? providerCoverage
        : LIST_KINDS;
  const offered = new Set<Facet>([...base, ...(savedKinds ?? [])]);
  return LIST_KINDS.filter((kind) => offered.has(kind));
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
 * A captured URL segment, percent-decoded when it decodes cleanly. A malformed
 * sequence (a lone `%`) keeps the raw text: recognition runs while the user
 * types, the server preview re-reads the URL itself, and a throw here would
 * take down the page.
 */
function decodeListUrlCapture(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

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
        if (value) params.push({ key: capture.param, value: decodeListUrlCapture(value) });
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

/**
 * The shipped logo for a list provider, or null when none exists and the tile
 * keeps its abbreviation. Metadata sites resolve through the rating-source
 * logos; everything else (Plex, plugin providers) through the plugin logos.
 */
export function providerLogoSrc(providerType: string): string | null {
  if (!providerType.trim()) return null;
  return ratingSourceInfo(providerType).logoSrc ?? getPluginLogoSources({ providerType })?.src ?? null;
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

export type ListEpisodePolicyKind = "MONITOR_SPECIALS" | "FILLER_POLICY" | "RECAP_POLICY";

/**
 * The label key for what a route's "inherit" choice resolves to in its library,
 * or null when that cannot be told. Only Anime libraries carry these settings;
 * a Series title that inherits never monitors specials.
 */
export function inheritedListEpisodePolicyLabelKey(
  kind: ListEpisodePolicyKind,
  facet: Facet,
  library:
    | {
        settings?: {
          monitorSpecials?: boolean | null;
          fillerPolicy?: string | null;
          recapPolicy?: string | null;
        } | null;
      }
    | null
    | undefined,
): string | null {
  if (kind === "MONITOR_SPECIALS") {
    const enabled = facet === "SERIES" ? false : library?.settings?.monitorSpecials;
    if (enabled == null) return null;
    return enabled ? "search.seasonFolder.enabled" : "search.seasonFolder.disabled";
  }
  if (kind === "FILLER_POLICY") {
    const policy = library?.settings?.fillerPolicy;
    if (!policy) return null;
    return policy === "SKIP_FILLER" ? "settings.fillerPolicySkipFiller" : "settings.fillerPolicyDownloadAll";
  }
  const policy = library?.settings?.recapPolicy;
  if (!policy) return null;
  return policy === "SKIP_RECAP" ? "settings.recapPolicySkipRecap" : "settings.recapPolicyDownloadAll";
}

/**
 * The per-sync cap a new follow starts with, so a large list fills the library
 * over several syncs instead of all at once. The form lets it be cleared.
 */
export const DEFAULT_LIST_MAX_PER_SYNC = 25;

export function emptyListDraft(name: string, kinds: readonly Facet[]): ListSubscriptionDraft {
  return {
    name,
    kinds: [...kinds],
    mode: "ADD",
    routes: [],
    filters: [],
    maxPerSync: DEFAULT_LIST_MAX_PER_SYNC,
    onLeave: "KEEP",
  };
}

export function subscriptionToDraft(subscription: ListSubscription): ListSubscriptionDraft {
  return {
    name: subscription.name,
    kinds: [...subscription.kinds],
    mode: subscription.mode,
    routes: subscription.routes.map(listRouteInput),
    filters: subscription.filters.map(listFilterInput),
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

/*
 * Query results carry `__typename` and whatever else the selection asked for,
 * and GraphQL input types reject unknown fields. Inputs are therefore built
 * field by field from the declared input shape, never by spreading a result.
 */

export function listParamInput(param: ListParam): ListParam {
  return { key: param.key, value: param.value };
}

export function listRouteInput(route: ListRoute): ListRouteInput {
  return {
    kind: route.kind,
    libraryId: route.libraryId,
    qualityProfileId: route.qualityProfileId,
    rootFolderId: route.rootFolderId,
    monitorType: route.monitorType,
    minAvailability: route.minAvailability,
    useSeasonFolders: route.useSeasonFolders,
    releaseNumbering: route.releaseNumbering,
    tags: [...route.tags],
  };
}

export function listFilterInput(filter: ListFilter): ListFilterInput {
  return {
    facet: filter.facet ?? null,
    matchAny: filter.matchAny ?? false,
    minimums: (filter.minimums ?? []).map(({ source, value }) => ({ source, value })),
    unresolvedLabels: [...(filter.unresolvedLabels ?? [])],
    kind: filter.kind,
    scale: filter.scale,
    value: filter.value,
    from: filter.from,
    to: filter.to,
    values: [...filter.values],
  };
}

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
  scope: "PUBLIC" | "PERSONAL";
  credentialId?: string;
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
      .map(listRouteInput),
    filters: draft.filters.map(listFilterInput),
    maxPerSync: draft.maxPerSync,
    onLeave: draft.onLeave,
  };
}

export function draftToSubscribeInput(source: ListSourceDraft, draft: ListSubscriptionDraft): SubscribeListInput {
  return {
    scope: source.credentialId ? "PERSONAL" : "PUBLIC",
    ...(source.credentialId ? { credentialId: source.credentialId } : {}),
    provider: source.provider,
    sourceType: source.sourceType,
    params: source.params.filter((param) => param.value.trim()).map(listParamInput),
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

/** External ids as the exclusion form's field shows them: "tmdb:603, imdb:tt0133093". */
export function formatExternalIdList(ids: readonly ExternalId[]): string {
  return ids.map((id) => `${id.source}:${id.value}`).join(", ");
}

/**
 * What the exclusion form takes from a metadata search result: its name, its
 * year and every id it is known by.
 */
export function exclusionFieldsFromMetadataResult(
  result: MetadataResultIdentity & { name: string; year: number | null },
  kind: Facet,
): { title: string; year: string; ids: string } {
  return {
    title: result.name,
    year: result.year ? String(result.year) : "",
    ids: formatExternalIdList(metadataResultExternalIds(result, kind)),
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
/** How many syncs a list's history shows before the reader asks for the rest. */
export const LIST_SYNC_RUNS_SHOWN = 20;
/** The most syncs one read returns; the week of history the server keeps fits in it. */
export const LIST_SYNC_RUNS_MAX = 1000;

/**
 * The slice of a list's sync history to show. A collapsed history is read one
 * run past what it shows, which is how it learns that older runs exist.
 */
export function shownListSyncRuns<T>(runs: readonly T[], expanded: boolean): { runs: T[]; more: boolean } {
  if (expanded) return { runs: [...runs], more: false };
  return { runs: runs.slice(0, LIST_SYNC_RUNS_SHOWN), more: runs.length > LIST_SYNC_RUNS_SHOWN };
}

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

/**
 * The next refresh of the one poll that follows every queued sync, however
 * many lists were queued. Each watched list keeps its own budget, counted from
 * when its sync was queued; lists past it are dropped. Null once none is left.
 */
export function listSyncWatchSchedule(
  attempt: number,
  startedAtById: ReadonlyMap<string, number>,
  now: number,
): { delay: number; keep: string[] } | null {
  let delay: number | null = null;
  const keep: string[] = [];
  for (const [id, startedAt] of startedAtById) {
    const next = listSyncPollDelayMs(attempt, now - startedAt);
    if (next === null) continue;
    delay = next;
    keep.push(id);
  }
  return delay === null ? null : { delay, keep };
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

/**
 * Live events after which a title's list memberships may read differently: a
 * public list adding the title, or a list dropping it.
 */
export function titleListMembershipChanged(titleId: string | null | undefined): DomainEventPredicate {
  return allOf(forTitle(titleId), forEventTypes("LIST_TITLE_ADDED", "LIST_TITLE_LEFT"));
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

/**
 * Undo one member's failed policy change without touching anyone else's row.
 * The row is put back only while it still shows the value that failed, so a
 * change that has since succeeded, for this member or another, is kept.
 */
export function rollBackMemberListPolicy(
  current: readonly MemberListPolicy[],
  previous: MemberListPolicy,
  attempted: MemberListPolicy["policy"],
): MemberListPolicy[] {
  return current.map((entry) =>
    entry.user.id === previous.user.id && entry.policy === attempted ? previous : entry,
  );
}
