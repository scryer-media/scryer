import test from "node:test";
import assert from "node:assert/strict";
import {
  allRowsSelected,
  invertedRowSelection,
  rowsForBulkAction,
  toggledRowSelection,
  withRowTargets,
  withRowsMatching,
} from "./manual-import-row-selection.ts";

const ROWS = [
  { candidateId: "c1", fileName: "Synthetic.Show.S01E01.mkv" },
  { candidateId: "c2", fileName: "Synthetic.Show.S01E02.mkv" },
  { candidateId: "c3", fileName: "Other.Download.Sample.mkv" },
];

test("rows are selected, cleared, inverted and matched by file name", () => {
  let selected = toggledRowSelection(new Set(), "c1");
  assert.deepEqual([...selected], ["c1"]);
  assert.deepEqual([...toggledRowSelection(selected, "c1")], []);

  assert.deepEqual([...allRowsSelected(ROWS)], ["c1", "c2", "c3"]);
  assert.deepEqual([...invertedRowSelection(ROWS, selected)], ["c2", "c3"]);

  selected = withRowsMatching(ROWS, new Set(), "  SAMPLE ");
  assert.deepEqual([...selected], ["c3"]);
  assert.deepEqual([...withRowsMatching(ROWS, selected, "   ")], ["c3"]);
});

test("a bulk action works on the selected rows, or every row when none is", () => {
  assert.deepEqual(
    rowsForBulkAction(ROWS, new Set()).map((row) => row.candidateId),
    ["c1", "c2", "c3"],
  );
  assert.deepEqual(
    rowsForBulkAction(ROWS, new Set(["c3", "c1"])).map(
      (row) => row.candidateId,
    ),
    ["c1", "c3"],
  );
});

test("applying targets to the selected rows leaves the other rows' choices", () => {
  const mappings = { c1: "skip", c2: "episode:e9", c3: "episode:e7" };
  const next = withRowTargets(
    mappings,
    rowsForBulkAction(ROWS, new Set(["c1"])),
    () => "episode:e1",
  );
  assert.deepEqual(next, {
    c1: "episode:e1",
    c2: "episode:e9",
    c3: "episode:e7",
  });

  const kept = withRowTargets(mappings, ROWS, (row) =>
    row.candidateId === "c2" ? "episode:e9" : null,
  );
  assert.equal(kept, mappings);
});
