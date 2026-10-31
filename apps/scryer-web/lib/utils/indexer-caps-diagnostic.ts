import type { IndexerCapsSource } from "../graphql/release-search.ts";

const SOURCE_KEYS: Readonly<Record<IndexerCapsSource, string>> = {
  FRESH: "indexerSearch.caps.fresh",
  MANAGED_METADATA: "indexerSearch.caps.managed",
  REFRESHED: "indexerSearch.caps.refreshed",
  STALE: "indexerSearch.caps.stale",
  PERMISSIVE: "indexerSearch.caps.permissive",
  PROVIDER_DECLARATIONS: "indexerSearch.caps.provider",
};

const REASON_KEYS: Readonly<Record<string, string>> = {
  INDEXER_CAPS_REFRESH_FAILED: "indexerSearch.caps.refreshFailed",
  INDEXER_CAPS_REFRESH_DEFERRED: "indexerSearch.caps.refreshDeferred",
};

export function indexerCapsDiagnostic(
  entry: {
    capsSource?: string | null;
    capsReason?: string | null;
    capsRetryAfterSeconds?: number | null;
  },
  t: (key: string, values?: Record<string, string | number>) => string,
): string | undefined {
  if (!entry.capsSource) return undefined;
  const parts = [
    t(
      Object.hasOwn(SOURCE_KEYS, entry.capsSource)
        ? SOURCE_KEYS[entry.capsSource as IndexerCapsSource]
        : "indexerSearch.caps.unknown",
    ),
  ];
  if (entry.capsReason) {
    parts.push(
      t(
        Object.hasOwn(REASON_KEYS, entry.capsReason)
          ? REASON_KEYS[entry.capsReason]!
          : "indexerSearch.caps.refreshUnavailable",
      ),
    );
  }
  if (entry.capsRetryAfterSeconds != null && entry.capsRetryAfterSeconds >= 0) {
    parts.push(
      t("indexerSearch.caps.retryAfter", {
        seconds: entry.capsRetryAfterSeconds,
      }),
    );
  }
  return parts.join(" · ");
}
