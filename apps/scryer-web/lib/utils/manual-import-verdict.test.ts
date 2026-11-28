import assert from "node:assert/strict";
import test from "node:test";
import {
  MANUAL_IMPORT_VERDICT_MAX_EPISODES_PER_ROW,
  MANUAL_IMPORT_VERDICT_MAX_ROWS,
  chunkManualImportVerdictRequests,
  currentManualImportVerdict,
  manualImportVerdictKey,
  pendingManualImportVerdictRequests,
} from "./manual-import-verdict.ts";
import {
  SERIES_MOVIE_TARGET_PREFIX,
  UNASSIGNED_TARGET,
  episodeTargetValue,
} from "./episode-target.ts";

test("only episode mappings that changed since their last request are asked about", () => {
  const mappings = {
    a: episodeTargetValue(["e1"]),
    b: episodeTargetValue(["e2", "e3"]),
    c: UNASSIGNED_TARGET,
    d: `${SERIES_MOVIE_TARGET_PREFIX}link-1`,
  };
  assert.deepEqual(pendingManualImportVerdictRequests(mappings, {}), [
    { candidateId: "a", episodeIds: ["e1"] },
    { candidateId: "b", episodeIds: ["e2", "e3"] },
  ]);
  assert.deepEqual(
    pendingManualImportVerdictRequests(mappings, {
      a: manualImportVerdictKey(["e1"]),
      b: manualImportVerdictKey(["e3", "e2"]),
    }),
    [],
  );
  assert.deepEqual(
    pendingManualImportVerdictRequests(
      { ...mappings, a: episodeTargetValue(["e4"]) },
      {
        a: manualImportVerdictKey(["e1"]),
        b: manualImportVerdictKey(["e2", "e3"]),
      },
    ),
    [{ candidateId: "a", episodeIds: ["e4"] }],
  );
});

test("an estimate is shown only for the mapping it was asked for", () => {
  const estimate = {
    key: manualImportVerdictKey(["e1", "e2"]),
    verdict: "DOWNGRADE" as const,
    episodeCount: 2,
  };
  assert.equal(
    currentManualImportVerdict(estimate, episodeTargetValue(["e2", "e1"])),
    estimate,
  );
  assert.equal(
    currentManualImportVerdict(estimate, episodeTargetValue(["e1"])),
    null,
  );
  assert.equal(currentManualImportVerdict(estimate, UNASSIGNED_TARGET), null);
  assert.equal(
    currentManualImportVerdict(
      { ...estimate, verdict: null },
      episodeTargetValue(["e1", "e2"]),
    ),
    null,
  );
});

test("requests go out in batches the server accepts", () => {
  const requests = Array.from(
    { length: MANUAL_IMPORT_VERDICT_MAX_ROWS * 2 + 1 },
    (_, index) => ({ candidateId: `c${index}`, episodeIds: ["e1"] }),
  );
  const chunks = chunkManualImportVerdictRequests(requests);
  assert.deepEqual(
    chunks.map((chunk) => chunk.length),
    [MANUAL_IMPORT_VERDICT_MAX_ROWS, MANUAL_IMPORT_VERDICT_MAX_ROWS, 1],
  );
  assert.deepEqual(chunks.flat(), requests);
  assert.deepEqual(chunkManualImportVerdictRequests([]), []);
});

test("a file mapped to more episodes than the server estimates is not asked about", () => {
  const tooMany = Array.from(
    { length: MANUAL_IMPORT_VERDICT_MAX_EPISODES_PER_ROW + 1 },
    (_, index) => `e${index}`,
  );
  assert.deepEqual(
    pendingManualImportVerdictRequests(
      { a: episodeTargetValue(tooMany), b: episodeTargetValue(["e1"]) },
      {},
    ),
    [{ candidateId: "b", episodeIds: ["e1"] }],
  );
});
