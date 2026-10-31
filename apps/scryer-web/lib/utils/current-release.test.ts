import assert from "node:assert/strict";
import test from "node:test";
import { currentReleaseNames, releaseIdentityName } from "./current-release.ts";

test("current release uses the grab identity after library rename, not quality similarity", () => {
  const names = currentReleaseNames([
    {
      role: "PRIMARY",
      grabbedReleaseTitle: " Film.2025.1080p.WEB-DL-GROUP ",
      sceneName: "Film.2025.1080p.WEB-DL-OTHER",
      filePath: "/library/Film (2025)/Film.mkv",
    },
  ]);
  assert.ok(names.has(releaseIdentityName("film.2025.1080p.web-dl-group")));
  assert.ok(!names.has(releaseIdentityName("Film.2025.1080p.WEB-DL-OTHER")));
  assert.ok(!names.has("film"));
});

test("live primary and additional files identify owned releases", () => {
  assert.deepEqual(
    [
      ...currentReleaseNames([
        {
          role: "additional",
          grabbedReleaseTitle: "Old.Release",
          filePath: "/library/old.mkv",
        },
        {
          role: "primary",
          grabbedReleaseTitle: "New.Release",
          filePath: "/library/new.mkv",
        },
      ]),
    ],
    ["old.release", "new.release"],
  );
  assert.equal(currentReleaseNames([]).size, 0);
});

test("scanned files use scene or original file identity without fuzzy title matching", () => {
  assert.deepEqual(
    [
      ...currentReleaseNames([
        {
          role: "primary",
          sceneName: "Show.S01E01-GROUP",
          filePath: "/library/renamed.mkv",
        },
        {
          role: "primary",
          originalFilePath: "D:\\downloads\\Show.S01E02-GROUP.mkv",
          filePath: "/library/renamed2.mkv",
        },
      ]),
    ],
    ["show.s01e01-group", "show.s01e02-group"],
  );
});

test("the on-disk filename is the final fallback when a row carries no release evidence", () => {
  assert.deepEqual(
    [
      ...currentReleaseNames([
        { role: "primary", filePath: "/library/Film.1080p-GROUP.mkv" },
        {
          role: "primary",
          filePath: "D:\\Library\\Show.S01E01-GROUP.MKV",
          grabbedReleaseTitle: "  ",
          sceneName: null,
          originalFilePath: "",
        },
      ]),
    ],
    ["film.1080p-group", "show.s01e01-group"],
  );
});

test("the on-disk filename is ignored once any persisted release evidence exists", () => {
  assert.deepEqual(
    [
      ...currentReleaseNames([
        {
          role: "primary",
          grabbedReleaseTitle: "Grabbed.Release",
          filePath: "/library/Renamed.Film.mkv",
        },
        {
          role: "primary",
          sceneName: "Scene.Release",
          filePath: "/library/Renamed.Show.mkv",
        },
        {
          role: "primary",
          originalFilePath: "/downloads/Original.Release.mkv",
          filePath: "/library/Renamed.Other.mkv",
        },
      ]),
    ],
    ["grabbed.release", "scene.release", "original.release"],
  );
});

test("episode, season and series-movie scopes exclude unrelated owned releases", () => {
  const files = [
    {
      role: "primary",
      filePath: "/library/a.mkv",
      grabbedReleaseTitle: "Show.E01-GROUP",
      episodeId: "ep-a",
      episode: { collectionId: "season-a" },
      seriesMovieLinkIds: [],
    },
    {
      role: "additional",
      filePath: "/library/b.mkv",
      grabbedReleaseTitle: "Show.E02-GROUP",
      episodeId: "ep-b",
      episode: { collectionId: "season-b" },
      seriesMovieLinkIds: ["movie-b"],
    },
  ];
  assert.deepEqual(
    [...currentReleaseNames(files, { episodeId: "ep-a" })],
    ["show.e01-group"],
  );
  assert.deepEqual(
    [...currentReleaseNames(files, { collectionId: "season-b" })],
    ["show.e02-group"],
  );
  assert.deepEqual(
    [...currentReleaseNames(files, { seriesMovieLinkId: "movie-b" })],
    ["show.e02-group"],
  );
  assert.equal(currentReleaseNames(files, { episodeId: "missing" }).size, 0);
});

test("fallback identities strip every supported video extension", () => {
  for (const extension of [
    "mkv",
    "mp4",
    "avi",
    "wmv",
    "mov",
    "m4v",
    "ts",
    "m2ts",
    "webm",
    "flv",
    "ogv",
    "strm",
    "mpg",
    "mpeg",
    "vob",
    "iso",
  ]) {
    for (const suffix of [extension, extension.toUpperCase()]) {
      assert.deepEqual(
        [
          ...currentReleaseNames([
            {
              role: "primary",
              originalFilePath: `/downloads/Movie.Release.${suffix}`,
              filePath: "/library/renamed.mkv",
            },
            {
              role: "primary",
              originalFilePath: `D:\\downloads\\Other.Release.${suffix}`,
              filePath: "/library/renamed.mkv",
            },
          ]),
        ],
        ["movie.release", "other.release"],
        suffix,
      );
    }
  }
});

test("fallback normalization preserves unknown suffixes and persisted identities", () => {
  assert.deepEqual(
    [
      ...currentReleaseNames([
        {
          role: "primary",
          originalFilePath: "/downloads/Movie.Release",
          filePath: "/library/renamed.mkv",
        },
        {
          role: "primary",
          grabbedReleaseTitle: "Grabbed.iso",
          filePath: "/library/renamed.iso",
        },
        {
          role: "primary",
          sceneName: "Scene.iso",
          filePath: "/library/renamed.iso",
        },
      ]),
    ],
    ["movie.release", "grabbed.iso", "scene.iso"],
  );
});
