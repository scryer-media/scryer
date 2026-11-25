import type { DeletePreview } from "../types/delete-preview.ts";
import type { MediaIssue } from "../types/media-issues.ts";

// The ordinary delete preview only asks for typed confirmation above its size
// threshold. Recycle approvals always require the operator to type it.
export function mediaIssueRecycleApprovalVariables(
  issue: Pick<MediaIssue, "id" | "status" | "remediation">,
  preview: Pick<DeletePreview, "fingerprint" | "mediaCount"> | null,
  typedConfirmation: string,
  canRecycle: boolean,
) {
  if (
    !canRecycle ||
    issue.status !== "PENDING" ||
    issue.remediation !== "RECYCLE_AND_SEARCH" ||
    preview?.mediaCount !== 1 ||
    !preview.fingerprint.trim() ||
    typedConfirmation.trim() !== "DELETE"
  ) {
    return null;
  }
  return {
    issueId: issue.id,
    previewFingerprint: preview.fingerprint,
    typedConfirmation: typedConfirmation.trim(),
  };
}

export function confirmedContentBlockRemoval(
  blockId: string,
  result: {
    data?: { clearLibraryContentBlock?: { id: string } | null } | null;
    error?: unknown;
  },
): boolean {
  return !result.error && result.data?.clearLibraryContentBlock?.id === blockId;
}
