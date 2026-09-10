import assert from "node:assert/strict";
import test from "node:test";
import { titleMoveHistoryDetails } from "./history-moves.ts";

const event = {
  eventType: "title_moved",
  titleId: "title",
  sourcePath: "/old/Film",
  destPath: "/new/Film",
  dataJson: {
    source_title_id: "title",
    source_title_name: "Film",
    source_library_id: "movies",
    destination_library_id: "movies",
    source_library_name: "Movies",
    destination_library_name: "Movies",
    mode: "move_with_scryer",
    completed_with_warnings: false,
  },
};

test("root moves show folders and the completed outcome without internal IDs", () => {
  const details = Object.fromEntries(titleMoveHistoryDetails(event)!.map(({ key, value }) => [key, value]));
  assert.equal(details.operation, "Root move");
  assert.equal(details.source_path, "/old/Film");
  assert.equal(details.dest_path, "/new/Film");
  assert.equal(details.status, "Completed");
  assert.equal(details.merged_from, undefined);
  assert.equal(details.source_library_id, undefined);
});

test("manual and catalog-only moves never claim file verification", () => {
  for (const [mode, expected] of [
    ["user_moved_files", "Files moved manually; catalog updated"],
    ["catalog_only", "Catalog updated"],
    ["files_already_there", "Existing files validated; catalog updated"],
  ]) {
    const details = titleMoveHistoryDetails({ ...event, dataJson: { ...event.dataJson, mode } })!;
    assert.equal(details.find((entry) => entry.key === "method")!.value, expected);
  }
});

test("merged transfers retain source identity and warning details on the surviving title", () => {
  const details = titleMoveHistoryDetails({ ...event, titleId: "survivor", dataJson: {
    ...event.dataJson, destination_library_id: "archive", destination_library_name: "Archive",
    completed_with_warnings: true, detail: "Kept season (from Movies).nfo",
  } })!;
  const values = Object.fromEntries(details.map(({ key, value }) => [key, value]));
  assert.equal(values.operation, "Library transfer");
  assert.equal(values.destination_library, "Archive");
  assert.equal(values.merged_from, "Film");
  assert.equal(values.status, "Completed with warnings");
  assert.equal(values.notes, "Kept season (from Movies).nfo");
});

test("other history types keep their existing renderer", () => {
  assert.equal(titleMoveHistoryDetails({ ...event, eventType: "imported" }), null);
});
