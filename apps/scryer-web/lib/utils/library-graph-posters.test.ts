import assert from "node:assert/strict";
import test from "node:test";

import { graphPosterBounds } from "./library-graph-posters.ts";

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
