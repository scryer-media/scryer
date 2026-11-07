import type { Translate } from "@/components/root/types";

/**
 * The server names why a media server's watch history was last read
 * incompletely: `watchSignalSync.lastErrorCode`, beside the English sentence
 * in `lastError`. Each code listed here is said in the viewer's language. A
 * code this client does not know, or a sweep recorded before codes were
 * stored (no code at all), shows the server's sentence instead.
 */
export const WATCH_SYNC_REASON_KEYS: Readonly<Record<string, string>> = {
  WATCH_SYNC_CREDENTIAL_MISSING: "settings.watchSyncReason.credentialMissing",
  WATCH_SYNC_PROVIDER_UNSUPPORTED:
    "settings.watchSyncReason.providerUnsupported",
  WATCH_SYNC_BASE_URL_INVALID: "settings.watchSyncReason.baseUrlInvalid",
  WATCH_SYNC_PLEX_SERVER_NOT_SELECTED:
    "settings.watchSyncReason.plexServerNotSelected",
  WATCH_SYNC_PLEX_SERVER_UNREACHABLE:
    "settings.watchSyncReason.plexServerUnreachable",
  WATCH_SYNC_PLEX_ACCOUNT_ID_INVALID:
    "settings.watchSyncReason.plexAccountIdInvalid",
  WATCH_SYNC_PARTICIPANT_ID_MISSING:
    "settings.watchSyncReason.participantIdMissing",
  WATCH_SYNC_ACCESS_DENIED: "settings.watchSyncReason.accessDenied",
  WATCH_SYNC_REQUEST_REJECTED: "settings.watchSyncReason.requestRejected",
  WATCH_SYNC_REQUEST_FAILED: "settings.watchSyncReason.requestFailed",
  WATCH_SYNC_RESPONSE_UNREADABLE: "settings.watchSyncReason.responseUnreadable",
  WATCH_SYNC_ITEMS_UNREADABLE: "settings.watchSyncReason.itemsUnreadable",
  WATCH_SYNC_PAGING_NOT_HONOURED: "settings.watchSyncReason.pagingNotHonoured",
  WATCH_SYNC_FILTER_NOT_HONOURED: "settings.watchSyncReason.filterNotHonoured",
  WATCH_SYNC_UNMAPPED_PLAYED_MOVIES:
    "settings.watchSyncReason.unmappedPlayedMovies",
  WATCH_SYNC_STORE_FAILED: "settings.watchSyncReason.storeFailed",
  WATCH_SYNC_SOURCE_UNAVAILABLE: "settings.watchSyncReason.sourceUnavailable",
};

/**
 * Why the last watch-history read was incomplete, for display: the translated
 * reason when the code is known, otherwise the server's sentence, and `null`
 * when the server gave neither.
 */
export function watchSyncReasonText(
  t: Translate,
  code: string | null | undefined,
  sentence: string | null | undefined,
): string | null {
  if (code && Object.hasOwn(WATCH_SYNC_REASON_KEYS, code)) {
    return t(WATCH_SYNC_REASON_KEYS[code]);
  }
  return sentence ?? null;
}
