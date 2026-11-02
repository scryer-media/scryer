import assert from "node:assert/strict";
import test from "node:test";
import {
  initialSetupIntroPhase,
  setupIntroPhaseForTarget,
} from "./setup-intro-phase.ts";

test("only the setup route opens on the welcome logo", () => {
  assert.equal(initialSetupIntroPhase("/setup", true), "playing");
  assert.equal(initialSetupIntroPhase("/dashboard", true), "waiting");
  assert.equal(initialSetupIntroPhase("/", true), "waiting");
});

test("a load that may not animate never plays the welcome", () => {
  assert.equal(initialSetupIntroPhase("/setup", false), "done");
  assert.equal(initialSetupIntroPhase("/dashboard", false), "done");
});

test("a page refresh ends the wait without showing the logo", () => {
  assert.equal(setupIntroPhaseForTarget("waiting", false), "done");
});

test("a first-run screen outside setup still gets the welcome", () => {
  assert.equal(setupIntroPhaseForTarget("waiting", true), "playing");
});

test("a later target never restarts a settled welcome", () => {
  assert.equal(setupIntroPhaseForTarget("playing", false), "playing");
  assert.equal(setupIntroPhaseForTarget("done", true), "done");
});
