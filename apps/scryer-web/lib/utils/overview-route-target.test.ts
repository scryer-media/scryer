import assert from "node:assert/strict";
import test from "node:test";
import { overviewTargetFromNavigation } from "./overview-route-target.ts";

const route = { view: "anime", librarySlug: "anime", slug: "second-title" };
const target = {
  view: "anime", librarySlug: "anime", slug: "second-title",
  id: "second", libraryId: "library",
};

test("clicked titles resolve immediately without clearing the catalog selection", () => {
  assert.deepEqual(overviewTargetFromNavigation({ scryerOverviewTarget: target }, route), {
    id: "second", libraryId: "library", librarySlug: "anime", slug: "second-title",
  });
});

test("previous titles and other libraries cannot seed a new route", () => {
  for (const change of [
    { view: "series" }, { librarySlug: "other-library" }, { slug: "first-title" },
  ]) {
    assert.equal(overviewTargetFromNavigation({
      scryerOverviewTarget: { ...target, ...change },
    }, route), null);
  }
});

test("direct links and malformed navigation state still use server resolution", () => {
  for (const state of [null, undefined, false, {}, { scryerOverviewTarget: null },
    { scryerOverviewTarget: { ...target, id: 42 } },
    { scryerOverviewTarget: { ...target, id: " " } }]) {
    assert.equal(overviewTargetFromNavigation(state, route), null);
  }
  assert.equal(overviewTargetFromNavigation({ scryerOverviewTarget: target }, {
    ...route, slug: null,
  }), null);
});
