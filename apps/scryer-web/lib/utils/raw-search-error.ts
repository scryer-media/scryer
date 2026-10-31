import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

/**
 * Why a query search could not start, continue or complete. The server sends
 * these as `failureReasonCode` on the search snapshot, or as
 * `extensions.reason` on a refused start or continue; a reason this client
 * does not know shows the server's sentence.
 */
export const RAW_SEARCH_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  RAW_SEARCH_RETAINED_STATE_FULL: "indexerSearch.error.capacity",
  RAW_SEARCH_CURSOR_REPEATED: "indexerSearch.error.cursor",
  RAW_SEARCH_CURSOR_INVALID: "indexerSearch.error.offset",
  RAW_SEARCH_OFFSET_INVALID: "indexerSearch.error.offset",
  RAW_SEARCH_CANCELLED: "indexerSearch.error.cancelled",
  RAW_SEARCH_HISTORY_EXPIRED: "indexerSearch.error.expired",
  RAW_SEARCH_ALL_SOURCES_LIMITED: "indexerSearch.error.limited",
  RAW_SEARCH_FAILED: "indexerSearch.error.unavailable",
  RAW_SEARCH_NO_SOURCE_SUCCEEDED: "indexerSearch.error.unavailable",
};

function refusalKey(reason: string | null | undefined): string | null {
  return reason && Object.hasOwn(RAW_SEARCH_REFUSAL_KEYS, reason)
    ? RAW_SEARCH_REFUSAL_KEYS[reason]!
    : null;
}

/** A failed snapshot's reason in the viewer's language, else the server's sentence, else the fallback. */
export function rawSearchFailureMessage(
  failure: { failureReasonCode?: string | null; failureReason?: string | null },
  t: (key: string) => string,
  fallback: string,
): string {
  const key = refusalKey(failure.failureReasonCode);
  return key ? t(key) : (failure.failureReason ?? fallback);
}

/** What to show when starting or continuing a query search is refused. */
export function rawSearchErrorMessage(
  error: unknown,
  t: (key: string) => string,
  fallback: string,
): string {
  const key = refusalKey(
    graphQlErrorExtension(error, "VALIDATION_ERROR", "reason") ??
      graphQlErrorExtension(error, "TEMPORARY_UNAVAILABLE", "reason"),
  );
  return key ? t(key) : userFacingGraphQlErrorMessage(error, fallback, t);
}
