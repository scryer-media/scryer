import type { ImportSkipReason } from "../types/import-history.ts";

/** Labels for the complete public import skip vocabulary. */
export const IMPORT_SKIP_REASON_KEYS = {
  ALREADY_IMPORTED: "importHistory.skipReason.ALREADY_IMPORTED",
  DUPLICATE_FILE: "importHistory.skipReason.DUPLICATE_FILE",
  POST_DOWNLOAD_RULE_BLOCKED:
    "importHistory.skipReason.POST_DOWNLOAD_RULE_BLOCKED",
  POLICY_MISMATCH: "importHistory.skipReason.POLICY_MISMATCH",
  UNRESOLVED_IDENTITY: "importHistory.skipReason.UNRESOLVED_IDENTITY",
  UNPARSEABLE_EPISODE: "importHistory.skipReason.UNPARSEABLE_EPISODE",
  NO_VIDEO_FILES: "importHistory.skipReason.NO_VIDEO_FILES",
  UNWANTED_EXECUTABLES: "importHistory.skipReason.UNWANTED_EXECUTABLES",
  DOWNLOAD_IN_PROGRESS: "importHistory.skipReason.DOWNLOAD_IN_PROGRESS",
  SOURCE_CHANGED: "importHistory.skipReason.SOURCE_CHANGED",
  SCENE_SUBTITLE_PENDING: "importHistory.skipReason.SCENE_SUBTITLE_PENDING",
  SOURCE_UNAVAILABLE: "importHistory.skipReason.SOURCE_UNAVAILABLE",
  DISK_FULL: "importHistory.skipReason.DISK_FULL",
  PERMISSION_DENIED: "importHistory.skipReason.PERMISSION_DENIED",
  PASSWORD_REQUIRED: "importHistory.skipReason.PASSWORD_REQUIRED",
  ARCHIVE_EXTRACTION_PLUGIN_REQUIRED:
    "importHistory.skipReason.ARCHIVE_EXTRACTION_PLUGIN_REQUIRED",
  ARCHIVE_EXTRACTION_TIMED_OUT:
    "importHistory.skipReason.ARCHIVE_EXTRACTION_TIMED_OUT",
} satisfies Record<ImportSkipReason, string>;

export function importSkipReasonLabelKey(reason: string): string | null {
  const code = reason.toUpperCase();
  return Object.hasOwn(IMPORT_SKIP_REASON_KEYS, code)
    ? IMPORT_SKIP_REASON_KEYS[code as ImportSkipReason]
    : null;
}
