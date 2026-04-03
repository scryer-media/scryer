import type { Facet, LibraryScanSummary } from "./titles";

export const libraryScanStatusValues = [
  "discovering",
  "running",
  "completed",
  "warning",
  "failed",
] as const;

export type LibraryScanStatus = (typeof libraryScanStatusValues)[number];

export type LibraryScanPhaseProgress = {
  total: number;
  completed: number;
  failed: number;
};

export type LibraryScanProgress = {
  sessionId: string;
  facet: Facet;
  status: LibraryScanStatus;
  startedAt: string;
  updatedAt: string;
  foundTitles: number;
  metadataTotalKnown: boolean;
  fileTotalKnown: boolean;
  metadataProgress: LibraryScanPhaseProgress;
  fileProgress: LibraryScanPhaseProgress;
  summary?: LibraryScanSummary | null;
};
