import assert from "node:assert/strict";
import test from "node:test";

import {
  graphPosterBounds,
  graphPosterInViewport,
  graphPosterHoverable,
  graphRelationOpacity,
  changedGraphViewportEdges,
  createGraphPosterAdmission,
  prepareGraphPosterImages,
} from "./library-graph-posters.ts";

test("viewport relations require both poster bounds, including partially visible posters", () => {
  const visible = (point: { x: number; y: number }) =>
    graphPosterInViewport(point, 30, 800, 600);
  const relation = (a: { x: number; y: number }, b: { x: number; y: number }) =>
    visible(a) && visible(b);
  const center = { x: 400, y: 300 };
  assert.equal(relation(center, { x: 790, y: 590 }), true);
  assert.equal(relation(center, { x: -10, y: 300 }), true);
  assert.equal(relation(center, { x: -21, y: 300 }), false);
  assert.equal(relation({ x: -50, y: 300 }, { x: 850, y: 300 }), false);
  assert.equal(relation(center, { x: 400, y: 631 }), false);
  assert.equal(graphPosterInViewport(center, 30, 0, 600), false);
});

test("hover requires a loaded readable poster instead of a dot or placeholder", () => {
  assert.equal(graphPosterHoverable(2, 1, true), false);
  assert.equal(graphPosterHoverable(30, 1, false), false);
  assert.equal(graphPosterHoverable(30, 0.2, true), false);
  assert.equal(graphPosterHoverable(9, 0.5, true), true);
  assert.equal(graphPosterHoverable(30, 1, true), true);
  assert.equal(graphPosterHoverable(2, 1, true), false);
});

test("relations fade reversibly after dots become readable posters", () => {
  assert.equal(graphRelationOpacity(9), 0);
  assert.equal(graphRelationOpacity(16.5), 0.5);
  assert.equal(graphRelationOpacity(24), 1);
  assert.equal(graphRelationOpacity(100), 1);
  assert.equal(graphRelationOpacity(100, 0), 0);
  assert.equal(graphRelationOpacity(24, 0.5), 0.5);
  const sizes = Array.from({ length: 121 }, (_, index) => index / 4);
  const forward = sizes.map((size) => graphRelationOpacity(size));
  assert.deepEqual(
    [...sizes].reverse().map((size) => graphRelationOpacity(size)),
    [...forward].reverse(),
  );
  forward.forEach((opacity, index) => {
    assert.ok(opacity >= 0 && opacity <= 1);
    assert.equal(opacity * 64, Math.round(opacity * 64));
    if (index) assert.ok(opacity >= forward[index - 1]);
  });
});

test("camera frames only update edges adjacent to viewport crossings or fade changes", () => {
  const previous = new Set(Array.from({ length: 600 }, (_, i) => `node:${i}`));
  const visited: string[] = [];
  const incident = (key: string) => {
    visited.push(key);
    return ["shared", `edge:${key}`];
  };
  assert.deepEqual(
    changedGraphViewportEdges(previous, new Set(previous), incident),
    [],
  );
  assert.deepEqual(visited, []);
  const next = new Set(previous);
  next.delete("node:0");
  next.add("node:600");
  assert.deepEqual(changedGraphViewportEdges(previous, next, incident), [
    "shared",
    "edge:node:0",
    "edge:node:600",
  ]);
  assert.deepEqual(visited, ["node:0", "node:600"]);
  visited.length = 0;
  assert.deepEqual(
    changedGraphViewportEdges(next, next, incident, ["node:2"]),
    ["shared", "edge:node:2"],
  );
  assert.deepEqual(visited, ["node:2"]);
});

class TestPosterImage {
  complete = false;
  naturalWidth = 0;
  src = "";
  decoding = "async";
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
}

test("every visible poster finishes beyond the old 240-title cutoff with bounded requests", () => {
  const cache = new Map<string, HTMLImageElement>();
  const visible = new Set(Array.from({ length: 600 }, (_, i) => `poster:${i}`));
  const images: TestPosterImage[] = [];
  let changes = 0;
  const reconcile = () =>
    prepareGraphPosterImages(
      cache,
      visible,
      () => changes++,
      () => {
        const image = new TestPosterImage();
        images.push(image);
        return image as unknown as HTMLImageElement;
      },
    );
  for (let batch = 0; batch < 50; batch++) {
    reconcile();
    const pending = images.filter((image) => !image.complete);
    assert.equal(pending.length, 12);
    for (const image of pending) {
      image.complete = true;
      image.naturalWidth = 70;
      image.onload?.();
    }
  }
  reconcile();
  assert.equal(changes, 600);
  assert.equal(cache.size, 600);
  assert.equal(images.length, 600);
  for (const url of visible) assert.equal(cache.get(url)?.naturalWidth, 70);
});

test("panning cancels stale loads, bounds offscreen retention, and errors free request slots", () => {
  const cache = new Map<string, HTMLImageElement>();
  const images: TestPosterImage[] = [];
  const createImage = () => {
    const image = new TestPosterImage();
    images.push(image);
    return image as unknown as HTMLImageElement;
  };
  prepareGraphPosterImages(
    cache,
    new Set(["old-pending"]),
    () => {},
    createImage,
    2,
    2,
  );
  const oldPending = images[0];
  for (let i = 0; i < 5; i++) {
    const image = new TestPosterImage();
    image.complete = true;
    cache.set(`offscreen:${i}`, image as unknown as HTMLImageElement);
  }
  const visible = new Set(["new:1", "new:2", "new:3"]);
  let changes = 0;
  const reconcile = () =>
    prepareGraphPosterImages(
      cache,
      visible,
      () => changes++,
      createImage,
      2,
      2,
    );
  reconcile();
  assert.equal(oldPending.src, "");
  assert.equal(oldPending.onload, null);
  assert.equal(oldPending.onerror, null);
  assert.equal([...cache.keys()].filter((url) => !visible.has(url)).length, 2);
  assert.equal(cache.has("new:3"), false);
  const failed = images[1];
  failed.complete = true;
  failed.onerror?.();
  reconcile();
  assert.equal(changes, 1);
  assert.equal(cache.has("new:3"), true);
  assert.equal(
    [...cache.values()].filter((image) => !image.complete).length,
    2,
  );
});

test("poster admission caps pending images and excludes offscreen or zoomed-out titles", () => {
  const admit = createGraphPosterAdmission(800, 600, 8, 240);
  for (let index = 0; index < 10_000; index++) {
    assert.equal(admit({ x: -100, y: 300 }, 10), false);
    assert.equal(admit({ x: 400, y: 300 }, 2), false);
  }
  let requests = 0;
  for (let index = 0; index < 10_000; index++) {
    if (admit({ x: 400, y: 300 }, 10)) requests++;
  }
  assert.equal(requests, 240);
  const nextViewport = createGraphPosterAdmission(800, 600, 8, 240);
  assert.equal(nextViewport({ x: -5, y: 300 }, 10), true);
  assert.equal(
    createGraphPosterAdmission(0, 0, 0, 240)({ x: 0, y: 0 }, 10),
    false,
  );
});

test("graph posters stay centred and keep their 2:3 aspect ratio at every size", () => {
  for (const size of [2, 9, 28, 60]) {
    const box = graphPosterBounds({ x: 120, y: 80 }, size);
    assert.equal(box.width / box.height, 2 / 3);
    assert.equal((box.left + box.right) / 2, 120);
    assert.equal((box.top + box.bottom) / 2, 80);
    assert.equal(box.height, size * 2);
  }
});

test("poster corners extend beyond a circle while the side gaps remain outside", () => {
  const box = graphPosterBounds({ x: 0, y: 0 }, 60);
  // This is inside the poster, but outside the old circular click target.
  const corner = { x: 35, y: 55 };
  assert.ok(Math.hypot(corner.x, corner.y) > 60);
  assert.ok(corner.x < box.right && corner.y < box.bottom);
  // This used to be inside the circle, but lies beside the narrower poster.
  assert.ok(55 > box.right);
});

test("selection and keyboard borders surround the poster evenly", () => {
  const point = { x: 100, y: 200 };
  const poster = graphPosterBounds(point, 30);
  for (const padding of [2, 5]) {
    const border = graphPosterBounds(point, 30, padding);
    assert.equal(poster.left - border.left, padding);
    assert.equal(poster.top - border.top, padding);
    assert.equal(border.right - poster.right, padding);
    assert.equal(border.bottom - poster.bottom, padding);
    assert.ok(
      Math.abs(
        border.left + border.cornerRadius - poster.left - poster.cornerRadius,
      ) < 1e-9,
    );
    assert.ok(
      Math.abs(
        border.top + border.cornerRadius - poster.top - poster.cornerRadius,
      ) < 1e-9,
    );
  }
});

test("rounded corners scale with the poster without consuming its straight edges", () => {
  const small = graphPosterBounds({ x: 0, y: 0 }, 15);
  const large = graphPosterBounds({ x: 0, y: 0 }, 30);
  assert.ok(small.cornerRadius > 0);
  assert.ok(small.cornerRadius < small.width / 2);
  assert.equal(large.cornerRadius, small.cornerRadius * 2);
});
