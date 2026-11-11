import assert from "node:assert/strict";
import test from "node:test";
import {
  canResolvePendingImportSelection,
  ignorePendingImportBatch,
  resolvablePendingImportSelection,
  togglePendingImportSelection,
} from "./pending-import-selection.ts";

const conflict = {
  id: "conflict",
  facet: "MOVIE",
  status: "PENDING",
  reason: "title_already_owns_another_folder",
};
const unmatched = { ...conflict, id: "unmatched", reason: "metadata_missing" };

test("a selection can be resolved when it holds at least one supported conflict", () => {
  assert.equal(canResolvePendingImportSelection([]), false);
  assert.equal(
    canResolvePendingImportSelection([conflict, { ...conflict, id: "second" }]),
    true,
  );
  assert.equal(canResolvePendingImportSelection([conflict, unmatched]), true);
  assert.equal(canResolvePendingImportSelection([unmatched]), false);
  assert.equal(
    canResolvePendingImportSelection([{ ...conflict, facet: "SERIES" }]),
    false,
  );
  assert.equal(
    canResolvePendingImportSelection([{ ...conflict, facet: "ANIME" }]),
    false,
  );
  assert.equal(
    canResolvePendingImportSelection([{ ...conflict, status: "IGNORED" }]),
    false,
  );
});

test("the resolvable subset keeps only supported conflicts in selection order", () => {
  const second = { ...conflict, id: "second" };
  const mixed = [
    unmatched,
    conflict,
    { ...conflict, id: "series", facet: "SERIES" },
    { ...conflict, id: "ignored", status: "IGNORED" },
    second,
  ];
  assert.deepEqual(resolvablePendingImportSelection(mixed), [conflict, second]);
  assert.deepEqual(resolvablePendingImportSelection([unmatched]), []);
  assert.deepEqual(resolvablePendingImportSelection([]), []);
  assert.deepEqual(resolvablePendingImportSelection([conflict, second]), [
    conflict,
    second,
  ]);
});

test("page selection includes non-conflicts, retains other pages, and deduplicates", () => {
  const otherPage = { ...unmatched, id: "other-page" };
  const selected = togglePendingImportSelection(
    [otherPage, conflict],
    [conflict, unmatched, conflict],
    true,
  );
  assert.deepEqual(
    selected.map((item) => item.id),
    ["other-page", "conflict", "unmatched"],
  );
  assert.deepEqual(
    togglePendingImportSelection(selected, [conflict, unmatched], false),
    [otherPage],
  );
  assert.deepEqual(
    togglePendingImportSelection(
      [],
      [{ ...conflict, status: "IGNORED" }],
      true,
    ),
    [],
  );
  assert.deepEqual(
    togglePendingImportSelection(
      [conflict],
      [{ ...conflict, reason: "changed" }],
      true,
    ),
    [{ ...conflict, reason: "changed" }],
  );
});

test("bulk ignore sends chunked requests in order, continues after failures, and returns only failed items for retry", async () => {
  const calls: string[][] = [];
  const failure = { graphQLErrors: [{ message: "denied" }] };
  const chunkFailure = new Error("network failure");
  let active = 0;
  const last = { ...unmatched, id: "last" };
  const fourth = { ...unmatched, id: "fourth" };
  const result = await ignorePendingImportBatch(
    [
      conflict,
      unmatched,
      conflict,
      last,
      { ...conflict, id: "ignored", status: "IGNORED" },
      fourth,
    ],
    async (ids) => {
      assert.equal(active, 0);
      active++;
      calls.push(ids);
      await Promise.resolve();
      active--;
      if (ids.includes(fourth.id)) throw chunkFailure;
      return {
        ignoredIds: ids.filter((id) => id !== unmatched.id),
        failed: ids
          .filter((id) => id === unmatched.id)
          .map((id) => ({ id, error: failure })),
      };
    },
    2,
  );
  assert.deepEqual(calls, [
    [conflict.id, unmatched.id],
    [last.id, fourth.id],
  ]);
  assert.deepEqual(result.ignored, [conflict]);
  assert.deepEqual(result.failed, [
    { item: unmatched, error: failure },
    { item: last, error: chunkFailure },
    { item: fourth, error: chunkFailure },
  ]);
  const retried: string[][] = [];
  const retry = await ignorePendingImportBatch(
    result.failed.map(({ item }) => item),
    async (ids) => {
      retried.push(ids);
      return { ignoredIds: ids, failed: [] };
    },
  );
  assert.deepEqual(retried, [[unmatched.id, last.id, fourth.id]]);
  assert.deepEqual(retry, { ignored: [unmatched, last, fourth], failed: [] });
});

test("empty and all-failed batches report no successes", async () => {
  const empty = await ignorePendingImportBatch([], async () => {
    assert.fail("must not run");
  });
  assert.deepEqual(empty, { ignored: [], failed: [] });
  const result = await ignorePendingImportBatch(
    [conflict, unmatched],
    async () => {
      throw new Error("failure");
    },
  );
  assert.deepEqual(result.ignored, []);
  assert.deepEqual(
    result.failed.map(({ item }) => item.id),
    [conflict.id, unmatched.id],
  );
  const unanswered = await ignorePendingImportBatch([conflict], async () => ({
    ignoredIds: [],
    failed: [],
  }));
  assert.deepEqual(unanswered, {
    ignored: [],
    failed: [{ item: conflict, error: undefined }],
  });
});
