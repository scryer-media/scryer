import test from "node:test";
import assert from "node:assert/strict";
import { catalogDeletionRefresh } from "./catalog-deletion-refresh.ts";
import {
  createReactiveRefreshEngine,
  type DomainEvent,
} from "../reactive/domain-event-feed.ts";

test("committed deletions name the removed titles, ignoring replay and other facets", () => {
  const engine = createReactiveRefreshEngine({
    scheduler: { setTimeout: () => 1, clearTimeout: () => {} },
  });
  const removed: string[][] = [];
  let unknown = 0;
  engine.register(
    catalogDeletionRefresh("SERIES", {
      removed: (titleIds) => removed.push(titleIds),
      unknown: () => {
        unknown += 1;
      },
    }),
  );
  const event: DomainEvent = {
    sequence: 1,
    eventId: "deleted",
    eventType: "title_deleted",
    titleId: "removed",
    facet: "series",
    streamKind: "title",
    streamId: "removed",
  };
  engine.handleEvent({ ...event, sequence: 1, facet: "movie" });
  engine.flush();
  assert.deepEqual(removed, []);

  engine.handleEvent({ ...event, sequence: 2 });
  engine.handleEvent({ ...event, sequence: 3, titleId: "other" });
  engine.handleEvent({ ...event, sequence: 4 });
  engine.flush();
  assert.deepEqual(removed, [["removed", "other"]]);

  engine.handleEvent({ ...event, sequence: 4 });
  engine.flush();
  assert.equal(removed.length, 1);
  assert.equal(unknown, 0);
});

test("a degraded refresh reports unknown deletions instead of a title list", () => {
  const engine = createReactiveRefreshEngine({
    scheduler: { setTimeout: () => 1, clearTimeout: () => {} },
  });
  const removed: string[][] = [];
  let unknown = 0;
  engine.register(
    catalogDeletionRefresh("MOVIE", {
      removed: (titleIds) => removed.push(titleIds),
      unknown: () => {
        unknown += 1;
      },
    }),
  );
  engine.runAll();
  engine.flush();
  assert.equal(unknown, 1);
  assert.deepEqual(removed, []);
});
