import assert from "node:assert/strict";
import test from "node:test";
import {
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
