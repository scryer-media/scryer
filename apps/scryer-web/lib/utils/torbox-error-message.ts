/**
 * Conditions the TorBox download client reports, as a refused request or as
 * the attention reason of a queued or failed download. Each is said in the
 * viewer's language; a reason this client does not know shows the server's
 * sentence.
 */
export const TORBOX_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  TORBOX_TOKEN_REJECTED: "error.reason.TORBOX_TOKEN_REJECTED",
  TORBOX_PLAN_RESTRICTED: "error.reason.TORBOX_PLAN_RESTRICTED",
  TORBOX_ACTIVE_LIMIT: "error.reason.TORBOX_ACTIVE_LIMIT",
  TORBOX_COOLDOWN_LIMIT: "error.reason.TORBOX_COOLDOWN_LIMIT",
  TORBOX_MONTHLY_LIMIT: "error.reason.TORBOX_MONTHLY_LIMIT",
  TORBOX_HOURLY_LIMIT: "error.reason.TORBOX_HOURLY_LIMIT",
  TORBOX_DOWNLOAD_TOO_LARGE: "error.reason.TORBOX_DOWNLOAD_TOO_LARGE",
  TORBOX_NOT_CACHED: "error.reason.TORBOX_NOT_CACHED",
  TORBOX_INVALID_TORRENT: "error.reason.TORBOX_INVALID_TORRENT",
  TORBOX_INVALID_NZB: "error.reason.TORBOX_INVALID_NZB",
  TORBOX_DUPLICATE_ITEM: "error.reason.TORBOX_DUPLICATE_ITEM",
  TORBOX_ITEM_NOT_FOUND: "error.reason.TORBOX_ITEM_NOT_FOUND",
  TORBOX_LINK_OFFLINE: "error.reason.TORBOX_LINK_OFFLINE",
  TORBOX_RATE_LIMITED: "error.reason.TORBOX_RATE_LIMITED",
  TORBOX_UNAVAILABLE: "error.reason.TORBOX_UNAVAILABLE",
  TORBOX_FILE_INFECTED: "error.reason.TORBOX_FILE_INFECTED",
  TORBOX_FILE_ZIPPED: "error.reason.TORBOX_FILE_ZIPPED",
  TORBOX_NO_VIDEO_FILE: "error.reason.TORBOX_NO_VIDEO_FILE",
  TORBOX_REMOTE_FAILED: "error.reason.TORBOX_REMOTE_FAILED",
  TORBOX_ITEM_MISSING: "error.reason.TORBOX_ITEM_MISSING",
  TORBOX_STALLED: "error.reason.TORBOX_STALLED",
  TORBOX_FILE_PATHS_UNUSABLE: "error.reason.TORBOX_FILE_PATHS_UNUSABLE",
  TORBOX_SOURCE_MISSING: "error.reason.TORBOX_SOURCE_MISSING",
  TORBOX_ACTION_UNSUPPORTED: "error.reason.TORBOX_ACTION_UNSUPPORTED",
  TORBOX_JOB_NOT_FOUND: "error.reason.TORBOX_JOB_NOT_FOUND",
  TORBOX_TOKEN_REQUIRED: "error.reason.TORBOX_TOKEN_REQUIRED",
};

/**
 * The GraphQL error code each TorBox reason arrives with. A reason the server
 * only records on a download (never raises) is listed with the validation
 * code, which is what it would carry if it were raised.
 */
export const TORBOX_REFUSAL_CODES: Readonly<Record<string, string>> = {
  ...Object.fromEntries(
    Object.keys(TORBOX_REFUSAL_KEYS).map((reason) => [
      reason,
      "VALIDATION_ERROR",
    ]),
  ),
  TORBOX_ACTIVE_LIMIT: "TEMPORARY_UNAVAILABLE",
  TORBOX_COOLDOWN_LIMIT: "TEMPORARY_UNAVAILABLE",
  TORBOX_HOURLY_LIMIT: "TEMPORARY_UNAVAILABLE",
  TORBOX_RATE_LIMITED: "TEMPORARY_UNAVAILABLE",
  TORBOX_UNAVAILABLE: "TEMPORARY_UNAVAILABLE",
  TORBOX_ITEM_NOT_FOUND: "NOT_FOUND",
  TORBOX_JOB_NOT_FOUND: "NOT_FOUND",
};
