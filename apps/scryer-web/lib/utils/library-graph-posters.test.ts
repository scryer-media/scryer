import assert from "node:assert/strict";
import test from "node:test";

import {
  createGraphPosterSprites,
  drawGraphPoster,
  type GraphPosterSprites,
  graphPosterArtworkOpacity,
  graphPosterBounds,
  graphPosterInViewport,
  graphPosterHoverable,
  graphRelationOpacity,
  changedGraphViewportEdges,
  createGraphPosterAdmission,
  graphPosterUrls,
  graphPosterVariant,
  isOriginalGraphPosterUrl,
  prepareGraphPosterImages,
} from "./library-graph-posters.ts";

test("readable posters have artwork without waiting for the neighborhood crossfade", () => {
  assert.equal(graphPosterArtworkOpacity(6.3), 0);
  assert.ok(Math.abs(graphPosterArtworkOpacity(9) - 0.5) < Number.EPSILON);
  assert.equal(graphPosterArtworkOpacity(12), 1);
  assert.equal(graphPosterArtworkOpacity(30), 1);
  assert.equal(graphPosterArtworkOpacity(2, 0), 1);
  const sizes = Array.from({ length: 61 }, (_, i) => i / 2);
  const values = sizes.map((size) => graphPosterArtworkOpacity(size));
  assert.deepEqual(
    [...sizes].reverse().map((size) => graphPosterArtworkOpacity(size)),
    [...values].reverse(),
  );
  for (let i = 1; i < values.length; i++) assert.ok(values[i] >= values[i - 1]);
});

function fakeSprites() {
  const sprite = (name: string) => ({ name }) as unknown as HTMLCanvasElement;
  return {
    mask: () => sprite("mask"),
    backing: (color: string) => sprite(`backing ${color}`),
    artwork: () => sprite("artwork"),
  } as unknown as GraphPosterSprites;
}

test("zero-opacity artwork cannot leave a visible empty backing", () => {
  const drawn: unknown[] = [];
  drawGraphPoster(
    {
      globalAlpha: 0,
      drawImage: (...args: unknown[]) => drawn.push(args),
    } as unknown as CanvasRenderingContext2D,
    fakeSprites(),
    {} as HTMLImageElement,
    { x: 0, y: 0 },
    30,
    "#000",
  );
  assert.deepEqual(drawn, []);
});

test("a fading poster clears zones under its shape even with a translucent surface", () => {
  const calls: { sprite: string; opacity: number; composite: string }[] = [];
  const context = {
    globalAlpha: 0.18,
    globalCompositeOperation: "source-over",
    drawImage(sprite: { name: string }) {
      calls.push({
        sprite: sprite.name,
        opacity: this.globalAlpha,
        composite: this.globalCompositeOperation,
      });
    },
  };
  drawGraphPoster(
    context as unknown as CanvasRenderingContext2D,
    fakeSprites(),
    { naturalWidth: 70, naturalHeight: 105 } as HTMLImageElement,
    { x: 50, y: 50 },
    30,
    "rgba(8, 13, 25, 0.35)",
  );
  assert.deepEqual(calls, [
    { sprite: "mask", opacity: 1, composite: "destination-out" },
    {
      sprite: "backing rgba(8, 13, 25, 0.35)",
      opacity: 1,
      composite: "source-over",
    },
    { sprite: "artwork", opacity: 0.18, composite: "source-over" },
  ]);
  assert.equal(context.globalAlpha, 0.18);
  assert.equal(context.globalCompositeOperation, "source-over");
});

test("poster artwork is decoded before it is cut, once per image", async () => {
  let created = 0;
  let ready = 0;
  const decoding: (() => void)[] = [];
  const createCanvas = () => {
    created += 1;
    return {
      width: 0,
      height: 0,
      getContext: () =>
        ({
          beginPath() {},
          roundRect() {},
          clip() {},
          fillRect() {},
          drawImage() {},
        }) as unknown as CanvasRenderingContext2D,
    };
  };
  const sprites = createGraphPosterSprites(
    () => {
      ready += 1;
    },
    createCanvas,
    (image) =>
      new Promise((resolve) =>
        decoding.push(() =>
          resolve({
            source: image,
            width: image.naturalWidth,
            height: image.naturalHeight,
          }),
        ),
      ),
  );
  const image = { naturalWidth: 70, naturalHeight: 105 } as HTMLImageElement;
  assert.equal(sprites.artwork(image), null);
  assert.equal(sprites.artwork(image), null);
  assert.equal(decoding.length, 1);
  assert.equal(created, 0);
  decoding[0]();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(ready, 1);
  const sprite = sprites.artwork(image);
  assert.equal(sprite?.height, 105);
  assert.equal(sprite?.width, 70);
  assert.equal(sprites.artwork(image), sprite);
  assert.equal(created, 1);
});

test("poster shapes are cut once, and the backing once per colour", () => {
  let created = 0;
  const sprites = createGraphPosterSprites(
    () => {},
    () => {
      created += 1;
      return { width: 0, height: 0, getContext: () => null };
    },
  );
  sprites.mask();
  sprites.mask();
  sprites.backing("#000");
  sprites.backing("#000");
  sprites.backing("#111");
  assert.equal(created, 3);
});

test("a poster still being decoded is not drawn", () => {
  const drawn: unknown[] = [];
  const sprites = {
    ...fakeSprites(),
    artwork: () => null,
  } as unknown as GraphPosterSprites;
  const shown = drawGraphPoster(
    {
      globalAlpha: 1,
      globalCompositeOperation: "source-over",
      drawImage: (...args: unknown[]) => drawn.push(args),
    } as unknown as CanvasRenderingContext2D,
    sprites,
    {} as HTMLImageElement,
    { x: 0, y: 0 },
    30,
    "#000",
  );
  assert.equal(shown, false);
  assert.deepEqual(drawn, []);
});

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

test("posters load the smallest stored variant that stays sharp at their drawn size", () => {
  // Size is the poster's half-height in CSS pixels.
  assert.equal(graphPosterVariant(12, 1), "w70");
  assert.equal(graphPosterVariant(60, 1), "w70");
  assert.equal(graphPosterVariant(60, 2), "w250");
  assert.equal(graphPosterVariant(200, 1), "w250");
  assert.equal(graphPosterVariant(170, 2), "original");
  assert.equal(graphPosterVariant(60, 0), "w70");
  const sizes = Array.from({ length: 400 }, (_, i) => i);
  const order = ["w70", "w250", "original"];
  const ranks = sizes.map((size) => order.indexOf(graphPosterVariant(size, 2)));
  for (let i = 1; i < ranks.length; i++) assert.ok(ranks[i] >= ranks[i - 1]);
});

test("a growing poster asks for its sharper variant and keeps the others as stand-ins", () => {
  const url = "/images/media/fixture-hash/w70";
  assert.deepEqual(graphPosterUrls(url, 12, 1), [
    "/images/media/fixture-hash/w70",
    "/images/media/fixture-hash/original",
    "/images/media/fixture-hash/w250",
  ]);
  assert.deepEqual(graphPosterUrls(url, 300, 2), [
    "/images/media/fixture-hash/original",
    "/images/media/fixture-hash/w250",
    "/images/media/fixture-hash/w70",
  ]);
  // A URL without stored variants has only itself to offer.
  assert.deepEqual(graphPosterUrls("https://example.test/a.jpg", 300, 2), [
    "https://example.test/a.jpg",
  ]);
  assert.equal(isOriginalGraphPosterUrl("/images/media/h/original"), true);
  assert.equal(isOriginalGraphPosterUrl("/images/media/h/original?v=2"), true);
  assert.equal(isOriginalGraphPosterUrl("/images/media/h/w250"), false);
});

test("offscreen full-size posters keep a smaller allowance than thumbnails", () => {
  const cache = new Map<string, HTMLImageElement>();
  for (let i = 0; i < 6; i++) {
    for (const variant of ["original", "w70"]) {
      const image = new TestPosterImage();
      image.complete = true;
      image.naturalWidth = 70;
      cache.set(
        `/images/media/h${i}/${variant}`,
        image as unknown as HTMLImageElement,
      );
    }
  }
  const visible = new Set(["/images/media/h5/original"]);
  prepareGraphPosterImages(
    cache,
    visible,
    () => {},
    () => new TestPosterImage() as unknown as HTMLImageElement,
    12,
    240,
    isOriginalGraphPosterUrl,
    2,
  );
  const kept = [...cache.keys()];
  // The visible one stays, plus the two most recent offscreen originals.
  assert.deepEqual(
    kept.filter((url) => url.endsWith("/original")),
    [
      "/images/media/h3/original",
      "/images/media/h4/original",
      "/images/media/h5/original",
    ],
  );
  assert.equal(kept.filter((url) => url.endsWith("/w70")).length, 6);
});
