/**
 * Pure helpers behind the operator dashboard.
 *
 * Everything here is presentation-free: the view layer turns a `UsageTone` into
 * app tokens and an i18n key, so the thresholds and the sorting rules stay in
 * one testable place instead of being re-derived per panel.
 */

export type UsageTag = "none" | "low" | "crit";

/**
 * Severity of a "how full is it" percentage.
 *
 * `tone` names one of the app's existing semantic colour families rather than a
 * literal colour, so quota bars and storage rings tint themselves from the same
 * `--scry-success|warning|danger-*` tokens the rest of the app uses.
 *
 * The design's ramp has four steps but the app has three semantic families, so
 * 65-79% and 80-89% share the `warning` family and are told apart by the `low`
 * tag; `crit` marks the 90%+ step that also turns the free-space figure red.
 */
export type UsageTone = {
  tone: "success" | "warning" | "danger";
  tag: UsageTag;
};

/**
 * The single usage ramp. Quota bars and storage rings both call this — do not
 * fork the thresholds per panel.
 */
export function usageTone(percent: number): UsageTone {
  if (!Number.isFinite(percent)) {
    return { tone: "success", tag: "none" };
  }
  if (percent >= 90) {
    return { tone: "danger", tag: "crit" };
  }
  if (percent >= 80) {
    return { tone: "warning", tag: "low" };
  }
  if (percent >= 65) {
    return { tone: "warning", tag: "none" };
  }
  return { tone: "success", tag: "none" };
}

/**
 * Percentage of `total` used by `used`, or null when either side is missing or
 * the total is zero. Null means "unknown", which callers must render as an
 * unavailable state rather than as 0%.
 */
export function usagePercent(
  used: number | null | undefined,
  total: number | null | undefined,
): number | null {
  if (
    used === null ||
    used === undefined ||
    total === null ||
    total === undefined ||
    !Number.isFinite(used) ||
    !Number.isFinite(total) ||
    total <= 0 ||
    used < 0
  ) {
    return null;
  }
  return Math.min(100, (used / total) * 100);
}

/** Terabytes with one decimal, for figures that must fit a narrow tile. */
export function formatTerabytes(bytes: number | null | undefined): string | null {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes) || bytes < 0) {
    return null;
  }
  return (bytes / 1_000_000_000_000).toFixed(1);
}

export type FormattedStorageSize = {
  value: string;
  unit: "GB" | "TB";
};

export function formatStorageSize(
  bytes: number | null | undefined,
): FormattedStorageSize | null {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes) || bytes < 0) {
    return null;
  }
  if (bytes < 1_000_000_000_000) {
    return { value: (bytes / 1_000_000_000).toFixed(1), unit: "GB" };
  }
  return { value: (bytes / 1_000_000_000_000).toFixed(1), unit: "TB" };
}

/**
 * Compact age of an instant: `41m`, `6h`, `3d`. Used where a row has room for a
 * couple of characters only; the full timestamp goes in the element's title.
 *
 * Returns null for missing or unparseable input so callers render an em dash
 * instead of a bogus age. Future instants clamp to the "now" bucket.
 */
export function formatCompactAge(
  isoDate: string | null | undefined,
  nowMs: number = Date.now(),
): string | null {
  if (!isoDate) {
    return null;
  }
  const parsed = Date.parse(isoDate);
  if (Number.isNaN(parsed)) {
    return null;
  }
  const elapsedMs = nowMs - parsed;
  if (elapsedMs < 60_000) {
    return "now";
  }
  const minutes = Math.floor(elapsedMs / 60_000);
  if (minutes < 60) {
    return `${minutes}m`;
  }
  const hours = Math.floor(elapsedMs / 3_600_000);
  if (hours < 24) {
    return `${hours}h`;
  }
  return `${Math.floor(elapsedMs / 86_400_000)}d`;
}

/**
 * Compact time until an instant: `now`, `in 41m`, `in 6h`, `in 3d` — the
 * forward-looking twin of {@link formatCompactAge}, phrased the same way as the
 * settings tables so one string can describe a wait in either place.
 */
export function formatCompactCountdown(
  isoDate: string | null | undefined,
  nowMs: number = Date.now(),
): string | null {
  if (!isoDate) {
    return null;
  }
  const parsed = Date.parse(isoDate);
  if (Number.isNaN(parsed)) {
    return null;
  }
  const remainingMs = parsed - nowMs;
  if (remainingMs < 60_000) {
    return "now";
  }
  const minutes = Math.floor(remainingMs / 60_000);
  if (minutes < 60) {
    return `in ${minutes}m`;
  }
  const hours = Math.floor(remainingMs / 3_600_000);
  if (hours < 24) {
    return `in ${hours}h`;
  }
  return `in ${Math.floor(remainingMs / 86_400_000)}d`;
}

/**
 * True when moving from `from` to `to` crosses a major version, which is the
 * only plugin update the strip flags as breaking. Unparseable versions are
 * treated as non-breaking: a bad version string is not evidence of a break.
 */
export function isBreakingVersionChange(
  from: string | null | undefined,
  to: string | null | undefined,
): boolean {
  const fromMajor = majorVersion(from);
  const toMajor = majorVersion(to);
  if (fromMajor === null || toMajor === null) {
    return false;
  }
  return toMajor > fromMajor;
}

function majorVersion(value: string | null | undefined): number | null {
  const match = /^\s*v?(\d+)/.exec(value ?? "");
  if (!match) {
    return null;
  }
  const major = Number.parseInt(match[1], 10);
  return Number.isFinite(major) ? major : null;
}

export type StorageRootUsage = {
  path: string;
  libraryId: string;
  libraryName: string;
  facet: string;
  usedBytes: number | null;
  totalBytes: number | null;
};

export type StorageLibraryGroup<T extends StorageRootUsage = StorageRootUsage> = {
  libraryId: string;
  libraryName: string;
  facet: string;
  roots: T[];
};

/**
 * Group roots by library, order libraries by their worst root descending, and
 * order roots inside a library the same way. Roots whose usage is unknown sort
 * last within their library and count as "no pressure" when ranking libraries,
 * so an uninspectable mount never masquerades as the fullest one.
 */
export function groupStorageRootsByLibrary<T extends StorageRootUsage>(
  roots: readonly T[],
): StorageLibraryGroup<T>[] {
  const groups = new Map<string, StorageLibraryGroup<T>>();
  for (const root of roots) {
    const group = groups.get(root.libraryId);
    if (group) {
      group.roots.push(root);
      continue;
    }
    groups.set(root.libraryId, {
      libraryId: root.libraryId,
      libraryName: root.libraryName,
      facet: root.facet,
      roots: [root],
    });
  }

  for (const group of groups.values()) {
    group.roots.sort(compareRootsByUsageDesc);
  }

  return Array.from(groups.values()).sort((left, right) => {
    const difference = worstRootPercent(right.roots) - worstRootPercent(left.roots);
    if (difference !== 0) {
      return difference;
    }
    return left.libraryName.localeCompare(right.libraryName);
  });
}

function compareRootsByUsageDesc(
  left: StorageRootUsage,
  right: StorageRootUsage,
): number {
  const leftPercent = usagePercent(left.usedBytes, left.totalBytes);
  const rightPercent = usagePercent(right.usedBytes, right.totalBytes);
  if (leftPercent === null && rightPercent === null) {
    return left.path.localeCompare(right.path);
  }
  if (leftPercent === null) {
    return 1;
  }
  if (rightPercent === null) {
    return -1;
  }
  if (leftPercent !== rightPercent) {
    return rightPercent - leftPercent;
  }
  return left.path.localeCompare(right.path);
}

function worstRootPercent(roots: readonly StorageRootUsage[]): number {
  let worst = -1;
  for (const root of roots) {
    const percent = usagePercent(root.usedBytes, root.totalBytes);
    if (percent !== null && percent > worst) {
      worst = percent;
    }
  }
  return worst;
}

export type IndexerHealthInput = {
  isEnabled: boolean;
  lastHealthStatus: string | null;
  lastErrorMessage: string | null;
  rateLimitedUntil?: string | null;
};

export type IndexerHealthSummary = {
  /** Enabled indexers with no recorded failure. */
  healthy: number;
  /** Enabled indexers, i.e. the denominator of "N/M healthy". */
  enabled: number;
  /** Enabled indexers currently reporting a failure. */
  erroring: number;
};

/**
 * Statuses that mean "this provider is currently failing".
 *
 * Indexers report health as `healthy`/`unhealthy` while download clients report
 * `error`/`failed`, so both vocabularies are accepted — the settings pages make
 * the same distinction, and treating only one of them as a failure would let a
 * broken client show as OK.
 */
const ERRORING_PROVIDER_STATUSES = new Set(["unhealthy", "error", "failed"]);

/** True when an enabled provider is currently reporting a failure. */
export function isProviderErroring(
  isEnabled: boolean,
  lastHealthStatus: string | null | undefined,
  lastError: string | null | undefined,
): boolean {
  if (!isEnabled) {
    return false;
  }
  const status = lastHealthStatus?.trim().toLowerCase();
  if (status && ERRORING_PROVIDER_STATUSES.has(status)) {
    return true;
  }
  return Boolean(lastError?.trim());
}

/**
 * True while an indexer is waiting out a rate-limit cooldown it will come out
 * of by itself.
 */
export function isProviderCoolingDown(
  rateLimitedUntil: string | null | undefined,
  now: Date = new Date(),
): boolean {
  if (!rateLimitedUntil) {
    return false;
  }
  const until = Date.parse(rateLimitedUntil);
  return Number.isFinite(until) && until > now.getTime();
}

/**
 * Header counts for the indexer panel. Disabled indexers are excluded from both
 * sides: they are not broken, they are switched off. A cooling indexer is
 * excluded from `erroring` for the same reason: the indexer asked for the
 * pause and it ends on its own.
 */
export function summarizeIndexerHealth(
  indexers: readonly IndexerHealthInput[],
): IndexerHealthSummary {
  let enabled = 0;
  let erroring = 0;
  for (const indexer of indexers) {
    if (!indexer.isEnabled) {
      continue;
    }
    enabled += 1;
    if (isProviderCoolingDown(indexer.rateLimitedUntil)) {
      continue;
    }
    if (
      isProviderErroring(
        indexer.isEnabled,
        indexer.lastHealthStatus,
        indexer.lastErrorMessage,
      )
    ) {
      erroring += 1;
    }
  }
  return { healthy: enabled - erroring, enabled, erroring };
}

export type ClientActivityCounts = {
  /** Items actively transferring or being handled right now. */
  active: number;
  /** Items the client is holding but not yet working on. */
  queued: number;
};

const ACTIVE_CLIENT_DISPLAY_STATES = new Set([
  "DOWNLOADING",
  "POST_PROCESSING",
  "IMPORTING",
  "REMOVING",
]);

const QUEUED_CLIENT_DISPLAY_STATES = new Set([
  "QUEUED",
  "PAUSED",
  "IMPORT_PENDING",
  "IMPORT_BLOCKED",
]);

/**
 * Fold the queue into per-client ACTIVE and QUEUE counts.
 *
 * Terminal states (completed, failed, ignored) are deliberately counted as
 * neither: the panel answers "is this client connected and working", so a row
 * of finished history must not inflate either column.
 */
export function aggregateClientActivity(
  items: readonly { clientId: string; displayState: string }[],
): Map<string, ClientActivityCounts> {
  const counts = new Map<string, ClientActivityCounts>();
  for (const item of items) {
    const clientId = item.clientId?.trim();
    if (!clientId) {
      continue;
    }
    const state = item.displayState?.trim().toUpperCase() ?? "";
    const bucket = counts.get(clientId) ?? { active: 0, queued: 0 };
    if (ACTIVE_CLIENT_DISPLAY_STATES.has(state)) {
      bucket.active += 1;
    } else if (QUEUED_CLIENT_DISPLAY_STATES.has(state)) {
      bucket.queued += 1;
    }
    counts.set(clientId, bucket);
  }
  return counts;
}

/**
 * How many things are waiting on the operator right now: the number the header
 * line leads with. Zero means the header prints the all-clear instead.
 */
export type ProviderSortEntry = {
  needsAttention: boolean;
  usage: number;
  name: string;
};

/**
 * Dashboard provider ordering: rows needing attention first, then the most
 * used descending, then name so equally quiet providers keep a stable order.
 */
export function compareProviderRows(
  left: ProviderSortEntry,
  right: ProviderSortEntry,
): number {
  if (left.needsAttention !== right.needsAttention) {
    return left.needsAttention ? -1 : 1;
  }
  if (left.usage !== right.usage) {
    return right.usage - left.usage;
  }
  return left.name.localeCompare(right.name);
}

export function attentionTotal(counts: {
  requests: number;
  imports: number;
  pluginUpdates: number;
  indexerErrors: number;
}): number {
  return (
    Math.max(0, counts.requests) +
    Math.max(0, counts.imports) +
    Math.max(0, counts.pluginUpdates) +
    Math.max(0, counts.indexerErrors)
  );
}
