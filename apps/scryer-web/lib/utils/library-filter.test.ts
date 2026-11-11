import assert from "node:assert/strict";
import test from "node:test";
import type { Facet, LibraryRecord } from "@/lib/types";
import {
  defaultLibrariesInFacetOrder,
  facetHasEnabledLibrary,
  normalizeLibraryFilterSelection,
  selectableLibraries,
} from "./library-filter.ts";

function library(
  id: string,
  facet: Facet,
  enabled?: boolean,
  isDefault = false,
): LibraryRecord {
  return {
    id,
    facet,
    name: id,
    slug: id,
    isDefault,
    ...(enabled === undefined ? {} : { enabled }),
    roots: [],
  };
}

function librariesByFacet(
  ...libraries: LibraryRecord[]
): Record<Facet, LibraryRecord[]> {
  return {
    MOVIE: libraries.filter((entry) => entry.facet === "MOVIE"),
    SERIES: libraries.filter((entry) => entry.facet === "SERIES"),
    ANIME: libraries.filter((entry) => entry.facet === "ANIME"),
  };
}

test("default library settings include every facet and exclude custom libraries", () => {
  const libraries = [
    library("anime-default", "ANIME", true, true),
    library("movie-custom", "MOVIE", true),
    library("series-default", "SERIES", false, true),
    library("movie-default", "MOVIE", false, true),
  ];

  assert.deepEqual(
    defaultLibrariesInFacetOrder(libraries).map(({ id }) => id),
    ["movie-default", "series-default", "anime-default"],
  );
});

test("disabled libraries are excluded from selectable lists and selections", () => {
  const libraries = [
    library("movies", "MOVIE", true),
    library("disabled-movies", "MOVIE", false),
    library("legacy-movies", "MOVIE"),
  ];

  assert.deepEqual(
    selectableLibraries(libraries).map(({ id }) => id),
    ["movies", "legacy-movies"],
  );
  assert.deepEqual(
    normalizeLibraryFilterSelection(["movies", "disabled-movies"], libraries),
    ["movies"],
  );
});

test("a selection covering every enabled library normalizes to all", () => {
  const libraries = [
    library("movies", "MOVIE", true),
    library("disabled-movies", "MOVIE", false),
    library("legacy-movies", "MOVIE"),
  ];

  assert.deepEqual(
    normalizeLibraryFilterSelection(["movies", "legacy-movies"], libraries),
    [],
  );
});

test("the last enabled library controls whether its facet remains visible", () => {
  const enabled = library("movies", "MOVIE", true);
  const disabled = library("disabled-movies", "MOVIE", false);

  assert.equal(
    facetHasEnabledLibrary("MOVIE", {
      viewable: librariesByFacet(enabled, disabled),
      requestable: librariesByFacet(),
      manageable: librariesByFacet(),
    }),
    true,
  );
  assert.equal(
    facetHasEnabledLibrary("MOVIE", {
      viewable: librariesByFacet(disabled),
      requestable: librariesByFacet(),
      manageable: librariesByFacet(),
    }),
    false,
  );
});

test("a disabled library does not hide another enabled library in the facet", () => {
  assert.equal(
    facetHasEnabledLibrary("SERIES", {
      viewable: librariesByFacet(
        library("enabled-series", "SERIES", true),
        library("disabled-series", "SERIES", false),
      ),
      requestable: librariesByFacet(),
      manageable: librariesByFacet(),
    }),
    true,
  );
});

test("a requester can see a facet through REQUEST without VIEW", () => {
  assert.equal(
    facetHasEnabledLibrary("ANIME", {
      viewable: librariesByFacet(),
      requestable: librariesByFacet(library("requested-anime", "ANIME", true)),
      manageable: librariesByFacet(),
    }),
    true,
  );
});

test("a title manager sees enabled facets but not disabled-only facets", () => {
  const enabled = library("managed-series", "SERIES", true);
  const disabled = library("disabled-series", "SERIES", false);

  assert.equal(
    facetHasEnabledLibrary("SERIES", {
      viewable: librariesByFacet(),
      requestable: librariesByFacet(),
      manageable: librariesByFacet(enabled),
    }),
    true,
  );
  assert.equal(
    facetHasEnabledLibrary("SERIES", {
      viewable: librariesByFacet(),
      requestable: librariesByFacet(),
      manageable: librariesByFacet(disabled),
    }),
    false,
  );
});
