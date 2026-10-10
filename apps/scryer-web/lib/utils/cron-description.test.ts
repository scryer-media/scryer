import assert from "node:assert/strict";
import test from "node:test";

import {
  cronstrueLocaleFor,
  describeCronExpression,
  describeScriptSchedule,
  loadCronDescriptionLocale,
} from "./cron-description.ts";

test("cron expressions are described in English by default", () => {
  assert.equal(describeCronExpression("*/15 * * * *", "eng"), "Every 15 minutes");
  assert.equal(
    describeCronExpression("30 4 * * 1", null, { use24HourTimeFormat: true }),
    "At 04:30, only on Monday",
  );
});

test("an interface language is described in its own locale once loaded", async () => {
  assert.equal(describeCronExpression("*/15 * * * *", "deu"), "Every 15 minutes");
  await loadCronDescriptionLocale("deu");
  assert.equal(describeCronExpression("*/15 * * * *", "deu"), "Alle 15 Minuten");
});

test("languages cronstrue does not translate fall back to English", () => {
  assert.equal(cronstrueLocaleFor("zh-HK"), "zh_TW");
  assert.equal(cronstrueLocaleFor("xyz"), "en");
  assert.equal(cronstrueLocaleFor(undefined), "en");
});

test("an unreadable or empty expression has no description", () => {
  assert.equal(describeCronExpression("not a cron", "eng"), null);
  assert.equal(describeCronExpression("   ", "eng"), null);
  assert.equal(describeCronExpression("0 0 3 * * *", "eng"), null);
  assert.equal(describeCronExpression("0 3 * *", "eng"), null);
});

test("a cron schedule is described locally and other kinds keep the server text", () => {
  assert.equal(
    describeScriptSchedule({ kind: "CRON", expression: "*/15 * * * *" }, "*/15 * * * *", "eng"),
    "Every 15 minutes",
  );
  assert.equal(
    describeScriptSchedule({ kind: "CRON", expression: "0 0 3 * * *" }, "0 0 3 * * *", "eng"),
    "0 0 3 * * *",
  );
  assert.equal(
    describeScriptSchedule({ kind: "DAILY", expression: null }, "Every day at 03:00", "eng"),
    "Every day at 03:00",
  );
  assert.equal(describeScriptSchedule(null, null, "eng"), null);
});
