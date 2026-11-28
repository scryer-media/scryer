import type { DeletePreview } from "../types/delete-preview.ts";
import type {
  MediaIssue,
  MediaIssueReason,
} from "../types/media-issues.ts";

// The ordinary delete preview only asks for typed confirmation above its size
// threshold. Recycle approvals always require the operator to type it.
/** The preview's reason when the reported file's only other name is its
 * download copy: the confirm then recycles both names, or is cancelled. */
export const MEDIA_ISSUE_HARD_LINKED_COPY_REASON =
  "MEDIA_RECYCLE_HARDLINKED_DOWNLOAD_COPY";

export function mediaIssueRecycleApprovalVariables(
  issue: Pick<MediaIssue, "id" | "status" | "remediation">,
  preview: Pick<DeletePreview, "fingerprint" | "mediaCount"> | null,
  typedConfirmation: string,
  canRecycle: boolean,
  hardLinkReason: string | null = null,
) {
  const linkedPair = hardLinkReason === MEDIA_ISSUE_HARD_LINKED_COPY_REASON;
  if (
    !canRecycle ||
    issue.status !== "PENDING" ||
    issue.remediation !== "RECYCLE_AND_SEARCH" ||
    // The reported file, and at most its copy in the download folder.
    !preview ||
    preview.mediaCount < 1 ||
    preview.mediaCount > 2 ||
    // A reason this client does not know is never confirmed.
    (hardLinkReason !== null && !linkedPair) ||
    (linkedPair && preview.mediaCount !== 2) ||
    !preview.fingerprint.trim() ||
    typedConfirmation.trim() !== "DELETE"
  ) {
    return null;
  }
  return {
    issueId: issue.id,
    previewFingerprint: preview.fingerprint,
    typedConfirmation: typedConfirmation.trim(),
    ...(linkedPair ? { recycleBothNames: true } : {}),
  };
}

export const MEDIA_ISSUE_REASONS = [
  "corrupted",
  "wrong_content",
  "wrong_language",
  "audio_out_of_sync",
  "poor_quality",
  "other",
] as const satisfies readonly MediaIssueReason[];

const mediaIssueReasonKeys = {
  corrupted: "mediaIssue.reason.corrupted",
  wrong_content: "mediaIssue.reason.wrongContent",
  wrong_language: "mediaIssue.reason.wrongLanguage",
  audio_out_of_sync: "mediaIssue.reason.audioOutOfSync",
  poor_quality: "mediaIssue.reason.poorQuality",
  other: "mediaIssue.reason.other",
} as const satisfies Record<MediaIssueReason, string>;

/** The label key for a report reason; an unknown reason reads as `other`. */
export function mediaIssueReasonKey(reason: string): string {
  return Object.hasOwn(mediaIssueReasonKeys, reason)
    ? mediaIssueReasonKeys[reason as MediaIssueReason]
    : mediaIssueReasonKeys.other;
}

const mediaIssueRefusalKeys: Readonly<Record<string, string>> = {
  MEDIA_REMEDIATION_NOT_AVAILABLE: "mediaIssue.remediationUnavailable",
  MEDIA_REGISTERED_MEDIA_PATH_CHANGED_BEFORE_INSPECTION:
    "mediaIssue.reportedFileChanged",
  MEDIA_SOURCE_CHANGED_DURING_INSPECTION: "mediaIssue.reportedFileChanged",
  MEDIA_RECYCLE_FAILED: "mediaIssue.recycleFailed",
  MEDIA_RECYCLE_HARDLINKED_FILE: "error.reason.MEDIA_RECYCLE_HARDLINKED_FILE",
  MEDIA_RECYCLE_HARDLINKED_DOWNLOAD_COPY:
    "error.reason.MEDIA_RECYCLE_HARDLINKED_DOWNLOAD_COPY",
  MEDIA_RECYCLE_UNSUPPORTED_PLATFORM:
    "error.reason.MEDIA_RECYCLE_UNSUPPORTED_PLATFORM",
  MEDIA_DETAILS_TOO_LONG: "error.reason.MEDIA_DETAILS_TOO_LONG",
  MEDIA_REASON_REQUIRES_DETAILS: "error.reason.MEDIA_REASON_REQUIRES_DETAILS",
  MEDIA_REASON_UNKNOWN: "error.reason.MEDIA_REASON_UNKNOWN",
};

/** The message key for a bad-file refusal code, or `undefined` for a code
 * this client does not know. */
export function mediaIssueRefusalKey(
  reason: string | null | undefined,
): string | undefined {
  return reason && Object.hasOwn(mediaIssueRefusalKeys, reason)
    ? mediaIssueRefusalKeys[reason]
    : undefined;
}
