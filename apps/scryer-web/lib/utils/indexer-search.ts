// Pure helpers behind the Indexers › Search pane (spec 0002, D5/D9/D10/D14).
// Facets, filters, sorting, health tones and saved searches are all derived in
// the browser from the snapshot the interactive-search job already returns, so
// every one of them is a plain function over `Release[]` that a test can drive.
import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import type { Release } from "@/lib/types";
import { indexerSearchResultRowId } from "./dom-ids.ts";

/** An indexer that answered slower than this reads as "slow" (D15). */
export const SLOW_INDEXER_MS = 1_000;

export type IndexerSearchSortKey =
  | "newest"
  | "size"
  | "age"
  | "seeders"
  | "priority";

export type IndexerSearchFacetGroupKey =
  | "protocol"
  | "resolution"
  | "source"
  | "audio"
  | "flags"
  | "indexer";

export type IndexerSearchFacetItem = {
  /** `<group>:<value>`; the identity the selection set stores. */
  key: string;
  value: string;
  /** Translation key for semantic values; absent when the value is scene vocabulary. */
  labelKey?: string;
  label: string;
  count: number;
};

export type IndexerSearchFacetGroup = {
  key: IndexerSearchFacetGroupKey;
  labelKey: string;
  items: IndexerSearchFacetItem[];
};

export type IndexerSearchFilters = {
  /** Selected facet keys; within a group they OR, across groups they AND. */
  facets: string[];
  minSizeGiB: number | null;
  maxSizeGiB: number | null;
  minSeeders: number | null;
  maxAgeDays: number | null;
  /** Refine-rail range, in GiB, over the bounds of the current result set. */
  sizeRangeGiB: [number, number] | null;
};

export const EMPTY_INDEXER_SEARCH_FILTERS: IndexerSearchFilters = {
  facets: [],
  minSizeGiB: null,
  maxSizeGiB: null,
  minSeeders: null,
  maxAgeDays: null,
  sizeRangeGiB: null,
};

const GIB = 1024 * 1024 * 1024;

export function indexerSearchRowKey(release: Release): string {
  return indexerSearchResultRowId(release);
}

export function releaseProtocol(release: Release): "usenet" | "torrent" | null {
  switch (release.sourceKind) {
    case "NZB_FILE":
    case "NZB_URL":
      return "usenet";
    case "TORRENT_FILE":
    case "MAGNET_URI":
      return "torrent";
    default:
      return null;
  }
}

/**
 * Whether the release has a file Scryer can hand the browser (D17). A magnet is
 * a pointer, not a file, and a release with no URL at all has nothing to fetch.
 */
export function isDownloadableRelease(release: Release): boolean {
  const url = (release.downloadUrl ?? release.link ?? "").trim();
  return /^https?:\/\//i.test(url);
}

/** The subset of `releases` a browser download can actually produce a file for. */
export function downloadableReleases(releases: Release[]): Release[] {
  return releases.filter((release) => isDownloadableRelease(release));
}

export function isReleaseRejected(release: Release): boolean {
  return (release.qualityProfileDecision?.blockCodes?.length ?? 0) > 0;
}

export function releaseBlockCode(release: Release): string | null {
  return release.qualityProfileDecision?.blockCodes?.[0] ?? null;
}

/**
 * Coarse resolution bucket. The parser reports the raw `<height>p` token, which
 * would scatter the facet across a dozen one-hit values; the operator only ever
 * refines by the four buckets the handoff names.
 */
export function resolutionBucket(
  quality: string | null | undefined,
): string | null {
  const match = /^(\d{3,4})p$/i.exec(quality?.trim() ?? "");
  if (!match) {
    return null;
  }
  const height = Number(match[1]);
  if (height >= 2160) return "2160p";
  if (height >= 1080) return "1080p";
  if (height >= 720) return "720p";
  return "SD";
}

function facetKey(group: IndexerSearchFacetGroupKey, value: string): string {
  return `${group}:${value}`;
}

/** Every facet key one release carries; the basis for both counts and filtering. */
export function releaseFacetKeys(release: Release): string[] {
  const keys: string[] = [];
  const protocol = releaseProtocol(release);
  if (protocol) {
    keys.push(facetKey("protocol", protocol));
  }
  const parsed = release.parsedRelease;
  const resolution = resolutionBucket(parsed?.quality);
  if (resolution) {
    keys.push(facetKey("resolution", resolution));
  }
  const source = parsed?.isRemux ? "REMUX" : parsed?.source?.trim();
  if (source) {
    keys.push(facetKey("source", source));
  }
  if (parsed?.isAtmos) keys.push(facetKey("audio", "atmos"));
  if (parsed?.isDolbyVision) keys.push(facetKey("audio", "dolbyVision"));
  if (parsed?.detectedHdr) keys.push(facetKey("audio", "hdr"));
  if (release.freeleech === true) keys.push(facetKey("flags", "freeleech"));
  if (parsed?.isProperUpload) keys.push(facetKey("flags", "proper"));
  const indexer = release.source?.trim();
  if (indexer) {
    keys.push(facetKey("indexer", indexer));
  }
  return keys;
}

const SEMANTIC_FACET_LABEL_KEYS: Record<string, string> = {
  "protocol:usenet": "indexerSearch.facet.usenet",
  "protocol:torrent": "indexerSearch.facet.torrent",
  "audio:atmos": "indexerSearch.facet.atmos",
  "audio:dolbyVision": "indexerSearch.facet.dolbyVision",
  "audio:hdr": "indexerSearch.facet.hdr",
  "flags:freeleech": "indexerSearch.facet.freeleech",
  "flags:proper": "indexerSearch.facet.proper",
};

const FACET_GROUPS: { key: IndexerSearchFacetGroupKey; labelKey: string }[] = [
  { key: "protocol", labelKey: "indexerSearch.facet.protocol" },
  { key: "resolution", labelKey: "indexerSearch.facet.resolution" },
  { key: "source", labelKey: "indexerSearch.facet.source" },
  { key: "audio", labelKey: "indexerSearch.facet.audio" },
  { key: "flags", labelKey: "indexerSearch.facet.flags" },
  { key: "indexer", labelKey: "indexerSearch.facet.indexer" },
];

const RESOLUTION_ORDER = ["2160p", "1080p", "720p", "SD"];

/**
 * Facet groups with counts over the **full** result set, so ticking one facet
 * never moves another's count within a search (handoff §6).
 */
export function buildIndexerSearchFacets(
  releases: Release[],
): IndexerSearchFacetGroup[] {
  const counts = new Map<string, number>();
  for (const release of releases) {
    for (const key of releaseFacetKeys(release)) {
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
  }

  return FACET_GROUPS.map(({ key: groupKey, labelKey }) => {
    const items: IndexerSearchFacetItem[] = [];
    for (const [key, count] of counts) {
      const [group, ...rest] = key.split(":");
      if (group !== groupKey) {
        continue;
      }
      const value = rest.join(":");
      items.push({
        key,
        value,
        labelKey: SEMANTIC_FACET_LABEL_KEYS[key],
        label: value,
        count,
      });
    }
    items.sort((left, right) => {
      if (groupKey === "resolution") {
        return (
          RESOLUTION_ORDER.indexOf(left.value) -
          RESOLUTION_ORDER.indexOf(right.value)
        );
      }
      if (left.count !== right.count) {
        return right.count - left.count;
      }
      return left.value.localeCompare(right.value);
    });
    return { key: groupKey, labelKey, items };
  }).filter((group) => group.items.length > 0);
}

/** Size bounds of the result set in GiB, rounded outwards; null when unknown. */
export function releaseSizeBoundsGiB(
  releases: Release[],
): [number, number] | null {
  let min = Number.POSITIVE_INFINITY;
  let max = 0;
  for (const release of releases) {
    const bytes = release.sizeBytes;
    if (bytes == null || bytes <= 0) {
      continue;
    }
    const gib = bytes / GIB;
    if (gib < min) min = gib;
    if (gib > max) max = gib;
  }
  if (!Number.isFinite(min) || max <= 0) {
    return null;
  }
  return [Math.floor(min * 10) / 10, Math.ceil(max * 10) / 10];
}

export function releaseAgeMs(release: Release, nowMs: number): number | null {
  if (!release.publishedAt) {
    return null;
  }
  const published = Date.parse(release.publishedAt);
  if (Number.isNaN(published)) {
    return null;
  }
  return Math.max(0, nowMs - published);
}

export type ReleaseAge = {
  unitKey: string;
  value: number;
};

/** Coarse age, as the handoff prints it: `4 h`, `12 d`, `6 mo`, `2 y`. */
export function formatReleaseAge(ageMs: number | null): ReleaseAge | null {
  if (ageMs == null) {
    return null;
  }
  const hours = Math.floor(ageMs / 3_600_000);
  if (hours < 24) {
    return { unitKey: "indexerSearch.age.hours", value: hours };
  }
  const days = Math.floor(hours / 24);
  if (days < 60) {
    return { unitKey: "indexerSearch.age.days", value: days };
  }
  const months = Math.floor(days / 30);
  if (months < 24) {
    return { unitKey: "indexerSearch.age.months", value: months };
  }
  return { unitKey: "indexerSearch.age.years", value: Math.floor(days / 365) };
}

/** Binary size, one decimal from GiB up, as the results table prints it. */
export function formatReleaseSize(bytes: number | null | undefined): string {
  if (bytes == null || bytes <= 0) {
    return "—";
  }
  if (bytes >= GIB) {
    return `${(bytes / GIB).toFixed(1)} GiB`;
  }
  if (bytes >= 1024 * 1024) {
    return `${Math.round(bytes / (1024 * 1024))} MiB`;
  }
  return `${Math.max(1, Math.round(bytes / 1024))} KiB`;
}

export function totalReleaseBytes(releases: Release[]): number {
  return releases.reduce((total, release) => total + (release.sizeBytes ?? 0), 0);
}

export function filterIndexerSearchReleases(
  releases: Release[],
  filters: IndexerSearchFilters,
  nowMs: number,
): Release[] {
  const selectedByGroup = new Map<string, Set<string>>();
  for (const key of filters.facets) {
    const group = key.split(":")[0] ?? "";
    const bucket = selectedByGroup.get(group) ?? new Set<string>();
    bucket.add(key);
    selectedByGroup.set(group, bucket);
  }

  return releases.filter((release) => {
    const keys = new Set(releaseFacetKeys(release));
    for (const selected of selectedByGroup.values()) {
      let matched = false;
      for (const key of selected) {
        if (keys.has(key)) {
          matched = true;
          break;
        }
      }
      if (!matched) {
        return false;
      }
    }

    const bytes = release.sizeBytes ?? null;
    if (bytes != null) {
      const gib = bytes / GIB;
      if (filters.minSizeGiB != null && gib < filters.minSizeGiB) return false;
      if (filters.maxSizeGiB != null && gib > filters.maxSizeGiB) return false;
      if (filters.sizeRangeGiB) {
        const [low, high] = filters.sizeRangeGiB;
        if (gib < low || gib > high) return false;
      }
    }

    // A seeder floor is a torrent question; usenet results report no swarm and
    // are never excluded by it.
    if (
      filters.minSeeders != null &&
      release.seeders != null &&
      release.seeders < filters.minSeeders
    ) {
      return false;
    }

    if (filters.maxAgeDays != null) {
      const ageMs = releaseAgeMs(release, nowMs);
      if (ageMs != null && ageMs > filters.maxAgeDays * 86_400_000) {
        return false;
      }
    }

    return true;
  });
}

function publishedAtMs(release: Release): number {
  if (!release.publishedAt) {
    return 0;
  }
  const parsed = Date.parse(release.publishedAt);
  return Number.isNaN(parsed) ? 0 : parsed;
}

export function sortIndexerSearchReleases(
  releases: Release[],
  sortKey: IndexerSearchSortKey,
  priorityByIndexer: ReadonlyMap<string, number>,
): Release[] {
  const sorted = [...releases];
  sorted.sort((left, right) => {
    switch (sortKey) {
      case "size":
        return (right.sizeBytes ?? 0) - (left.sizeBytes ?? 0);
      case "age":
        return publishedAtMs(left) - publishedAtMs(right);
      case "seeders":
        return (right.seeders ?? -1) - (left.seeders ?? -1);
      case "priority": {
        const leftPriority =
          priorityByIndexer.get(left.source ?? "") ?? Number.MAX_SAFE_INTEGER;
        const rightPriority =
          priorityByIndexer.get(right.source ?? "") ?? Number.MAX_SAFE_INTEGER;
        if (leftPriority !== rightPriority) {
          return leftPriority - rightPriority;
        }
        return publishedAtMs(right) - publishedAtMs(left);
      }
      case "newest":
      default:
        return publishedAtMs(right) - publishedAtMs(left);
    }
  });
  return sorted;
}

/**
 * Retry-failed merge (D9): rows the retry returned replace their earlier
 * selves, rows it did not return survive, and new rows land at the end so the
 * table does not reshuffle under the operator.
 */
export function mergeIndexerSearchReleases(
  base: Release[],
  incoming: Release[],
): Release[] {
  const incomingByKey = new Map(
    incoming.map((release) => [indexerSearchRowKey(release), release]),
  );
  const merged = base.map((release) => {
    const key = indexerSearchRowKey(release);
    const replacement = incomingByKey.get(key);
    if (replacement) {
      incomingByKey.delete(key);
      return replacement;
    }
    return release;
  });
  return [...merged, ...incomingByKey.values()];
}

export function mergeIndexerProgress(
  base: InteractiveSearchIndexerProgress[],
  incoming: InteractiveSearchIndexerProgress[],
): InteractiveSearchIndexerProgress[] {
  const incomingById = new Map(
    incoming.map((entry) => [entry.indexerId, entry]),
  );
  const merged = base.map((entry) => {
    const replacement = incomingById.get(entry.indexerId);
    if (replacement) {
      incomingById.delete(entry.indexerId);
      return replacement;
    }
    return entry;
  });
  return [...merged, ...incomingById.values()];
}

export type IndexerHealthTone = "pending" | "ok" | "slow" | "failed" | "skipped";

export function indexerHealthTone(
  entry: InteractiveSearchIndexerProgress,
): IndexerHealthTone {
  switch (entry.status) {
    case "FAILED":
      return "failed";
    case "SKIPPED":
      return "skipped";
    case "COMPLETED":
      // There is no SLOW status on the wire; slow is a reading of elapsed time.
      return (entry.elapsedMs ?? 0) > SLOW_INDEXER_MS ? "slow" : "ok";
    default:
      return "pending";
  }
}

export type IndexerSearchHealth = {
  total: number;
  answered: number;
  pending: number;
  failedIndexerIds: string[];
  /** Slowest indexer call; with a concurrent fan-out that is the wall time. */
  elapsedMs: number;
};

export function summarizeIndexerHealth(
  indexers: InteractiveSearchIndexerProgress[],
): IndexerSearchHealth {
  let answered = 0;
  let pending = 0;
  let elapsedMs = 0;
  const failedIndexerIds: string[] = [];
  for (const entry of indexers) {
    if (entry.status === "PENDING" || entry.status === "SEARCHING") {
      pending += 1;
    } else {
      answered += 1;
    }
    if (entry.status === "FAILED") {
      failedIndexerIds.push(entry.indexerId);
    }
    elapsedMs = Math.max(elapsedMs, entry.elapsedMs ?? 0);
  }
  return {
    total: indexers.length,
    answered,
    pending,
    failedIndexerIds,
    elapsedMs,
  };
}

export function indexerPriorityByName(
  indexers: InteractiveSearchIndexerProgress[],
): Map<string, number> {
  return new Map(indexers.map((entry) => [entry.name, entry.priority]));
}

/** Newznab categories are typed as a comma- or space-separated numeric list. */
export function parseCategoryList(raw: string): string[] {
  return [
    ...new Set(
      raw
        .split(/[\s,]+/)
        .map((value) => value.trim())
        .filter((value) => /^\d+$/.test(value)),
    ),
  ];
}

export type SavedIndexerSearch = {
  query: string;
  kind: string;
  indexerIds: string[];
  categories: string[];
};

export const SAVED_INDEXER_SEARCHES_KEY = "scryer.indexer-search.saved";
export const MAX_SAVED_INDEXER_SEARCHES = 20;

export function parseSavedIndexerSearches(
  raw: string | null | undefined,
): SavedIndexerSearch[] {
  if (!raw) {
    return [];
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) {
    return [];
  }
  const entries: SavedIndexerSearch[] = [];
  for (const candidate of parsed) {
    if (typeof candidate !== "object" || candidate === null) {
      continue;
    }
    const record = candidate as Record<string, unknown>;
    const query = typeof record.query === "string" ? record.query.trim() : "";
    const kind = typeof record.kind === "string" ? record.kind : "";
    if (!query || !kind) {
      continue;
    }
    entries.push({
      query,
      kind,
      indexerIds: Array.isArray(record.indexerIds)
        ? record.indexerIds.filter(
            (value): value is string => typeof value === "string",
          )
        : [],
      categories: Array.isArray(record.categories)
        ? record.categories.filter(
            (value): value is string => typeof value === "string",
          )
        : [],
    });
    if (entries.length >= MAX_SAVED_INDEXER_SEARCHES) {
      break;
    }
  }
  return entries;
}

/** Newest first, one entry per (query, kind), capped at 20 (D10). */
export function addSavedIndexerSearch(
  saved: SavedIndexerSearch[],
  entry: SavedIndexerSearch,
): SavedIndexerSearch[] {
  const query = entry.query.trim();
  if (!query) {
    return saved;
  }
  const next = saved.filter(
    (candidate) => candidate.query !== query || candidate.kind !== entry.kind,
  );
  return [{ ...entry, query }, ...next].slice(0, MAX_SAVED_INDEXER_SEARCHES);
}

export function readSavedIndexerSearches(): SavedIndexerSearch[] {
  if (typeof window === "undefined") {
    return [];
  }
  try {
    return parseSavedIndexerSearches(
      window.localStorage.getItem(SAVED_INDEXER_SEARCHES_KEY),
    );
  } catch {
    // Private-mode or storage-disabled browsers: saved searches are a
    // convenience, never a precondition for searching.
    return [];
  }
}

export function writeSavedIndexerSearches(saved: SavedIndexerSearch[]): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(
      SAVED_INDEXER_SEARCHES_KEY,
      JSON.stringify(saved),
    );
  } catch {
    // Ignored for the same reason as the read above.
  }
}
