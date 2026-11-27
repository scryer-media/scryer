import assert from "node:assert/strict";
import test from "node:test";

import {
  SMG_SCRYER_UPDATE_SNOOZE_MS,
  parseSmgScryerUpdateDismissal,
  smgScryerUpdateSnoozeRemainingMs,
} from "./smg-update-snooze.ts";

const NOW = 1_800_000_000_000;
const stored = (dismissedAt: number, value = "v1.2.3:1.2.3") =>
  JSON.stringify({ value, dismissedAt });

test("a fresh dismissal hides the reminder", () => {
  assert.deepEqual(parseSmgScryerUpdateDismissal(stored(NOW), NOW), {
    value: "v1.2.3:1.2.3",
    dismissedAt: NOW,
  });
});

test("the reminder stays hidden until the snooze has fully run", () => {
  const lastHiddenMoment = NOW + SMG_SCRYER_UPDATE_SNOOZE_MS - 1;
  assert.ok(parseSmgScryerUpdateDismissal(stored(NOW), lastHiddenMoment));
  assert.equal(
    parseSmgScryerUpdateDismissal(
      stored(NOW),
      NOW + SMG_SCRYER_UPDATE_SNOOZE_MS,
    ),
    null,
  );
});

test("a dismissal stored before the snooze existed no longer hides the reminder", () => {
  assert.equal(parseSmgScryerUpdateDismissal("v1.2.3:1.2.3", NOW), null);
});

test("an unreadable or incomplete dismissal does not hide the reminder", () => {
  for (const raw of [
    null,
    "",
    "{",
    "null",
    "42",
    JSON.stringify({ value: "v1.2.3:1.2.3" }),
    JSON.stringify({ dismissedAt: NOW }),
    JSON.stringify({ value: 7, dismissedAt: NOW }),
    JSON.stringify({ value: "v1.2.3:1.2.3", dismissedAt: "yesterday" }),
  ]) {
    assert.equal(parseSmgScryerUpdateDismissal(raw, NOW), null, String(raw));
  }
});

test("a dismissal dated in the future does not hide the reminder", () => {
  assert.equal(parseSmgScryerUpdateDismissal(stored(NOW + 60_000), NOW), null);
});

test("the remaining time counts down to zero and never goes negative", () => {
  const dismissal = { value: "v1.2.3:1.2.3", dismissedAt: NOW };
  assert.equal(
    smgScryerUpdateSnoozeRemainingMs(dismissal, NOW),
    SMG_SCRYER_UPDATE_SNOOZE_MS,
  );
  assert.equal(
    smgScryerUpdateSnoozeRemainingMs(dismissal, NOW + 60_000),
    SMG_SCRYER_UPDATE_SNOOZE_MS - 60_000,
  );
  assert.equal(
    smgScryerUpdateSnoozeRemainingMs(
      dismissal,
      NOW + SMG_SCRYER_UPDATE_SNOOZE_MS * 3,
    ),
    0,
  );
});
