import type { Facet, LibraryScanProgress } from "@/lib/types";

export function isTerminalLibraryScanStatus(
  status: LibraryScanProgress["status"],
): boolean {
  return (
    status === "COMPLETED" ||
    status === "CANCELED" ||
    status === "WARNING" ||
    status === "FAILED"
  );
}

export function defaultLibraryIdForFacet(facet: Facet): string {
  return `${facet.toLowerCase()}_default_library`;
}

export function findActiveLibraryScanSession(
  sessions: LibraryScanProgress[],
  facet: Facet,
  libraryId?: string | null,
): LibraryScanProgress | null {
  return (
    sessions.find(
      (session) =>
        session.facet === facet &&
        (libraryId == null ||
          session.libraryId == null ||
          session.libraryId === libraryId) &&
        !isTerminalLibraryScanStatus(session.status),
    ) ?? null
  );
}

export function activeLibraryScanSessionsForSelection(
  sessions: LibraryScanProgress[],
  facet: Facet,
  selectedLibraryIds: string[],
): LibraryScanProgress[] {
  const selected = new Set(selectedLibraryIds);
  return sessions.filter(
    (session) =>
      session.facet === facet &&
      !isTerminalLibraryScanStatus(session.status) &&
      (session.libraryId == null ||
        selected.size === 0 ||
        selected.has(session.libraryId)),
  );
}

export function libraryScanSessionIds(
  sessions: LibraryScanProgress[],
): string[] {
  return sessions
    .map((session) => session.sessionId)
    .sort((left, right) => left.localeCompare(right));
}

export function didActiveLibraryScanSessionEnd(
  previousSessionIds: string[],
  sessions: LibraryScanProgress[],
): boolean {
  if (previousSessionIds.length === 0) {
    return false;
  }
  const activeSessionIds = new Set(
    sessions.map((session) => session.sessionId),
  );
  return previousSessionIds.some(
    (sessionId) => !activeSessionIds.has(sessionId),
  );
}

export function libraryScanProgressKey(
  sessions: LibraryScanProgress[],
): string {
  return [...sessions]
    .sort((left, right) => left.sessionId.localeCompare(right.sessionId))
    .map((session) =>
      [
        session.sessionId,
        session.mediaAnalysisProgress.total,
        session.mediaAnalysisProgress.completed,
        session.mediaAnalysisProgress.failed,
        session.summary?.imported ?? 0,
        session.updatedAt,
      ].join(":"),
    )
    .join("|");
}

export type LibraryScanPendingUiState = {
  loading: boolean;
  sessionId: string | null;
};

export function isLibraryScanTargetBusy(
  stateByLibraryId: Record<string, LibraryScanPendingUiState>,
  targetLibraryId: string | null,
  activeSession: LibraryScanProgress | null,
  getSessionById: (sessionId: string) => LibraryScanProgress | null,
): boolean {
  if (activeSession) {
    return true;
  }
  if (!targetLibraryId) {
    return false;
  }
  const state = stateByLibraryId[targetLibraryId];
  return Boolean(
    state?.loading || (state?.sessionId && !getSessionById(state.sessionId)),
  );
}
