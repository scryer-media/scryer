import type { DeletePreview } from "./delete-preview.ts";

export type MediaIssueRemediation = "MARK_BAD" | "RECYCLE_AND_SEARCH";

/** The recycle preview, and why its file's other name changes the choice. */
export type MediaIssueRecyclePreviewPayload = {
  preview: DeletePreview;
  hardLinkReason: string | null;
};

export type MediaIssueReason =
  | "corrupted"
  | "wrong_content"
  | "wrong_language"
  | "audio_out_of_sync"
  | "poor_quality"
  | "other";

export type MediaIssueStatus = "PENDING" | "APPROVED" | "REJECTED" | "CANCELED";

export type MediaIssueExecutionStatus =
  | "NONE"
  | "BLOCKED"
  | "RECYCLING"
  | "RECYCLE_FAILED"
  | "NO_ELIGIBLE_RELEASE"
  | "DOWNLOAD_QUEUED"
  | "REPLACEMENT_IMPORTED"
  | "SEARCHING";

export type MediaIssue = {
  id: string;
  titleId: string;
  mediaFileId: string;
  affectedEpisodeIds: string[];
  reasonCode: string;
  remediation: MediaIssueRemediation;
  authorizedRemediation: MediaIssueRemediation | null;
  status: MediaIssueStatus;
  executionStatus: MediaIssueExecutionStatus;
  executionErrorCode: string | null;
  details: string | null;
  createdAt: string;
};
