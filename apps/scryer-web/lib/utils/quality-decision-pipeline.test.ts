import test from "node:test";
import assert from "node:assert/strict";
import {
  DEFAULT_DECISION_PIPELINE,
  addDecisionGate,
  decidedByLabel,
  availableDecisionGates,
  decisionPipelineForIgnoreProperRepack,
  decisionPipelineNotices,
  decisionPipelineUsesTierCutoff,
  ignoresProperRepack,
  moveDecisionGate,
  normalizeDecisionPipeline,
  removeDecisionGate,
} from "./quality-decision-pipeline.ts";

const translate = (key: string, values?: Record<string, string | number>) =>
  values ? `${key}(${JSON.stringify(values)})` : key;

test("normalisation trims, upper-cases, drops repeats and keeps unknown gates", () => {
  assert.deepEqual(
    normalizeDecisionPipeline([
      " revision ",
      "QUALITY_TIER",
      "REVISION",
      "",
      7,
      "FUTURE_GATE",
    ]),
    ["REVISION", "QUALITY_TIER", "FUTURE_GATE"],
  );
  assert.deepEqual(normalizeDecisionPipeline([]), []);
  assert.equal(normalizeDecisionPipeline(undefined), null);
  assert.equal(normalizeDecisionPipeline("QUALITY_TIER"), null);
});

test("the PROPER/REPACK setting is the presence of the revision step", () => {
  assert.equal(ignoresProperRepack(DEFAULT_DECISION_PIPELINE), false);
  assert.equal(ignoresProperRepack(["QUALITY_TIER"]), true);
  assert.equal(ignoresProperRepack([]), true);
  assert.deepEqual(
    decisionPipelineForIgnoreProperRepack(false),
    DEFAULT_DECISION_PIPELINE,
  );
  assert.deepEqual(decisionPipelineForIgnoreProperRepack(true), [
    "QUALITY_TIER",
  ]);
});

test("moving a gate reorders it and ignores out-of-range targets", () => {
  const order = ["QUALITY_TIER", "REVISION", "PREFERRED_LANGUAGE"];
  assert.deepEqual(moveDecisionGate(order, "PREFERRED_LANGUAGE", 0), [
    "PREFERRED_LANGUAGE",
    "QUALITY_TIER",
    "REVISION",
  ]);
  assert.deepEqual(moveDecisionGate(order, "QUALITY_TIER", 2), [
    "REVISION",
    "PREFERRED_LANGUAGE",
    "QUALITY_TIER",
  ]);
  assert.deepEqual(moveDecisionGate(order, "QUALITY_TIER", -1), order);
  assert.deepEqual(moveDecisionGate(order, "QUALITY_TIER", 3), order);
  assert.deepEqual(moveDecisionGate(order, "OPTIONAL_LANGUAGES", 0), order);
});

test("adding inserts once at the clamped position; removing drops the gate", () => {
  assert.deepEqual(addDecisionGate(["QUALITY_TIER"], "REVISION"), [
    "QUALITY_TIER",
    "REVISION",
  ]);
  assert.deepEqual(addDecisionGate(["QUALITY_TIER"], "REVISION", 0), [
    "REVISION",
    "QUALITY_TIER",
  ]);
  assert.deepEqual(addDecisionGate(["QUALITY_TIER"], "REVISION", 9), [
    "QUALITY_TIER",
    "REVISION",
  ]);
  assert.deepEqual(addDecisionGate(["QUALITY_TIER"], "QUALITY_TIER", 0), [
    "QUALITY_TIER",
  ]);
  assert.deepEqual(
    removeDecisionGate(["QUALITY_TIER", "FUTURE_GATE"], "FUTURE_GATE"),
    ["QUALITY_TIER"],
  );
});

test("available gates are the known ones not in the order", () => {
  assert.deepEqual(availableDecisionGates(DEFAULT_DECISION_PIPELINE), [
    "PREFERRED_LANGUAGE",
    "OPTIONAL_LANGUAGES",
  ]);
  assert.deepEqual(
    availableDecisionGates([
      "QUALITY_TIER",
      "REVISION",
      "PREFERRED_LANGUAGE",
      "OPTIONAL_LANGUAGES",
    ]),
    [],
  );
  assert.deepEqual(availableDecisionGates(["REVISION", "FUTURE_GATE"]), [
    "QUALITY_TIER",
    "PREFERRED_LANGUAGE",
    "OPTIONAL_LANGUAGES",
  ]);
});

test("notices follow tier, revision and language positions", () => {
  assert.deepEqual(decisionPipelineNotices(DEFAULT_DECISION_PIPELINE), []);
  assert.deepEqual(
    decisionPipelineNotices([
      ...DEFAULT_DECISION_PIPELINE,
      "PREFERRED_LANGUAGE",
    ]),
    ["language_needs_library_languages"],
  );
  assert.deepEqual(decisionPipelineNotices([]), [
    "tier_absent",
    "revision_absent",
  ]);
  assert.deepEqual(
    decisionPipelineNotices(["OPTIONAL_LANGUAGES", "QUALITY_TIER", "REVISION"]),
    ["tier_below_language", "language_needs_library_languages"],
  );
  assert.deepEqual(decisionPipelineNotices(["QUALITY_TIER", "FUTURE_GATE"]), [
    "revision_absent",
  ]);
  assert.equal(decisionPipelineUsesTierCutoff(["REVISION"]), false);
  assert.equal(
    decisionPipelineUsesTierCutoff(["REVISION", "QUALITY_TIER"]),
    true,
  );
});

test("the deciding step reads as a label, score and unknown steps included", () => {
  assert.equal(decidedByLabel(null, translate), null);
  assert.equal(decidedByLabel(undefined, translate), null);
  assert.equal(decidedByLabel("  ", translate), null);
  assert.equal(
    decidedByLabel("SCORE", translate),
    `qualityProfile.decidedBy(${JSON.stringify({
      step: "qualityProfile.decisionGateScore",
    })})`,
  );
  assert.equal(
    decidedByLabel("quality_tier", translate),
    `qualityProfile.decidedBy(${JSON.stringify({
      step: "qualityProfile.decisionGateQualityTier",
    })})`,
  );
  assert.equal(
    decidedByLabel("FUTURE_STEP", translate),
    `qualityProfile.decidedBy(${JSON.stringify({
      step: `qualityProfile.decisionGateUnknown(${JSON.stringify({ value: "FUTURE_STEP" })})`,
    })})`,
  );
});
