import assert from "node:assert/strict";
import test from "node:test";

import { historyHoverDetails } from "./history-hover.ts";

// The shape the API serializes a domain event into for `dataJson`.
function importPayload(title: Record<string, unknown>): string {
  return JSON.stringify({
    type: "import_completed",
    data: {
      title,
      source_title: "Synthetic.Show.S01E01.1080p",
      imported_count: 1,
    },
  });
}

test("history hover prefers the live title poster the API hydrates", () => {
  const details = historyHoverDetails({
    facet: "series",
    posterUrl: "https://example.invalid/live-poster.jpg",
    dataJson: importPayload({
      title_name: "Synthetic Show",
      facet: "series",
      poster_url: "https://example.invalid/snapshot-poster.jpg",
    }),
  });

  assert.deepEqual(details, {
    facet: "SERIES",
    posterUrl: "https://example.invalid/live-poster.jpg",
  });
});

test("history hover reads the snapshot poster nested under the event data", () => {
  const details = historyHoverDetails({
    facet: null,
    posterUrl: null,
    dataJson: importPayload({
      title_name: "Synthetic Anime",
      facet: "anime",
      poster_url: "https://example.invalid/snapshot-poster.jpg",
    }),
  });

  assert.deepEqual(details, {
    facet: "ANIME",
    posterUrl: "https://example.invalid/snapshot-poster.jpg",
  });
});

test("history hover still reads a bare payload title and a blank live poster", () => {
  const details = historyHoverDetails({
    facet: "MOVIE",
    posterUrl: "  ",
    dataJson: {
      title: { facet: "movie", posterUrl: "/images/synthetic.jpg" },
    },
  });

  assert.deepEqual(details, {
    facet: "MOVIE",
    posterUrl: "/images/synthetic.jpg",
  });
});

test("history hover has no poster when neither source carries one", () => {
  const details = historyHoverDetails({
    facet: "series",
    dataJson: importPayload({ title_name: "Synthetic Show", poster_url: null }),
  });

  assert.deepEqual(details, { facet: "SERIES", posterUrl: null });
});
