import assert from "node:assert/strict";
import test from "node:test";
import {
  canTestRuleSet,
  buildRuleSetTestInput,
  RuleSetTestRequestController,
  ruleSetTestFingerprint,
  sizeBytesFromGib,
  shouldApplyRuleSetTestResponse,
} from "./rule-set-test-preview.ts";
import { testRuleSetMutation } from "../graphql/mutations.ts";

const draft = {
  name: "rule",
  description: "",
  regoSource: "package test",
  enabled: true,
  priority: 0,
  appliedFacets: [],
};

test("preview requires a title, release name, and an episode for episodic titles", () => {
  const movie = { titleId: "title", episodeId: null, releaseName: "x", sizeGib: "" };
  assert.equal(canTestRuleSet(movie, false), true);
  assert.equal(canTestRuleSet(movie, true), false);
  assert.equal(
    canTestRuleSet({ ...movie, episodeId: "episode", releaseName: "" }, true),
    false,
  );
});

test("preview becomes stale for either draft or input changes", () => {
  const selection = { titleId: "title", episodeId: null, releaseName: "release", sizeGib: "" };
  const baseline = ruleSetTestFingerprint(draft, selection, null, null, null);
  assert.notEqual(baseline, ruleSetTestFingerprint({ ...draft, enabled: false }, selection, null, null, null));
  assert.notEqual(baseline, ruleSetTestFingerprint(draft, { ...selection, sizeGib: "2" }, null, null, null));
});

test("saved previews are distinct from draft previews and retain the selected rule identity", () => {
  const selection = { titleId: "title", episodeId: null, releaseName: "release", sizeGib: "" };
  const saved = ruleSetTestFingerprint(null, selection, null, null, "installed-rule");
  assert.notEqual(saved, ruleSetTestFingerprint(draft, selection, null, null, null));
  assert.notEqual(saved, ruleSetTestFingerprint(null, selection, null, null, "other-rule"));
});

test("saved preview requests contain only the installed rule identity and selection", () => {
  const input = buildRuleSetTestInput({
    draft,
    editRuleSetId: "edit-rule",
    copySourceRuleSetId: "copy-source",
    testRuleSetId: "installed-rule",
    titleId: "title",
    episodeId: "episode",
    releaseName: "release",
    sizeBytes: 42,
  });
  assert.deepEqual(input, {
    testRuleSetId: "installed-rule",
    titleId: "title",
    episodeId: "episode",
    releaseName: "release",
    sizeBytes: 42,
  });
});

test("delayed preview response is discarded after committed inputs change", async () => {
  let release!: () => void;
  const delayed = new Promise<void>((resolve) => {
    release = resolve;
  });
  const requestFingerprint = "before";
  let committedFingerprint = requestFingerprint;
  const response = delayed.then(() =>
    shouldApplyRuleSetTestResponse(1, 1, requestFingerprint, committedFingerprint),
  );
  committedFingerprint = "after";
  release();
  assert.equal(await response, false);
});

test("request controller blocks duplicate submission and releases its owner", async () => {
  let release!: () => void;
  const delayed = new Promise<void>((resolve) => {
    release = resolve;
  });
  const controller = new RuleSetTestRequestController();
  const request = controller.begin();
  assert.equal(request, 1);
  assert.equal(controller.begin(), null);
  const completion = delayed.then(() => controller.finish(request!));
  release();
  assert.equal(await completion, true);
  assert.equal(controller.begin(), 2);
});

test("disposed preview controller ignores delayed completion", async () => {
  let release!: () => void;
  const delayed = new Promise<void>((resolve) => {
    release = resolve;
  });
  const controller = new RuleSetTestRequestController();
  const request = controller.begin();
  controller.dispose();
  const completion = delayed.then(() => controller.finish(request!));
  release();
  assert.equal(await completion, false);
});

test("controller reactivation permits a new request without reviving the old one", () => {
  const controller = new RuleSetTestRequestController();
  controller.activate();
  const oldRequest = controller.begin();
  controller.dispose();
  controller.activate();
  const newRequest = controller.begin();
  assert.notEqual(oldRequest, newRequest);
  assert.equal(controller.isCurrent(oldRequest!), false);
  assert.equal(controller.isCurrent(newRequest!), true);
});

test("size conversion preserves unknown values and rejects unsafe bytes", () => {
  assert.deepEqual(sizeBytesFromGib(""), { value: undefined });
  assert.deepEqual(sizeBytesFromGib("1"), { value: 1024 ** 3 });
  assert.equal("error" in sizeBytesFromGib("1e20"), true);
});

test("preview operation asks for structured evaluation errors", () => {
  assert.match(testRuleSetMutation, /errors \{\s+code\s+message\s+ruleSetId\s+\}/);
  assert.match(testRuleSetMutation, /entries \{\s+code\s+delta\s+blocked\s+kind\s+\}/);
  assert.match(testRuleSetMutation, /releaseGroup[\s\S]*videoCodec[\s\S]*audioLanguages/);
});
