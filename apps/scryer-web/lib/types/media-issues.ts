export type MediaIssueRemediation = "MARK_BAD" | "RECYCLE_AND_SEARCH";

export type MediaIssueStatus = "PENDING" | "APPROVED" | "REJECTED" | "CANCELED";

export type MediaIssueExecutionStatus =
  | "NONE"
  | "BLOCKED"
  | "RECYCLING"
  | "RECYCLE_FAILED"
  | "NO_ELIGIBLE_RELEASE"
  | "DOWNLOAD_QUEUED"
  | "REPLACEMENT_IMPORTED"
  | "SEARCHING"
  | "COMPLETE"
  | "FAILED";

export type MediaIssue = {
  id: string;
  titleId: string;
  mediaFileId: string;
  affectedEpisodeIds: string[];
  remediation: MediaIssueRemediation;
  authorizedRemediation: MediaIssueRemediation | null;
  status: MediaIssueStatus;
  executionStatus: MediaIssueExecutionStatus;
  executionErrorCode: string | null;
  details: string | null;
  createdAt: string;
};

export type LibraryContentBlock = {
  id: string;
  sizeBytes: number;
  sampleBlake3: string;
  reason: string | null;
  createdAt: string;
};
