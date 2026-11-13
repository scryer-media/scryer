import assert from "node:assert/strict";
import test from "node:test";
import { requestDialogLibraryId } from "./request-dialog-library.ts";

const libraries = [
  { id: "synthetic-movies-b", isDefault: false },
  { id: "synthetic-movies-default", isDefault: true },
];

test("a refreshed library list keeps a choice that is still listed", () => {
  assert.equal(
    requestDialogLibraryId(libraries, "synthetic-movies-b"),
    "synthetic-movies-b",
  );
});

test("a choice that left the list falls back to the default library", () => {
  assert.equal(
    requestDialogLibraryId(libraries, "synthetic-removed"),
    "synthetic-movies-default",
  );
});

test("without a default the first library is chosen, and none gives empty", () => {
  assert.equal(
    requestDialogLibraryId([{ id: "synthetic-only", isDefault: false }], ""),
    "synthetic-only",
  );
  assert.equal(requestDialogLibraryId([], "synthetic-removed"), "");
});
