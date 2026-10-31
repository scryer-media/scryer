import assert from "node:assert/strict";
import test from "node:test";

import { TITLE_OVERVIEW_REFRESH_KINDS } from "./title-overview-refresh-kinds.ts";

test("title overview refresh kinds include import lifecycle outcomes", () => {
  assert.equal(TITLE_OVERVIEW_REFRESH_KINDS.has("movie_downloaded"), true);
  assert.equal(
    TITLE_OVERVIEW_REFRESH_KINDS.has("series_episode_imported"),
    true,
  );
  assert.equal(TITLE_OVERVIEW_REFRESH_KINDS.has("import_rejected"), true);
});

test("title overview ordinary refresh kinds exclude hydration-only transitions", () => {
  assert.equal(
    TITLE_OVERVIEW_REFRESH_KINDS.has("metadata_hydration_started"),
    false,
  );
  assert.equal(
    TITLE_OVERVIEW_REFRESH_KINDS.has("metadata_hydration_failed"),
    false,
  );
});
