import assert from "node:assert/strict";
import test from "node:test";
import { sortReleaseSearchResults } from "./release-search-sort.ts";
import type { Release } from "../types/index.ts";

const release = (title: string, score?: number): Release =>
  ({
    title,
    source: "fixture",
    sizeBytes: 1_000_000_000,
    ...(score === undefined
      ? {}
      : {
          qualityProfileDecision: {
            allowed: true,
            releaseScore: score,
            preferenceScore: score,
            blockCodes: [],
            scoringLog: [],
          },
        }),
  }) as Release;

test("recommended retains backend quality order despite a lower raw score", () => {
  const high = release("1080p", 600);
  const low = release("720p", 900);
  const snapshot = [high, low];
  assert.deepEqual(sortReleaseSearchResults(snapshot, "recommended", "desc"), [
    high,
    low,
  ]);
  assert.deepEqual(sortReleaseSearchResults(snapshot, "score", "desc"), [
    low,
    high,
  ]);
  assert.deepEqual(snapshot, [high, low]);
  const top = release("2160p", 500);
  assert.deepEqual(
    sortReleaseSearchResults([top, ...snapshot], "recommended", "asc"),
    [top, high, low],
  );
});

test("explicit sort ties preserve backend size-fit order and missing scores are stable", () => {
  const plausible = release("Z plausible", 500);
  const outlier = release("A outlier", 500);
  const unknown = release("Z unknown");
  const another = release("A unknown");
  assert.deepEqual(
    sortReleaseSearchResults(
      [plausible, outlier, unknown, another],
      "score",
      "desc",
    ),
    [plausible, outlier, unknown, another],
  );
  assert.deepEqual(
    sortReleaseSearchResults([plausible, outlier], "size", "desc"),
    [plausible, outlier],
  );
});
