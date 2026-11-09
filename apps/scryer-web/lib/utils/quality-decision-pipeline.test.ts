import test from "node:test";
import assert from "node:assert/strict";
import {
  DECISION_PIPELINE_PRESETS,
  DEFAULT_DECISION_PIPELINE,
  addDecisionGate,
  availableDecisionGates,
  decisionPipelineForIgnoreProperRepack,
  decisionPipelineNotices,
  decisionPipelinePresetId,
  decisionPipelineUsesTierCutoff,
  describeDecisionPipeline,
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
    "PREFERRED_LANGUAGE",
    "OPTIONAL_LANGUAGES",
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
  assert.deepEqual(availableDecisionGates(DEFAULT_DECISION_PIPELINE), []);
  assert.deepEqual(availableDecisionGates(["REVISION", "FUTURE_GATE"]), [
    "QUALITY_TIER",
    "PREFERRED_LANGUAGE",
    "OPTIONAL_LANGUAGES",
  ]);
});

test("presets are recognised only by exact order", () => {
  for (const preset of DECISION_PIPELINE_PRESETS) {
    assert.equal(decisionPipelinePresetId(preset.pipeline), preset.id);
  }
  assert.deepEqual(
    DECISION_PIPELINE_PRESETS.find((preset) => preset.id === "language_first")
      ?.pipeline,
    ["PREFERRED_LANGUAGE", "QUALITY_TIER", "REVISION", "OPTIONAL_LANGUAGES"],
  );
  assert.equal(decisionPipelinePresetId(["REVISION", "QUALITY_TIER"]), null);
});

test("notices follow tier, revision and language positions", () => {
  assert.deepEqual(decisionPipelineNotices(DEFAULT_DECISION_PIPELINE), [
    "language_needs_library_languages",
  ]);
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

test("the summary chains each step in order and handles score only", () => {
  assert.equal(
    describeDecisionPipeline([], translate),
    "qualityProfile.decisionSummaryScoreOnly",
  );
  assert.equal(
    describeDecisionPipeline(["QUALITY_TIER"], translate),
    `qualityProfile.decisionSummary(${JSON.stringify({
      steps: "qualityProfile.decisionGateQualityTierSummary",
    })})`,
  );
  const steps: string[] = [];
  const recording = (key: string, values?: Record<string, string | number>) => {
    if (key === "qualityProfile.decisionSummaryJoin") {
      return `${values?.previous} > ${values?.next}`;
    }
    if (key === "qualityProfile.decisionSummary") {
      steps.push(String(values?.steps));
      return "done";
    }
    return values?.value ? `?${values.value}` : key.split(".")[1];
  };
  describeDecisionPipeline(
    ["PREFERRED_LANGUAGE", "FUTURE_GATE", "REVISION"],
    recording,
  );
  assert.deepEqual(steps, [
    "decisionGatePreferredLanguageSummary > ?FUTURE_GATE > decisionGateRevisionSummary",
  ]);
});
