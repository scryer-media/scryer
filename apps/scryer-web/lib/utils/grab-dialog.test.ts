import assert from "node:assert/strict";
import test from "node:test";

import type { Release } from "@/lib/types";
import type { TitleRecord } from "@/lib/types/titles";
import {
  episodeSubjectIncomplete,
  episodeSubjectInput,
  releaseRejectionCodes,
  titleGapLabel,
  titleHoldsFile,
  titleIsEpisodic,
} from "./grab-dialog.ts";

function title(overrides: Partial<TitleRecord>): TitleRecord {
  return {
    id: "title-1",
    name: "The Expanse",
    facet: "SERIES",
    libraryId: "library-1",
    monitored: true,
    tags: [],
    ...overrides,
  } as TitleRecord;
}

function release(overrides: Partial<Release>): Release {
  return {
    source: "NZBgeek",
    title: "Some.Release.1080p",
    link: null,
    downloadUrl: "https://indexer.test/nzb/1",
    sizeBytes: 1,
    publishedAt: null,
    ...overrides,
  };
}


test("an episodic title reports its missing monitored episodes", () => {
  assert.deepEqual(
    titleGapLabel(title({ episodesMonitored: 62, episodesOwned: 58 })),
    { key: "grabDialog.gap.missing", params: { count: 4 }, complete: false },
  );
});

test("an episodic title with nothing outstanding reads complete", () => {
  assert.deepEqual(
    titleGapLabel(title({ episodesMonitored: 62, episodesOwned: 62 })),
    { key: "grabDialog.gap.complete", complete: true },
  );
});

test("owning more than is monitored never reports a negative gap", () => {
  assert.deepEqual(
    titleGapLabel(title({ episodesMonitored: 10, episodesOwned: 12 })),
    { key: "grabDialog.gap.complete", complete: true },
  );
});

test("a movie is wanted until it holds a file", () => {
  const wanted = title({ facet: "MOVIE", episodesOwned: 0, sizeBytes: 0 });
  assert.deepEqual(titleGapLabel(wanted), {
    key: "grabDialog.gap.wanted",
    complete: false,
  });
  assert.deepEqual(titleGapLabel(title({ facet: "MOVIE", sizeBytes: 42 })), {
    key: "grabDialog.gap.complete",
    complete: true,
  });
});

test("replacing a file is only offered for a target that holds one", () => {
  assert.equal(titleHoldsFile(title({ episodesOwned: 0, sizeBytes: 0 })), false);
  assert.equal(titleHoldsFile(title({ episodesOwned: 1 })), true);
  assert.equal(titleHoldsFile(title({ sizeBytes: 1 })), true);
});

test("only series and anime targets take a season/episode narrowing", () => {
  assert.equal(titleIsEpisodic(title({ facet: "SERIES" })), true);
  assert.equal(titleIsEpisodic(title({ facet: "ANIME" })), true);
  assert.equal(titleIsEpisodic(title({ facet: "MOVIE" })), false);
  assert.equal(titleIsEpisodic(null), false);
});

test("rejection codes are de-duplicated across the batch", () => {
  const codes = releaseRejectionCodes([
    release({
      qualityProfileDecision: {
        allowed: false,
        blockCodes: ["QUALITY_NOT_ALLOWED"],
        releaseScore: 0,
        preferenceScore: 0,
        scoringLog: [],
      },
    }),
    release({
      qualityProfileDecision: {
        allowed: false,
        blockCodes: ["QUALITY_NOT_ALLOWED", "SIZE_TOO_LARGE"],
        releaseScore: 0,
        preferenceScore: 0,
        scoringLog: [],
      },
    }),
    release({}),
  ]);
  assert.deepEqual(codes, ["QUALITY_NOT_ALLOWED", "SIZE_TOO_LARGE"]);
});


test("a season/episode narrowing needs a season; a season alone is the whole season", () => {
  assert.deepEqual(episodeSubjectInput("", ""), {});
  assert.deepEqual(episodeSubjectInput(" 6 ", " 4 "), {
    season: "6",
    episode: "4",
  });
  assert.deepEqual(episodeSubjectInput(" 6 ", ""), { season: "6" });
  assert.deepEqual(episodeSubjectInput("", "4"), {});
});

test("only an episode without a season is reported as incomplete", () => {
  assert.equal(episodeSubjectIncomplete("", ""), false);
  assert.equal(episodeSubjectIncomplete("6", "4"), false);
  assert.equal(episodeSubjectIncomplete("6", ""), false);
  assert.equal(episodeSubjectIncomplete("", "4"), true);
});
