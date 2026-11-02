import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";

/** Server reason codes for releases no available download client accepts. */
const REASON_KEYS: Readonly<Record<string, string>> = {
  RELEASE_SEARCH_NO_USENET_CLIENT:
    "title.contextReleaseSearchUnroutable.noUsenetClient",
  RELEASE_SEARCH_NO_TORRENT_CLIENT:
    "title.contextReleaseSearchUnroutable.noTorrentClient",
  RELEASE_SEARCH_NO_HTTP_CLIENT:
    "title.contextReleaseSearchUnroutable.noHttpClient",
  RELEASE_SEARCH_USENET_CLIENT_PLUGIN_UNAVAILABLE:
    "title.contextReleaseSearchUnroutable.usenetClientPluginUnavailable",
  RELEASE_SEARCH_TORRENT_CLIENT_PLUGIN_UNAVAILABLE:
    "title.contextReleaseSearchUnroutable.torrentClientPluginUnavailable",
  RELEASE_SEARCH_HTTP_CLIENT_PLUGIN_UNAVAILABLE:
    "title.contextReleaseSearchUnroutable.httpClientPluginUnavailable",
};

const UNKNOWN_REASON_KEY = "title.contextReleaseSearchUnroutable.unknown";

const REASON_ORDER = Object.keys(REASON_KEYS);

/** Releases a title search set aside for one reason, across every indexer. */
export type UnroutableResultsSummary = {
  reason: string;
  count: number;
  indexerNames: string[];
};

/** The i18n key for a reason code; an unknown code gets a generic line. */
export function unroutableResultsKey(reason: string): string {
  return Object.hasOwn(REASON_KEYS, reason)
    ? REASON_KEYS[reason]!
    : UNKNOWN_REASON_KEY;
}

/**
 * Totals the releases each indexer had removed, per reason, in a stable
 * order: known reasons first, then unknown codes alphabetically.
 */
export function summarizeUnroutableResults(
  indexers: InteractiveSearchIndexerProgress[],
): UnroutableResultsSummary[] {
  const byReason = new Map<string, UnroutableResultsSummary>();
  for (const indexer of indexers) {
    for (const entry of indexer.unroutableResults ?? []) {
      if (entry.count <= 0) continue;
      const summary = byReason.get(entry.reason) ?? {
        reason: entry.reason,
        count: 0,
        indexerNames: [],
      };
      summary.count += entry.count;
      if (!summary.indexerNames.includes(indexer.name)) {
        summary.indexerNames.push(indexer.name);
      }
      byReason.set(entry.reason, summary);
    }
  }
  const rank = (reason: string) => {
    const index = REASON_ORDER.indexOf(reason);
    return index === -1 ? REASON_ORDER.length : index;
  };
  return [...byReason.values()].sort(
    (left, right) =>
      rank(left.reason) - rank(right.reason) ||
      left.reason.localeCompare(right.reason),
  );
}
