import assert from "node:assert/strict";
import test from "node:test";

import {
  findConflictingLibraryNamesByRootPath,
  normalizeComparableLibraryRootPath,
  normalizeLibraryRootDrafts,
  trimLibraryRootPath,
  validateLibraryRootPaths,
} from "./library-root-validation.ts";

test("library root normalization preserves filesystem roots", () => {
  assert.equal(trimLibraryRootPath(" / "), "/");
  assert.equal(trimLibraryRootPath("C:\\"), "C:\\");
  assert.equal(trimLibraryRootPath("C:/"), "C:/");
  assert.equal(
    trimLibraryRootPath("\\\\server\\share\\"),
    "\\\\server\\share\\",
  );
  assert.equal(trimLibraryRootPath("/media/movies/"), "/media/movies");
  assert.equal(normalizeComparableLibraryRootPath("C:\\", "windows"), "c:/");
  assert.equal(
    normalizeComparableLibraryRootPath("\\\\SERVER\\Share\\", "windows"),
    "//server/share",
  );
});

test("library root comparison follows the runtime filesystem style", () => {
  assert.notEqual(
    normalizeComparableLibraryRootPath("/data/TV", "unix"),
    normalizeComparableLibraryRootPath("/data/tv", "unix"),
  );
  assert.equal(
    normalizeComparableLibraryRootPath("C:\\Media\\TV", "windows"),
    normalizeComparableLibraryRootPath("c:/media/tv", "windows"),
  );
  assert.notEqual(
    normalizeComparableLibraryRootPath("/data/TV"),
    normalizeComparableLibraryRootPath("/data/tv"),
  );
});

test("draft dedupe preserves Unix case variants and folds Windows variants", () => {
  const roots = [
    { path: "/data/TV", isDefault: true },
    { path: "/data/tv", isDefault: false },
  ];
  assert.equal(normalizeLibraryRootDrafts(roots, "unix").length, 2);
  assert.equal(normalizeLibraryRootDrafts(roots, "windows").length, 1);
});

test("cross-library conflicts and invalid badges respect Unix case", () => {
  const conflicts = findConflictingLibraryNamesByRootPath(
    [{ path: "/data/TV" }, { path: "/data/tv" }],
    [
      {
        id: "other",
        name: "Other Series",
        roots: [{ path: "/data/tv" }],
      },
    ],
    null,
    "unix",
  );
  assert.equal(conflicts.has("/data/TV"), false);
  assert.deepEqual(conflicts.get("/data/tv"), ["Other Series"]);

  const invalidPathKeys = new Set([
    normalizeComparableLibraryRootPath("/data/tv", "unix"),
  ]);
  assert.equal(
    invalidPathKeys.has(normalizeComparableLibraryRootPath("/data/TV", "unix")),
    false,
  );
  assert.equal(
    invalidPathKeys.has(normalizeComparableLibraryRootPath("/data/tv", "unix")),
    true,
  );
});

test("root validation distinguishes invalid paths from unavailable validation", async () => {
  const result = await validateLibraryRootPaths(
    ["/valid", "/missing", "/unreadable", "/unknown"],
    async (path) => {
      if (path === "/missing" || path === "/unreadable") {
        return {
          graphQLErrors: [{ extensions: { code: "VALIDATION_ERROR" } }],
        };
      }
      if (path === "/unknown") {
        return { graphQLErrors: [{ extensions: { code: "RATE_LIMITED" } }] };
      }
      return null;
    },
  );

  assert.deepEqual(result.invalidPaths.sort(), ["/missing", "/unreadable"]);
  assert.deepEqual(result.validPaths, ["/valid"]);
  assert.equal(result.unavailable, true);
});

test("root validation limits concurrent requests", async () => {
  let active = 0;
  let peak = 0;
  await validateLibraryRootPaths(
    Array.from({ length: 12 }, (_, index) => `/path-${index}`),
    async () => {
      active += 1;
      peak = Math.max(peak, active);
      await new Promise((resolve) => setTimeout(resolve, 1));
      active -= 1;
      return null;
    },
    4,
  );
  assert.equal(peak, 4);
});
