import assert from "node:assert/strict";
import test from "node:test";

import {
  selectBackdropVariantUrl,
  selectMediaImageVariantUrl,
  selectPosterVariantUrl,
} from "./poster-images.ts";

test("rewriting an opaque media-image route preserves its token and base path", () => {
  assert.equal(
    selectMediaImageVariantUrl(
      "/scryer/images/media/opaque-token/w250?cache=1#poster",
      "w70",
    ),
    "/scryer/images/media/opaque-token/w70?cache=1#poster",
  );
});

test("episode stills select the optimized w300 media variant", () => {
  assert.equal(
    selectMediaImageVariantUrl("/images/media/episode-token/original", "w300"),
    "/images/media/episode-token/w300",
  );
});

test("rewriting an absolute opaque media-image route preserves its origin", () => {
  assert.equal(
    selectPosterVariantUrl(
      "https://scryer.example/base/images/media/opaque-token/original",
      "w250",
    ),
    "https://scryer.example/base/images/media/opaque-token/w250",
  );
});

test("rewriting a local poster variant drops the source variant version token", () => {
  assert.equal(
    selectPosterVariantUrl(
      "/images/titles/title-1/poster/w250?v=posterw250digest#poster",
      "w70",
    ),
    "/images/titles/title-1/poster/w70#poster",
  );
});

test("selecting the current local poster variant preserves its version token", () => {
  assert.equal(
    selectPosterVariantUrl(
      "/images/titles/title-1/poster/w250?v=posterw250digest",
      "w250",
    ),
    "/images/titles/title-1/poster/w250?v=posterw250digest",
  );
});

test("rewriting a legacy local w500 poster selects a supported variant", () => {
  assert.equal(
    selectPosterVariantUrl(
      "/images/titles/title-1/poster/w500?v=posterw500digest",
      "w250",
    ),
    "/images/titles/title-1/poster/w250",
  );
});

test("provider URLs are never rewritten into a different upstream request", () => {
  const upstream = "https://image.tmdb.org/t/p/w500/poster.jpg";
  assert.equal(selectPosterVariantUrl(upstream, "w70"), upstream);
  assert.equal(selectBackdropVariantUrl(upstream, "w1280"), upstream);
});

test("rewriting a local fanart variant selects w1280 and drops stale version token", () => {
  assert.equal(
    selectBackdropVariantUrl(
      "/images/titles/title-1/fanart/w780?v=fanartw780digest#backdrop",
      "w1280",
    ),
    "/images/titles/title-1/fanart/w1280#backdrop",
  );
});
