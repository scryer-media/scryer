import assert from "node:assert/strict";
import test from "node:test";
import {
  drawGraphRelations,
  graphRelationSegment,
  hitGraphRelation,
  type GraphRelation,
  type GraphRelationNode,
} from "./library-graph-relations.ts";

function canvas() {
  const strokes: {
    color: string;
    opacity: number;
    width: number;
    segments: number;
  }[] = [];
  const context = {
    strokeStyle: "",
    globalAlpha: 1,
    lineWidth: 1,
    lineCap: "butt",
    segments: 0,
    save() {},
    restore() {},
    beginPath() {
      this.segments = 0;
    },
    moveTo() {},
    lineTo() {
      this.segments++;
    },
    stroke() {
      strokes.push({
        color: this.strokeStyle,
        opacity: this.globalAlpha,
        width: this.lineWidth,
        segments: this.segments,
      });
    },
  };
  return { context: context as unknown as CanvasRenderingContext2D, strokes };
}

const node = (x: number, y = 0, size = 15, opacity = 1): GraphRelationNode => ({
  point: { x, y },
  size,
  opacity,
});
const relation = (key = "ab", from = "a", to = "b"): GraphRelation => ({
  key,
  from,
  to,
  color: "#718096",
  size: 1,
  kinds: ["semantic"],
});

test("relations stop at poster edges and skip overlapping or coincident posters", () => {
  assert.deepEqual(graphRelationSegment(node(0), node(100)), {
    start: { x: 10, y: 0 },
    end: { x: 90, y: 0 },
  });
  assert.deepEqual(graphRelationSegment(node(0), node(100, 100)), {
    start: { x: 10, y: 10 },
    end: { x: 90, y: 90 },
  });
  assert.equal(graphRelationSegment(node(0), node(10)), null);
  assert.equal(graphRelationSegment(node(0), node(0)), null);
});

test("drawing admits only relations with two visible endpoints and nonzero fade", () => {
  const { context, strokes } = canvas();
  const nodes = new Map([
    ["a", node(0)],
    ["b", node(100, 0, 15, 0.5)],
  ]);
  const edges = [relation(), relation("ac", "a", "c")];
  const hits = drawGraphRelations(context, edges, nodes, null);
  assert.deepEqual(
    hits.map((hit) => hit.key),
    ["ab"],
  );
  assert.equal(strokes[0].opacity, 0.5);
  nodes.set("b", node(100, 0, 15, 0));
  assert.equal(drawGraphRelations(context, edges, nodes, null).length, 0);
});

test("dense visible relations use bounded stroke batches instead of one draw per edge", () => {
  const { context, strokes } = canvas();
  const nodes = new Map([
    ["a", node(0)],
    ["b", node(100)],
  ]);
  const edges = Array.from({ length: 1_000 }, (_, index) =>
    relation(String(index)),
  );
  assert.equal(drawGraphRelations(context, edges, nodes, null).length, 1_000);
  assert.equal(strokes.length, 8);
  assert.equal(
    strokes.reduce((sum, stroke) => sum + stroke.segments, 0),
    1_000,
  );
  assert.ok(
    strokes.every(
      (stroke) =>
        stroke.segments <= 128 &&
        stroke.color === "#718096" &&
        stroke.opacity === 1 &&
        stroke.width === 1,
    ),
  );
});

test("active highlights keep the same opacity across zoom and zero overview fade", () => {
  for (const scale of [0.25, 0.5, 1, 2, 4]) {
    for (const fade of [0, 0.01, 0.5, 1]) {
      const { context, strokes } = canvas();
      const nodes = new Map([
        ["a", node(0, 0, 15 * scale, fade)],
        ["b", node(100 * scale, 0, 15 * scale, fade)],
      ]);
      const hits = drawGraphRelations(
        context,
        [relation(), relation("missing", "a", "offscreen")],
        nodes,
        { key: "a", accent: "#ffffff" },
      );
      assert.equal(hits.length, 1);
      assert.equal(hits[0].opacity, 0.9);
      assert.deepEqual(strokes, [
        { color: "#ffffff", opacity: 0.9, width: 2, segments: 1 },
      ]);
    }
  }
});

test("node focus highlights only its own connections without changing the relation store", () => {
  const { context, strokes } = canvas();
  const nodes = new Map([
    ["a", node(0)],
    ["b", node(100)],
    ["c", node(200)],
  ]);
  const edges = [relation(), relation("bc", "b", "c")];
  const hits = drawGraphRelations(context, edges, nodes, {
    key: "a",
    accent: "#ffffff",
  });
  assert.deepEqual(
    hits.map((hit) => hit.key),
    ["ab"],
  );
  assert.deepEqual(strokes, [
    { color: "#ffffff", opacity: 0.9, width: 2, segments: 1 },
  ]);
  assert.equal(drawGraphRelations(context, edges, nodes, null).length, 2);
});

test("relation hover hits the painted segment only when both endpoint posters are readable", () => {
  const { context } = canvas();
  const hits = drawGraphRelations(
    context,
    [relation()],
    new Map([
      ["a", node(0)],
      ["b", node(100)],
    ]),
    null,
  );
  const ready = new Set(["a", "b"]);
  assert.equal(hitGraphRelation(hits, { x: 50, y: 2 }, ready)?.key, "ab");
  assert.equal(hitGraphRelation(hits, { x: 50, y: 5 }, ready), null);
  assert.equal(hitGraphRelation(hits, { x: 50, y: 0 }, new Set(["a"])), null);
  assert.equal(hitGraphRelation(hits, { x: 0, y: 0 }, ready), null);
});
