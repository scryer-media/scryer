import assert from "node:assert/strict";
import test from "node:test";

import {
  catalogRootValidationState,
  configuredCatalogLibraries,
  resolveCatalogSurfacePhase,
} from "./catalog-bootstrap-policy.ts";

test("existing catalog content takes precedence over invalid roots", () => {
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: true,
      loadedTitleCount: 3,
      rootValidationState: "invalid",
    }),
    "content",
  );
});

test("an empty catalog distinguishes invalid, valid, and unavailable roots", () => {
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: true,
      loadedTitleCount: 0,
      rootValidationState: "invalid",
    }),
    "rootsInvalid",
  );
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: true,
      loadedTitleCount: 0,
      rootValidationState: "valid",
    }),
    "empty",
  );
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: true,
      loadedTitleCount: 0,
      rootValidationState: "unavailable",
    }),
    "empty",
  );
});

test("mixed roots remain usable and unavailable validation does not block", () => {
  assert.equal(
    catalogRootValidationState({
      validPaths: ["/data/TV"],
      invalidPaths: ["/data/tv"],
      unavailable: false,
    }),
    "valid",
  );
  assert.equal(
    catalogRootValidationState({
      validPaths: [],
      invalidPaths: ["/data/tv"],
      unavailable: true,
    }),
    "unavailable",
  );
  assert.equal(
    catalogRootValidationState({
      validPaths: [],
      invalidPaths: ["/data/tv"],
      unavailable: false,
    }),
    "invalid",
  );
});

test("a missing configured root replaces only an empty catalog", () => {
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: false,
      loadedTitleCount: null,
      rootValidationState: "notRun",
    }),
    "resolving",
  );
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: false,
      loadedTitleCount: 0,
      rootValidationState: "notRun",
    }),
    "rootsMissing",
  );
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: false,
      loadedTitleCount: 4,
      rootValidationState: "notRun",
    }),
    "content",
  );
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: false,
      hasConfiguredRoots: false,
      loadedTitleCount: 1,
      rootValidationState: "notRun",
    }),
    "content",
  );
});

test("bootstrap roots are demoted only after validation confirms they are missing", () => {
  const libraries = [
    {
      isBootstrapDefaultRootSet: true,
      roots: [{ path: "/data/anime" }],
    },
  ];
  const beforeValidation = configuredCatalogLibraries(libraries);
  assert.equal(beforeValidation.length, 1);
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: beforeValidation.length > 0,
      loadedTitleCount: 12,
      rootValidationState: "notRun",
    }),
    "content",
  );

  const afterMissingValidation = configuredCatalogLibraries(libraries, [
    "/data/anime",
  ]);
  assert.deepEqual(afterMissingValidation, []);
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: afterMissingValidation.length > 0,
      loadedTitleCount: 0,
      rootValidationState: "invalid",
    }),
    "rootsMissing",
  );
});

test("explicit invalid roots remain configured", () => {
  const configured = configuredCatalogLibraries(
    [
      {
        isBootstrapDefaultRootSet: false,
        roots: [{ path: "/data/missing" }],
      },
    ],
    ["/data/missing"],
  );
  assert.equal(configured.length, 1);
});
