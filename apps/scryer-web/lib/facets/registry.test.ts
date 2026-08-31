import assert from "node:assert/strict";
import test from "node:test";

import { facetById, facetByMetadataKey, metadataFacetGraphqlValue } from "./registry.ts";

test("metadata facet keys resolve to canonical GraphQL enum values", () => {
  assert.equal(facetByMetadataKey("movie")?.id, "MOVIE");
  assert.equal(facetByMetadataKey("series")?.id, "SERIES");
  assert.equal(facetByMetadataKey("anime")?.id, "ANIME");
});

test("metadata facet values normalize to canonical GraphQL enum values", () => {
  assert.equal(metadataFacetGraphqlValue("movie"), "MOVIE");
  assert.equal(metadataFacetGraphqlValue("SERIES"), "SERIES");
  assert.equal(metadataFacetGraphqlValue(" anime "), "ANIME");
  assert.equal(metadataFacetGraphqlValue(null), "SERIES");
});

test("canonical facets use the canonical facet lookup", () => {
  assert.equal(facetById("MOVIE")?.metadataKey, "movie");
  assert.equal(facetById("SERIES")?.metadataKey, "series");
  assert.equal(facetById("ANIME")?.metadataKey, "anime");

  // @ts-expect-error Canonical GraphQL enum values are not metadata keys.
  assert.equal(facetByMetadataKey("ANIME"), undefined);
});
