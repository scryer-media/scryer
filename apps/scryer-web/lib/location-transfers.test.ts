import assert from "node:assert/strict";
import test from "node:test";
import {
  acceptTransferSnapshot,
  splitTransferPath,
  transferArtworkRequest,
  transferOperationProgress,
  transferTitleProgress,
  type TransferSnapshot,
  type TransferView,
  type TransferTitle,
} from "./location-transfers.ts";

test("artwork requests are bounded, parameterized, and stable across hot ordering", () => {
  assert.deepEqual(
    transferArtworkRequest(["b", "a", "a"]),
    transferArtworkRequest(["a", "b"]),
  );
  const request = transferArtworkRequest(
    Array.from({ length: 5000 }, (_, i) => `title-${i}`),
  );
  assert.equal(Object.keys(request.variables).length, 50);
  assert.equal(request.query.includes("title-"), false);
  assert.match(request.query, /title49: title\(id: \$id49\)/);
  assert.deepEqual(transferArtworkRequest([]).variables, {});
});

const scope = { operationId: "op", page: null, requestGeneration: 1 };
function snapshot(
  generation: number | string,
  revision: number | string,
  progressBasisPoints = 1000,
): TransferSnapshot {
  return {
    generation,
    revision,
    progressBasisPoints,
    operation: { id: "op" },
    titles: [],
    totalCount: 5000,
    hasMore: true,
    etaSeconds: null,
  } as unknown as TransferSnapshot;
}

test("late polling cannot replace subscription snapshots, including restart and large revisions", () => {
  let view: TransferView = { scope, snapshot: null };
  view = acceptTransferSnapshot(
    view,
    scope,
    snapshot(1, "9007199254740993", 8000),
  );
  assert.equal(
    acceptTransferSnapshot(view, scope, snapshot(1, "9007199254740992")),
    view,
  );
  view = acceptTransferSnapshot(view, scope, snapshot(2, 1, 6000));
  assert.equal(view.snapshot?.progressBasisPoints, 8000);
  assert.equal(
    acceptTransferSnapshot(view, scope, snapshot(1, "999999999999999999")),
    view,
  );
  assert.equal(acceptTransferSnapshot(view, scope, snapshot(2, 1)), view);
});

test("abandoned pages, previous navigation and old request generations never enter current scope", () => {
  const page = { ...scope, page: 2, requestGeneration: 3 };
  const view: TransferView = { scope: page, snapshot: null };
  assert.equal(acceptTransferSnapshot(view, scope, snapshot(3, 99)), view);
  assert.equal(
    acceptTransferSnapshot(
      view,
      { ...page, requestGeneration: 2 },
      snapshot(3, 99),
    ),
    view,
  );
  assert.equal(
    acceptTransferSnapshot(
      view,
      { ...page, operationId: "elsewhere" },
      snapshot(3, 99),
    ),
    view,
  );
  assert.equal(
    acceptTransferSnapshot(view, page, snapshot(3, 99)).snapshot?.revision,
    99,
  );
});

test("progress remains monotonic under every ordering of streaming and fallback responses", () => {
  for (let rotation = 0; rotation < 100; rotation++) {
    let view: TransferView = { scope, snapshot: null };
    let previous = 0;
    for (let index = 0; index < 100; index++) {
      const revision = (index * 37 + rotation) % 100;
      view = acceptTransferSnapshot(
        view,
        scope,
        snapshot(1, revision, revision * 99),
      );
      assert.ok(view.snapshot!.progressBasisPoints >= previous);
      previous = view.snapshot!.progressBasisPoints;
    }
    assert.equal(view.snapshot?.revision, 99);
  }
});

test("file work gives placement and verification equal shares; no-op rows finish without bytes", () => {
  const row = {
    bytesTotal: 1000,
    copyBytes: 1000,
    verificationBytes: 100,
    state: "VERIFYING",
  } as TransferTitle;
  assert.ok(Math.abs(transferTitleProgress(row) - 55) < 1e-10);
  assert.equal(
    transferTitleProgress({ ...row, verificationBytes: 1000 }),
    99.9,
  );
  assert.equal(
    transferTitleProgress({ ...row, bytesTotal: 0, state: "SKIPPED" }),
    100,
  );
});

test("operation and title progress stay below 100 until finalization completes", () => {
  for (const state of [
    "VERIFYING",
    "RECONCILING",
    "CLEANING_UP",
    "FAILED",
    "CANCELED",
  ] as const) {
    const incoming = snapshot(1, 2, 10000);
    incoming.operation.state = state;
    assert.equal(transferOperationProgress(incoming), 99.9);
    if (state !== "CANCELED") {
      assert.equal(
        transferTitleProgress({
          bytesTotal: 100,
          copyBytes: 100,
          verificationBytes: 100,
          state,
        } as TransferTitle),
        99.9,
      );
    }
  }
  const complete = snapshot(1, 3, 9999);
  complete.operation.state = "COMPLETED";
  assert.equal(transferOperationProgress(complete), 100);
  assert.equal(
    transferTitleProgress({
      bytesTotal: 100,
      state: "COMPLETED",
    } as TransferTitle),
    100,
  );
});

test("leading directories are separated from intact filenames on every platform", () => {
  for (const path of [
    "/long/early/directories/Show.S01E01.mkv",
    "C:\\long\\directories\\Show.S01E01.mkv",
    "Show.S01E01.mkv",
  ]) {
    const parts = splitTransferPath(path);
    assert.equal(parts.filename, "Show.S01E01.mkv");
    assert.equal(parts.directory + parts.filename, path);
  }
});
