import assert from "node:assert/strict";
import test from "node:test";

import {
  addSetupRoot,
  bootstrapSetupMediaRoots,
  plannedSetupRootSaves,
  removeSetupRoot,
  replaceSetupRoot,
  runAdvisorySetupMediaPathSave,
  setDefaultSetupRoot,
  setupMediaLibraries,
  setupMediaRootsFromLibraries,
  type SetupMediaLibraries,
} from "./setup-media-paths.ts";

const defaultRoots = bootstrapSetupMediaRoots();

function library(id: string, facet: string, paths: string[], isDefault = true) {
  return {
    id,
    facet: facet as "MOVIE",
    isDefault,
    roots: paths.map((path, i) => ({ id: `${id}-${i}`, path, isDefault: i === 0 })),
  };
}

test("confirmed missing setup paths warn but still save and advance", async () => {
  let saved = false;
  let validationState: unknown = null;
  let savedValidationState: unknown = null;

  await runAdvisorySetupMediaPathSave({
    roots: defaultRoots,
    validatePath: async () => ({
      graphQLErrors: [{ extensions: { code: "VALIDATION_ERROR" } }],
    }),
    onValidation: (state) => {
      validationState = state;
    },
    save: async () => {
      saved = true;
    },
    onSaved: (state) => {
      savedValidationState = state;
    },
  });

  assert.deepEqual(validationState, {
    invalidPaths: ["/data/movies", "/data/series", "/data/anime"],
    unavailable: false,
  });
  assert.equal(saved, true);
  assert.deepEqual(savedValidationState, validationState);
});

test("unavailable setup validation remains advisory", async () => {
  let saved = false;
  let validationState: unknown = null;

  await runAdvisorySetupMediaPathSave({
    roots: defaultRoots,
    validatePath: async () => ({
      graphQLErrors: [{ extensions: { code: "SERVICE_UNAVAILABLE" } }],
    }),
    onValidation: (state) => {
      validationState = state;
    },
    save: async () => {
      saved = true;
    },
    onSaved: () => {},
  });

  assert.deepEqual(validationState, { invalidPaths: [], unavailable: true });
  assert.equal(saved, true);
});

test("a setup path save failure prevents advancement", async () => {
  let advanced = false;

  await assert.rejects(
    runAdvisorySetupMediaPathSave({
      roots: defaultRoots,
      validatePath: async () => null,
      onValidation: () => {},
      save: async () => {
        throw new Error("save failed");
      },
      onSaved: () => {
        advanced = true;
      },
    }),
    /save failed/,
  );

  assert.equal(advanced, false);
});

test("a configured instance shows every root of each default library", () => {
  const libraries = setupMediaLibraries([
    library("movies", "MOVIE", ["/media/films", "/media/films-4k"]),
    library("kids", "MOVIE", ["/media/kids"], false),
    library("series", "SERIES", ["/media/tv"]),
  ]);
  const roots = setupMediaRootsFromLibraries(libraries);

  assert.deepEqual(roots.movies, [
    { path: "/media/films", isDefault: true },
    { path: "/media/films-4k", isDefault: false },
  ]);
  assert.deepEqual(roots.series, [{ path: "/media/tv", isDefault: true }]);
  // No default anime library was read, so the bootstrap folder stands in.
  assert.deepEqual(roots.anime, defaultRoots.anime);
});

test("editing roots keeps one default and no duplicates", () => {
  let roots = addSetupRoot([], "/media/films");
  assert.deepEqual(roots, [{ path: "/media/films", isDefault: true }]);

  roots = addSetupRoot(roots, "/media/films-4k");
  roots = addSetupRoot(roots, "/media/films");
  assert.equal(roots.length, 2);

  roots = setDefaultSetupRoot(roots, 1);
  assert.deepEqual(roots.map((root) => root.isDefault), [false, true]);

  assert.equal(replaceSetupRoot(roots, 0, "/media/films-4k"), roots);
  roots = replaceSetupRoot(roots, 0, "/media/movies");
  assert.deepEqual(roots.map((root) => root.path), ["/media/movies", "/media/films-4k"]);

  roots = removeSetupRoot(roots, 1);
  assert.deepEqual(roots, [{ path: "/media/movies", isDefault: true }]);
});

test("setup saves only the libraries whose roots changed", () => {
  const libraries: SetupMediaLibraries = setupMediaLibraries([
    library("movies", "MOVIE", ["/media/films", "/media/films-4k"]),
    library("series", "SERIES", ["/media/tv"]),
    library("anime", "ANIME", ["/media/anime"]),
  ]);
  const roots = setupMediaRootsFromLibraries(libraries);

  assert.deepEqual(plannedSetupRootSaves(roots, libraries), []);

  assert.deepEqual(
    plannedSetupRootSaves(
      {
        movies: setDefaultSetupRoot(roots.movies, 1),
        series: addSetupRoot(roots.series, "/media/tv-2"),
        // Emptied: a library keeps at least one root, so it is left alone.
        anime: [],
      },
      libraries,
    ),
    [
      {
        field: "movies",
        libraryId: "movies",
        roots: [
          { path: "/media/films", isDefault: false },
          { path: "/media/films-4k", isDefault: true },
        ],
      },
      {
        field: "series",
        libraryId: "series",
        roots: [
          { path: "/media/tv", isDefault: true },
          { path: "/media/tv-2", isDefault: false },
        ],
      },
    ],
  );
});
