import assert from "node:assert/strict";
import test from "node:test";
import { createRailScrollGroup } from "./rail-scroll-group.ts";

/** A rail that clamps like a real scroll container. */
function rail(maxScrollLeft: number, scrollLeft = 0) {
  let position = scrollLeft;
  return {
    get scrollLeft() {
      return position;
    },
    set scrollLeft(value: number) {
      position = Math.min(value, maxScrollLeft);
    },
  };
}

test("scrolling one rail moves the others", () => {
  const group = createRailScrollGroup();
  const original = rail(500);
  const dub = rail(500);
  group.add(original);
  group.add(dub);

  original.scrollLeft = 120;
  group.scrolled(original);
  assert.equal(dub.scrollLeft, 120);

  dub.scrollLeft = 40;
  group.scrolled(dub);
  assert.equal(original.scrollLeft, 40);
});

test("a shorter rail's clamped echo does not pull the scrolled rail back", () => {
  const group = createRailScrollGroup();
  const original = rail(500);
  const dub = rail(100);
  group.add(original);
  group.add(dub);

  original.scrollLeft = 300;
  group.scrolled(original);
  assert.equal(dub.scrollLeft, 100);

  group.scrolled(dub);
  assert.equal(original.scrollLeft, 300);

  original.scrollLeft = 60;
  group.scrolled(original);
  assert.equal(dub.scrollLeft, 60);
});

test("a rail joining the group starts at the group's position", () => {
  const group = createRailScrollGroup();
  const original = rail(500, 200);
  const dub = rail(500);
  group.add(original);
  group.add(dub);
  assert.equal(dub.scrollLeft, 200);

  group.scrolled(dub);
  assert.equal(original.scrollLeft, 200);
});

test("a removed rail stops following", () => {
  const group = createRailScrollGroup();
  const original = rail(500);
  const dub = rail(500);
  group.add(original);
  const remove = group.add(dub);
  remove();

  original.scrollLeft = 80;
  group.scrolled(original);
  assert.equal(dub.scrollLeft, 0);
});
