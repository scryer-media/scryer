import test from "node:test";
import assert from "node:assert/strict";
import { catalogDeletionRefresh } from "./catalog-deletion-refresh.ts";
import {
  createReactiveRefreshEngine,
  type DomainEvent,
} from "../reactive/domain-event-feed.ts";

test("committed deletions refresh the matching catalog, including replay and fallback", () => {
  const engine = createReactiveRefreshEngine({
    scheduler: { setTimeout: () => 1, clearTimeout: () => {} },
  });
  let refreshes = 0;
  engine.register(
    catalogDeletionRefresh("SERIES", () => {
      refreshes += 1;
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
  assert.equal(refreshes, 0);
  engine.handleEvent({ ...event, sequence: 2 });
  engine.flush();
  assert.equal(refreshes, 1);
  engine.handleEvent({ ...event, sequence: 2 });
  engine.flush();
  assert.equal(refreshes, 1);
  engine.runAll();
  engine.flush();
  assert.equal(refreshes, 2);
});
