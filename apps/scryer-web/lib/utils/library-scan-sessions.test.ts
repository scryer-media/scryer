import assert from "node:assert/strict";
import test from "node:test";

import type { LibraryScanProgress } from "../types/library-scans.ts";
import {
  activeLibraryScanSessionsForSelection,
  defaultLibraryIdForFacet,
  didActiveLibraryScanSessionEnd,
  findActiveLibraryScanSession,
  isLibraryScanTargetBusy,
  libraryScanProgressKey,
  libraryScanSessionIds,
} from "./library-scan-sessions.ts";

function scan(
  sessionId: string,
  libraryId: string,
  updatedAt: string,
): LibraryScanProgress {
  return {
    sessionId,
    facet: "MOVIE",
    libraryId,
    mode: "FULL",
    status: "RUNNING",
    startedAt: updatedAt,
    updatedAt,
    foundTitles: 1,
    titleMatchTotalKnown: true,
    hydrationTotalKnown: false,
    mediaAnalysisTotalKnown: false,
    titleMatchProgress: { total: 1, completed: 1, failed: 0 },
    hydrationProgress: { total: 0, completed: 0, failed: 0 },
    mediaAnalysisProgress: { total: 1, completed: 0, failed: 0 },
    summary: null,
  };
}

test("active scan lookup is scoped to the requested library", () => {
  const libraryA = scan("scan-a", "library-a", "2026-07-13T00:00:00Z");
  const libraryB = scan("scan-b", "library-b", "2026-07-13T00:00:01Z");
  const sessions = [libraryA, libraryB];

  assert.equal(
    findActiveLibraryScanSession(sessions, "MOVIE", "library-a"),
    libraryA,
  );
  assert.equal(
    findActiveLibraryScanSession(sessions, "MOVIE", "library-b"),
    libraryB,
  );
  assert.equal(
    findActiveLibraryScanSession(sessions, "MOVIE", "library-c"),
    null,
  );
});

test("an unscoped facet scan is active for every library in that facet", () => {
  const unscoped = {
    ...scan("legacy-scan", "unused", "2026-07-13T00:00:00Z"),
    libraryId: null,
  };

  assert.equal(
    findActiveLibraryScanSession([unscoped], "MOVIE", "library-a"),
    unscoped,
  );
  assert.equal(
    findActiveLibraryScanSession([unscoped], "MOVIE", "library-b"),
    unscoped,
  );
  assert.equal(
    findActiveLibraryScanSession([unscoped], "SERIES", "series-library"),
    null,
  );
});

test("selected-library refreshes include only relevant concurrent scans", () => {
  const libraryA = scan("scan-a", "library-a", "2026-07-13T00:00:00Z");
  const libraryB = scan("scan-b", "library-b", "2026-07-13T00:00:01Z");

  assert.deepEqual(
    activeLibraryScanSessionsForSelection([libraryA, libraryB], "MOVIE", [
      "library-b",
    ]),
    [libraryB],
  );
  assert.deepEqual(
    activeLibraryScanSessionsForSelection([libraryA, libraryB], "MOVIE", []),
    [libraryA, libraryB],
  );
});

test("aggregate progress key changes when either scan advances", () => {
  const libraryA = scan("scan-a", "library-a", "2026-07-13T00:00:00Z");
  const libraryB = scan("scan-b", "library-b", "2026-07-13T00:00:01Z");
  const before = libraryScanProgressKey([libraryA, libraryB]);
  const advancedB = {
    ...libraryB,
    updatedAt: "2026-07-13T00:00:02Z",
    mediaAnalysisProgress: { total: 1, completed: 1, failed: 0 },
  };

  assert.notEqual(libraryScanProgressKey([libraryA, advancedB]), before);
});

test("detects one concurrent scan ending while another remains active", () => {
  const libraryA = scan("scan-a", "library-a", "2026-07-13T00:00:00Z");
  const libraryB = scan("scan-b", "library-b", "2026-07-13T00:00:01Z");
  const previousSessionIds = libraryScanSessionIds([libraryA, libraryB]);

  assert.equal(
    didActiveLibraryScanSessionEnd(previousSessionIds, [libraryB]),
    true,
  );
  assert.equal(
    didActiveLibraryScanSessionEnd(previousSessionIds, [libraryA, libraryB]),
    false,
  );
});

test("default library ids match backend facet defaults", () => {
  assert.equal(defaultLibraryIdForFacet("MOVIE"), "movie_default_library");
  assert.equal(defaultLibraryIdForFacet("SERIES"), "series_default_library");
  assert.equal(defaultLibraryIdForFacet("ANIME"), "anime_default_library");
});

test("pending scan state is busy only for its target library", () => {
  const state: Record<string, { loading: boolean; sessionId: string | null }> =
    {
      "library-a": { loading: true, sessionId: null },
      "library-b": { loading: false, sessionId: null },
    };
  const noSession = () => null;
  assert.equal(
    isLibraryScanTargetBusy(state, "library-a", null, noSession),
    true,
  );
  assert.equal(
    isLibraryScanTargetBusy(state, "library-b", null, noSession),
    false,
  );

  state["library-a"] = { loading: false, sessionId: "scan-a" };
  assert.equal(
    isLibraryScanTargetBusy(state, "library-a", null, noSession),
    true,
  );
  assert.equal(
    isLibraryScanTargetBusy(state, "library-b", null, noSession),
    false,
  );
});
