import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { createCatalogSummaryLoader } from "./catalog-summary-loader.ts";
import {
  buildTitlesQuery,
  titleCatalogCountsQuery,
  titleCatalogManagedBytesQuery,
  titlesQuery,
} from "../graphql/queries.ts";
import { titleCatalogProjectionForTable } from "./title-catalog-query.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test("scope changes suppress old results and coalesce to the latest scope", async () => {
  const loader = createCatalogSummaryLoader(() =>
    assert.fail("unexpected failure"),
  );
  const first = deferred<() => void>();
  const seen: string[] = [];
  const pending = loader.run("a", () => first.promise);
  assert.equal(
    loader.run("a", async () => {
      throw new Error("duplicate");
    }),
    pending,
  );
  loader.run("b", async () => () => seen.push("b"));
  loader.run("c", async () => () => seen.push("c"));
  first.resolve(() => seen.push("a"));
  await pending;
  assert.deepEqual(seen, ["c"]);
  await loader.run("c", async () => {
    throw new Error("cached scope refetched");
  });
});

/** Lets queued promise callbacks run; setImmediate is not mocked. */
function flush() {
  return new Promise<void>((resolve) => setImmediate(resolve));
}

async function settle() {
  for (let i = 0; i < 10; i += 1) await flush();
}

function fakeClock(t: TestContext) {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 0 });
  return async (ms: number) => {
    t.mock.timers.tick(ms);
    await settle();
  };
}

test("mutation invalidation publishes the finished load, reloads after the gap, and bytes stay independent", async (t) => {
  const advance = fakeClock(t);
  const errors: unknown[] = [];
  const counts = createCatalogSummaryLoader((error) => errors.push(error), {
    now: Date.now,
  });
  const bytes = createCatalogSummaryLoader((error) => errors.push(error), {
    now: Date.now,
  });
  const first = deferred<() => void>();
  const bytesResult = deferred<() => void>();
  const seen: string[] = [];
  const pending = counts.run("scope", () => first.promise);
  const bytesPending = bytes.run("library", () => bytesResult.promise);
  await settle();
  counts.run("scope", async () => () => seen.push("fresh"), true);
  first.resolve(() => seen.push("first"));
  await settle();
  assert.deepEqual(seen, ["first"]);
  await advance(1_499);
  assert.deepEqual(seen, ["first"]);
  await advance(1);
  await pending;
  assert.deepEqual(seen, ["first", "fresh"]);
  const failing = counts.run(
    "scope",
    async () => {
      throw new Error("offline");
    },
    true,
  );
  await advance(1_500);
  await failing;
  assert.equal(errors.length, 1);
  assert.deepEqual(seen, ["first", "fresh"]);
  bytes.dispose();
  bytesResult.resolve(() => seen.push("late bytes"));
  await bytesPending;
  assert.deepEqual(seen, ["first", "fresh"]);
});

test("a slow load doubles the gap before the next same-scope load", async (t) => {
  const advance = fakeClock(t);
  const loader = createCatalogSummaryLoader(
    () => assert.fail("unexpected failure"),
    { now: Date.now },
  );
  const slow = deferred<() => void>();
  const starts: number[] = [];
  const seen: string[] = [];
  const pending = loader.run("Emberfall", () => {
    starts.push(Date.now());
    return slow.promise;
  });
  await settle();
  loader.run(
    "Emberfall",
    async () => {
      starts.push(Date.now());
      return () => seen.push("second");
    },
    true,
  );
  await advance(1_000);
  slow.resolve(() => seen.push("first"));
  await settle();
  assert.deepEqual(seen, ["first"]);
  await advance(999);
  assert.deepEqual(seen, ["first"]);
  await advance(1);
  await pending;
  assert.deepEqual(seen, ["first", "second"]);
  assert.deepEqual(starts, [0, 2_000]);
});

test("a scope change skips the gap and drops the old scope's reload", async (t) => {
  const advance = fakeClock(t);
  const loader = createCatalogSummaryLoader(
    () => assert.fail("unexpected failure"),
    { now: Date.now },
  );
  const seen: string[] = [];
  const pending = loader.run("Emberfall", async () => () => seen.push("a1"));
  await settle();
  loader.run("Emberfall", async () => () => seen.push("a2"), true);
  await advance(100);
  assert.deepEqual(seen, ["a1"]);
  loader.run("Cinderwake", async () => () => seen.push("b"));
  await settle();
  await pending;
  assert.deepEqual(seen, ["a1", "b"]);
  assert.equal(Date.now(), 100);
});

test("a stream of invalidations publishes periodically and always runs the final load", async (t) => {
  const advance = fakeClock(t);
  const loader = createCatalogSummaryLoader(
    () => assert.fail("unexpected failure"),
    { now: Date.now },
  );
  const seen: number[] = [];
  let loads = 0;
  const load = (value: number) => async () => {
    loads += 1;
    return () => seen.push(value);
  };
  let latest = loader.run("Ashgrove", load(0));
  await settle();
  for (let event = 1; event <= 30; event += 1) {
    await advance(100);
    latest = loader.run("Ashgrove", load(event), true);
    await settle();
  }
  // Three seconds of events, one every 100 ms: loads start at 0, 1.5 s and
  // 3 s with the latest event queued at that moment, then the final load.
  assert.deepEqual(seen, [0, 14, 29]);
  await advance(1_500);
  await latest;
  assert.deepEqual(seen, [0, 14, 29, 30]);
  assert.equal(loads, 4);
});

test("disposing during the gap cancels the reload", async (t) => {
  const advance = fakeClock(t);
  const loader = createCatalogSummaryLoader(
    () => assert.fail("unexpected failure"),
    { now: Date.now },
  );
  const seen: string[] = [];
  const pending = loader.run("Emberfall", async () => () => seen.push("a1"));
  await settle();
  loader.run("Emberfall", async () => () => seen.push("a2"), true);
  await advance(100);
  loader.dispose();
  await pending;
  await advance(5_000);
  assert.deepEqual(seen, ["a1"]);
});

test("a reload asked for at any moment as a load finishes still runs once after it", async (t) => {
  const advance = fakeClock(t);
  // Each pass lands the reload a different number of microtasks after the
  // first load's answer, so every point of the finishing run is covered.
  for (let ticks = 0; ticks <= 12; ticks += 1) {
    const loader = createCatalogSummaryLoader(
      () => assert.fail("unexpected failure"),
      { now: Date.now },
    );
    const first = deferred<() => void>();
    const seen: string[] = [];
    let reloads = 0;
    loader.run("Duskmere", () => first.promise);
    await settle();
    first.resolve(() => seen.push("first"));
    for (let i = 0; i < ticks; i += 1) await Promise.resolve();
    const reload = loader.run(
      "Duskmere",
      async () => {
        reloads += 1;
        return () => seen.push("reload");
      },
      true,
    );
    // Let the gap's timer be set before the clock moves.
    await settle();
    assert.deepEqual(seen, ["first"], `ticks=${ticks}`);
    await advance(1_500);
    await reload;
    assert.deepEqual(seen, ["first", "reload"], `ticks=${ticks}`);
    assert.equal(reloads, 1, `ticks=${ticks}`);
    loader.dispose();
  }
});

test("a scope that changes and comes back during the gap still waits it out", async (t) => {
  const advance = fakeClock(t);
  const loader = createCatalogSummaryLoader(
    () => assert.fail("unexpected failure"),
    { now: Date.now },
  );
  const seen: string[] = [];
  const starts: number[] = [];
  const pending = loader.run("Emberfall", async () => () => seen.push("a1"));
  await settle();
  loader.run("Emberfall", async () => () => seen.push("a2"), true);
  await advance(100);
  loader.run("Cinderwake", async () => () => seen.push("b"));
  loader.run("Emberfall", async () => {
    starts.push(Date.now());
    return () => seen.push("a3");
  });
  await settle();
  assert.deepEqual(seen, ["a1"]);
  await advance(1_399);
  assert.deepEqual(seen, ["a1"]);
  await advance(1);
  await pending;
  assert.deepEqual(seen, ["a1", "a3"]);
  assert.deepEqual(starts, [1_500]);
});

test("poster pages for all facets request no table enrichment or catalog aggregates", () => {
  for (const facet of ["movie", "series", "anime"]) {
    const projection = titleCatalogProjectionForTable({
      facet,
      visibleColumns: {},
      sort: { key: "name", direction: "asc" },
    });
    assert.ok(Object.values(projection).every((value) => !value));
    const query = buildTitlesQuery(
      { ...projection, includeSettings: false },
      { includeAggregates: false },
    );
    assert.match(query, /hasMore/);
    assert.match(query, /tags/);
    assert.match(buildTitlesQuery(projection), /effectiveUseSeasonFolders/);
    assert.doesNotMatch(
      query,
      /totalCount|filterCounts|managedBytes|episodesOwned|sizeBytes|currentQualityTier|effectiveMetadataLanguage|metadataLanguageOverride|inheritsMetadataLanguage|effectiveUseSeasonFolders|inheritsUseSeasonFolders|useSeasonFoldersOverride/,
    );
  }
  assert.doesNotMatch(titleCatalogCountsQuery, /items|hasMore|managedBytes/);
  // The title picker shows matches only, so it asks for no catalog aggregates.
  assert.match(titlesQuery, /items/);
  assert.doesNotMatch(titlesQuery, /totalCount|filterCounts|managedBytes/);
  assert.doesNotMatch(
    titleCatalogManagedBytesQuery,
    /items|hasMore|totalCount|filterCounts/,
  );
});
