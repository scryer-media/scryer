const REDACTED_HISTORY_SECRET = "[redacted]";
const HISTORY_API_KEY_QUERY_PARAM_PATTERN =
  /(\b(?:api_?key)=)([^&#\s"'<>),\]}]+)/gi;

function looksLikeHistorySecretKey(key: string): boolean {
  const normalized = key.replace(/[^a-zA-Z0-9]/g, "").toLowerCase();
  return normalized === "apikey" || normalized.endsWith("apikey");
}

function sanitizeHistoryValue(value: unknown, key?: string): unknown {
  if (typeof value === "string") {
    return key && looksLikeHistorySecretKey(key)
      ? REDACTED_HISTORY_SECRET
      : redactHistoryApiKeys(value);
  }

  if (Array.isArray(value)) {
    return value.map((entry) => sanitizeHistoryValue(entry));
  }

  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(
        ([entryKey, entryValue]) => [
          entryKey,
          sanitizeHistoryValue(entryValue, entryKey),
        ],
      ),
    );
  }

  return value;
}

export function redactHistoryApiKeys(value: string): string {
  return value.replace(
    HISTORY_API_KEY_QUERY_PARAM_PATTERN,
    `$1${REDACTED_HISTORY_SECRET}`,
  );
}

export function formatSanitizedHistoryValue(
  value: unknown,
  key?: string,
): string {
  if (value === null || value === undefined) return "\u2014";
  if (typeof value === "string") {
    return key && looksLikeHistorySecretKey(key)
      ? REDACTED_HISTORY_SECRET
      : redactHistoryApiKeys(value);
  }
  if (typeof value === "number") return String(value);
  if (typeof value === "boolean") return value ? "Yes" : "No";
  return JSON.stringify(sanitizeHistoryValue(value, key));
}
