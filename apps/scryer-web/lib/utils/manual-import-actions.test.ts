import assert from "node:assert/strict";
import test from "node:test";

import {
  compareManualImportSeasonLabels,
  directMovieManualImportMappings,
  manualImportActions,
} from "./manual-import-actions.ts";

test("manual import season labels sort numerically", () => {
  const labels = ["Season 1", "Season 10", "Season 2", "Season 20"];

  assert.deepEqual(labels.sort(compareManualImportSeasonLabels), [
    "Season 1",
    "Season 2",
    "Season 10",
    "Season 20",
  ]);
});

test("direct movie manual import maps only the largest candidate", () => {
  assert.deepEqual(
    directMovieManualImportMappings([
      { candidateId: "sample", sizeBytes: 12_000_000 },
      { candidateId: "movie", sizeBytes: 4_200_000_000 },
      { candidateId: "featurette", sizeBytes: 300_000_000 },
    ]),
    [{ candidateId: "movie" }],
  );
});

test("direct movie manual import treats unknown sizes as smallest and keeps ties stable", () => {
  assert.deepEqual(
    directMovieManualImportMappings([
      { candidateId: "unknown-size", sizeBytes: null },
      { candidateId: "first-of-tie", sizeBytes: 100 },
      { candidateId: "second-of-tie", sizeBytes: 100 },
    ]),
    [{ candidateId: "first-of-tie" }],
  );
  assert.deepEqual(directMovieManualImportMappings([{ candidateId: "only" }]), [
    { candidateId: "only" },
  ]);
});

test("direct movie manual import maps nothing without candidates", () => {
  assert.deepEqual(directMovieManualImportMappings([]), []);
});

for (const facet of ["MOVIE", "SERIES", "ANIME"] as const) {
  test(`pending ${facet} import exposes no manual action`, () => {
    assert.deepEqual(
      manualImportActions({
        displayState: "IMPORT_PENDING",
        facet,
        hasTitle: true,
      }),
      { direct: false, interactive: false },
    );
  });
}

for (const displayState of ["IMPORT_BLOCKED", "IMPORT_FAILED"] as const) {
  test(`${displayState} series and anime imports use interactive mapping`, () => {
    for (const facet of ["SERIES", "ANIME"] as const) {
      assert.deepEqual(
        manualImportActions({ displayState, facet, hasTitle: true }),
        { direct: false, interactive: true },
      );
    }
  });

  test(`${displayState} movie imports use the direct action`, () => {
    assert.deepEqual(
      manualImportActions({
        displayState,
        facet: "MOVIE",
        hasTitle: true,
      }),
      { direct: true, interactive: false },
    );
  });
}

test("manual import actions require an assigned title", () => {
  assert.deepEqual(
    manualImportActions({
      displayState: "IMPORT_BLOCKED",
      facet: "series",
      hasTitle: false,
    }),
    { direct: false, interactive: false },
  );
});

test("manual import actions tolerate legacy lowercase facet values", () => {
  assert.deepEqual(
    manualImportActions({
      displayState: "IMPORT_BLOCKED",
      facet: "series",
      hasTitle: true,
    }),
    { direct: false, interactive: true },
  );
});
