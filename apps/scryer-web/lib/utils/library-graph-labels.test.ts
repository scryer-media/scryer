import assert from "node:assert/strict";
import test from "node:test";
import {
  createRectIndex,
  showOverviewTitleLabels,
} from "./library-graph-labels.ts";

test("close overview shows labels including when some nodes are tiny", () => {
  const nodes = Array.from({ length: 24 }, (_, i) => ({ r: i === 0 ? 4 : 28 }));
  assert.equal(showOverviewTitleLabels(nodes, 1200, 1000), true);
});

test("small library zoomed out keeps title labels hidden", () => {
  const nodes = Array.from({ length: 24 }, () => ({ r: 13 }));
  assert.equal(showOverviewTitleLabels(nodes, 1200, 1000), false);
});

test("crowded views hide labels even with large nodes", () => {
  const nodes = Array.from({ length: 200 }, () => ({ r: 28 }));
  assert.equal(showOverviewTitleLabels(nodes, 2400, 1800), false);
  assert.equal(showOverviewTitleLabels(nodes.slice(0, 30), 400, 400), false);
});

test("empty view has no title labels", () => {
  assert.equal(showOverviewTitleLabels([], 1200, 1000), false);
});

test("rect index answers exactly as a scan of every rectangle", () => {
  let seed = 7;
  const random = () => {
    seed = (seed * 1_103_515_245 + 12_345) % 2_147_483_648;
    return seed / 2_147_483_648;
  };
  const width = 900;
  const height = 700;
  const rects = Array.from({ length: 3_000 }, () => {
    const x = random() * (width + 200) - 100;
    const y = random() * (height + 200) - 100;
    const w = 2 + random() * 40;
    const h = 2 + random() * 40;
    return { left: x, top: y, right: x + w, bottom: y + h };
  });
  const index = createRectIndex(rects, width, height);
  for (let probe = 0; probe < 2_000; probe += 1) {
    const x = random() * width;
    const y = random() * height;
    const w = 1 + random() * 120;
    const h = 1 + random() * 30;
    const scan = rects.some(
      (rect) =>
        rect.left < x + w &&
        rect.right > x &&
        rect.top < y + h &&
        rect.bottom > y,
    );
    assert.equal(index.overlaps(x, y, x + w, y + h), scan);
  }
});

test("rect index treats touching edges as clear", () => {
  const index = createRectIndex(
    [{ left: 10, top: 10, right: 20, bottom: 20 }],
    100,
    100,
  );
  assert.equal(index.overlaps(20, 10, 30, 20), false);
  assert.equal(index.overlaps(19, 10, 30, 20), true);
});
