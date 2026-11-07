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
  setupRootsNeedStoredRoots,
  type SetupMediaLibraries,
} from "./setup-media-paths.ts";

const defaultRoots = bootstrapSetupMediaRoots();

function library(id: string, facet: string, paths: string[], isDefault = true) {
  return {
    id,
    facet: facet as "MOVIE",
    isDefault,
    roots: paths.map((path, i) => ({
      id: `${id}-${i}`,
      path,
      isDefault: i === 0,
    })),
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
    { id: "movies-0", path: "/media/films", isDefault: true },
    { id: "movies-1", path: "/media/films-4k", isDefault: false },
  ]);
  assert.deepEqual(roots.series, [
    { id: "series-0", path: "/media/tv", isDefault: true },
  ]);
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
  assert.deepEqual(
    roots.map((root) => root.isDefault),
    [false, true],
  );

  assert.equal(replaceSetupRoot(roots, 0, "/media/films-4k"), roots);
  roots = replaceSetupRoot(roots, 0, "/media/movies");
  assert.deepEqual(
    roots.map((root) => root.path),
    ["/media/movies", "/media/films-4k"],
  );

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
          { id: "movies-0", path: "/media/films", isDefault: false },
          { id: "movies-1", path: "/media/films-4k", isDefault: true },
        ],
        removedRootIds: [],
      },
      {
        field: "series",
        libraryId: "series",
        roots: [
          { id: "series-0", path: "/media/tv", isDefault: true },
          { path: "/media/tv-2", isDefault: false },
        ],
        removedRootIds: [],
      },
    ],
  );
});

test("a stored root keeps its id through a path edit and a new row has none", () => {
  const libraries: SetupMediaLibraries = setupMediaLibraries([
    library("movies", "MOVIE", ["/media/films", "/media/films-4k"]),
  ]);
  const roots = setupMediaRootsFromLibraries(libraries);
  const edited = addSetupRoot(
    setDefaultSetupRoot(replaceSetupRoot(roots.movies, 0, "/media/movies"), 1),
    "/media/movies-extra",
  );

  assert.deepEqual(
    plannedSetupRootSaves({ ...roots, movies: edited }, libraries),
    [
      {
        field: "movies",
        libraryId: "movies",
        roots: [
          { id: "movies-0", path: "/media/movies", isDefault: false },
          { id: "movies-1", path: "/media/films-4k", isDefault: true },
          { path: "/media/movies-extra", isDefault: false },
        ],
        removedRootIds: [],
      },
    ],
  );
});

test("removing a stored root row lists its id for removal", () => {
  const libraries: SetupMediaLibraries = setupMediaLibraries([
    library("movies", "MOVIE", ["/media/films", "/media/films-4k"]),
  ]);
  const roots = setupMediaRootsFromLibraries(libraries);

  assert.deepEqual(
    plannedSetupRootSaves(
      { ...roots, movies: removeSetupRoot(roots.movies, 0) },
      libraries,
    ),
    [
      {
        field: "movies",
        libraryId: "movies",
        roots: [{ id: "movies-1", path: "/media/films-4k", isDefault: true }],
        removedRootIds: ["movies-0"],
      },
    ],
  );
});

test("rows edited before the stored roots loaded are never saved over them", () => {
  const libraries: SetupMediaLibraries = setupMediaLibraries([
    library("movies", "MOVIE", ["/media/films", "/media/films-4k"]),
  ]);

  // Rows still seeded from the bootstrap folders must not be saved while a
  // default library has stored roots: they would replace those roots.
  assert.equal(setupRootsNeedStoredRoots(false, libraries), true);
  assert.equal(setupRootsNeedStoredRoots(true, libraries), false);

  // With no stored roots there is nothing to drop, so the bootstrap rows save.
  assert.equal(setupRootsNeedStoredRoots(false, {}), false);
  assert.equal(
    setupRootsNeedStoredRoots(
      false,
      setupMediaLibraries([library("movies", "MOVIE", [])]),
    ),
    false,
  );
});
